//! # DevIt Output Format System
//!
//! This module provides compression and formatting capabilities for DevIt MCP responses
//! to optimize token usage for AI assistants while maintaining backward compatibility.

use crate::core::{DevItError, DevItResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Output format options for DevIt MCP responses
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum OutputFormat {
    /// Standard verbose JSON format (baseline, 100% tokens)
    Json,
    /// Abbreviated JSON with shortened field names (target: 60% token reduction)
    Compact,
    /// Pipe-delimited tabular format (target: 80% token reduction)
    Table,
    /// Binary MessagePack format (future extension)
    MessagePack,
}

impl Default for OutputFormat {
    fn default() -> Self {
        OutputFormat::Json
    }
}

impl OutputFormat {
    /// Parse output format from string
    pub fn from_str(s: &str) -> DevItResult<Self> {
        match s.to_lowercase().as_str() {
            "json" => Ok(OutputFormat::Json),
            "compact" => Ok(OutputFormat::Compact),
            "table" => Ok(OutputFormat::Table),
            "messagepack" | "msgpack" => Ok(OutputFormat::MessagePack),
            _ => Err(DevItError::InvalidFormat {
                format: s.to_string(),
                supported: vec![
                    "json".to_string(),
                    "compact".to_string(),
                    "table".to_string(),
                ],
            }),
        }
    }

    /// Get expected compression ratio for this format
    pub fn expected_compression_ratio(&self) -> f32 {
        match self {
            OutputFormat::Json => 1.0,         // Baseline
            OutputFormat::Compact => 0.4,      // 60% reduction
            OutputFormat::Table => 0.2,        // 80% reduction
            OutputFormat::MessagePack => 0.15, // 85% reduction (future)
        }
    }

    /// Get format description for help systems
    pub fn description(&self) -> &'static str {
        match self {
            OutputFormat::Json => "Standard verbose JSON format with full field names",
            OutputFormat::Compact => "Abbreviated JSON with shortened field names (60% smaller)",
            OutputFormat::Table => "Pipe-delimited tabular format (80% smaller)",
            OutputFormat::MessagePack => "Binary MessagePack format (85% smaller, future)",
        }
    }
}

/// Trait for data structures that can be compressed to different output formats
pub trait Compressible {
    /// Convert the structure to the specified format
    fn to_format(&self, format: &OutputFormat) -> DevItResult<String>;

    /// Get actual compression ratio achieved for this format
    fn get_compression_ratio(&self, format: &OutputFormat) -> DevItResult<f32>;

    /// Get field mappings for compact format (long_name -> short_name)
    fn get_field_mappings() -> HashMap<String, String>;

    /// Get available fields that can be filtered
    fn get_available_fields() -> Vec<String>;

    /// Convert to compact JSON with abbreviated field names
    fn to_compact_json(&self) -> DevItResult<String> {
        self.to_format(&OutputFormat::Compact)
    }

    /// Convert to table format (pipe-delimited)
    fn to_table_format(&self) -> DevItResult<String> {
        self.to_format(&OutputFormat::Table)
    }
}

/// Field mapping system for abbreviating JSON field names
pub struct FieldMappings;

impl FieldMappings {
    /// Core file operation field mappings
    pub const FILE_MAPPINGS: &'static [(&'static str, &'static str)] = &[
        // FileEntry fields
        ("name", "n"),
        ("path", "f"),       // f for file
        ("entry_type", "t"), // t for type
        ("size", "s"),
        ("modified", "m"),
        ("permissions", "p"),
        // FilePermissions fields
        ("readable", "r"),
        ("writable", "w"),
        ("executable", "x"),
        // FileContent fields
        ("content", "c"),
        ("lines", "l"),
        ("encoding", "e"),
        // SearchMatch fields
        ("file", "sf"), // sf for search file
        ("line_number", "ln"),
        ("line", "sl"), // sl for search line
        ("context_before", "cb"),
        ("context_after", "ca"),
        // SearchResults fields
        ("pattern", "pat"),
        ("files_searched", "fs"),
        ("total_matches", "tm"),
        ("matches", "mt"),   // mt for matches
        ("truncated", "tc"), // tc for truncated
        // ProjectStructure fields
        ("root", "rt"), // rt for root
        ("project_type", "pt"),
        ("tree", "tr"), // tr for tree
        ("total_files", "tf"),
        ("total_dirs", "td"),
        // TreeNode fields
        ("node_type", "nt"),
        ("children", "ch"),
        // Common abbreviations
        ("directory", "dir"),
        ("symlink", "sym"),
        ("unknown", "unk"),
    ];

    /// Get mapping from long name to short name
    pub fn get_mapping() -> HashMap<String, String> {
        Self::FILE_MAPPINGS
            .iter()
            .map(|(long, short)| (long.to_string(), short.to_string()))
            .collect()
    }

    /// Get reverse mapping from short name to long name
    pub fn get_reverse_mapping() -> HashMap<String, String> {
        Self::FILE_MAPPINGS
            .iter()
            .map(|(long, short)| (short.to_string(), long.to_string()))
            .collect()
    }

    /// Apply field mappings to a JSON string
    pub fn apply_mappings(json_str: &str) -> DevItResult<String> {
        let mappings = Self::get_mapping();
        let mut result = json_str.to_string();

        for (long, short) in mappings {
            let pattern = format!("\"{}\":", long);
            let replacement = format!("\"{}\":", short);
            result = result.replace(&pattern, &replacement);
        }

        Ok(result)
    }
}

