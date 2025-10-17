//! Compression Performance Benchmarks
//!
//! This module benchmarks the performance of different output formats
//! and measures compression efficiency for various data structures.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use devit_cli::core::file_ops::{
    FileContent, FileEntry, FilePermissions, FileType, ProjectStructure, SearchMatch,
    SearchResults, TreeNode,
};
use devit_cli::core::formats::{Compressible, FormatUtils, OutputFormat};
use std::path::PathBuf;
use std::time::SystemTime;

fn create_sample_file_entry(size_factor: usize) -> FileEntry {
    FileEntry {
        name: format!("sample_file_{}.rs", size_factor),
        path: PathBuf::from(format!("/project/src/sample_file_{}.rs", size_factor)),
        entry_type: FileType::File,
        size: Some(1024 * size_factor as u64),
        modified: Some(SystemTime::now()),
        permissions: FilePermissions {
            readable: true,
            writable: true,
            executable: false,
        },
    }
}

fn create_sample_file_list(count: usize) -> Vec<FileEntry> {
    (0..count)
        .map(|i| create_sample_file_entry(i + 1))
        .collect()
}

fn create_sample_file_content(size_kb: usize) -> FileContent {
    let content = "fn main() {\n    println!(\"Hello, world!\");\n}\n".repeat(size_kb * 10);
    let lines = content
        .lines()
        .enumerate()
        .map(|(i, line)| format!("{}: {}", i + 1, line))
        .collect();

    FileContent {
        path: PathBuf::from("/project/src/main.rs"),
        content,
        size: (size_kb * 1024) as u64,
        lines: Some(lines),
        encoding: "utf-8".to_string(),
    }
}

fn create_sample_search_results(match_count: usize) -> SearchResults {
    let matches = (0..match_count)
        .map(|i| SearchMatch {
            file: PathBuf::from(format!("/project/src/file_{}.rs", i)),
            line_number: i + 1,
            line: format!("pub fn function_{}() {{", i),
            context_before: vec![format!("// Function {}", i)],
            context_after: vec!["    // Implementation".to_string()],
        })
        .collect();

    SearchResults {
        pattern: "pub fn".to_string(),
        path: PathBuf::from("/project/src"),
        files_searched: match_count * 2,
        total_matches: match_count,
        matches,
        truncated: false,
    }
}

fn create_sample_project_structure(depth: usize) -> ProjectStructure {
    fn create_tree_node(name: &str, depth: usize, current_depth: usize) -> TreeNode {
        let children = if current_depth < depth {
            Some(
                (0..3)
                    .map(|i| create_tree_node(&format!("{}_{}", name, i), depth, current_depth + 1))
                    .collect(),
            )
        } else {
            None
        };

        TreeNode {
            name: name.to_string(),
            path: PathBuf::from(format!("/project/{}", name)),
            node_type: if children.is_some() {
                FileType::Directory
            } else {
                FileType::File
            },
            children,
            size: if children.is_none() { Some(1024) } else { None },
            modified: Some(SystemTime::now()),
            permissions: FilePermissions {
                readable: true,
                writable: true,
                executable: children.is_some(),
            },
        }
    }

    ProjectStructure {
        root: PathBuf::from("/project"),
        project_type: "rust".to_string(),
        tree: create_tree_node("project", depth, 0),
        total_files: 3_usize.pow(depth as u32),
        total_dirs: if depth > 0 {
            (3_usize.pow(depth as u32) - 1) / 2
        } else {
            0
        },
    }
}

fn bench_file_entry_formats(c: &mut Criterion) {
    let mut group = c.benchmark_group("file_entry_formats");

    for size in [1, 10, 100].iter() {
        let file_entry = create_sample_file_entry(*size);

        group.bench_with_input(BenchmarkId::new("json", size), size, |b, _| {
            b.iter(|| file_entry.to_format(&OutputFormat::Json).unwrap())
        });

        group.bench_with_input(BenchmarkId::new("compact", size), size, |b, _| {
            b.iter(|| file_entry.to_format(&OutputFormat::Compact).unwrap())
        });

        group.bench_with_input(BenchmarkId::new("table", size), size, |b, _| {
            b.iter(|| file_entry.to_format(&OutputFormat::Table).unwrap())
        });
    }

    group.finish();
}

fn bench_file_list_formats(c: &mut Criterion) {
    let mut group = c.benchmark_group("file_list_formats");

    for count in [10, 100, 1000].iter() {
        let file_list = create_sample_file_list(*count);

        group.bench_with_input(BenchmarkId::new("json", count), count, |b, _| {
            b.iter(|| file_list.to_format(&OutputFormat::Json).unwrap())
        });

        group.bench_with_input(BenchmarkId::new("compact", count), count, |b, _| {
            b.iter(|| file_list.to_format(&OutputFormat::Compact).unwrap())
        });

        group.bench_with_input(BenchmarkId::new("table", count), count, |b, _| {
            b.iter(|| file_list.to_format(&OutputFormat::Table).unwrap())
        });
    }

    group.finish();
}

fn bench_file_content_formats(c: &mut Criterion) {
    let mut group = c.benchmark_group("file_content_formats");

    for size_kb in [1, 10, 100].iter() {
        let file_content = create_sample_file_content(*size_kb);

        group.bench_with_input(BenchmarkId::new("json", size_kb), size_kb, |b, _| {
            b.iter(|| file_content.to_format(&OutputFormat::Json).unwrap())
        });

        group.bench_with_input(BenchmarkId::new("compact", size_kb), size_kb, |b, _| {
            b.iter(|| file_content.to_format(&OutputFormat::Compact).unwrap())
        });

        group.bench_with_input(BenchmarkId::new("table", size_kb), size_kb, |b, _| {
            b.iter(|| file_content.to_format(&OutputFormat::Table).unwrap())
        });
    }

    group.finish();
}

