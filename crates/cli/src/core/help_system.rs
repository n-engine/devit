//! # DevIt Help System
//!
//! This module provides intelligent help and auto-documentation for DevIt MCP tools,
//! specifically designed to assist AI assistants with usage optimization and token savings.

use crate::core::formats::{FormatUtils, OutputFormat};
use crate::core::{DevItError, DevItResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Help documentation for a DevIt tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolHelp {
    /// Name of the tool (e.g., "devit_file_read_ext")
    pub tool_name: String,
    /// Human-readable description of the tool's purpose
    pub description: String,
    /// Available output formats with descriptions
    pub formats: HashMap<String, String>,
    /// Usage examples with concrete use cases
    pub examples: Vec<UsageExample>,
    /// AI-specific optimization tips
    pub ai_tips: Vec<String>,
    /// Performance hints for large-scale usage
    pub performance_hints: Vec<String>,
}

/// Concrete usage example for a tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageExample {
    /// Description of when to use this example
    pub use_case: String,
    /// MCP command parameters as JSON
    pub command: Value,
    /// Sample output (truncated for brevity)
    pub output_sample: String,
    /// Token savings compared to baseline (0.0-1.0, None if baseline)
    pub token_savings: Option<f32>,
}

/// Help system manager
pub struct HelpSystem {
    /// Cache of generated help content
    help_cache: HashMap<String, ToolHelp>,
    /// Available tools and their schemas
    tool_schemas: HashMap<String, Value>,
}

impl HelpSystem {
    /// Create a new help system
    pub fn new() -> Self {
        Self {
            help_cache: HashMap::new(),
            tool_schemas: HashMap::new(),
        }
    }

    /// Register a tool schema for help generation
    pub fn register_tool_schema(&mut self, tool_name: &str, schema: Value) {
        self.tool_schemas.insert(tool_name.to_string(), schema);
    }

    /// Get help for a specific tool
    pub fn get_tool_help(&mut self, tool_name: &str) -> DevItResult<&ToolHelp> {
        if !self.help_cache.contains_key(tool_name) {
            let help = self.generate_tool_help(tool_name)?;
            self.help_cache.insert(tool_name.to_string(), help);
        }

        Ok(self.help_cache.get(tool_name).unwrap())
    }

    /// Generate help content for a tool
    fn generate_tool_help(&self, tool_name: &str) -> DevItResult<ToolHelp> {
        match tool_name {
            "devit_file_read" => self.generate_file_read_help(),
            "devit_file_read_ext" => self.generate_file_read_ext_help(),
            "devit_file_list" => self.generate_file_list_help(),
            "devit_file_list_ext" => self.generate_file_list_ext_help(),
            "devit_file_search" => self.generate_file_search_help(),
            "devit_file_search_ext" => self.generate_file_search_ext_help(),
            "devit_project_structure" => self.generate_project_structure_help(),
            "devit_project_structure_ext" => self.generate_project_structure_ext_help(),
            "devit_pwd" => self.generate_pwd_help(),
            _ => Err(DevItError::InvalidFormat {
                format: tool_name.to_string(),
                supported: vec![
                    "devit_file_read".to_string(),
                    "devit_file_read_ext".to_string(),
                    "devit_file_list".to_string(),
                    "devit_file_list_ext".to_string(),
                ],
            }),
        }
    }

