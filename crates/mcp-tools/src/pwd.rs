use std::sync::Arc;

use async_trait::async_trait;
use mcp_core::{McpResult, McpTool};
use serde_json::{json, Value};

use crate::errors::internal_error;
use crate::file_read::FileSystemContext;

pub struct PwdTool {
    context: Arc<FileSystemContext>,
}

impl PwdTool {
    pub fn new(context: Arc<FileSystemContext>) -> Self {
        Self { context }
    }
}

#[async_trait]
impl McpTool for PwdTool {
    fn name(&self) -> &str {
        "devit_pwd"
    }

    fn description(&self) -> &str {
        "Return the canonical working directory and detected project root"
    }

    async fn execute(&self, _params: Value) -> McpResult<Value> {
        let root = self.context.root();
        let canonical = root.canonicalize().map_err(|err| {
            internal_error(format!("Cannot canonicalize working directory: {err}"))
        })?;

        Ok(json!({
            "content": [{
                "type": "text",
                "text": format!(
                    "📁 Current working directory: {}\n\n✅ Auto-detected project root\n🔍 Path resolution enforced via FileSystemContext",
                    canonical.display()
                )
            }],
            "metadata": {
                "working_directory": canonical.to_string_lossy(),
                "original_path": root.to_string_lossy(),
                "auto_detected": true
            }
        }))
    }

    fn input_schema(&self) -> Value {
        json!({"type": "object", "properties": {}, "additionalProperties": false})
    }
}
