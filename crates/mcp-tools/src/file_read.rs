use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use devit_cli::core::{
    file_ops::FileContent as CoreFileContent,
    formats::{Compressible, OutputFormat},
    fs::FsService,
};
use mcp_core::{McpResult, McpTool};
use serde_json::{json, Map, Number, Value};

use crate::errors::{
    internal_error, invalid_diff_error, io_error, policy_block_error, validation_error,
};

const MAX_FILE_SIZE: u64 = 1024 * 1024; // 1 MB

#[derive(Clone, Copy)]
enum FileReadMode {
    Basic,
    Extended,
}

pub struct FileReadTool {
    context: Arc<FileSystemContext>,
    mode: FileReadMode,
}

impl FileReadTool {
    pub fn new(context: Arc<FileSystemContext>) -> Self {
        Self {
            context,
            mode: FileReadMode::Basic,
        }
    }

    pub fn new_extended(context: Arc<FileSystemContext>) -> Self {
        Self {
            context,
            mode: FileReadMode::Extended,
        }
    }

    async fn render_structured(
        &self,
        path: &str,
        format: &OutputFormat,
        fields: Option<&[String]>,
        line_numbers: bool,
        offset: Option<u64>,
        limit: Option<u64>,
    ) -> McpResult<String> {
        let service = FsService::new(self.context.root().to_path_buf())
            .map_err(|err| internal_error(err.to_string()))?;

        service
            .read_ext(
                path,
                format,
                fields,
                Some(line_numbers),
                offset.map(|value| value as u32),
                limit.map(|value| value as u32),
            )
            .await
            .map_err(|err| internal_error(err.to_string()))
    }
}

#[async_trait]
impl McpTool for FileReadTool {
    fn name(&self) -> &str {
        match self.mode {
            FileReadMode::Basic => "devit_file_read",
            FileReadMode::Extended => "devit_file_read_ext",
        }
    }

    fn description(&self) -> &str {
        match self.mode {
            FileReadMode::Basic => {
                "Read file content with security validation and optional line numbers"
            }
            FileReadMode::Extended => {
                "Read file content with compression, field filtering, and token optimization"
            }
        }
    }

    async fn execute(&self, params: Value) -> McpResult<Value> {
        let path = params
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| validation_error("Le paramètre 'path' est requis"))?;

        let line_numbers = params
            .get("line_numbers")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let offset_raw = params.get("offset").and_then(Value::as_u64);
        let limit_raw = params.get("limit").and_then(Value::as_u64);

        let offset = offset_raw.map(|value| value as usize);
        let limit = limit_raw.map(|value| value as usize);

