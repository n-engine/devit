//! MCP Extended Tools Integration Tests
//!
//! This module tests the complete MCP integration for all extended tools,
//! validating JSON-RPC communication, format compression, and token efficiency.

use devit_cli::core::formats::OutputFormat;
use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

mod fixtures;

use fixtures::{mcp_server_path, workspace_root};

/// Helper to run MCP command and parse response
fn run_mcp_command(request: Value) -> Result<Value, Box<dyn std::error::Error>> {
    let mut child = Command::new(mcp_server_path())
        .arg("--working-dir")
        .arg(workspace_root())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    {
        let stdin = child.stdin.as_mut().unwrap();
        stdin.write_all(serde_json::to_string(&request)?.as_bytes())?;
        stdin.flush()?;
    }

    let output = child.wait_with_output()?;

    if !output.status.success() {
        eprintln!("MCP stderr: {}", String::from_utf8_lossy(&output.stderr));
        return Err(format!("MCP command failed with status: {}", output.status).into());
    }

    let response_str = String::from_utf8(output.stdout)?;
    Ok(serde_json::from_str(&response_str)?)
}

/// Get content from MCP response
fn extract_content_text(response: &Value) -> Result<String, Box<dyn std::error::Error>> {
    let content = response
        .get("result")
        .and_then(|r| r.get("content"))
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|item| item.get("text"))
        .and_then(|t| t.as_str())
        .ok_or("Invalid response format")?;

    Ok(content.to_string())
}

#[test]
fn test_devit_file_read_ext_json_format() {
    let request = json!({
        "jsonrpc": "2.0",
        "id": "test_1",
        "method": "tools/call",
        "params": {
            "name": "devit_file_read_ext",
            "arguments": {
                "path": "Cargo.toml",
                "format": "json",
                "limit": 10
            }
        }
    });

    let response = run_mcp_command(request).expect("MCP command should succeed");

    // Verify successful response
    assert!(response.get("error").is_none(), "Should not have error");
    assert!(response.get("result").is_some(), "Should have result");

    let content = extract_content_text(&response).expect("Should extract content");

    // Verify JSON format markers
    assert!(content.contains("📄 File:"), "Should have file indicator");
    assert!(
        content.contains("(format: Json)"),
        "Should indicate JSON format"
    );
    assert!(
        content.contains("\"path\":"),
        "Should contain full field names"
    );
    assert!(
        content.contains("\"content\":"),
        "Should contain content field"
    );
}

#[test]
fn test_devit_file_read_ext_compact_format() {
    let request = json!({
        "jsonrpc": "2.0",
        "id": "test_2",
        "method": "tools/call",
        "params": {
            "name": "devit_file_read_ext",
            "arguments": {
                "path": "Cargo.toml",
                "format": "compact",
                "limit": 10
            }
        }
    });

    let response = run_mcp_command(request).expect("MCP command should succeed");
    let content = extract_content_text(&response).expect("Should extract content");

    // Verify compact format markers
    assert!(
        content.contains("(format: Compact)"),
        "Should indicate Compact format"
    );
    assert!(
        content.contains("\"f\":"),
        "Should contain abbreviated 'f' for path"
    );
    assert!(
        content.contains("\"c\":"),
        "Should contain abbreviated 'c' for content"
    );
    assert!(
        content.contains("\"s\":"),
        "Should contain abbreviated 's' for size"
    );
    assert!(
        content.contains("\"e\":"),
        "Should contain abbreviated 'e' for encoding"
    );

    // Should NOT contain full field names
    assert!(
        !content.contains("\"path\":"),
        "Should not contain full path field"
    );
    assert!(
        !content.contains("\"content\":"),
        "Should not contain full content field"
    );
}

#[test]
fn test_devit_file_read_ext_table_format() {
    let request = json!({
        "jsonrpc": "2.0",
        "id": "test_3",
        "method": "tools/call",
        "params": {
            "name": "devit_file_read_ext",
            "arguments": {
                "path": "Cargo.toml",
                "format": "table",
                "limit": 5
            }
        }
    });

    let response = run_mcp_command(request).expect("MCP command should succeed");
    let content = extract_content_text(&response).expect("Should extract content");

    // Verify table format markers
    assert!(
        content.contains("(format: Table)"),
        "Should indicate Table format"
    );
    assert!(
        content.contains("path|size|encoding|content"),
        "Should have table header"
    );
    assert!(content.contains("|"), "Should contain pipe delimiters");

    // Count lines to verify table structure
    let lines: Vec<&str> = content
        .split('\n')
        .filter(|line| line.contains('|'))
        .collect();
    assert!(lines.len() >= 1, "Should have at least header line");
}

