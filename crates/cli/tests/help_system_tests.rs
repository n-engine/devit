//! Help System Tests for DevIt MCP Format System
//!
//! This module tests the help system capabilities and validates the
//! auto-generated documentation for all MCP tools.

use devit_cli::core::formats::OutputFormat;
use devit_cli::core::help_system::{HelpSystem, ToolHelp, UsageExample};
use serde_json::Value;

#[test]
fn test_help_content_generation() {
    let mut help_system = HelpSystem::new();

    // Test that all main tools have help content
    let tools = vec![
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

    for tool in tools {
        let help = help_system.get_tool_help(tool).unwrap();

        // Validate required fields
        assert!(
            !help.tool_name.is_empty(),
            "Tool name should not be empty for {}",
            tool
        );
        assert!(
            !help.description.is_empty(),
            "Description should not be empty for {}",
            tool
        );
        assert!(
            !help.examples.is_empty(),
            "Examples should not be empty for {}",
            tool
        );
        assert!(
            !help.ai_tips.is_empty(),
            "AI tips should not be empty for {}",
            tool
        );
        assert!(
            !help.formats.is_empty(),
            "Formats should not be empty for {}",
            tool
        );
    }
}

#[test]
fn test_help_json_structure_validation() {
    let mut help_system = HelpSystem::new();
    let help = help_system.get_tool_help("devit_file_read_ext").unwrap();

    // Validate JSON serialization
    let json_str = serde_json::to_string(&help).unwrap();
    let parsed: Value = serde_json::from_str(&json_str).unwrap();

    // Check required fields exist in JSON
    assert!(parsed.get("tool_name").is_some());
    assert!(parsed.get("description").is_some());
    assert!(parsed.get("formats").is_some());
    assert!(parsed.get("examples").is_some());
    assert!(parsed.get("ai_tips").is_some());
    assert!(parsed.get("performance_hints").is_some());

    // Validate examples structure
    let examples = parsed.get("examples").unwrap().as_array().unwrap();
    assert!(!examples.is_empty());

    for example in examples {
        assert!(example.get("use_case").is_some());
        assert!(example.get("command").is_some());
        assert!(example.get("output_sample").is_some());
        // token_savings can be null for baseline examples
    }
}

#[test]
fn test_extended_tools_have_compression_formats() {
    let mut help_system = HelpSystem::new();

    let ext_tools = vec![
        "devit_file_read_ext",
        "devit_file_list_ext",
        "devit_file_search_ext",
        "devit_project_structure_ext",
    ];

    for tool in ext_tools {
        let help = help_system.get_tool_help(tool).unwrap();

        // Extended tools should have all three formats
        assert!(
            help.formats.contains_key("json"),
            "{} should have json format",
            tool
        );
        assert!(
            help.formats.contains_key("compact"),
            "{} should have compact format",
            tool
        );
        assert!(
            help.formats.contains_key("table"),
            "{} should have table format",
            tool
        );

        // Should have examples with token savings
        let has_savings_example = help.examples.iter().any(|ex| ex.token_savings.is_some());
        assert!(
            has_savings_example,
            "{} should have examples with token savings",
            tool
        );
    }
}

#[test]
fn test_baseline_tools_format_structure() {
    let mut help_system = HelpSystem::new();

    let baseline_tools = vec![
        "devit_file_read",
        "devit_file_list",
        "devit_file_search",
        "devit_project_structure",
        "devit_pwd",
    ];

    for tool in baseline_tools {
        let help = help_system.get_tool_help(tool).unwrap();

        // Baseline tools should primarily have JSON format
        assert!(
            help.formats.contains_key("json"),
            "{} should have json format",
            tool
        );

        // Examples should not have token savings (they are the baseline)
        for example in &help.examples {
            assert!(
                example.token_savings.is_none(),
                "{} baseline examples should not have token savings",
                tool
            );
        }
    }
}

#[test]
fn test_token_savings_calculation_accuracy() {
    let help_system = HelpSystem::new();

    // Test with realistic JSON output
    let json_output = r#"{
        "path": "/home/user/project/src/main.rs",
        "content": "fn main() {\n    println!(\"Hello, world!\");\n    let x = 42;\n    process_data(x);\n}",
        "size": 1024,
        "lines": [
            "1: fn main() {",
            "2:     println!(\"Hello, world!\");",
            "3:     let x = 42;",
            "4:     process_data(x);",
            "5: }"
        ],
        "encoding": "utf-8"
    }"#;

    let compact_savings = help_system.calculate_token_savings(json_output, &OutputFormat::Compact);
    let table_savings = help_system.calculate_token_savings(json_output, &OutputFormat::Table);
    let json_savings = help_system.calculate_token_savings(json_output, &OutputFormat::Json);

    // Validate savings ranges
    assert_eq!(json_savings, 0.0, "JSON baseline should have 0 savings");
    assert!(
        compact_savings > 0.3 && compact_savings < 0.8,
        "Compact savings should be between 30-80%, got {}",
        compact_savings
    );
    assert!(
        table_savings > 0.6 && table_savings < 0.9,
        "Table savings should be between 60-90%, got {}",
        table_savings
    );
    assert!(
        table_savings > compact_savings,
        "Table should save more than compact"
    );
}

#[test]
fn test_usage_examples_are_valid_json() {
    let mut help_system = HelpSystem::new();
    let help = help_system.get_tool_help("devit_file_read_ext").unwrap();

    for example in &help.examples {
        // Validate that command field is valid JSON
        let _parsed_command: Value = serde_json::from_value(example.command.clone()).expect(
            &format!("Command should be valid JSON: {:?}", example.command),
        );

        // Validate use case is meaningful
        assert!(!example.use_case.is_empty(), "Use case should not be empty");
        assert!(
            example.use_case.len() > 10,
            "Use case should be descriptive"
        );

        // Validate output sample is not empty
        assert!(
            !example.output_sample.is_empty(),
            "Output sample should not be empty"
        );
    }
}