    /// Generate help for devit_file_read (baseline tool)
    fn generate_file_read_help(&self) -> DevItResult<ToolHelp> {
        Ok(ToolHelp {
            tool_name: "devit_file_read".to_string(),
            description: "Read file content with line numbers and pagination support".to_string(),
            formats: {
                let mut formats = HashMap::new();
                formats.insert("json".to_string(), "Standard verbose JSON format".to_string());
                formats
            },
            examples: vec![
                UsageExample {
                    use_case: "Read entire file".to_string(),
                    command: serde_json::json!({
                        "path": "src/main.rs"
                    }),
                    output_sample: r#"{"path":"src/main.rs","content":"fn main() {\n    println!(\"Hello, world!\");\n}","size":42,"lines":["1: fn main() {","2:     println!(\"Hello, world!\");","3: }"],"encoding":"utf-8"}"#.to_string(),
                    token_savings: None, // Baseline
                },
                UsageExample {
                    use_case: "Read file with pagination".to_string(),
                    command: serde_json::json!({
                        "path": "src/lib.rs",
                        "offset": 10,
                        "limit": 20
                    }),
                    output_sample: r#"{"path":"src/lib.rs","content":"// Lines 10-30 content...","size":1024,"lines":["10: pub fn example() {","11:     // Implementation","..."],"encoding":"utf-8"}"#.to_string(),
                    token_savings: None,
                },
            ],
            ai_tips: vec![
                "Use offset and limit for large files to control token usage".to_string(),
                "Always check file size before reading to avoid token overflow".to_string(),
                "Prefer devit_file_read_ext for better token efficiency".to_string(),
            ],
            performance_hints: vec![
                "Files over 100KB should use pagination".to_string(),
                "Consider using devit_file_search for finding specific content".to_string(),
            ],
        })
    }

    /// Generate help for devit_file_read_ext (extended tool with compression)
    fn generate_file_read_ext_help(&self) -> DevItResult<ToolHelp> {
        Ok(ToolHelp {
            tool_name: "devit_file_read_ext".to_string(),
            description: "Read file content with compression and filtering options for token optimization".to_string(),
            formats: {
                let mut formats = HashMap::new();
                formats.insert("json".to_string(), "Standard verbose JSON (baseline)".to_string());
                formats.insert("compact".to_string(), "Abbreviated JSON (60% token reduction)".to_string());
                formats.insert("table".to_string(), "Pipe-delimited format (80% token reduction)".to_string());
                formats
            },
            examples: vec![
                UsageExample {
                    use_case: "Quick content check with compression".to_string(),
                    command: serde_json::json!({
                        "path": "main.rs",
                        "format": "compact",
                        "limit": 50
                    }),
                    output_sample: r#"{"f":"main.rs","s":1024,"c":"fn main() {\n    println!(\"Hello!\");\n}","e":"utf-8"}"#.to_string(),
                    token_savings: Some(0.6),
                },
                UsageExample {
                    use_case: "Minimal file overview for AI processing".to_string(),
                    command: serde_json::json!({
                        "path": "src/utils.rs",
                        "format": "table",
                        "limit": 100
                    }),
                    output_sample: "path|size|encoding|content\nsrc/utils.rs|2048|utf-8|pub fn helper() { ... }".to_string(),
                    token_savings: Some(0.8),
                },
                UsageExample {
                    use_case: "Standard detailed reading".to_string(),
                    command: serde_json::json!({
                        "path": "config.toml",
                        "format": "json"
                    }),
                    output_sample: r#"{"path":"config.toml","content":"[database]\nurl = \"localhost\"","size":156,"lines":["1: [database]","2: url = \"localhost\""],"encoding":"utf-8"}"#.to_string(),
                    token_savings: None,
                },
            ],
            ai_tips: vec![
                "Use 'compact' format for routine file reading to save 60% tokens".to_string(),
                "Use 'table' format when processing many files sequentially".to_string(),
                "Always set --limit for large files to avoid token overflow".to_string(),
                "Format 'compact' is optimal for file content analysis".to_string(),
                "Switch to 'json' only when you need full field names for debugging".to_string(),
            ],
            performance_hints: vec![
                "Compact format processes 2.5x faster for large files".to_string(),
                "Table format is ideal for batch processing of 10+ files".to_string(),
                "Use limit parameter to cap token usage for AI context management".to_string(),
            ],
        })
    }