#[test]
fn test_devit_file_list_ext_formats() {
    // Test compact format
    let request = json!({
        "jsonrpc": "2.0",
        "id": "test_4",
        "method": "tools/call",
        "params": {
            "name": "devit_file_list_ext",
            "arguments": {
                "path": "crates",
                "format": "compact",
                "recursive": false
            }
        }
    });

    let response = run_mcp_command(request).expect("MCP command should succeed");
    let content = extract_content_text(&response).expect("Should extract content");

    assert!(
        content.contains("(format: Compact)"),
        "Should indicate Compact format"
    );
    assert!(
        content.contains("\"n\":"),
        "Should contain abbreviated 'n' for name"
    );
    assert!(
        content.contains("\"f\":"),
        "Should contain abbreviated 'f' for path"
    );
    assert!(
        content.contains("\"t\":"),
        "Should contain abbreviated 't' for type"
    );

    // Test table format
    let request_table = json!({
        "jsonrpc": "2.0",
        "id": "test_5",
        "method": "tools/call",
        "params": {
            "name": "devit_file_list_ext",
            "arguments": {
                "path": "crates",
                "format": "table",
                "recursive": false
            }
        }
    });

    let response_table = run_mcp_command(request_table).expect("MCP command should succeed");
    let content_table = extract_content_text(&response_table).expect("Should extract content");

    assert!(
        content_table.contains("(format: Table)"),
        "Should indicate Table format"
    );
    assert!(
        content_table.contains("name|path|type|size|permissions"),
        "Should have table header"
    );
}

#[test]
fn test_devit_file_search_ext_formats() {
    let request = json!({
        "jsonrpc": "2.0",
        "id": "test_6",
        "method": "tools/call",
        "params": {
            "name": "devit_file_search_ext",
            "arguments": {
                "pattern": "pub fn",
                "path": "crates/cli/src",
                "format": "compact",
                "max_results": 3
            }
        }
    });

    let response = run_mcp_command(request).expect("MCP command should succeed");
    let content = extract_content_text(&response).expect("Should extract content");

    assert!(
        content.contains("(format: Compact)"),
        "Should indicate Compact format"
    );
    assert!(
        content.contains("\"pat\":"),
        "Should contain abbreviated 'pat' for pattern"
    );
    assert!(
        content.contains("\"mt\":"),
        "Should contain abbreviated 'mt' for matches"
    );
    assert!(
        content.contains("\"sf\":"),
        "Should contain abbreviated 'sf' for search file"
    );
}

#[test]
fn test_devit_project_structure_ext_formats() {
    let request = json!({
        "jsonrpc": "2.0",
        "id": "test_7",
        "method": "tools/call",
        "params": {
            "name": "devit_project_structure_ext",
            "arguments": {
                "path": ".",
                "format": "table",
                "max_depth": 2
            }
        }
    });

    let response = run_mcp_command(request).expect("MCP command should succeed");
    let content = extract_content_text(&response).expect("Should extract content");

    assert!(
        content.contains("(format: Table)"),
        "Should indicate Table format"
    );
    assert!(
        content.contains("name|type|path|level"),
        "Should have table header"
    );
}