/// Utility functions for format conversion
pub struct FormatUtils;

impl FormatUtils {
    /// Calculate actual compression ratio
    pub fn calculate_compression_ratio(original: &str, compressed: &str) -> f32 {
        if original.is_empty() {
            return 1.0;
        }
        compressed.len() as f32 / original.len() as f32
    }

    /// Convert JSON to table format with pipe delimiters
    pub fn json_to_table_format(
        json_value: &serde_json::Value,
        headers: &[&str],
    ) -> DevItResult<String> {
        let mut result = String::new();

        // Add headers
        result.push_str(&headers.join("|"));
        result.push('\n');

        match json_value {
            serde_json::Value::Array(arr) => {
                for item in arr {
                    let mut row = Vec::new();
                    for header in headers {
                        let value = item
                            .get(header)
                            .map(|v| format_table_value(v))
                            .unwrap_or_else(|| "".to_string());
                        row.push(value);
                    }
                    result.push_str(&row.join("|"));
                    result.push('\n');
                }
            }
            serde_json::Value::Object(_) => {
                let mut row = Vec::new();
                for header in headers {
                    let value = json_value
                        .get(header)
                        .map(|v| format_table_value(v))
                        .unwrap_or_else(|| "".to_string());
                    row.push(value);
                }
                result.push_str(&row.join("|"));
                result.push('\n');
            }
            _ => {
                return Err(DevItError::InvalidFormat {
                    format: "table".to_string(),
                    supported: vec!["array".to_string(), "object".to_string()],
                });
            }
        }

        Ok(result)
    }

    /// Estimate token count for a string (rough approximation)
    pub fn estimate_token_count(text: &str) -> usize {
        // Rough approximation: 1 token ≈ 4 characters for text
        // JSON overhead, punctuation, etc. counted
        (text.len() as f32 / 3.5) as usize
    }
}

/// Format a JSON value for table display
fn format_table_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.replace('|', "\\|"), // Escape pipes
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Array(_) => "[array]".to_string(),
        serde_json::Value::Object(_) => "[object]".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_output_format_from_str() {
        assert_eq!(OutputFormat::from_str("json").unwrap(), OutputFormat::Json);
        assert_eq!(
            OutputFormat::from_str("compact").unwrap(),
            OutputFormat::Compact
        );
        assert_eq!(
            OutputFormat::from_str("table").unwrap(),
            OutputFormat::Table
        );
        assert!(OutputFormat::from_str("invalid").is_err());
    }

    #[test]
    fn test_compression_ratios() {
        assert_eq!(OutputFormat::Json.expected_compression_ratio(), 1.0);
        assert_eq!(OutputFormat::Compact.expected_compression_ratio(), 0.4);
        assert_eq!(OutputFormat::Table.expected_compression_ratio(), 0.2);
    }

    #[test]
    fn test_field_mappings() {
        let mappings = FieldMappings::get_mapping();
        assert_eq!(mappings.get("path"), Some(&"f".to_string()));
        assert_eq!(mappings.get("size"), Some(&"s".to_string()));
        assert_eq!(mappings.get("content"), Some(&"c".to_string()));
    }

    #[test]
    fn test_apply_mappings() {
        let json = r#"{"path": "/test", "size": 1024}"#;
        let compressed = FieldMappings::apply_mappings(json).unwrap();
        assert_eq!(compressed, r#"{"f": "/test", "s": 1024}"#);
    }

    #[test]
    fn test_compression_ratio_calculation() {
        let original = "hello world";
        let compressed = "hello";
        let ratio = FormatUtils::calculate_compression_ratio(original, compressed);
        assert!((ratio - 0.45).abs() < 0.01); // 5/11 ≈ 0.45
    }

    #[test]
    fn test_token_estimation() {
        let text = "hello world test";
        let tokens = FormatUtils::estimate_token_count(text);
        assert!(tokens > 0);
        assert!(tokens < text.len()); // Should be less than character count
    }
}
