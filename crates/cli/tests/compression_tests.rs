//! Compression Tests for DevIt MCP Format System
//!
//! This module tests the compression capabilities and format conversions
//! to ensure they meet the target compression ratios and round-trip correctly.

use devit_cli::core::file_ops::{
    FileContent, FileEntry, FilePermissions, FileType, SearchMatch, SearchResults,
};
use devit_cli::core::formats::{Compressible, FormatUtils, OutputFormat};
use std::path::PathBuf;
use std::time::SystemTime;

#[test]
fn test_format_compression_ratios_file_entry() {
    // Create a sample FileEntry with realistic data
    let file_entry = FileEntry {
        name: "test_file.rs".to_string(),
        path: PathBuf::from("/home/user/project/src/test_file.rs"),
        entry_type: FileType::File,
        size: Some(1024),
        modified: Some(SystemTime::now()),
        permissions: FilePermissions {
            readable: true,
            writable: true,
            executable: false,
        },
    };

    // Test JSON format (baseline)
    let json_output = file_entry.to_format(&OutputFormat::Json).unwrap();
    let json_ratio = file_entry
        .get_compression_ratio(&OutputFormat::Json)
        .unwrap();
    assert!(
        (json_ratio - 1.0).abs() < 0.01,
        "JSON ratio should be ~1.0, got {}",
        json_ratio
    );

    // Test Compact format (target: 60% reduction = 0.4 ratio)
    let compact_output = file_entry.to_format(&OutputFormat::Compact).unwrap();
    let compact_ratio = file_entry
        .get_compression_ratio(&OutputFormat::Compact)
        .unwrap();
    println!(
        "JSON length: {}, Compact length: {}, Ratio: {}",
        json_output.len(),
        compact_output.len(),
        compact_ratio
    );

    // Compact should be significantly smaller
    assert!(
        compact_ratio < 0.8,
        "Compact ratio should be < 0.8, got {}",
        compact_ratio
    );
    assert!(
        compact_output.len() < json_output.len(),
        "Compact should be smaller than JSON"
    );

    // Test Table format (target: 80% reduction = 0.2 ratio)
    let table_output = file_entry.to_format(&OutputFormat::Table).unwrap();
    let table_ratio = file_entry
        .get_compression_ratio(&OutputFormat::Table)
        .unwrap();
    println!(
        "Table length: {}, Ratio: {}",
        table_output.len(),
        table_ratio
    );

    // Table should be the most compressed
    assert!(
        table_ratio < compact_ratio,
        "Table should be more compressed than compact"
    );
    assert!(
        table_output.contains('|'),
        "Table output should contain pipe delimiters"
    );
}

#[test]
fn test_format_compression_ratios_file_list() {
    // Create a list of FileEntry objects
    let file_list = vec![
        FileEntry {
            name: "main.rs".to_string(),
            path: PathBuf::from("/project/src/main.rs"),
            entry_type: FileType::File,
            size: Some(2048),
            modified: Some(SystemTime::now()),
            permissions: FilePermissions {
                readable: true,
                writable: true,
                executable: false,
            },
        },
        FileEntry {
            name: "lib.rs".to_string(),
            path: PathBuf::from("/project/src/lib.rs"),
            entry_type: FileType::File,
            size: Some(1536),
            modified: Some(SystemTime::now()),
            permissions: FilePermissions {
                readable: true,
                writable: false,
                executable: false,
            },
        },
        FileEntry {
            name: "tests".to_string(),
            path: PathBuf::from("/project/tests"),
            entry_type: FileType::Directory,
            size: None,
            modified: Some(SystemTime::now()),
            permissions: FilePermissions {
                readable: true,
                writable: true,
                executable: true,
            },
        },
    ];

    let json_output = file_list.to_format(&OutputFormat::Json).unwrap();
    let compact_output = file_list.to_format(&OutputFormat::Compact).unwrap();
    let table_output = file_list.to_format(&OutputFormat::Table).unwrap();

    println!("List JSON length: {}", json_output.len());
    println!("List Compact length: {}", compact_output.len());
    println!("List Table length: {}", table_output.len());

    // Verify compression progression
    assert!(
        compact_output.len() < json_output.len(),
        "Compact should be smaller than JSON"
    );
    assert!(
        table_output.len() < json_output.len(),
        "Table should be smaller than JSON"
    );

    // Verify table format structure
    let lines: Vec<&str> = table_output.lines().collect();
    assert!(lines.len() >= 2, "Table should have header + data lines");
    assert!(
        lines[0].contains("name|path|type|size|permissions"),
        "Table header should be correct"
    );
}

