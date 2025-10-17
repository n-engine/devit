use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use devit_cli::core::{
    file_ops::{FileEntry, FileType, SearchMatch, TreeNode},
    formats::OutputFormat,
    fs::FsService,
};
use mcp_core::{McpResult, McpTool};
use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::errors::{internal_error, io_error, validation_error};
use crate::file_read::FileSystemContext;

const MAX_LISTING_ENTRIES: usize = 5000;
const MAX_SEARCH_RESULTS: usize = 200;
const DEFAULT_CONTEXT_LINES: usize = 2;
const MAX_STRUCTURE_DEPTH: usize = 8;

#[derive(Clone)]
pub struct FileExplorer {
    fs: Arc<FileSystemContext>,
    service: Arc<FsService>,
}

impl FileExplorer {
    pub fn new(fs: Arc<FileSystemContext>) -> McpResult<Self> {
        let service = FsService::new(fs.root().to_path_buf())
            .map_err(|err| internal_error(format!("FsService init failed: {err}")))?;
        Ok(Self {
            fs,
            service: Arc::new(service),
        })
    }

    fn relative_from_root(&self, canonical: &Path) -> PathBuf {
        match canonical.strip_prefix(self.fs.root()) {
            Ok(stripped) if stripped.as_os_str().is_empty() => PathBuf::from("."),
            Ok(stripped) => stripped.to_path_buf(),
            Err(_) => canonical.to_path_buf(),
        }
    }

    pub(crate) async fn resolve(&self, raw: &str) -> McpResult<PathBuf> {
        self.fs.resolve_path(raw)
    }

    pub(crate) async fn list_entries(
        &self,
        raw_path: &str,
        recursive: bool,
    ) -> McpResult<Vec<FileEntryMetadata>> {
        let canonical = self.fs.resolve_path(raw_path)?;
        let relative = self.relative_from_root(&canonical);
        let entries = self
            .service
            .list(&relative, recursive)
            .await
            .map_err(|err| internal_error(err.to_string()))?;
        let mut collected = Vec::new();

        for entry in entries {
            if !self.should_include(entry.path.as_path()) {
                continue;
            }

            collected.push(FileEntryMetadata::from(&entry));
            if collected.len() >= MAX_LISTING_ENTRIES {
                break;
            }
        }

        Ok(collected)
    }

    pub(crate) async fn search_matches(
        &self,
        pattern: &str,
        raw_path: &str,
        context_lines: usize,
        max_results: usize,
    ) -> McpResult<(Vec<SearchMatchMetadata>, bool)> {
        let canonical = self.fs.resolve_path(raw_path)?;
        let relative = self.relative_from_root(&canonical);
        let results = self
            .service
            .search(pattern, &relative, Some(context_lines))
            .await
            .map_err(|err| internal_error(err.to_string()))?;

        let truncated = results.truncated || results.matches.len() > max_results;
        let matches = results
            .matches
            .into_iter()
            .filter(|m| self.should_include(m.file.as_path()))
            .take(max_results)
            .map(|m| SearchMatchMetadata::from(&m))
            .collect();

        Ok((matches, truncated))
    }

    pub(crate) async fn project_tree(&self, raw_path: &str) -> McpResult<ProjectNode> {
        let canonical = self.fs.resolve_path(raw_path)?;
        let relative = self.relative_from_root(&canonical);
        let structure = self
            .service
            .project_structure(&relative, Some(MAX_STRUCTURE_DEPTH as u8))
            .await
            .map_err(|err| internal_error(err.to_string()))?;
        Ok(self.build_node(&structure.tree, 0))
    }

    pub(crate) async fn list_ext(
        &self,
        path: &str,
        format: &OutputFormat,
        fields: Option<&[String]>,
        recursive: Option<bool>,
        include_hidden: Option<bool>,
        include_patterns: Option<&[String]>,
        exclude_patterns: Option<&[String]>,
    ) -> McpResult<String> {
        let canonical = self.fs.resolve_path(path)?;
        let relative = self.relative_from_root(&canonical);
        self.service
            .list_ext(
                &relative,
                format,
                fields,
                recursive,
                include_hidden,
                include_patterns,
                exclude_patterns,
            )
            .await
            .map_err(|err| internal_error(err.to_string()))
    }

