use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use mcp_core::{McpResult, McpTool};
use serde_json::{json, Value};

use crate::errors::{internal_error, validation_error};
use crate::file_read::FileSystemContext;

pub struct DirectoryListTool {
    context: Arc<FileSystemContext>,
}

impl DirectoryListTool {
    pub fn new(context: Arc<FileSystemContext>) -> Self {
        Self { context }
    }
}

#[async_trait]
impl McpTool for DirectoryListTool {
    fn name(&self) -> &str {
        "devit_directory_list"
    }

    fn description(&self) -> &str {
        "List directories/files relative to the DevIt sandbox root"
    }

    async fn execute(&self, params: Value) -> McpResult<Value> {
        let root = self.context.root();

        let request_path = params.get("path").and_then(Value::as_str).unwrap_or(".");

        let include_files = params
            .get("include_files")
            .and_then(Value::as_bool)
            .unwrap_or(true);

        let max_depth = params
            .get("max_depth")
            .and_then(Value::as_u64)
            .map(|n| n as usize)
            .unwrap_or(2);

        if max_depth == 0 {
            return Err(validation_error("max_depth must be greater than 0"));
        }

        let base = self.context.resolve_path(request_path)?;
        if !base.is_dir() {
            return Err(validation_error("Specified path is not a directory"));
        }

        let listing = collect_entries(&base, root, include_files, max_depth)?;

        Ok(json!({
            "content": [{
                "type": "json",
                "json": listing
            }],
            "metadata": {
                "base_path": base.strip_prefix(root).unwrap_or(&base).to_string_lossy(),
                "max_depth": max_depth,
                "include_files": include_files
            }
        }))
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Directory to list (relative to sandbox root)",
                    "default": "."
                },
                "max_depth": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Maximum depth of traversal",
                    "default": 2
                },
                "include_files": {
                    "type": "boolean",
                    "description": "Include files in the output",
                    "default": true
                }
            },
            "additionalProperties": false
        })
    }
}

fn collect_entries(
    start: &Path,
    root: &Path,
    include_files: bool,
    max_depth: usize,
) -> McpResult<Value> {
    let mut stack = vec![(start.to_path_buf(), 0_usize)];
    let mut output = Vec::new();

    while let Some((dir, depth)) = stack.pop() {
        let mut children = Vec::new();
        let read_dir = std::fs::read_dir(&dir).map_err(|err| {
            internal_error(format!("Failed to read directory {}: {err}", dir.display()))
        })?;

        for entry in read_dir {
            let entry = entry.map_err(|err| {
                internal_error(format!("Failed to load entry in {}: {err}", dir.display()))
            })?;
            let path = entry.path();

            let relative = path
                .strip_prefix(root)
                .unwrap_or(path.as_path())
                .to_string_lossy()
                .into_owned();

            if path.is_dir() {
                children.push(json!({"path": relative, "kind": "directory"}));
                if depth + 1 < max_depth {
                    stack.push((path, depth + 1));
                }
            } else if include_files {
                children.push(json!({"path": relative, "kind": "file"}));
            }
        }

        output.push(json!({
            "path": dir.strip_prefix(root).unwrap_or(dir.as_path()).to_string_lossy(),
            "kind": "directory",
            "children": children
        }));
    }

    Ok(Value::Array(output))
}
