use crate::rules::{Rule, RuleEngine};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Serialize, Deserialize)]
pub struct AnalysisResult {
    pub file: PathBuf,
    pub issues: Vec<Issue>,
    pub stats: FileStats,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Issue {
    pub rule_id: String,
    pub line: usize,
    pub column: usize,
    pub severity: Severity,
    pub message: String,
    pub suggestion: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileStats {
    pub lines: usize,
    pub functions: usize,
    pub complexity: usize,
}

pub struct CodeAnalyzer {
    rule_engine: RuleEngine,
}

impl CodeAnalyzer {
    pub fn new(rules_path: PathBuf) -> Result<Self> {
        let rule_engine = RuleEngine::load_from_file(&rules_path)
            .context("Failed to load rules")?;
        
        Ok(Self { rule_engine })
    }
    
    pub fn analyze_path(&self, path: &Path, verbose: bool) -> Result<Vec<AnalysisResult>> {
        let mut results = Vec::new();
        
        if path.is_file() {
            if self.is_c_file(path) {
                results.push(self.analyze_file(path, verbose)?);
            }
        } else {
            for entry in WalkDir::new(path)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file() && self.is_c_file(e.path()))
            {
                if verbose {
                    eprintln!("Analyzing: {}", entry.path().display());
                }
                results.push(self.analyze_file(entry.path(), verbose)?);
            }
        }
        
        Ok(results)
    }
    
    fn analyze_file(&self, path: &Path, _verbose: bool) -> Result<AnalysisResult> {
        let content = std::fs::read_to_string(path)
            .context(format!("Failed to read file: {}", path.display()))?;
        
        let mut issues = Vec::new();
        let lines: Vec<&str> = content.lines().collect();
        
        // Apply all rules to the file
        for (line_num, line) in lines.iter().enumerate() {
            let line_issues = self.rule_engine.check_line(line, line_num + 1);
            issues.extend(line_issues);
        }
        
        // Apply function-level rules
        let function_issues = self.rule_engine.check_functions(&content);
        issues.extend(function_issues);
        
        let stats = self.calculate_stats(&content);
        
        Ok(AnalysisResult {
            file: path.to_path_buf(),
            issues,
            stats,
        })
    }
    
    fn is_c_file(&self, path: &Path) -> bool {
        match path.extension() {
            Some(ext) => matches!(ext.to_str(), Some("c") | Some("h")),
            None => false,
        }
    }
    
    fn calculate_stats(&self, content: &str) -> FileStats {
        let lines = content.lines().count();
        let functions = content.matches("(){").count(); // Simple heuristic
        let complexity = self.calculate_complexity(content);
        
        FileStats {
            lines,
            functions,
            complexity,
        }
    }
    
    fn calculate_complexity(&self, content: &str) -> usize {
        // Simple cyclomatic complexity approximation
        let if_count = content.matches("if").count();
        let while_count = content.matches("while").count();
        let for_count = content.matches("for").count();
        let switch_count = content.matches("switch").count();
        
        1 + if_count + while_count + for_count + switch_count
    }
    
    pub fn print_results(&self, results: &[AnalysisResult]) {
        for result in results {
            println!("\n=== {} ===", result.file.display());
            println!("Lines: {}, Functions: {}, Complexity: {}", 
                     result.stats.lines, result.stats.functions, result.stats.complexity);
            
            if result.issues.is_empty() {
                println!("✅ No issues found");
                continue;
            }
            
            for issue in &result.issues {
                let severity_icon = match issue.severity {
                    Severity::Error => "❌",
                    Severity::Warning => "⚠️",
                    Severity::Info => "ℹ️",
                };
                
                println!("{} Line {}: {} [{}]", 
                         severity_icon, issue.line, issue.message, issue.rule_id);
                
                if let Some(suggestion) = &issue.suggestion {
                    println!("   💡 Suggestion: {}", suggestion);
                }
            }
        }
        
        // Summary
        let total_issues: usize = results.iter().map(|r| r.issues.len()).sum();
        let total_files = results.len();
        
        println!("\n📊 Summary: {} issues found in {} files", total_issues, total_files);
    }
}