    pub(crate) async fn search_ext(
        &self,
        pattern: &str,
        path: &str,
        format: &OutputFormat,
        fields: Option<&[String]>,
        context_lines: Option<u8>,
        file_pattern: Option<&str>,
        max_results: Option<usize>,
    ) -> McpResult<String> {
        let canonical = self.fs.resolve_path(path)?;
        let relative = self.relative_from_root(&canonical);
        self.service
            .search_ext(
                pattern,
                &relative,
                format,
                fields,
                context_lines,
                file_pattern,
                max_results,
            )
            .await
            .map_err(|err| internal_error(err.to_string()))
    }

    pub(crate) async fn project_structure_ext(
        &self,
        path: &str,
        format: &OutputFormat,
        fields: Option<&[String]>,
        max_depth: Option<u8>,
    ) -> McpResult<String> {
        let canonical = self.fs.resolve_path(path)?;
        let relative = self.relative_from_root(&canonical);
        self.service
            .project_structure_ext(&relative, format, fields, max_depth)
            .await
            .map_err(|err| internal_error(err.to_string()))
    }

    fn build_node(&self, node: &TreeNode, depth: usize) -> ProjectNode {
        let kind = match node.node_type {
            FileType::Directory => "directory",
            FileType::Symlink => "symlink",
            _ => "file",
        };

        let mut children_nodes = Vec::new();
        if depth < MAX_STRUCTURE_DEPTH {
            if let Some(children) = &node.children {
                for child in children {
                    if !self.should_include(child.path.as_path()) {
                        continue;
                    }
                    children_nodes.push(self.build_node(child, depth + 1));
                }
            }
        }

        ProjectNode {
            name: node.name.clone(),
            path: node.path.to_string_lossy().to_string(),
            kind,
            children: if children_nodes.is_empty() {
                None
            } else {
                Some(children_nodes)
            },
        }
    }