fn bench_search_results_formats(c: &mut Criterion) {
    let mut group = c.benchmark_group("search_results_formats");

    for match_count in [10, 100, 500].iter() {
        let search_results = create_sample_search_results(*match_count);

        group.bench_with_input(
            BenchmarkId::new("json", match_count),
            match_count,
            |b, _| b.iter(|| search_results.to_format(&OutputFormat::Json).unwrap()),
        );

        group.bench_with_input(
            BenchmarkId::new("compact", match_count),
            match_count,
            |b, _| b.iter(|| search_results.to_format(&OutputFormat::Compact).unwrap()),
        );

        group.bench_with_input(
            BenchmarkId::new("table", match_count),
            match_count,
            |b, _| b.iter(|| search_results.to_format(&OutputFormat::Table).unwrap()),
        );
    }

    group.finish();
}

fn bench_project_structure_formats(c: &mut Criterion) {
    let mut group = c.benchmark_group("project_structure_formats");

    for depth in [2, 3, 4].iter() {
        let project_structure = create_sample_project_structure(*depth);

        group.bench_with_input(BenchmarkId::new("json", depth), depth, |b, _| {
            b.iter(|| project_structure.to_format(&OutputFormat::Json).unwrap())
        });

        group.bench_with_input(BenchmarkId::new("compact", depth), depth, |b, _| {
            b.iter(|| project_structure.to_format(&OutputFormat::Compact).unwrap())
        });

        group.bench_with_input(BenchmarkId::new("table", depth), depth, |b, _| {
            b.iter(|| project_structure.to_format(&OutputFormat::Table).unwrap())
        });
    }

    group.finish();
}

fn bench_compression_ratios(c: &mut Criterion) {
    let mut group = c.benchmark_group("compression_ratios");

    // Test compression ratio calculation performance
    let file_list = create_sample_file_list(100);

    group.bench_function("calculate_ratio_compact", |b| {
        b.iter(|| {
            let json_output = file_list.to_format(&OutputFormat::Json).unwrap();
            let compact_output = file_list.to_format(&OutputFormat::Compact).unwrap();
            FormatUtils::calculate_compression_ratio(&json_output, &compact_output)
        })
    });

    group.bench_function("calculate_ratio_table", |b| {
        b.iter(|| {
            let json_output = file_list.to_format(&OutputFormat::Json).unwrap();
            let table_output = file_list.to_format(&OutputFormat::Table).unwrap();
            FormatUtils::calculate_compression_ratio(&json_output, &table_output)
        })
    });

    group.finish();
}

fn bench_token_estimation(c: &mut Criterion) {
    let mut group = c.benchmark_group("token_estimation");

    let file_content = create_sample_file_content(10);
    let json_output = file_content.to_format(&OutputFormat::Json).unwrap();
    let compact_output = file_content.to_format(&OutputFormat::Compact).unwrap();
    let table_output = file_content.to_format(&OutputFormat::Table).unwrap();

    group.bench_function("estimate_tokens_json", |b| {
        b.iter(|| FormatUtils::estimate_token_count(&json_output))
    });

    group.bench_function("estimate_tokens_compact", |b| {
        b.iter(|| FormatUtils::estimate_token_count(&compact_output))
    });

    group.bench_function("estimate_tokens_table", |b| {
        b.iter(|| FormatUtils::estimate_token_count(&table_output))
    });

    group.finish();
}

fn bench_field_mappings(c: &mut Criterion) {
    use devit_cli::core::formats::FieldMappings;

    let mut group = c.benchmark_group("field_mappings");

    let file_list = create_sample_file_list(50);
    let json_output = file_list.to_format(&OutputFormat::Json).unwrap();

    group.bench_function("apply_mappings", |b| {
        b.iter(|| FieldMappings::apply_mappings(&json_output).unwrap())
    });

    group.bench_function("get_mapping", |b| b.iter(|| FieldMappings::get_mapping()));

    group.bench_function("get_reverse_mapping", |b| {
        b.iter(|| FieldMappings::get_reverse_mapping())
    });

    group.finish();
}

// Benchmark measuring actual compression effectiveness
fn measure_compression_effectiveness(c: &mut Criterion) {
    let mut group = c.benchmark_group("compression_effectiveness");

    // Different data sizes to test compression scaling
    for &size in &[10, 100, 1000] {
        let file_list = create_sample_file_list(size);

        group.bench_with_input(
            BenchmarkId::new("measure_all_formats", size),
            &size,
            |b, _| {
                b.iter(|| {
                    let json_output = file_list.to_format(&OutputFormat::Json).unwrap();
                    let compact_output = file_list.to_format(&OutputFormat::Compact).unwrap();
                    let table_output = file_list.to_format(&OutputFormat::Table).unwrap();

                    let compact_ratio =
                        FormatUtils::calculate_compression_ratio(&json_output, &compact_output);
                    let table_ratio =
                        FormatUtils::calculate_compression_ratio(&json_output, &table_output);

                    let json_tokens = FormatUtils::estimate_token_count(&json_output);
                    let compact_tokens = FormatUtils::estimate_token_count(&compact_output);
                    let table_tokens = FormatUtils::estimate_token_count(&table_output);

                    (
                        compact_ratio,
                        table_ratio,
                        json_tokens,
                        compact_tokens,
                        table_tokens,
                    )
                })
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_file_entry_formats,
    bench_file_list_formats,
    bench_file_content_formats,
    bench_search_results_formats,
    bench_project_structure_formats,
    bench_compression_ratios,
    bench_token_estimation,
    bench_field_mappings,
    measure_compression_effectiveness
);

criterion_main!(benches);