#[test]
fn test_ai_tips_quality() {
    let mut help_system = HelpSystem::new();
    let help = help_system.get_tool_help("devit_file_read_ext").unwrap();

    // AI tips should be actionable and specific
    for tip in &help.ai_tips {
        assert!(!tip.is_empty(), "AI tip should not be empty");
        assert!(tip.len() > 20, "AI tip should be substantial: {}", tip);

        // Should contain actionable language
        let actionable_words = [
            "use", "set", "prefer", "combine", "switch", "always", "avoid", "optimal", "ideal",
            "best",
        ];
        let has_actionable = actionable_words
            .iter()
            .any(|word| tip.to_lowercase().contains(word));
        assert!(
            has_actionable,
            "AI tip should contain actionable language: {}",
            tip
        );
    }
}

#[test]
fn test_performance_hints_specificity() {
    let mut help_system = HelpSystem::new();
    let help = help_system.get_tool_help("devit_file_read_ext").unwrap();

    for hint in &help.performance_hints {
        assert!(!hint.is_empty(), "Performance hint should not be empty");

        // Performance hints should mention specific metrics or improvements
        let performance_indicators = [
            "faster",
            "efficient",
            "token",
            "memory",
            "time",
            "reduce",
            "optimize",
            "batch",
        ];
        let has_performance = performance_indicators
            .iter()
            .any(|word| hint.to_lowercase().contains(word));
        assert!(
            has_performance,
            "Performance hint should mention performance aspects: {}",
            hint
        );
    }
}

#[test]
fn test_all_tools_overview() {
    let mut help_system = HelpSystem::new();
    let overview = help_system.get_all_tools_help().unwrap();

    assert_eq!(overview.tool_name, "devit_help_all");
    assert!(!overview.description.is_empty());
    assert!(!overview.ai_tips.is_empty());
    assert!(!overview.performance_hints.is_empty());

    // Should have comprehensive AI guidance
    let overview_tips = overview.ai_tips.join(" ").to_lowercase();
    assert!(
        overview_tips.contains("_ext"),
        "Should recommend _ext versions"
    );
    assert!(
        overview_tips.contains("compact"),
        "Should mention compact format"
    );
    assert!(
        overview_tips.contains("table"),
        "Should mention table format"
    );
    assert!(
        overview_tips.contains("token"),
        "Should mention token efficiency"
    );
}

#[test]
fn test_help_caching_performance() {
    let mut help_system = HelpSystem::new();

    // Time the first call (should generate)
    let start = std::time::Instant::now();
    let _help1 = help_system.get_tool_help("devit_file_read_ext").unwrap();
    let first_call_duration = start.elapsed();

    // Time the second call (should use cache)
    let start = std::time::Instant::now();
    let _help2 = help_system.get_tool_help("devit_file_read_ext").unwrap();
    let second_call_duration = start.elapsed();

    // Second call should be significantly faster
    assert!(
        second_call_duration < first_call_duration / 2,
        "Cached call should be much faster: first={:?}, second={:?}",
        first_call_duration,
        second_call_duration
    );
}

#[test]
fn test_format_descriptions_accuracy() {
    let mut help_system = HelpSystem::new();
    let help = help_system.get_tool_help("devit_file_read_ext").unwrap();

    // Validate format descriptions match expected compression ratios
    let json_desc = help.formats.get("json").unwrap();
    let compact_desc = help.formats.get("compact").unwrap();
    let table_desc = help.formats.get("table").unwrap();

    assert!(
        json_desc.to_lowercase().contains("baseline")
            || json_desc.to_lowercase().contains("standard")
    );
    assert!(compact_desc.contains("60%") && compact_desc.to_lowercase().contains("reduction"));
    assert!(table_desc.contains("80%") && table_desc.to_lowercase().contains("reduction"));
}

#[test]
fn test_schema_integration_readiness() {
    let mut help_system = HelpSystem::new();

    // Test that we can register schemas (for future MCP integration)
    let sample_schema = serde_json::json!({
        "type": "object",
        "properties": {
            "path": {"type": "string"},
            "format": {"type": "string", "enum": ["json", "compact", "table"]},
            "limit": {"type": "integer"}
        }
    });

    help_system.register_tool_schema("devit_file_read_ext", sample_schema);

    // Should not crash and should be ready for future schema-based help generation
    let help = help_system.get_tool_help("devit_file_read_ext").unwrap();
    assert!(!help.tool_name.is_empty());
}

#[test]
fn test_multilingual_readiness() {
    let mut help_system = HelpSystem::new();
    let help = help_system.get_tool_help("devit_file_read_ext").unwrap();

    // Current implementation is English, but structure should support multilingual
    // This test ensures the structure is ready for i18n expansion

    assert!(!help.description.is_empty());
    assert!(
        help.description
            .chars()
            .all(|c| c.is_ascii() || c.is_whitespace() || "(),-".contains(c)),
        "Description should be in ASCII for current English implementation"
    );

    // AI tips should be practical and clear
    for tip in &help.ai_tips {
        assert!(
            tip.contains("'")
                || tip.contains("\"")
                || tip.contains("format")
                || tip.contains("use")
                || tip.contains("--")
                || tip.contains("file"),
            "AI tips should contain quoted format names or action words: {}",
            tip
        );
    }
}