    fn should_include(&self, path: &Path) -> bool {
        let blacklist = [".git", "target", "node_modules", "__pycache__"];
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if blacklist.contains(&name) {
                return false;
            }
        }
        true
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct FileEntryMetadata {
    name: String,
    path: String,
    kind: &'static str,
    size: Option<u64>,
}

impl From<&FileEntry> for FileEntryMetadata {
    fn from(entry: &FileEntry) -> Self {
        let kind = match &entry.entry_type {
            FileType::Directory => "directory",
            FileType::Symlink => "symlink",
            FileType::File => "file",
        };

        Self {
            name: entry.name.clone(),
            path: entry.path.to_string_lossy().to_string(),
            kind,
            size: entry.size,
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct SearchMatchMetadata {
    file: String,
    line_number: usize,
    line: String,
    context_before: Vec<String>,
    context_after: Vec<String>,
}

impl From<&SearchMatch> for SearchMatchMetadata {
    fn from(entry: &SearchMatch) -> Self {
        Self {
            file: entry.file.to_string_lossy().to_string(),
            line_number: entry.line_number,
            line: entry.line.clone(),
            context_before: entry.context_before.clone(),
            context_after: entry.context_after.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct ProjectNode {
    name: String,
    path: String,
    kind: &'static str,
    children: Option<Vec<ProjectNode>>,
}

pub struct FileListTool {
    explorer: Arc<FileExplorer>,
}

impl FileListTool {
    pub fn new(explorer: Arc<FileExplorer>) -> Self {
        Self { explorer }
    }
}

#[async_trait]
impl McpTool for FileListTool {
    fn name(&self) -> &str {
        "devit_file_list"
    }

    fn description(&self) -> &str {
        "List directory contents with lightweight metadata"
    }

    async fn execute(&self, params: Value) -> McpResult<Value> {
        let path = params
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| validation_error("The 'path' parameter is required."))?;

        let recursive = params
            .get("recursive")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let target = self.explorer.resolve(path).await?;
        if !target.exists() {
            return Err(io_error(
                "list directory",
                Some(&target),
                "Path not found".to_string(),
            ));
        }

        let entries = self.explorer.list_entries(path, recursive).await?;

        let mut summary = format!(
            "Directory listing for '{}' ({} entries, recursive: {})\n\n",
            path,
            entries.len(),
            recursive
        );
        for entry in &entries {
            summary.push_str(&format!("- {} ({})\n", entry.path, entry.kind));
        }

        Ok(json!({
            "content": [{"type": "text", "text": summary}],
            "metadata": {"entries": entries}
        }))
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "recursive": {"type": "boolean"}
            },
            "required": ["path"]
        })
    }
}

pub struct FileSearchTool {
    explorer: Arc<FileExplorer>,
}

impl FileSearchTool {
    pub fn new(explorer: Arc<FileExplorer>) -> Self {
        Self { explorer }
    }
}

#[async_trait]
impl McpTool for FileSearchTool {
    fn name(&self) -> &str {
        "devit_file_search"
    }

    fn description(&self) -> &str {
        "Regex search across files with context lines"
    }

    async fn execute(&self, params: Value) -> McpResult<Value> {
        let pattern = params
            .get("pattern")
            .and_then(Value::as_str)
            .ok_or_else(|| validation_error("The 'pattern' parameter is required."))?;

        let path = params.get("path").and_then(Value::as_str).unwrap_or(".");

        let context_lines = params
            .get("context_lines")
            .and_then(Value::as_u64)
            .map(|v| v as usize)
            .unwrap_or(DEFAULT_CONTEXT_LINES);

        let target = self.explorer.resolve(path).await?;
        if !target.exists() {
            return Err(io_error(
                "search files",
                Some(&target),
                "Path not found".to_string(),
            ));
        }

        let (matches, truncated) = self
            .explorer
            .search_matches(pattern, path, context_lines, MAX_SEARCH_RESULTS)
            .await?;

        let mut summary = format!(
            "Search '{}' under '{}' → {} matches (limit {})\n\n",
            pattern,
            path,
            matches.len(),
            MAX_SEARCH_RESULTS
        );
        for m in &matches {
            summary.push_str(&format!("{}:{} → {}\n", m.file, m.line_number, m.line));
        }

        Ok(json!({
            "content": [{"type": "text", "text": summary}],
            "metadata": {
                "matches": matches,
                "pattern": pattern,
                "path": target.to_string_lossy(),
                "truncated": truncated
            }
        }))
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string"},
                "path": {"type": "string"},
                "context_lines": {"type": "integer", "minimum": 0}
            },
            "required": ["pattern"]
        })
    }
}

pub struct ProjectStructureTool {
    explorer: Arc<FileExplorer>,
}

impl ProjectStructureTool {
    pub fn new(explorer: Arc<FileExplorer>) -> Self {
        Self { explorer }
    }
}

#[async_trait]
impl McpTool for ProjectStructureTool {
    fn name(&self) -> &str {
        "devit_project_structure"
    }

    fn description(&self) -> &str {
        "Generate a lightweight project tree"
    }

    async fn execute(&self, params: Value) -> McpResult<Value> {
        let path = params.get("path").and_then(Value::as_str).unwrap_or(".");

        let target = self.explorer.resolve(path).await?;
        if !target.exists() {
            return Err(io_error(
                "project structure",
                Some(&target),
                "Path not found".to_string(),
            ));
        }

        let tree = self.explorer.project_tree(path).await?;
        let mut summary = String::new();
        render_tree(&mut summary, &tree, 0);

        Ok(json!({
            "content": [{"type": "text", "text": summary}],
            "metadata": {"root": tree}
        }))
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"}
            }
        })
    }
}