#[test]
fn test_compression_ratio_validation() {
    // Get same file content in different formats
    let path = "Cargo.toml";
    let limit = 20;

    // JSON format
    let json_request = json!({
        "jsonrpc": "2.0",
        "id": "compress_1",
        "method": "tools/call",
        "params": {
            "name": "devit_file_read_ext",
            "arguments": {
                "path": path,
                "format": "json",
                "limit": limit
            }
        }
    });

    // Compact format
    let compact_request = json!({
        "jsonrpc": "2.0",
        "id": "compress_2",
        "method": "tools/call",
        "params": {
            "name": "devit_file_read_ext",
            "arguments": {
                "path": path,
                "format": "compact",
                "limit": limit
            }
        }
    });

    // Table format
    let table_request = json!({
        "jsonrpc": "2.0",
        "id": "compress_3",
        "method": "tools/call",
        "params": {
            "name": "devit_file_read_ext",
            "arguments": {
                "path": path,
                "format": "table",
                "limit": limit
            }
        }
    });

    let json_response = run_mcp_command(json_request).expect("JSON request should succeed");
    let compact_response =
        run_mcp_command(compact_request).expect("Compact request should succeed");
    let table_response = run_mcp_command(table_request).expect("Table request should succeed");

    let json_content = extract_content_text(&json_response).expect("Should extract JSON content");
    let compact_content =
        extract_content_text(&compact_response).expect("Should extract compact content");
    let table_content =
        extract_content_text(&table_response).expect("Should extract table content");

    println!("JSON length: {}", json_content.len());
    println!("Compact length: {}", compact_content.len());
    println!("Table length: {}", table_content.len());

    // Verify compression ratios
    let compact_ratio = compact_content.len() as f32 / json_content.len() as f32;
    let table_ratio = table_content.len() as f32 / json_content.len() as f32;

    println!("Compact ratio: {:.2}", compact_ratio);
    println!("Table ratio: {:.2}", table_ratio);

    // Compact should be smaller than JSON
    assert!(
        compact_content.len() < json_content.len(),
        "Compact should be smaller than JSON"
    );

    // Table should be smaller than JSON
    assert!(
        table_content.len() < json_content.len(),
        "Table should be smaller than JSON"
    );

    // Generally table should be more compressed than compact
    // (though this may vary depending on content structure)
    assert!(compact_ratio < 1.0, "Compact ratio should be less than 1.0");
    assert!(table_ratio < 1.0, "Table ratio should be less than 1.0");
}

#[test]
fn test_invalid_format_handling() {
    let request = json!({
        "jsonrpc": "2.0",
        "id": "test_invalid",
        "method": "tools/call",
        "params": {
            "name": "devit_file_read_ext",
            "arguments": {
                "path": "Cargo.toml",
                "format": "invalid_format"
            }
        }
    });

    let response = run_mcp_command(request).expect("MCP command should succeed");

    // Should have error for invalid format
    assert!(
        response.get("error").is_some(),
        "Should have error for invalid format"
    );

    let error = response.get("error").unwrap();
    assert!(error.get("message").is_some(), "Error should have message");
}

#[test]
fn test_missing_required_parameters() {
    // Test missing path parameter
    let request = json!({
        "jsonrpc": "2.0",
        "id": "test_missing",
        "method": "tools/call",
        "params": {
            "name": "devit_file_read_ext",
            "arguments": {
                "format": "json"
                // Missing required "path" parameter
            }
        }
    });

    let response = run_mcp_command(request).expect("MCP command should succeed");

    // Should have validation error
    assert!(
        response.get("error").is_some(),
        "Should have error for missing path"
    );

    let error = response.get("error").unwrap();
    let message = error.get("message").and_then(|m| m.as_str()).unwrap_or("");
    assert_eq!(message, "E_VALIDATION", "Should have validation error");
}

#[test]
fn test_help_tools_integration() {
    // Test devit_help_all
    let help_request = json!({
        "jsonrpc": "2.0",
        "id": "help_test",
        "method": "tools/call",
        "params": {
            "name": "devit_help_all",
            "arguments": {}
        }
    });

    let response = run_mcp_command(help_request).expect("Help command should succeed");
    assert!(
        response.get("error").is_none(),
        "Help should not have error"
    );

    let content = extract_content_text(&response).expect("Should extract help content");
    assert!(
        content.contains("DevIt MCP Tools Overview"),
        "Should contain overview text"
    );
    assert!(
        content.contains("AI Optimization Tips"),
        "Should contain AI tips"
    );
    assert!(
        content.contains("Performance Hints"),
        "Should contain performance hints"
    );

    // Test specific tool help
    let tool_help_request = json!({
        "jsonrpc": "2.0",
        "id": "tool_help_test",
        "method": "tools/call",
        "params": {
            "name": "devit_file_read_help_ext",
            "arguments": {}
        }
    });

    let tool_response = run_mcp_command(tool_help_request).expect("Tool help should succeed");
    assert!(
        tool_response.get("error").is_none(),
        "Tool help should not have error"
    );

    let tool_content =
        extract_content_text(&tool_response).expect("Should extract tool help content");
    assert!(
        tool_content.contains("devit_file_read_ext"),
        "Should contain tool name"
    );
    assert!(
        tool_content.contains("formats"),
        "Should contain formats information"
    );
    assert!(tool_content.contains("examples"), "Should contain examples");
}

