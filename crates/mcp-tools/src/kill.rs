// devit_kill - Terminate process by PID+start_ticks
// Version: v3.1

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[cfg(windows)]
use std::io;
use std::time::Duration;
use tokio::time::sleep;
#[cfg(target_family = "unix")]
use tracing::debug;
use tracing::info;
#[cfg(windows)]
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
#[cfg(windows)]
use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};

use mcp_core::{McpError, McpResult, McpTool};

/// DevIt Kill tool
pub struct DevitKill;

impl DevitKill {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KillRequest {
    pub pid: u32,
    #[serde(default = "default_signal")]
    pub signal: String, // "TERM" | "KILL" | "INT"
}

fn default_signal() -> String {
    "TERM".to_string()
}

#[cfg(target_family = "unix")]
fn parse_signal(sig: &str) -> Option<i32> {
    match sig.to_uppercase().as_str() {
        "TERM" => Some(libc::SIGTERM),
        "KILL" => Some(libc::SIGKILL),
        "INT" => Some(libc::SIGINT),
        _ => None,
    }
}

#[cfg(not(target_family = "unix"))]
fn parse_signal(_sig: &str) -> Option<i32> {
    None
}

#[async_trait]
impl McpTool for DevitKill {
    fn name(&self) -> &str {
        "devit_kill"
    }