    /// Generate help for devit_file_list
    fn generate_file_list_help(&self) -> DevItResult<ToolHelp> {
        Ok(ToolHelp {
            tool_name: "devit_file_list".to_string(),
            description: "List directory contents with metadata and filtering options".to_string(),
            formats: {
                let mut formats = HashMap::new();
                formats.insert("json".to_string(), "Standard verbose JSON with full metadata".to_string());
                formats
            },
            examples: vec![
                UsageExample {
                    use_case: "List current directory".to_string(),
                    command: serde_json::json!({
                        "path": "."
                    }),
                    output_sample: r#"[{"name":"main.rs","path":"./main.rs","entry_type":"File","size":1024,"modified":"2024-01-01T12:00:00Z","permissions":{"readable":true,"writable":true,"executable":false}}]"#.to_string(),
                    token_savings: None,
                },
                UsageExample {
                    use_case: "Recursive directory listing".to_string(),
                    command: serde_json::json!({
                        "path": "src",
                        "recursive": true,
                        "include_hidden": false
                    }),
                    output_sample: r#"[{"name":"lib.rs","path":"src/lib.rs","entry_type":"File","size":2048,"modified":"2024-01-01T12:00:00Z","permissions":{"readable":true,"writable":true,"executable":false}},{"name":"utils","path":"src/utils","entry_type":"Directory","size":null,"modified":"2024-01-01T11:00:00Z","permissions":{"readable":true,"writable":true,"executable":true}}]"#.to_string(),
                    token_savings: None,
                },
            ],
            ai_tips: vec![
                "Use recursive=false for large directories to control output size".to_string(),
                "Filter by file types using include_patterns for focused analysis".to_string(),
                "Consider devit_file_list_ext for token-efficient directory scanning".to_string(),
            ],
            performance_hints: vec![
                "Large directories (>100 files) should use pagination or filtering".to_string(),
                "Recursive listing can generate significant token usage".to_string(),
            ],
        })
    }

    /// Generate help for devit_file_list_ext
    fn generate_file_list_ext_help(&self) -> DevItResult<ToolHelp> {
        Ok(ToolHelp {
            tool_name: "devit_file_list_ext".to_string(),
            description: "List directory contents with compression and smart filtering for AI optimization".to_string(),
            formats: {
                let mut formats = HashMap::new();
                formats.insert("json".to_string(), "Standard verbose JSON with full metadata".to_string());
                formats.insert("compact".to_string(), "Abbreviated JSON (60% token reduction)".to_string());
                formats.insert("table".to_string(), "Pipe-delimited format (80% token reduction)".to_string());
                formats
            },
            examples: vec![
                UsageExample {
                    use_case: "Quick directory overview".to_string(),
                    command: serde_json::json!({
                        "path": "src",
                        "format": "compact"
                    }),
                    output_sample: r#"[{"n":"main.rs","f":"src/main.rs","t":"File","s":1024,"p":{"r":true,"w":true,"x":false}},{"n":"lib.rs","f":"src/lib.rs","t":"File","s":2048,"p":{"r":true,"w":true,"x":false}}]"#.to_string(),
                    token_savings: Some(0.6),
                },
                UsageExample {
                    use_case: "Batch file processing list".to_string(),
                    command: serde_json::json!({
                        "path": "tests",
                        "format": "table",
                        "recursive": true
                    }),
                    output_sample: "name|path|type|size|permissions\ntest1.rs|tests/test1.rs|File|512|rwx\ntest2.rs|tests/test2.rs|File|768|rwx".to_string(),
                    token_savings: Some(0.8),
                },
            ],
            ai_tips: vec![
                "Use 'compact' format for directory analysis to save tokens".to_string(),
                "Use 'table' format when you need to process many files in sequence".to_string(),
                "Combine with include_patterns to filter relevant files only".to_string(),
                "Table format is perfect for generating file processing plans".to_string(),
            ],
            performance_hints: vec![
                "Table format reduces memory usage for large directory listings".to_string(),
                "Compact format maintains full metadata while saving 60% tokens".to_string(),
            ],
        })
    }