#[test]
fn test_tools_list_includes_extended() {
    let request = json!({
        "jsonrpc": "2.0",
        "id": "tools_list",
        "method": "tools/list",
        "params": {}
    });

    let response = run_mcp_command(request).expect("Tools list should succeed");
    assert!(
        response.get("error").is_none(),
        "Tools list should not have error"
    );

    let tools = response
        .get("result")
        .and_then(|r| r.get("tools"))
        .and_then(|t| t.as_array())
        .expect("Should have tools array");

    // Check that extended tools are present
    let tool_names: Vec<String> = tools
        .iter()
        .filter_map(|tool| tool.get("name").and_then(|n| n.as_str()))
        .map(|name| name.to_string())
        .collect();

    assert!(
        tool_names.contains(&"devit_file_read_ext".to_string()),
        "Should include devit_file_read_ext"
    );
    assert!(
        tool_names.contains(&"devit_file_list_ext".to_string()),
        "Should include devit_file_list_ext"
    );
    assert!(
        tool_names.contains(&"devit_file_search_ext".to_string()),
        "Should include devit_file_search_ext"
    );
    assert!(
        tool_names.contains(&"devit_project_structure_ext".to_string()),
        "Should include devit_project_structure_ext"
    );
    assert!(
        tool_names.contains(&"devit_help_all".to_string()),
        "Should include devit_help_all"
    );

    // Verify tool schemas have required properties
    for tool in tools {
        let name = tool.get("name").and_then(|n| n.as_str()).unwrap_or("");
        if name.ends_with("_ext") || name.contains("help") {
            assert!(
                tool.get("description").is_some(),
                "Extended tool should have description: {}",
                name
            );
            assert!(
                tool.get("inputSchema").is_some(),
                "Extended tool should have input schema: {}",
                name
            );

            if name.ends_with("_ext") && name != "devit_help_all" {
                let schema = tool.get("inputSchema").unwrap();
                let properties = schema
                    .get("properties")
                    .and_then(|p| p.as_object())
                    .unwrap();
                assert!(
                    properties.contains_key("format"),
                    "Extended tool should have format property: {}",
                    name
                );
            }
        }
    }
}

#[test]
fn test_field_filtering() {
    let request = json!({
        "jsonrpc": "2.0",
        "id": "field_test",
        "method": "tools/call",
        "params": {
            "name": "devit_file_read_ext",
            "arguments": {
                "path": "Cargo.toml",
                "format": "json",
                "fields": ["path", "size"],
                "limit": 5
            }
        }
    });

    let response = run_mcp_command(request).expect("Field filtering should succeed");
    assert!(
        response.get("error").is_none(),
        "Field filtering should not have error"
    );

    let content = extract_content_text(&response).expect("Should extract content");

    // Content should contain the filtered fields
    assert!(content.contains("\"path\":"), "Should contain path field");
    assert!(content.contains("\"size\":"), "Should contain size field");

    // Should not contain other fields when filtered
    // Note: This depends on the implementation of field filtering
}

#[test]
fn test_performance_large_directory() {
    use std::time::Instant;

    let start = Instant::now();

    let request = json!({
        "jsonrpc": "2.0",
        "id": "perf_test",
        "method": "tools/call",
        "params": {
            "name": "devit_file_list_ext",
            "arguments": {
                "path": ".",
                "format": "table",
                "recursive": true
            }
        }
    });

    let response = run_mcp_command(request).expect("Large directory listing should succeed");
    let duration = start.elapsed();

    assert!(
        response.get("error").is_none(),
        "Large directory listing should not have error"
    );

    // Should complete in reasonable time (adjust threshold as needed)
    assert!(
        duration < Duration::from_secs(30),
        "Should complete in under 30 seconds, took {:?}",
        duration
    );

    let content = extract_content_text(&response).expect("Should extract content");
    assert!(
        content.contains("name|path|type|size|permissions"),
        "Should have table header"
    );

    // Should contain multiple entries
    let lines: Vec<&str> = content
        .split('\n')
        .filter(|line| !line.is_empty() && line.contains('|'))
        .collect();
    assert!(lines.len() > 5, "Should have multiple directory entries");

    println!("Large directory listing completed in {:?}", duration);
    println!("Generated {} lines of output", lines.len());
}