#[test]
fn test_format_round_trip_consistency() {
    let file_entry = FileEntry {
        name: "round_trip_test.txt".to_string(),
        path: PathBuf::from("/test/round_trip_test.txt"),
        entry_type: FileType::File,
        size: Some(512),
        modified: Some(SystemTime::now()),
        permissions: FilePermissions {
            readable: true,
            writable: true,
            executable: false,
        },
    };

    // Test JSON round trip
    let json_output = file_entry.to_format(&OutputFormat::Json).unwrap();
    let parsed_back: FileEntry = serde_json::from_str(&json_output).unwrap();
    assert_eq!(file_entry.name, parsed_back.name);
    assert_eq!(file_entry.path, parsed_back.path);
    assert_eq!(file_entry.size, parsed_back.size);

    // Test Compact round trip (should be valid JSON after field expansion)
    let compact_output = file_entry.to_format(&OutputFormat::Compact).unwrap();
    assert!(
        compact_output.contains("\"f\":"),
        "Compact should contain abbreviated field 'f' for path"
    );
    assert!(
        compact_output.contains("\"s\":"),
        "Compact should contain abbreviated field 's' for size"
    );
}

#[test]
fn test_search_results_compression() {
    let search_results = SearchResults {
        pattern: "fn test".to_string(),
        path: PathBuf::from("/project/src"),
        files_searched: 10,
        total_matches: 3,
        matches: vec![
            SearchMatch {
                file: PathBuf::from("/project/src/lib.rs"),
                line_number: 42,
                line: "fn test_function() {".to_string(),
                context_before: vec!["// This is a test".to_string()],
                context_after: vec!["    assert!(true);".to_string()],
            },
            SearchMatch {
                file: PathBuf::from("/project/src/main.rs"),
                line_number: 15,
                line: "fn test_main() {".to_string(),
                context_before: vec!["".to_string()],
                context_after: vec!["    println!(\"test\");".to_string()],
            },
        ],
        truncated: false,
    };

    let json_output = search_results.to_format(&OutputFormat::Json).unwrap();
    let compact_output = search_results.to_format(&OutputFormat::Compact).unwrap();
    let table_output = search_results.to_format(&OutputFormat::Table).unwrap();

    println!("Search JSON length: {}", json_output.len());
    println!("Search Compact length: {}", compact_output.len());
    println!("Search Table length: {}", table_output.len());

    // Verify basic compression
    assert!(
        compact_output.len() < json_output.len(),
        "Compact should be smaller"
    );

    // Verify table format structure
    assert!(
        table_output.contains("file|line|match|context"),
        "Table should have correct header"
    );
    assert!(
        table_output.contains("lib.rs"),
        "Table should contain file names"
    );
    assert!(
        table_output.contains("42"),
        "Table should contain line numbers"
    );
}

#[test]
fn test_file_content_compression() {
    let file_content = FileContent {
        path: PathBuf::from("/test/sample.txt"),
        content: "Hello, World!\nThis is a test file.\nWith multiple lines of content.".to_string(),
        size: 65,
        lines: Some(vec![
            "1: Hello, World!".to_string(),
            "2: This is a test file.".to_string(),
            "3: With multiple lines of content.".to_string(),
        ]),
        encoding: "utf-8".to_string(),
    };

    let json_output = file_content.to_format(&OutputFormat::Json).unwrap();
    let compact_output = file_content.to_format(&OutputFormat::Compact).unwrap();
    let table_output = file_content.to_format(&OutputFormat::Table).unwrap();

    println!("Content JSON length: {}", json_output.len());
    println!("Content Compact length: {}", compact_output.len());
    println!("Content Table length: {}", table_output.len());

    // Verify compression
    assert!(
        compact_output.len() < json_output.len(),
        "Compact should be smaller"
    );
    assert!(
        table_output.len() < json_output.len(),
        "Table should be smaller"
    );

    // Verify table content truncation for long content
    assert!(
        table_output.contains("path|size|encoding|content"),
        "Table should have correct header"
    );
}

