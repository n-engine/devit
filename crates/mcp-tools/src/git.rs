use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use async_trait::async_trait;
use mcp_core::{McpResult, McpTool};
use serde_json::{json, Value};

use crate::errors::{internal_error, validation_error};
use crate::file_read::FileSystemContext;

const DEFAULT_LOG_LIMIT: u64 = 20;
const MAX_LOG_LIMIT: u64 = 200;

fn run_git_command(root: &Path, args: &[String]) -> McpResult<std::process::Output> {
    let printable = format!("git {}", args.join(" "));

    Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|err| internal_error(format!("{printable} (spawn failed): {err}")))
}

fn repo_relative(context: &FileSystemContext, raw: &str) -> McpResult<(PathBuf, String)> {
    let resolved = context.resolve_path(raw)?;
    let relative = resolved
        .strip_prefix(context.root())
        .map_err(|_| internal_error("Path resolution failed (strip_prefix)"))?;

    let relative_string = relative.to_string_lossy().replace('\\', "/");

    Ok((relative.to_path_buf(), relative_string))
}

fn stringify_output(stdout: &[u8]) -> String {
    let text = String::from_utf8_lossy(stdout).to_string();
    if text.trim().is_empty() {
        "(no results)".to_string()
    } else {
        text
    }
}

fn log_limit(params: &Value) -> McpResult<u64> {
    let value = params
        .get("max_count")
        .and_then(Value::as_i64)
        .unwrap_or(DEFAULT_LOG_LIMIT as i64);

    if value <= 0 {
        return Err(validation_error("'max_count' doit être supérieur à 0"));
    }

    let value = value as u64;
    Ok(value.min(MAX_LOG_LIMIT))
}

fn json_text_response(text: String) -> Value {
    json!({
        "content": [{
            "type": "text",
            "text": text
        }]
    })
}

pub struct GitLogTool {
    context: Arc<FileSystemContext>,
}

impl GitLogTool {
    pub fn new(context: Arc<FileSystemContext>) -> Self {
        Self { context }
    }
}

#[async_trait]
impl McpTool for GitLogTool {
    fn name(&self) -> &str {
        "devit_git_log"
    }

    fn description(&self) -> &str {
        "Afficher l'historique git (git log --oneline)"
    }

    async fn execute(&self, params: Value) -> McpResult<Value> {
        let max_count = log_limit(&params)?;
        let mut args = vec![
            "log".to_string(),
            "--oneline".to_string(),
            format!("-n{max_count}"),
        ];

        if let Some(path) = params.get("path").and_then(Value::as_str) {
            let (_, relative) = repo_relative(&self.context, path)?;
            args.push("--".to_string());
            args.push(relative);
        }

        let output = run_git_command(self.context.root(), &args)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(internal_error(format!("git log failed: {}", stderr.trim())));
        }

        Ok(json_text_response(stringify_output(&output.stdout)))
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "max_count": {"type": "integer", "minimum": 1, "maximum": MAX_LOG_LIMIT}
            }
        })
    }
}

pub struct GitBlameTool {
    context: Arc<FileSystemContext>,
}

impl GitBlameTool {
    pub fn new(context: Arc<FileSystemContext>) -> Self {
        Self { context }
    }
}

#[async_trait]
impl McpTool for GitBlameTool {
    fn name(&self) -> &str {
        "devit_git_blame"
    }

    fn description(&self) -> &str {
        "Afficher git blame pour un fichier (optionnellement plage de lignes)"
    }

    async fn execute(&self, params: Value) -> McpResult<Value> {
        let path = params
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| validation_error("'path' est requis"))?;

        let (_, relative) = repo_relative(&self.context, path)?;
        let mut args = vec!["blame".to_string(), "--line-porcelain".to_string()];

        if let Some(line_start) = params.get("line_start").and_then(Value::as_i64) {
            let line_start = if line_start < 1 {
                return Err(validation_error("'line_start' doit être >= 1"));
            } else {
                line_start as u64
            };

            let line_end = params
                .get("line_end")
                .and_then(Value::as_i64)
                .map(|end| {
                    if end < 1 {
                        Err(validation_error("'line_end' doit être >= 1"))
                    } else if (end as u64) < line_start {
                        Err(validation_error("'line_end' doit être >= line_start"))
                    } else {
                        Ok(end as u64)
                    }
                })
                .transpose()?;

            let range = match line_end {
                Some(end) => format!("-L{},{}", line_start, end),
                None => format!("-L{},{}", line_start, line_start),
            };
            args.push(range);
        }

        args.push(relative);

        let output = run_git_command(self.context.root(), &args)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(internal_error(format!(
                "git blame failed: {}",
                stderr.trim()
            )));
        }

        Ok(json_text_response(stringify_output(&output.stdout)))
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "line_start": {"type": "integer", "minimum": 1},
                "line_end": {"type": "integer", "minimum": 1}
            },
            "required": ["path"]
        })
    }
}