fn render_tree(buffer: &mut String, node: &ProjectNode, depth: usize) {
    let indent = "  ".repeat(depth);
    buffer.push_str(&format!("{}- {} ({})\n", indent, node.name, node.kind));

    if let Some(children) = &node.children {
        for child in children {
            render_tree(buffer, child, depth + 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;
    use tempfile::tempdir;

    fn build_explorer() -> (Arc<FileExplorer>, tempfile::TempDir) {
        let tmp = tempdir().expect("tempdir");
        let ctx = Arc::new(FileSystemContext::new(tmp.path().to_path_buf()).expect("context"));
        let explorer = Arc::new(FileExplorer::new(Arc::clone(&ctx)).expect("explorer"));
        (explorer, tmp)
    }

    #[tokio::test]
    async fn search_tool_returns_matches() {
        let (explorer, tmp) = build_explorer();
        let file = tmp.path().join("sample.txt");
        std::fs::write(&file, "alpha\nbeta\nalpha beta\n").expect("seed file");

        let tool = FileSearchTool::new(explorer);
        let params = json!({"pattern": "alpha", "path": ".", "context_lines": 1});

        let response = tool.execute(params).await.expect("search success");
        let matches = response["metadata"]["matches"]
            .as_array()
            .expect("matches array");
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0]["line_number"].as_u64().unwrap(), 1);
    }

    #[tokio::test]
    async fn project_structure_tool_lists_tree() {
        let (explorer, tmp) = build_explorer();
        let nested = tmp.path().join("src");
        std::fs::create_dir(&nested).expect("dir");
        std::fs::write(nested.join("main.rs"), "fn main() {}\n").expect("file");

        let tool = ProjectStructureTool::new(explorer);
        let response = tool.execute(json!({"path": "."})).await.expect("structure");
        let text = response["content"][0]["text"]
            .as_str()
            .expect("text output");
        assert!(text.contains("src"));
        assert!(text.contains("main.rs"));
    }
}

pub struct FileListExtTool {
    explorer: Arc<FileExplorer>,
}

impl FileListExtTool {
    pub fn new(explorer: Arc<FileExplorer>) -> Self {
        Self { explorer }
    }
}

#[async_trait]
impl McpTool for FileListExtTool {
    fn name(&self) -> &str {
        "devit_file_list_ext"
    }

    fn description(&self) -> &str {
        "Token-optimized directory listing with field selection"
    }

    async fn execute(&self, params: Value) -> McpResult<Value> {
        let path = params
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| validation_error("The 'path' parameter is required."))?;

        let recursive = params
            .get("recursive")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let include_hidden = params
            .get("include_hidden")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let include_patterns = params
            .get("include_patterns")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
            });

        let exclude_patterns = params
            .get("exclude_patterns")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
            });

        let format = params
            .get("format")
            .and_then(Value::as_str)
            .unwrap_or("json");

        let output_format = match format {
            "json" => OutputFormat::Json,
            "text" | "table" => OutputFormat::Table,
            "compact" => OutputFormat::Compact,
            other => {
                return Err(validation_error(&format!(
                    "Invalid format '{}'. Use 'json', 'text', 'table', or 'compact'.",
                    other
                )));
            }
        };

        let fields = params.get("fields").and_then(Value::as_array).map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        });

        let target = self.explorer.resolve(path).await?;
        if !target.exists() {
            return Err(io_error(
                "file_list_ext",
                Some(&target),
                "Path not found".to_string(),
            ));
        }

        let rendered = self
            .explorer
            .list_ext(
                path,
                &output_format,
                fields.as_deref(),
                Some(recursive),
                Some(include_hidden),
                include_patterns.as_deref(),
                exclude_patterns.as_deref(),
            )
            .await?;

        let entries_json = match output_format {
            OutputFormat::Json | OutputFormat::Compact => {
                serde_json::from_str::<Value>(&rendered).ok()
            }
            _ => None,
        };

        let mut metadata = Map::new();
        metadata.insert("path".into(), json!(target.to_string_lossy()));
        metadata.insert("recursive".into(), json!(recursive));
        metadata.insert("format".into(), json!(format));
        metadata.insert("fields".into(), json!(fields));
        metadata.insert("include_hidden".into(), json!(include_hidden));
        metadata.insert("include_patterns".into(), json!(include_patterns));
        metadata.insert("exclude_patterns".into(), json!(exclude_patterns));
        metadata.insert("entries".into(), entries_json.unwrap_or(Value::Null));

        let format_label = match output_format {
            OutputFormat::Json => "Json",
            OutputFormat::Compact => "Compact",
            OutputFormat::Table => "Table",
            OutputFormat::MessagePack => "MessagePack",
        };
        let code_fence = match output_format {
            OutputFormat::Table => "table",
            _ => "json",
        };
        let header = format!(
            "📂 Directory: {} (format: {})",
            target.to_string_lossy(),
            format_label
        );
        let text_output = format!("{header}\n\n```{code_fence}\n{rendered}\n```");

        Ok(json!({
            "content": [{"type": "text", "text": text_output}],
            "metadata": Value::Object(metadata)
        }))
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "recursive": {"type": "boolean"},
                "format": {"type": "string", "enum": ["json", "text", "table", "compact"]},
                "fields": {"type": "array", "items": {"type": "string"}},
                "include_hidden": {"type": "boolean"},
                "include_patterns": {"type": "array", "items": {"type": "string"}},
                "exclude_patterns": {"type": "array", "items": {"type": "string"}}
            },
            "required": ["path"]
        })
    }
}