#[test]
fn test_compression_performance_benchmark() {
    // Create a larger dataset for performance testing
    let large_file_list: Vec<FileEntry> = (0..100)
        .map(|i| FileEntry {
            name: format!("file_{}.rs", i),
            path: PathBuf::from(format!("/project/src/file_{}.rs", i)),
            entry_type: if i % 10 == 0 {
                FileType::Directory
            } else {
                FileType::File
            },
            size: Some(1024 * (i + 1) as u64),
            modified: Some(SystemTime::now()),
            permissions: FilePermissions {
                readable: true,
                writable: i % 2 == 0,
                executable: i % 5 == 0,
            },
        })
        .collect();

    let start = std::time::Instant::now();
    let json_output = large_file_list.to_format(&OutputFormat::Json).unwrap();
    let json_time = start.elapsed();

    let start = std::time::Instant::now();
    let compact_output = large_file_list.to_format(&OutputFormat::Compact).unwrap();
    let compact_time = start.elapsed();

    let start = std::time::Instant::now();
    let table_output = large_file_list.to_format(&OutputFormat::Table).unwrap();
    let table_time = start.elapsed();

    println!("Performance Results for 100 files:");
    println!("JSON: {} bytes in {:?}", json_output.len(), json_time);
    println!(
        "Compact: {} bytes in {:?}",
        compact_output.len(),
        compact_time
    );
    println!("Table: {} bytes in {:?}", table_output.len(), table_time);

    // Verify significant compression for large datasets
    let compact_ratio = compact_output.len() as f32 / json_output.len() as f32;
    let table_ratio = table_output.len() as f32 / json_output.len() as f32;

    println!("Compact ratio: {:.2}", compact_ratio);
    println!("Table ratio: {:.2}", table_ratio);

    assert!(
        compact_ratio < 0.8,
        "Compact should achieve <80% of JSON size for large datasets"
    );
    assert!(
        table_ratio < 0.5,
        "Table should achieve <50% of JSON size for large datasets"
    );

    // Verify reasonable performance (should complete in reasonable time)
    assert!(
        json_time < std::time::Duration::from_millis(100),
        "JSON formatting should be fast"
    );
    assert!(
        compact_time < std::time::Duration::from_millis(200),
        "Compact formatting should be reasonable"
    );
    assert!(
        table_time < std::time::Duration::from_millis(200),
        "Table formatting should be reasonable"
    );
}

#[test]
fn test_token_estimation_accuracy() {
    let test_string = "This is a test string with various punctuation, numbers 123, and symbols!";
    let estimated_tokens = FormatUtils::estimate_token_count(test_string);

    // Rough validation - should be reasonable approximation
    assert!(estimated_tokens > 0, "Should estimate some tokens");
    assert!(
        estimated_tokens < test_string.len(),
        "Should be less than character count"
    );
    assert!(
        estimated_tokens > test_string.len() / 6,
        "Should be more than 1/6 of characters"
    );

    println!(
        "String length: {}, Estimated tokens: {}",
        test_string.len(),
        estimated_tokens
    );
}

#[test]
fn test_field_mappings_completeness() {
    use devit_cli::core::formats::FieldMappings;

    let mappings = FieldMappings::get_mapping();
    let reverse_mappings = FieldMappings::get_reverse_mapping();

    // Verify bidirectional mapping
    assert_eq!(
        mappings.len(),
        reverse_mappings.len(),
        "Mappings should be bidirectional"
    );

    // Verify key field mappings exist
    assert!(mappings.contains_key("path"), "Should have path mapping");
    assert!(mappings.contains_key("size"), "Should have size mapping");
    assert!(
        mappings.contains_key("content"),
        "Should have content mapping"
    );
    assert!(
        mappings.contains_key("permissions"),
        "Should have permissions mapping"
    );

    // Verify mapping application
    let test_json = r#"{"path": "/test", "size": 1024, "content": "hello"}"#;
    let compressed = FieldMappings::apply_mappings(test_json).unwrap();

    assert!(compressed.contains("\"f\":"), "Should map path to f");
    assert!(compressed.contains("\"s\":"), "Should map size to s");
    assert!(compressed.contains("\"c\":"), "Should map content to c");
}

#[test]
fn test_invalid_format_handling() {
    let file_entry = FileEntry {
        name: "test.txt".to_string(),
        path: PathBuf::from("/test.txt"),
        entry_type: FileType::File,
        size: Some(100),
        modified: Some(SystemTime::now()),
        permissions: FilePermissions {
            readable: true,
            writable: true,
            executable: false,
        },
    };

    // Test MessagePack (not yet supported)
    let result = file_entry.to_format(&OutputFormat::MessagePack);
    assert!(result.is_err(), "MessagePack should return error");

    if let Err(devit_cli::core::DevItError::InvalidFormat { format, supported }) = result {
        assert_eq!(format, "messagepack");
        assert!(supported.contains(&"json".to_string()));
        assert!(supported.contains(&"compact".to_string()));
        assert!(supported.contains(&"table".to_string()));
    } else {
        panic!("Should return InvalidFormat error");
    }
}
