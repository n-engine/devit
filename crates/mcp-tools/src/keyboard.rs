use std::env;

use async_trait::async_trait;
use mcp_core::{McpError, McpResult, McpTool};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::process::Command;
use tokio::time::{sleep, Duration};
use tracing::{debug, info};

use crate::{desktop_env_error, validation_error};

const DEFAULT_DELAY_MS: u64 = 35;

pub struct KeyboardTool {
    binary: String,
    default_delay_ms: u64,
}

impl KeyboardTool {
    pub fn new() -> Self {
        let binary = env::var("DEVIT_XDOTOOL_PATH").unwrap_or_else(|_| String::from("xdotool"));
        let default_delay_ms = env::var("DEVIT_KEYBOARD_DEFAULT_DELAY_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(DEFAULT_DELAY_MS);
        Self {
            binary,
            default_delay_ms,
        }
    }

    async fn execute_action(&self, action: &KeyboardAction, default_delay: u64) -> McpResult<()> {
        match action {
            KeyboardAction::Text {
                text,
                clear_modifiers,
                delay_ms,
            } => {
                let delay = delay_ms.unwrap_or(default_delay);
                self.run_xdotool(build_type_args(text, *clear_modifiers, delay))
                    .await
            }
            KeyboardAction::Key {
                keys,
                repeat,
                clear_modifiers,
                delay_ms,
            } => {
                if keys.is_empty() {
                    return Err(validation_error("key action requires non-empty keys array"));
                }
                let delay = delay_ms.unwrap_or(default_delay);
                self.run_xdotool(build_key_args(keys, *repeat, *clear_modifiers, delay))
                    .await
            }
            KeyboardAction::Sleep { millis } => {
                sleep(Duration::from_millis(*millis)).await;
                Ok(())
            }
        }
    }

    async fn run_xdotool(&self, args: Vec<String>) -> McpResult<()> {
        debug!(
            target: "devit_mcp_tools",
            "xdotool{} {}",
            if self.binary == "xdotool" {
                String::new()
            } else {
                format!(" ({})", self.binary)
            },
            args.join(" ")
        );

        let output = Command::new(&self.binary)
            .kill_on_drop(true)
            .args(&args)
            .output()
            .await
            .map_err(|err| McpError::ExecutionFailed(format!("failed to spawn xdotool: {err}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !stderr.is_empty() {
                debug!(target: "devit_mcp_tools", "xdotool stderr: {stderr}");
            }
            if !stdout.is_empty() {
                debug!(target: "devit_mcp_tools", "xdotool stdout: {stdout}");
            }
            let code = output.status.code();
            return Err(desktop_env_error("devit_keyboard", code, &stderr));
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum KeyboardAction {
    Text {
        text: String,
        #[serde(default = "default_true")]
        clear_modifiers: bool,
        #[serde(default)]
        delay_ms: Option<u64>,
    },
    Key {
        keys: Vec<String>,
        #[serde(default = "default_repeat")]
        repeat: u32,
        #[serde(default = "default_true")]
        clear_modifiers: bool,
        #[serde(default)]
        delay_ms: Option<u64>,
    },
    Sleep {
        millis: u64,
    },
}

fn default_true() -> bool {
    true
}

fn default_repeat() -> u32 {
    1
}

fn build_type_args(text: &str, clear_modifiers: bool, delay_ms: u64) -> Vec<String> {
    let mut args = vec!["type".to_string()];
    if clear_modifiers {
        args.push("--clearmodifiers".to_string());
    }
    if delay_ms > 0 {
        args.push("--delay".to_string());
        args.push(delay_ms.to_string());
    }
    args.push(text.to_string());
    args
}

fn build_key_args(
    keys: &[String],
    repeat: u32,
    clear_modifiers: bool,
    delay_ms: u64,
) -> Vec<String> {
    let mut args = vec!["key".to_string()];
    if repeat > 1 {
        args.push("--repeat".to_string());
        args.push(repeat.to_string());
    }
    if clear_modifiers {
        args.push("--clearmodifiers".to_string());
    }
    if delay_ms > 0 {
        args.push("--delay".to_string());
        args.push(delay_ms.to_string());
    }
    args.push(keys.join("+"));
    args
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyboardRequest {
    pub actions: Vec<KeyboardAction>,
    #[serde(default)]
    pub delay_ms: Option<u64>,
}

#[async_trait]
impl McpTool for KeyboardTool {
    fn name(&self) -> &str {
        "devit_keyboard"
    }

    fn description(&self) -> &str {
        "Control keyboard input via xdotool (type text, send key combos)."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "actions": {
                    "type": "array",
                    "description": "Sequence of keyboard actions to execute.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "type": { "type": "string", "enum": ["text", "key", "sleep"] },
                            "text": { "type": "string" },
                            "keys": { "type": "array", "items": { "type": "string" } },
                            "repeat": { "type": "number", "default": 1 },
                            "delay_ms": { "type": "number" },
                            "millis": { "type": "number" },
                            "clear_modifiers": { "type": "boolean", "default": true }
                        },
                        "required": ["type"],
                        "additionalProperties": false
                    }
                },
                "delay_ms": {
                    "type": "number",
                    "description": "Default delay (ms) between actions and repeated keys.",
                    "default": self.default_delay_ms
                }
            },
            "required": ["actions"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value) -> McpResult<Value> {
        if cfg!(not(target_os = "linux")) {
            return Err(McpError::InvalidRequest(
                "devit_keyboard is only supported on Linux (X11)".to_string(),
            ));
        }

        let request: KeyboardRequest = serde_json::from_value(params)
            .map_err(|err| McpError::InvalidRequest(err.to_string()))?;
        if request.actions.is_empty() {
            return Err(validation_error("actions array must not be empty"));
        }

        let default_delay = request.delay_ms.unwrap_or(self.default_delay_ms);
        let actions_len = request.actions.len();

        for (index, action) in request.actions.iter().enumerate() {
            self.execute_action(action, default_delay).await?;
            if index + 1 != actions_len && default_delay > 0 {
                sleep(Duration::from_millis(default_delay)).await;
            }
        }

        info!(
            target: "devit_mcp_tools",
            "tool devit_keyboard executed {} actions",
            actions_len
        );

        let structured = serde_json::to_value(&request.actions).unwrap_or_else(|_| Value::Null);

        Ok(json!({
            "content": [{
                "type": "text",
                "text": format!("⌨️ Keyboard actions executed ({actions_len})")
            }],
            "structuredContent": {
                "desktop": {
                    "tool": "keyboard",
                    "actions": structured
                }
            }
        }))
    }
}