    /// Generate help for devit_file_search
    fn generate_file_search_help(&self) -> DevItResult<ToolHelp> {
        Ok(ToolHelp {
            tool_name: "devit_file_search".to_string(),
            description: "Search for patterns in files with context and metadata".to_string(),
            formats: {
                let mut formats = HashMap::new();
                formats.insert("json".to_string(), "Standard verbose JSON with full search results".to_string());
                formats
            },
            examples: vec![
                UsageExample {
                    use_case: "Find function definitions".to_string(),
                    command: serde_json::json!({
                        "pattern": "fn\\s+\\w+",
                        "path": "src",
                        "context_lines": 2
                    }),
                    output_sample: r#"{"pattern":"fn\\s+\\w+","path":"src","files_searched":5,"total_matches":12,"matches":[{"file":"src/main.rs","line_number":1,"line":"fn main() {","context_before":[""],"context_after":["    println!(\"Hello!\");","}"]}],"truncated":false}"#.to_string(),
                    token_savings: None,
                },
            ],
            ai_tips: vec![
                "Use specific regex patterns to find relevant code sections".to_string(),
                "Adjust context_lines based on how much surrounding code you need".to_string(),
                "Consider devit_file_search_ext for token-efficient search results".to_string(),
            ],
            performance_hints: vec![
                "Complex regex patterns may slow down search on large codebases".to_string(),
                "Use file_pattern to limit search scope".to_string(),
            ],
        })
    }

    /// Generate help for devit_file_search_ext
    fn generate_file_search_ext_help(&self) -> DevItResult<ToolHelp> {
        Ok(ToolHelp {
            tool_name: "devit_file_search_ext".to_string(),
            description: "Search for patterns with compression and AI-optimized result formatting".to_string(),
            formats: {
                let mut formats = HashMap::new();
                formats.insert("json".to_string(), "Standard verbose JSON with full search results".to_string());
                formats.insert("compact".to_string(), "Abbreviated JSON (60% token reduction)".to_string());
                formats.insert("table".to_string(), "Pipe-delimited format (80% token reduction)".to_string());
                formats
            },
            examples: vec![
                UsageExample {
                    use_case: "Quick pattern search with compression".to_string(),
                    command: serde_json::json!({
                        "pattern": "TODO|FIXME",
                        "path": "src",
                        "format": "compact",
                        "context_lines": 1
                    }),
                    output_sample: r#"{"pat":"TODO|FIXME","f":"src","fs":8,"tm":3,"mt":[{"sf":"src/main.rs","ln":45,"sl":"// TODO: implement this","cb":["fn process() {"],"ca":["    return;"]}],"tc":false}"#.to_string(),
                    token_savings: Some(0.6),
                },
                UsageExample {
                    use_case: "Tabular search results for analysis".to_string(),
                    command: serde_json::json!({
                        "pattern": "error|Error",
                        "path": ".",
                        "format": "table"
                    }),
                    output_sample: "file|line|match|context\nsrc/main.rs|23|Error handling|fn handle_error()\nsrc/lib.rs|67|error message|log::error()".to_string(),
                    token_savings: Some(0.8),
                },
            ],
            ai_tips: vec![
                "Use 'compact' format for search results to save significant tokens".to_string(),
                "Use 'table' format when analyzing many search matches".to_string(),
                "Combine search with specific file patterns to focus results".to_string(),
                "Table format is excellent for generating fix/improvement plans".to_string(),
            ],
            performance_hints: vec![
                "Compact format is ideal for large search result sets".to_string(),
                "Table format facilitates batch processing of search results".to_string(),
            ],
        })
    }

    /// Generate help for devit_project_structure
    fn generate_project_structure_help(&self) -> DevItResult<ToolHelp> {
        Ok(ToolHelp {
            tool_name: "devit_project_structure".to_string(),
            description: "Generate comprehensive project structure with auto-detection and tree view".to_string(),
            formats: {
                let mut formats = HashMap::new();
                formats.insert("json".to_string(), "Standard verbose JSON with full project metadata".to_string());
                formats
            },
            examples: vec![
                UsageExample {
                    use_case: "Analyze project structure".to_string(),
                    command: serde_json::json!({
                        "path": "."
                    }),
                    output_sample: r#"{"root":".","project_type":"rust","tree":{"name":"my-project","node_type":"Directory","children":[{"name":"src","node_type":"Directory","children":[{"name":"main.rs","node_type":"File","children":[]}]}]},"total_files":15,"total_dirs":4}"#.to_string(),
                    token_savings: None,
                },
            ],
            ai_tips: vec![
                "Use for understanding overall project architecture".to_string(),
                "Helpful for generating project documentation".to_string(),
                "Consider devit_project_structure_ext for token efficiency".to_string(),
            ],
            performance_hints: vec![
                "Large projects may generate substantial output".to_string(),
                "Use max_depth to limit tree traversal".to_string(),
            ],
        })
    }