pub struct GitShowTool {
    context: Arc<FileSystemContext>,
}

impl GitShowTool {
    pub fn new(context: Arc<FileSystemContext>) -> Self {
        Self { context }
    }
}

#[async_trait]
impl McpTool for GitShowTool {
    fn name(&self) -> &str {
        "devit_git_show"
    }

    fn description(&self) -> &str {
        "Afficher le contenu d'un commit (git show)"
    }

    async fn execute(&self, params: Value) -> McpResult<Value> {
        let commit = params
            .get("commit")
            .and_then(Value::as_str)
            .ok_or_else(|| validation_error("'commit' est requis"))?;

        let mut args = vec!["show".to_string()];

        if let Some(path) = params.get("path").and_then(Value::as_str) {
            let (_, relative) = repo_relative(&self.context, path)?;
            args.push(format!("{}:{}", commit, relative));
        } else {
            args.push(commit.to_string());
        }

        let output = run_git_command(self.context.root(), &args)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(internal_error(format!(
                "git show failed: {}",
                stderr.trim()
            )));
        }

        Ok(json_text_response(stringify_output(&output.stdout)))
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "commit": {"type": "string"},
                "path": {"type": "string"}
            },
            "required": ["commit"]
        })
    }
}

pub struct GitDiffTool {
    context: Arc<FileSystemContext>,
}

impl GitDiffTool {
    pub fn new(context: Arc<FileSystemContext>) -> Self {
        Self { context }
    }
}

#[async_trait]
impl McpTool for GitDiffTool {
    fn name(&self) -> &str {
        "devit_git_diff"
    }

    fn description(&self) -> &str {
        "Afficher un diff git (range optionnel)"
    }

    async fn execute(&self, params: Value) -> McpResult<Value> {
        let mut args = vec!["diff".to_string()];

        if let Some(range) = params.get("range").and_then(Value::as_str) {
            if range.trim().is_empty() {
                return Err(validation_error("'range' ne peut pas être vide"));
            }
            args.push(range.to_string());
        }

        if let Some(path) = params.get("path").and_then(Value::as_str) {
            let (_, relative) = repo_relative(&self.context, path)?;
            args.push("--".to_string());
            args.push(relative);
        }

        let output = run_git_command(self.context.root(), &args)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(internal_error(format!(
                "git diff failed: {}",
                stderr.trim()
            )));
        }

        Ok(json_text_response(stringify_output(&output.stdout)))
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "range": {"type": "string"},
                "path": {"type": "string"}
            }
        })
    }
}

pub struct GitSearchTool {
    context: Arc<FileSystemContext>,
}

impl GitSearchTool {
    pub fn new(context: Arc<FileSystemContext>) -> Self {
        Self { context }
    }
}

#[async_trait]
impl McpTool for GitSearchTool {
    fn name(&self) -> &str {
        "devit_git_search"
    }

    fn description(&self) -> &str {
        "Rechercher via git grep ou git log -S"
    }

    async fn execute(&self, params: Value) -> McpResult<Value> {
        let pattern = params
            .get("pattern")
            .and_then(Value::as_str)
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
            .ok_or_else(|| validation_error("'pattern' est requis"))?;

        let mode = params
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("grep")
            .to_ascii_lowercase();

        let (mut args, treat_code_one_as_empty) = match mode.as_str() {
            "grep" => (
                vec!["grep".to_string(), "-n".to_string(), pattern.to_string()],
                true,
            ),
            "log" => {
                let max_count = log_limit(&params)?;
                (
                    vec![
                        "log".to_string(),
                        "--oneline".to_string(),
                        format!("-n{max_count}"),
                        "-S".to_string(),
                        pattern.to_string(),
                    ],
                    false,
                )
            }
            other => {
                let message = format!("'type' non supporté: {} (attendu 'grep' ou 'log')", other);
                return Err(validation_error(&message));
            }
        };

        if let Some(path) = params.get("path").and_then(Value::as_str) {
            let (_, relative) = repo_relative(&self.context, path)?;
            args.push("--".to_string());
            args.push(relative);
        }

        let output = run_git_command(self.context.root(), &args)?;

        if output.status.success() {
            return Ok(json_text_response(stringify_output(&output.stdout)));
        }

        if treat_code_one_as_empty && output.status.code() == Some(1) {
            return Ok(json_text_response("(no matches)".to_string()));
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(internal_error(format!(
            "git search failed: {}",
            stderr.trim()
        )))
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string"},
                "type": {"type": "string", "enum": ["grep", "log"], "default": "grep"},
                "max_count": {"type": "integer", "minimum": 1, "maximum": MAX_LOG_LIMIT},
                "path": {"type": "string"}
            },
            "required": ["pattern"]
        })
    }
}