        let format = params
            .get("format")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "text".to_string());

        let fields: Option<Vec<String>> = params
            .get("fields")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|value| value.as_str().map(|s| s.trim().to_string()))
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
            })
            .and_then(|vec| if vec.is_empty() { None } else { Some(vec) });

        if matches!(format.as_str(), "text" | "plain") && fields.is_some() {
            return Err(validation_error(
                "Le paramètre 'fields' est uniquement supporté pour les formats json, compact et table",
            ));
        }

        let canonical_path = self.context.resolve_path(path)?;
        let file_content = self
            .context
            .read_file(&canonical_path, line_numbers, offset, limit)?;

        let mut metadata = Map::new();
        metadata.insert(
            "path".to_string(),
            Value::String(file_content.path.to_string_lossy().to_string()),
        );
        metadata.insert(
            "size".to_string(),
            Value::Number(Number::from(file_content.size)),
        );
        metadata.insert(
            "encoding".to_string(),
            Value::String(file_content.encoding.clone()),
        );
        metadata.insert("line_numbers".to_string(), Value::Bool(line_numbers));
        metadata.insert(
            "line_count".to_string(),
            Value::Number(Number::from(file_content.content.lines().count() as u64)),
        );
        if let Some(raw) = offset_raw {
            metadata.insert("offset".to_string(), Value::Number(Number::from(raw)));
        }
        if let Some(raw) = limit_raw {
            metadata.insert("limit".to_string(), Value::Number(Number::from(raw)));
        }
        metadata.insert(
            "mode".to_string(),
            Value::String(
                match self.mode {
                    FileReadMode::Basic => "basic",
                    FileReadMode::Extended => "extended",
                }
                .to_string(),
            ),
        );
        if let Some(list) = fields.as_ref() {
            metadata.insert(
                "fields".to_string(),
                Value::Array(list.iter().cloned().map(Value::String).collect()),
            );
        }

        match format.as_str() {
            "text" | "plain" => {
                metadata.insert("format".to_string(), Value::String("text".to_string()));
                let text_output = if line_numbers {
                    file_content
                        .lines
                        .as_ref()
                        .map(|values| values.join("\n"))
                        .unwrap_or_else(|| file_content.content.clone())
                } else {
                    file_content.content.clone()
                };

                Ok(json!({
                    "content": [
                        {
                            "type": "text",
                            "text": text_output
                        }
                    ],
                    "metadata": metadata
                }))
            }
            "json" | "compact" | "table" => {
                let output_format = match format.as_str() {
                    "json" => OutputFormat::Json,
                    "compact" => OutputFormat::Compact,
                    "table" => OutputFormat::Table,
                    _ => unreachable!("handled above"),
                };

                let formatted = self
                    .render_structured(
                        path,
                        &output_format,
                        fields.as_ref().map(|vec| vec.as_slice()),
                        line_numbers,
                        offset_raw,
                        limit_raw,
                    )
                    .await?;

                let compression_ratio = file_content
                    .get_compression_ratio(&output_format)
                    .map_err(|err| internal_error(err.to_string()))?;

                let format_label = match output_format {
                    OutputFormat::Json => "Json",
                    OutputFormat::Compact => "Compact",
                    OutputFormat::Table => "Table",
                    OutputFormat::MessagePack => "MessagePack",
                };

                metadata.insert("format".to_string(), Value::String(format.to_string()));
                if let Some(number) = Number::from_f64(compression_ratio as f64) {
                    metadata.insert("compression_ratio".to_string(), Value::Number(number));
                }

                let code_fence = match output_format {
                    OutputFormat::Table => "table",
                    _ => "json",
                };

                let header = format!(
                    "📄 File: {} (format: {})",
                    file_content.path.to_string_lossy(),
                    format_label
                );

                let text_output = format!("{header}\n\n```{code_fence}\n{formatted}\n```");

                Ok(json!({
                    "content": [
                        {
                            "type": "text",
                            "text": text_output
                        }
                    ],
                    "metadata": metadata
                }))
            }
            other => Err(validation_error(&format!(
                "Format '{}' non supporté. Utilisez text, json, compact ou table.",
                other
            ))),
        }
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "line_numbers": {"type": "boolean"},
                "offset": {"type": "integer", "minimum": 0},
                "limit": {"type": "integer", "minimum": 1},
                "format": {
                    "type": "string",
                    "enum": ["text", "json", "compact", "table"],
                    "description": "Format de sortie (par défaut: text)"
                },
                "fields": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Limiter les champs retournés (formats json/compact/table)"
                }
            },
            "required": ["path"]
        })
    }
}

pub struct FileSystemContext {
    root_path: PathBuf,
}