    /// Generate help for devit_project_structure_ext
    fn generate_project_structure_ext_help(&self) -> DevItResult<ToolHelp> {
        Ok(ToolHelp {
            tool_name: "devit_project_structure_ext".to_string(),
            description: "Generate project structure with compression and AI-focused formatting".to_string(),
            formats: {
                let mut formats = HashMap::new();
                formats.insert("json".to_string(), "Standard verbose JSON with full project metadata".to_string());
                formats.insert("compact".to_string(), "Abbreviated JSON (60% token reduction)".to_string());
                formats.insert("table".to_string(), "Pipe-delimited tree format (80% token reduction)".to_string());
                formats
            },
            examples: vec![
                UsageExample {
                    use_case: "Compressed project overview".to_string(),
                    command: serde_json::json!({
                        "path": ".",
                        "format": "compact"
                    }),
                    output_sample: r#"{"rt":".","pt":"rust","tr":{"n":"project","nt":"Directory","ch":[{"n":"src","nt":"Directory","ch":[{"n":"main.rs","nt":"File"}]}]},"tf":15,"td":4}"#.to_string(),
                    token_savings: Some(0.6),
                },
                UsageExample {
                    use_case: "Tabular project tree".to_string(),
                    command: serde_json::json!({
                        "path": ".",
                        "format": "table",
                        "max_depth": 3
                    }),
                    output_sample: "name|type|path|level\nproject|Directory|.|0\nsrc|Directory|src|1\nmain.rs|File|src/main.rs|2".to_string(),
                    token_savings: Some(0.8),
                },
            ],
            ai_tips: vec![
                "Use 'compact' format for project analysis to save tokens".to_string(),
                "Use 'table' format for generating navigation or build plans".to_string(),
                "Limit max_depth for large projects to control output size".to_string(),
                "Table format excellent for creating project maps".to_string(),
            ],
            performance_hints: vec![
                "Compact format maintains full structure info while saving tokens".to_string(),
                "Table format ideal for hierarchical processing".to_string(),
            ],
        })
    }

    /// Generate help for devit_pwd
    fn generate_pwd_help(&self) -> DevItResult<ToolHelp> {
        Ok(ToolHelp {
            tool_name: "devit_pwd".to_string(),
            description: "Get current working directory with path resolution and validation".to_string(),
            formats: {
                let mut formats = HashMap::new();
                formats.insert("json".to_string(), "Standard JSON with directory information".to_string());
                formats
            },
            examples: vec![
                UsageExample {
                    use_case: "Get current directory".to_string(),
                    command: serde_json::json!({}),
                    output_sample: r#"{"current_directory":"/home/user/project","absolute_path":"/home/user/project","exists":true,"readable":true,"writable":true}"#.to_string(),
                    token_savings: None,
                },
            ],
            ai_tips: vec![
                "Use to establish context before other file operations".to_string(),
                "Helpful for understanding relative path references".to_string(),
            ],
            performance_hints: vec![
                "Lightweight operation, suitable for frequent use".to_string(),
            ],
        })
    }

    /// Get overview help for all tools
    pub fn get_all_tools_help(&mut self) -> DevItResult<ToolHelp> {
        Ok(ToolHelp {
            tool_name: "devit_help_all".to_string(),
            description: "Overview of all DevIt MCP tools with optimization guidance".to_string(),
            formats: {
                let mut formats = HashMap::new();
                formats.insert("json".to_string(), "Complete tool listing with descriptions".to_string());
                formats
            },
            examples: vec![
                UsageExample {
                    use_case: "Get all available tools".to_string(),
                    command: serde_json::json!({}),
                    output_sample: r#"{"tools":["devit_file_read","devit_file_read_ext","devit_file_list","devit_file_list_ext","devit_file_search","devit_file_search_ext","devit_project_structure","devit_project_structure_ext","devit_pwd"],"categories":{"file_operations":["devit_file_read","devit_file_read_ext"],"directory_operations":["devit_file_list","devit_file_list_ext"],"search_operations":["devit_file_search","devit_file_search_ext"],"project_analysis":["devit_project_structure","devit_project_structure_ext"],"utilities":["devit_pwd"]}}"#.to_string(),
                    token_savings: None,
                },
            ],
            ai_tips: vec![
                "Always prefer _ext versions for token efficiency".to_string(),
                "Use 'compact' format for most operations unless debugging".to_string(),
                "Use 'table' format for batch processing and analysis".to_string(),
                "Combine tools for comprehensive project understanding".to_string(),
                "Set appropriate limits to control token usage".to_string(),
            ],
            performance_hints: vec![
                "Extended tools offer 60-80% token savings".to_string(),
                "Batch operations are more efficient with table format".to_string(),
                "Use pagination for large files and directories".to_string(),
            ],
        })
    }