pub struct FileSearchExtTool {
    explorer: Arc<FileExplorer>,
}

impl FileSearchExtTool {
    pub fn new(explorer: Arc<FileExplorer>) -> Self {
        Self { explorer }
    }
}

#[async_trait]
impl McpTool for FileSearchExtTool {
    fn name(&self) -> &str {
        "devit_file_search_ext"
    }

    fn description(&self) -> &str {
        "Regex search with adjustable limits and condensed output"
    }

    async fn execute(&self, params: Value) -> McpResult<Value> {
        let pattern = params
            .get("pattern")
            .and_then(Value::as_str)
            .ok_or_else(|| validation_error("The 'pattern' parameter is required."))?;

        let path = params.get("path").and_then(Value::as_str).unwrap_or(".");

        let context_lines = params
            .get("context_lines")
            .and_then(Value::as_u64)
            .map(|v| v as usize)
            .unwrap_or(DEFAULT_CONTEXT_LINES);

        let max_results = params
            .get("max_results")
            .and_then(Value::as_u64)
            .map(|v| v as usize)
            .unwrap_or(MAX_SEARCH_RESULTS);

        let format = params
            .get("format")
            .and_then(Value::as_str)
            .unwrap_or("json");

        let output_format = match format {
            "json" => OutputFormat::Json,
            "text" | "table" => OutputFormat::Table,
            "compact" => OutputFormat::Compact,
            other => {
                return Err(validation_error(&format!(
                    "Invalid format '{}'. Use 'json', 'text', 'table', or 'compact'.",
                    other
                )));
            }
        };

        let fields = params.get("fields").and_then(Value::as_array).map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        });

        let file_pattern = params
            .get("file_pattern")
            .and_then(Value::as_str)
            .map(|s| s.to_string());

        let target = self.explorer.resolve(path).await?;
        if !target.exists() {
            return Err(io_error(
                "file_search_ext",
                Some(&target),
                "Path not found".to_string(),
            ));
        }

        let (matches, truncated) = self
            .explorer
            .search_matches(pattern, path, context_lines, max_results)
            .await?;

        let context_lines_u8 = context_lines.min(u8::MAX as usize) as u8;
        let rendered = self
            .explorer
            .search_ext(
                pattern,
                path,
                &output_format,
                fields.as_deref(),
                Some(context_lines_u8),
                file_pattern.as_deref(),
                Some(max_results),
            )
            .await?;

        let mut metadata = Map::new();
        metadata.insert(
            "matches".into(),
            serde_json::to_value(&matches).unwrap_or(Value::Null),
        );
        metadata.insert("pattern".into(), json!(pattern));
        metadata.insert("path".into(), json!(target.to_string_lossy()));
        metadata.insert("context_lines".into(), json!(context_lines));
        metadata.insert("limit".into(), json!(max_results));
        metadata.insert("format".into(), json!(format));
        metadata.insert("fields".into(), json!(fields));
        metadata.insert("file_pattern".into(), json!(file_pattern));
        metadata.insert("truncated".into(), json!(truncated));

        let format_label = match output_format {
            OutputFormat::Json => "Json",
            OutputFormat::Compact => "Compact",
            OutputFormat::Table => "Table",
            OutputFormat::MessagePack => "MessagePack",
        };
        let code_fence = match output_format {
            OutputFormat::Table => "table",
            _ => "json",
        };
        let header = format!(
            "🔍 Search: '{}' in {} (format: {})",
            pattern,
            target.to_string_lossy(),
            format_label
        );
        let text_output = format!("{header}\n\n```{code_fence}\n{rendered}\n```");

        Ok(json!({
            "content": [{"type": "text", "text": text_output}],
            "metadata": Value::Object(metadata)
        }))
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string"},
                "path": {"type": "string"},
                "context_lines": {"type": "integer", "minimum": 0},
                "max_results": {"type": "integer", "minimum": 1},
                "format": {"type": "string", "enum": ["json", "text", "table", "compact"]},
                "fields": {"type": "array", "items": {"type": "string"}},
                "file_pattern": {"type": "string"}
            },
            "required": ["pattern"]
        })
    }
}