    fn description(&self) -> &str {
        "Terminate a background process with PID validation"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pid": {
                    "type": "number",
                    "description": "Process ID to terminate"
                },
                "signal": {
                    "type": "string",
                    "enum": ["TERM", "KILL", "INT"],
                    "description": "Signal to send (default: TERM)"
                }
            },
            "required": ["pid"]
        })
    }

    async fn execute(&self, params: Value) -> McpResult<Value> {
        #[cfg_attr(not(target_family = "unix"), allow(unused_variables))]
        let request: KillRequest =
            serde_json::from_value(params).map_err(|e| McpError::InvalidRequest(e.to_string()))?;

        #[cfg(target_family = "unix")]
        {
            use devit_common::process_registry::{load_registry, save_registry, ProcessStatus};
            use devit_common::process_utils::{process_exists, read_proc_stat, verify_pgid_leader};

            // Reload registry
            let mut registry = load_registry().map_err(|e| {
                McpError::ExecutionFailed(format!("Failed to load registry: {}", e))
            })?;

            let pid = request.pid;

            let record = registry.get(pid).cloned().ok_or_else(|| {
                McpError::ExecutionFailed(format!("PID {} not in registry", request.pid))
            })?;

            // Validate start_ticks
            let current_stat = read_proc_stat(pid)
                .map_err(|_| McpError::ExecutionFailed(format!("Process {} not found", pid)))?;

            if current_stat.starttime != record.start_ticks {
                return Err(McpError::ExecutionFailed(format!(
                    "PID {} reused (start_ticks mismatch)",
                    pid
                )));
            }

            // Verify PGID
            if !verify_pgid_leader(record.pgid, record.start_ticks) {
                return Err(McpError::ExecutionFailed(format!(
                    "PGID {} leader validation failed",
                    record.pgid
                )));
            }

            // Parse signal
            let sig = parse_signal(&request.signal).ok_or_else(|| {
                McpError::InvalidRequest(format!("Unknown signal: {}", request.signal))
            })?;

            // Send signal to PGID (negative PID)
            let ret = unsafe { libc::kill(-(record.pgid as i32), sig) };
            if ret != 0 {
                return Err(McpError::ExecutionFailed(format!(
                    "kill failed: {}",
                    std::io::Error::last_os_error()
                )));
            }

            let signal_str = request.signal;

            // Wait briefly for process to exit (foreground ensures reaper-less setups update)
            let mut attempts = 0;
            let mut still_running = true;
            while attempts < 10 {
                if !process_exists(pid) {
                    still_running = false;
                    break;
                }
                sleep(Duration::from_millis(100)).await;
                attempts += 1;
            }

            debug!(
                target: "devit_mcp_tools",
                "tool devit_kill | pid {} state after {} attempts -> still_running={}",
                pid,
                attempts,
                still_running
            );

            // Update registry entry
            if let Some(entry) = registry.get_mut(pid) {
                entry.status = ProcessStatus::Exited;
                entry.terminated_by_signal = Some(sig);
                entry.exit_code = None;
            }

            if let Err(e) = save_registry(&registry) {
                return Err(McpError::ExecutionFailed(format!(
                    "Failed to save registry: {}",
                    e
                )));
            }

            let updated_record = registry.get(pid).cloned().unwrap_or(record.clone());

            let summary = format!(
                "🛑 Signal {} envoyé au pid {} (pgid {})",
                signal_str, pid, record.pgid
            );

            let record_snapshot = json!({
                "pid": pid,
                "pgid": record.pgid,
                "start_ticks": record.start_ticks,
                "started_at": record.started_at.clone(),
                "command": record.command.clone(),
                "args": record.args.clone(),
                "status": updated_record.status.clone(),
                "exit_code": updated_record.exit_code,
                "terminated_by_signal": updated_record.terminated_by_signal,
                "auto_kill_at": updated_record.auto_kill_at.clone()
            });

            info!(
                target: "devit_mcp_tools",
                "tool devit_kill completed | pid={} pgid={} signal={} status={:?}",
                pid,
                record.pgid,
                signal_str,
                updated_record.status
            );

            Ok(json!({
                "content": [
                    {
                        "type": "text",
                        "text": summary
                    }
                ],
                "structuredContent": {
                    "exec": {
                        "mode": "kill",
                        "pid": pid,
                        "signal": signal_str,
                        "record": record_snapshot
                    }
                }
            }))
        }

        #[cfg(windows)]
        {
            use devit_common::process_registry::{load_registry, save_registry, ProcessStatus};
            use devit_common::process_utils::{process_exists, read_proc_stat};

            let mut registry = load_registry().map_err(|e| {
                McpError::ExecutionFailed(format!("Failed to load registry: {}", e))
            })?;

            let pid = request.pid;

            let record = registry.get(pid).cloned().ok_or_else(|| {
                McpError::ExecutionFailed(format!("PID {} not in registry", request.pid))
            })?;

            let current_stat = read_proc_stat(pid)
                .map_err(|_| McpError::ExecutionFailed(format!("Process {} not found", pid)))?;

            if current_stat.starttime != record.start_ticks {
                return Err(McpError::ExecutionFailed(format!(
                    "PID {} reused (start_ticks mismatch)",
                    pid
                )));
            }

            let handle: HANDLE = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) };
            if handle.is_null() {
                return Err(McpError::ExecutionFailed(format!(
                    "OpenProcess failed: {}",
                    io::Error::last_os_error()
                )));
            }

            let terminate = unsafe { TerminateProcess(handle, 1) };
            let _ = unsafe { CloseHandle(handle) };
            if terminate == 0 {
                return Err(McpError::ExecutionFailed(format!(
                    "TerminateProcess failed: {}",
                    io::Error::last_os_error()
                )));
            }

            let mut attempts = 0;
            let mut still_running = true;
            while attempts < 10 {
                if !process_exists(pid) {
                    still_running = false;
                    break;
                }
                sleep(Duration::from_millis(100)).await;
                attempts += 1;
            }

            if let Some(entry) = registry.get_mut(pid) {
                entry.status = ProcessStatus::Exited;
                entry.exit_code = Some(1);
                entry.terminated_by_signal = None;
            }

            if let Err(e) = save_registry(&registry) {
                return Err(McpError::ExecutionFailed(format!(
                    "Failed to save registry: {}",
                    e
                )));
            }

            let updated_record = registry.get(pid).cloned().unwrap_or(record.clone());

            let summary = format!(
                "🛑 Process {} (command `{}`) terminated",
                pid, record.command
            );

            let record_snapshot = json!({
                "pid": pid,
                "pgid": record.pgid,
                "start_ticks": record.start_ticks,
                "started_at": record.started_at.clone(),
                "command": record.command.clone(),
                "args": record.args.clone(),
                "status": updated_record.status.clone(),
                "exit_code": updated_record.exit_code,
                "terminated_by_signal": updated_record.terminated_by_signal,
                "auto_kill_at": updated_record.auto_kill_at.clone()
            });

            info!(
                target: "devit_mcp_tools",
                "tool devit_kill completed | pid={} exit_code={:?} attempts={} still_running={}",
                pid,
                updated_record.exit_code,
                attempts,
                still_running
            );

            Ok(json!({
                "content": [
                    {
                        "type": "text",
                        "text": summary
                    }
                ],
                "structuredContent": {
                    "exec": {
                        "mode": "kill",
                        "pid": pid,
                        "signal": request.signal,
                        "record": record_snapshot
                    }
                }
            }))
        }
    }
}
