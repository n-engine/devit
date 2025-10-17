// devit_ps - Query process registry
// Version: v3.1

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use mcp_core::{McpError, McpResult, McpTool};
use tracing::info;

/// DevIt Ps tool
pub struct DevitPs;

impl DevitPs {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PsRequest {
    pub pid: Option<u32>,
}

#[async_trait]
impl McpTool for DevitPs {
    fn name(&self) -> &str {
        "devit_ps"
    }

    fn description(&self) -> &str {
        "Query process registry (list running/exited processes)"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pid": {
                    "type": "number",
                    "description": "Optional PID to query (if omitted, returns all)"
                }
            }
        })
    }

    async fn execute(&self, params: Value) -> McpResult<Value> {
        let request: PsRequest =
            serde_json::from_value(params).map_err(|e| McpError::InvalidRequest(e.to_string()))?;

        // Reload registry from disk (may have been updated by reaper)
        let registry = devit_common::process_registry::load_registry()
            .map_err(|e| McpError::ExecutionFailed(format!("Failed to load registry: {}", e)))?;

        let mut processes = Vec::new();
        let mut lines = Vec::new();

        let filtered = registry
            .iter()
            .filter(|(pid, _)| {
                if let Some(target_pid) = request.pid {
                    **pid == target_pid
                } else {
                    true
                }
            })
            .collect::<Vec<_>>();

        if filtered.is_empty() {
            lines.push("📋 Aucun processus suivi".to_string());
            info!(
                target: "devit_mcp_tools",
                "tool devit_ps called | pid_filter={:?} count=0",
                request.pid
            );
        } else {
            lines.push(format!("📋 Processus suivis ({})", filtered.len()));
            info!(
                target: "devit_mcp_tools",
                "tool devit_ps called | pid_filter={:?} count={}",
                request.pid,
                filtered.len()
            );
            for (pid, record) in filtered {
                let args_preview = if record.args.is_empty() {
                    "".to_string()
                } else {
                    format!(" {}", record.args.join(" "))
                };
                let status = format!("{:?}", record.status).to_lowercase();
                let auto_kill = record
                    .auto_kill_at
                    .as_ref()
                    .map(|ts| format!(" — auto-kill {}", ts.to_rfc3339()))
                    .unwrap_or_default();

                lines.push(format!(
                    "- pid {} (pgid {}) · {}{} · status: {}{}",
                    pid, record.pgid, record.command, args_preview, status, auto_kill
                ));

                processes.push(json!({
                    "pid": pid,
                    "pgid": record.pgid,
                    "start_ticks": record.start_ticks,
                    "started_at": record.started_at.clone(),
                    "command": record.command.clone(),
                    "args": record.args.clone(),
                    "status": record.status.clone(),
                    "exit_code": record.exit_code,
                    "terminated_by_signal": record.terminated_by_signal,
                    "auto_kill_at": record.auto_kill_at.clone(),
                }));
            }
        }

        Ok(json!({
            "content": [
                {
                    "type": "text",
                    "text": lines.join("\n")
                }
            ],
            "structuredContent": {
                "exec": {
                    "mode": "ps",
                    "processes": processes
                }
            }
        }))
    }
}
