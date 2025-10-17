use std::sync::Arc;

use async_trait::async_trait;
use devit_cli::core::help_system::{HelpSystem, ToolHelp};
use mcp_core::{McpResult, McpTool};
use serde_json::{json, Value};

use crate::errors::{internal_error, validation_error};
use crate::file_read::FileSystemContext;

/// Tool exposing DevIt CLI help topics to MCP clients.
pub struct HelpTool;

impl HelpTool {
    pub fn new(_fs_context: Arc<FileSystemContext>) -> Self {
        Self
    }

    fn generate_static_help(&self, topic: &str) -> McpResult<String> {
        let mut help_system = HelpSystem::new();
        if topic == "all" {
            let overview = help_system
                .get_all_tools_help()
                .map_err(|err| internal_error(err.to_string()))?;
            return Ok(render_tool_help(&overview));
        }

        match help_system.get_tool_help(topic) {
            Ok(help) => Ok(render_tool_help(help)),
            Err(_) => Err(validation_error(&format!(
                "Sujet d'aide inconnu: '{}'. Topics disponibles: {}",
                topic,
                FALLBACK_TOPICS.join(", ")
            ))),
        }
    }
}

#[async_trait]
impl McpTool for HelpTool {
    fn name(&self) -> &str {
        "devit_help"
    }

    fn description(&self) -> &str {
        "Show DevIt CLI help for a specific command or list all commands"
    }

    async fn execute(&self, params: Value) -> McpResult<Value> {
        let topic_label = params
            .get("topic")
            .and_then(Value::as_str)
            .unwrap_or("all")
            .trim()
            .to_string();

        let mut metadata = serde_json::Map::new();
        metadata.insert("topic".into(), Value::String(topic_label.clone()));

        metadata.insert("source".into(), Value::String("static".into()));
        let body = self.generate_static_help(&topic_label)?;

        Ok(json!({
            "content": [{
                "type": "text",
                "text": body
            }],
            "metadata": Value::Object(metadata)
        }))
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "topic": {
                    "type": "string",
                    "description": "Command to show help for (use 'all' to list everything)",
                    "default": "all"
                }
            },
            "additionalProperties": false
        })
    }
}

const FALLBACK_TOPICS: &[&str] = &[
    "devit_file_read",
    "devit_file_read_ext",
    "devit_file_list",
    "devit_file_list_ext",
    "devit_file_search",
    "devit_file_search_ext",
    "devit_project_structure",
    "devit_project_structure_ext",
    "devit_pwd",
];

fn render_tool_help(help: &ToolHelp) -> String {
    let mut lines = Vec::new();
    if help.tool_name == "devit_help_all" {
        lines.push("DevIt MCP Tools Overview".to_string());
        lines.push(String::from(""));
    }

    lines.push(format!("{} — {}", help.tool_name, help.description));

    if !help.formats.is_empty() {
        lines.push(String::from(""));
        lines.push(String::from("formats:"));
        for (name, desc) in &help.formats {
            lines.push(format!("  • {}: {}", name, desc));
        }
    }

    if !help.examples.is_empty() {
        lines.push(String::from(""));
        lines.push(String::from("examples:"));
        for example in &help.examples {
            let command = serde_json::to_string_pretty(&example.command)
                .unwrap_or_else(|_| String::from("{}"));
            lines.push(format!("  ▶ {}", example.use_case));
            lines.push(format!("    Commande: {}", command.replace('\n', "\n    ")));
            lines.push(format!(
                "    Sortie: {}",
                example.output_sample.replace('\n', "\n    ")
            ));
            if let Some(savings) = example.token_savings {
                lines.push(format!("    Token savings: {:.0}%", savings * 100.0));
            }
        }
    }

    if !help.ai_tips.is_empty() {
        lines.push(String::from(""));
        lines.push(String::from("AI Optimization Tips:"));
        for tip in &help.ai_tips {
            lines.push(format!("  • {}", tip));
        }
    }

    if !help.performance_hints.is_empty() {
        lines.push(String::from(""));
        lines.push(String::from("Performance Hints:"));
        for hint in &help.performance_hints {
            lines.push(format!("  • {}", hint));
        }
    }

    lines.join("\n")
}