impl FileSystemContext {
    pub fn new(root_path: PathBuf) -> McpResult<Self> {
        let canonical_root = root_path.canonicalize().map_err(|err| {
            io_error(
                "canonicalize repository root",
                Some(&root_path),
                err.to_string(),
            )
        })?;

        Ok(Self {
            root_path: canonical_root,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root_path
    }

    pub fn resolve_path(&self, raw_path: &str) -> McpResult<PathBuf> {
        let input_path = Path::new(raw_path);

        let path_str = raw_path;

        if input_path.is_absolute() {
            let canonical = input_path
                .canonicalize()
                .map_err(|err| io_error("canonicalize path", Some(input_path), err.to_string()))?;

            if canonical.starts_with(&self.root_path) {
                return Ok(canonical);
            }

            return Err(policy_block_error(
                "path_security_repo_boundary",
                "any",
                "patch",
                format!("Absolute path en dehors du projet: {}", raw_path),
            ));
        }

        if path_str.contains("../") || path_str.contains("..\\") {
            return Err(policy_block_error(
                "path_traversal_protection",
                "any",
                "patch",
                "Path traversal attempt detected",
            ));
        }

        if path_str.contains('\0') {
            return Err(policy_block_error(
                "path_security_null_byte",
                "any",
                "patch",
                "Null byte detected in path",
            ));
        }

        if path_str.len() > 4096 {
            return Err(policy_block_error(
                "path_security_length_limit",
                "any",
                "patch",
                "Path too long",
            ));
        }

        let joined = self.root_path.join(input_path);

        let canonical = if joined.exists() {
            joined
                .canonicalize()
                .map_err(|err| io_error("canonicalize path", Some(&joined), err.to_string()))?
        } else {
            self.manual_resolve(input_path)?
        };

        if !canonical.starts_with(&self.root_path) {
            return Err(policy_block_error(
                "path_security_repo_boundary",
                "any",
                "patch",
                format!(
                    "Path escapes repository: {} -> {}",
                    raw_path,
                    canonical.display()
                ),
            ));
        }

        Ok(canonical)
    }

    pub fn read_file(
        &self,
        canonical_path: &Path,
        line_numbers: bool,
        offset: Option<usize>,
        limit: Option<usize>,
    ) -> McpResult<CoreFileContent> {
        if !canonical_path.exists() {
            return Err(io_error(
                "read file content",
                Some(canonical_path),
                "File not found",
            ));
        }

        if !canonical_path.is_file() {
            return Err(invalid_diff_error("Path is not a file", None));
        }

        let metadata = fs::metadata(canonical_path)
            .map_err(|err| io_error("read file metadata", Some(canonical_path), err.to_string()))?;

        let file_size = metadata.len();
        if file_size > MAX_FILE_SIZE {
            return Err(invalid_diff_error(
                format!(
                    "File too large: {} bytes (max: {} bytes)",
                    file_size, MAX_FILE_SIZE
                ),
                None,
            ));
        }

        let content = fs::read_to_string(canonical_path)
            .map_err(|err| io_error("read file content", Some(canonical_path), err.to_string()))?;

        let filtered_content = if let (Some(offset), Some(limit)) = (offset, limit) {
            let lines: Vec<&str> = content.lines().collect();
            let start = offset.min(lines.len());
            let end = (offset + limit).min(lines.len());
            lines[start..end].join("\n")
        } else {
            content.clone()
        };

        let lines = if line_numbers {
            Some(
                filtered_content
                    .lines()
                    .enumerate()
                    .map(|(index, line)| format!("{:4}: {}", index + 1, line))
                    .collect(),
            )
        } else {
            None
        };

        let encoding = detect_encoding(&filtered_content);

        Ok(CoreFileContent {
            path: canonical_path.to_path_buf(),
            content: filtered_content,
            size: file_size,
            lines,
            encoding,
        })
    }

    fn manual_resolve(&self, target: &Path) -> McpResult<PathBuf> {
        let mut resolved = self.root_path.clone();

        for component in target.components() {
            match component {
                Component::Normal(name) => {
                    resolved.push(name);
                }
                Component::ParentDir => {
                    if !resolved.pop() || !resolved.starts_with(&self.root_path) {
                        return Err(policy_block_error(
                            "path_resolution_escape",
                            "any",
                            "patch",
                            "Path resolution would escape repository",
                        ));
                    }
                }
                Component::CurDir | Component::RootDir | Component::Prefix(_) => {
                    // Skip these components
                }
            }
        }

        Ok(resolved)
    }
}

fn detect_encoding(content: &str) -> String {
    if content.bytes().take(1000).any(|byte| byte > 127) {
        if content.starts_with('\u{FEFF}') {
            "utf-8-bom".to_string()
        } else {
            "utf-8".to_string()
        }
    } else {
        "utf-8".to_string()
    }
}
