use std::env;

use async_trait::async_trait;
use mcp_core::{McpError, McpResult, McpTool};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::process::Command;
use tokio::time::{sleep, Duration};
use tracing::{debug, info};

use crate::{desktop_env_error, validation_error};

const DEFAULT_DELAY_MS: u64 = 40;

pub struct MouseTool {
    binary: String,
    default_delay_ms: u64,
}

impl MouseTool {
    pub fn new() -> Self {
        let binary = env::var("DEVIT_XDOTOOL_PATH").unwrap_or_else(|_| String::from("xdotool"));
        let default_delay_ms = env::var("DEVIT_MOUSE_DEFAULT_DELAY_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(DEFAULT_DELAY_MS);
        Self {
            binary,
            default_delay_ms,
        }
    }

    async fn execute_action(&self, action: &MouseAction) -> McpResult<()> {
        match action {
            MouseAction::Move { x, y, sync } => {
                self.run_xdotool(&build_move_args(self.binary(), *x, *y, *sync, false))
                    .await
            }
            MouseAction::MoveRelative { dx, dy, sync } => {
                self.run_xdotool(&build_move_args(self.binary(), *dx, *dy, *sync, true))
                    .await
            }
            MouseAction::Click { button, count } => {
                self.run_xdotool(&build_click_args(*button, *count, None))
                    .await
            }
            MouseAction::Scroll {
                vertical,
                horizontal,
            } => {
                if *vertical == 0 && *horizontal == 0 {
                    return Err(validation_error(
                        "scroll action requires vertical and/or horizontal steps",
                    ));
                }
                if *vertical != 0 {
                    self.run_xdotool(&build_scroll_args(*vertical, Axis::Vertical))
                        .await?;
                }
                if *horizontal != 0 {
                    self.run_xdotool(&build_scroll_args(*horizontal, Axis::Horizontal))
                        .await?;
                }
                Ok(())
            }
            MouseAction::Sleep { millis } => {
                sleep(Duration::from_millis(*millis)).await;
                Ok(())
            }
        }
    }

    async fn run_xdotool(&self, args: &[String]) -> McpResult<()> {
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
            .args(args)
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
            return Err(desktop_env_error("devit_mouse", code, &stderr));
        }

        Ok(())
    }

    fn binary(&self) -> &str {
        &self.binary
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MouseAction {
    Move {
        x: i32,
        y: i32,
        #[serde(default = "default_true")]
        sync: bool,
    },
    MoveRelative {
        dx: i32,
        dy: i32,
        #[serde(default = "default_true")]
        sync: bool,
    },
    Click {
        #[serde(default = "default_button")]
        button: u8,
        #[serde(default = "default_repeat")]
        count: u32,
    },
    Scroll {
        #[serde(default)]
        vertical: i32,
        #[serde(default)]
        horizontal: i32,
    },
    Sleep {
        millis: u64,
    },
}

#[derive(Clone, Copy)]
enum Axis {
    Vertical,
    Horizontal,
}

fn default_true() -> bool {
    true
}

fn default_button() -> u8 {
    1
}

fn default_repeat() -> u32 {
    1
}

fn build_move_args(binary: &str, a: i32, b: i32, sync: bool, relative: bool) -> Vec<String> {
    let mut args = Vec::new();
    args.push(if relative {
        "mousemove_relative".to_string()
    } else {
        "mousemove".to_string()
    });
    if sync {
        args.push("--sync".to_string());
    }
    if !relative && binary == "xdotool" {
        args.push("--clearmodifiers".to_string());
    }
    args.push(a.to_string());
    args.push(b.to_string());
    args
}

fn build_click_args(button: u8, count: u32, delay: Option<u64>) -> Vec<String> {
    let mut args = vec!["click".to_string()];
    if count > 1 {
        args.push("--repeat".to_string());
        args.push(count.to_string());
        if let Some(delay_ms) = delay {
            args.push("--delay".to_string());
            args.push(delay_ms.to_string());
        }
    }
    args.push(button.to_string());
    args
}

fn build_scroll_args(amount: i32, axis: Axis) -> Vec<String> {
    let direction = match axis {
        Axis::Vertical => {
            if amount > 0 {
                5
            } else {
                4
            }
        }
        Axis::Horizontal => {
            if amount > 0 {
                7
            } else {
                6
            }
        }
    };
    build_click_args(direction, amount.unsigned_abs().max(1), Some(10))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MouseRequest {
    pub actions: Vec<MouseAction>,
    #[serde(default)]
    pub delay_ms: Option<u64>,
}

#[async_trait]
impl McpTool for MouseTool {
    fn name(&self) -> &str {
        "devit_mouse"
    }

    fn description(&self) -> &str {
        "Control the mouse via xdotool (move, click, scroll)."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "actions": {
                    "type": "array",
                    "description": "Sequence of mouse actions to execute.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "type": { "type": "string", "enum": ["move", "move_relative", "click", "scroll", "sleep"] },
                            "x": { "type": "number" },
                            "y": { "type": "number" },
                            "dx": { "type": "number" },
                            "dy": { "type": "number" },
                            "vertical": { "type": "number" },
                            "horizontal": { "type": "number" },
                            "button": { "type": "number", "default": 1 },
                            "count": { "type": "number", "default": 1 },
                            "millis": { "type": "number" },
                            "sync": { "type": "boolean", "default": true }
                        },
                        "required": ["type"],
                        "additionalProperties": false
                    }
                },
                "delay_ms": {
                    "type": "number",
                    "description": "Delay (ms) injected between actions.",
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
                "devit_mouse is only supported on Linux (X11)".to_string(),
            ));
        }

        let request: MouseRequest = serde_json::from_value(params)
            .map_err(|err| McpError::InvalidRequest(err.to_string()))?;
        if request.actions.is_empty() {
            return Err(validation_error("actions array must not be empty"));
        }

        let delay_between = request.delay_ms.unwrap_or(self.default_delay_ms);
        let actions_len = request.actions.len();

        for (index, action) in request.actions.iter().enumerate() {
            self.execute_action(action).await?;
            if index + 1 != actions_len && delay_between > 0 {
                sleep(Duration::from_millis(delay_between)).await;
            }
        }

        info!(
            target: "devit_mcp_tools",
            "tool devit_mouse executed {} actions",
            actions_len
        );

        let structured = serde_json::to_value(&request.actions).unwrap_or_else(|_| Value::Null);

        Ok(json!({
            "content": [{
                "type": "text",
                "text": format!("🖱️ Mouse actions executed ({actions_len})")
            }],
            "structuredContent": {
                "desktop": {
                    "tool": "mouse",
                    "actions": structured
                }
            }
        }))
    }
}