pub struct ProjectStructureExtTool {
    explorer: Arc<FileExplorer>,
}

impl ProjectStructureExtTool {
    pub fn new(explorer: Arc<FileExplorer>) -> Self {
        Self { explorer }
    }
}

#[async_trait]
impl McpTool for ProjectStructureExtTool {
    fn name(&self) -> &str {
        "devit_project_structure_ext"
    }

    fn description(&self) -> &str {
        "Compressed project structure tree"
    }

    async fn execute(&self, params: Value) -> McpResult<Value> {
        let path = params.get("path").and_then(Value::as_str).unwrap_or(".");

        let format = params
            .get("format")
            .and_then(Value::as_str)
            .unwrap_or("json");

        let output_format = match format {
            "json" => OutputFormat::Json,
            "text" | "table" => OutputFormat::Table,
            "compact" => OutputFormat::Compact,
            other => {
                return Err(validation_error(&format!(
                    "Invalid format '{}'. Use 'json', 'text', 'table', or 'compact'.",
                    other
                )));
            }
        };

        let fields = params.get("fields").and_then(Value::as_array).map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        });

        let max_depth = params
            .get("max_depth")
            .and_then(Value::as_u64)
            .map(|v| v.min(MAX_STRUCTURE_DEPTH as u64) as u8);

        let target = self.explorer.resolve(path).await?;
        if !target.exists() {
            return Err(io_error(
                "project_structure_ext",
                Some(&target),
                "Path not found".to_string(),
            ));
        }

        let rendered = self
            .explorer
            .project_structure_ext(path, &output_format, fields.as_deref(), max_depth)
            .await?;

        let structure_json = match output_format {
            OutputFormat::Json | OutputFormat::Compact => {
                serde_json::from_str::<Value>(&rendered).ok()
            }
            _ => None,
        };

        let mut metadata = Map::new();
        metadata.insert("path".into(), json!(target.to_string_lossy()));
        metadata.insert("format".into(), json!(format));
        metadata.insert("fields".into(), json!(fields));
        metadata.insert("max_depth".into(), json!(max_depth));
        metadata.insert("structure".into(), structure_json.unwrap_or(Value::Null));

        let format_label = match output_format {
            OutputFormat::Json => "Json",
            OutputFormat::Compact => "Compact",
            OutputFormat::Table => "Table",
            OutputFormat::MessagePack => "MessagePack",
        };
        let code_fence = match output_format {
            OutputFormat::Table => "table",
            _ => "json",
        };
        let header = format!(
            "📁 Project structure: {} (format: {})",
            target.to_string_lossy(),
            format_label
        );
        let text_output = format!("{header}\n\n```{code_fence}\n{rendered}\n```");

        Ok(json!({
            "content": [{"type": "text", "text": text_output}],
            "metadata": Value::Object(metadata)
        }))
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "format": {"type": "string", "enum": ["json", "text", "table", "compact"]},
                "fields": {"type": "array", "items": {"type": "string"}},
                "max_depth": {"type": "integer", "minimum": 1}
            }
        })
    }
}