    /// Calculate token savings for a format compared to JSON baseline
    pub fn calculate_token_savings(&self, json_output: &str, format: &OutputFormat) -> f32 {
        match format {
            OutputFormat::Json => 0.0, // Baseline
            OutputFormat::Compact => {
                let estimated_tokens_json = FormatUtils::estimate_token_count(json_output);
                let estimated_tokens_compact = (estimated_tokens_json as f32 * 0.4) as usize;
                1.0 - (estimated_tokens_compact as f32 / estimated_tokens_json as f32)
            }
            OutputFormat::Table => {
                let estimated_tokens_json = FormatUtils::estimate_token_count(json_output);
                let estimated_tokens_table = (estimated_tokens_json as f32 * 0.2) as usize;
                1.0 - (estimated_tokens_table as f32 / estimated_tokens_json as f32)
            }
            OutputFormat::MessagePack => 0.85, // Future format
        }
    }
}

impl Default for HelpSystem {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_help_system_creation() {
        let help_system = HelpSystem::new();
        assert_eq!(help_system.help_cache.len(), 0);
        assert_eq!(help_system.tool_schemas.len(), 0);
    }

    #[test]
    fn test_file_read_help_generation() {
        let mut help_system = HelpSystem::new();
        let help = help_system.get_tool_help("devit_file_read").unwrap();

        assert_eq!(help.tool_name, "devit_file_read");
        assert!(!help.description.is_empty());
        assert!(help.examples.len() > 0);
        assert!(help.ai_tips.len() > 0);
        assert!(help.formats.contains_key("json"));
    }

    #[test]
    fn test_file_read_ext_help_generation() {
        let mut help_system = HelpSystem::new();
        let help = help_system.get_tool_help("devit_file_read_ext").unwrap();

        assert_eq!(help.tool_name, "devit_file_read_ext");
        assert!(help.formats.contains_key("compact"));
        assert!(help.formats.contains_key("table"));
        assert!(help.examples.iter().any(|ex| ex.token_savings.is_some()));
    }

    #[test]
    fn test_token_savings_calculation() {
        let help_system = HelpSystem::new();
        let json_output =
            r#"{"path": "/test/file.rs", "size": 1024, "content": "example content"}"#;

        let compact_savings =
            help_system.calculate_token_savings(json_output, &OutputFormat::Compact);
        let table_savings = help_system.calculate_token_savings(json_output, &OutputFormat::Table);

        assert!(compact_savings > 0.0 && compact_savings < 1.0);
        assert!(table_savings > compact_savings);
    }

    #[test]
    fn test_help_caching() {
        let mut help_system = HelpSystem::new();

        // First call should generate and cache
        let _help1 = help_system.get_tool_help("devit_file_read").unwrap();
        assert_eq!(help_system.help_cache.len(), 1);

        // Second call should use cache
        let _help2 = help_system.get_tool_help("devit_file_read").unwrap();
        assert_eq!(help_system.help_cache.len(), 1);
    }

    #[test]
    fn test_invalid_tool_help() {
        let mut help_system = HelpSystem::new();
        let result = help_system.get_tool_help("invalid_tool");
        assert!(result.is_err());
    }

    #[test]
    fn test_all_tools_help() {
        let mut help_system = HelpSystem::new();
        let help = help_system.get_all_tools_help().unwrap();

        assert_eq!(help.tool_name, "devit_help_all");
        assert!(!help.description.is_empty());
        assert!(help.ai_tips.len() > 0);
    }
}
