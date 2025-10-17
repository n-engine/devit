use crate::analyzer::{Issue, Severity};
use anyhow::{Context, Result};
use regex::Regex;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct Rule {
    pub id: String,
    pub description: String,
    pub pattern: Regex,
    pub severity: Severity,
    pub suggestion: Option<String>,
    pub rule_type: RuleType,
}

#[derive(Debug, Clone)]
pub enum RuleType {
    LinePattern,
    FunctionPattern,
    GlobalPattern,
}

pub struct RuleEngine {
    rules: Vec<Rule>,
    rule_map: HashMap<String, Rule>,
}

impl RuleEngine {
    pub fn load_from_file(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)
            .context(format!("Failed to read rules file: {}", path.display()))?;
        
        let mut rules = Vec::new();
        let mut rule_map = HashMap::new();
        
        // Parse rules from file (simplified format)
        for (line_num, line) in content.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            
            match Self::parse_rule_line(line, line_num + 1) {
                Ok(rule) => {
                    rule_map.insert(rule.id.clone(), rule.clone());
                    rules.push(rule);
                }
                Err(e) => {
                    eprintln!("Warning: Failed to parse rule at line {}: {}", line_num + 1, e);
                }
            }
        }
        
        // Add default built-in rules if no rules loaded
        if rules.is_empty() {
            rules = Self::default_rules();
            for rule in &rules {
                rule_map.insert(rule.id.clone(), rule.clone());
            }
        }
        
        Ok(Self { rules, rule_map })
    }
    
    fn parse_rule_line(line: &str, line_num: usize) -> Result<Rule> {
        // Format: RULE_ID|SEVERITY|PATTERN|DESCRIPTION|SUGGESTION
        let parts: Vec<&str> = line.split('|').collect();
        if parts.len() < 4 {
            anyhow::bail!("Invalid rule format at line {}", line_num);
        }
        
        let id = parts[0].trim().to_string();
        let severity = match parts[1].trim().to_lowercase().as_str() {
            "error" => Severity::Error,
            "warning" => Severity::Warning,
            "info" => Severity::Info,
            _ => Severity::Warning,
        };
        
        let pattern = Regex::new(parts[2].trim())
            .context(format!("Invalid regex pattern: {}", parts[2]))?;
        
        let description = parts[3].trim().to_string();
        let suggestion = if parts.len() > 4 && !parts[4].trim().is_empty() {
            Some(parts[4].trim().to_string())
        } else {
            None
        };
        
        Ok(Rule {
            id,
            description,
            pattern,
            severity,
            suggestion,
            rule_type: RuleType::LinePattern,
        })
    }
    
    fn default_rules() -> Vec<Rule> {
        vec![
            Rule {
                id: "no_goto".to_string(),
                description: "Avoid using goto statements".to_string(),
                pattern: Regex::new(r"\bgoto\b").unwrap(),
                severity: Severity::Warning,
                suggestion: Some("Consider using structured control flow instead".to_string()),
                rule_type: RuleType::LinePattern,
            },
            Rule {
                id: "no_gets".to_string(),
                description: "Never use gets() function (security risk)".to_string(),
                pattern: Regex::new(r"\bgets\s*\(").unwrap(),
                severity: Severity::Error,
                suggestion: Some("Use fgets() instead".to_string()),
                rule_type: RuleType::LinePattern,
            },
            Rule {
                id: "line_too_long".to_string(),
                description: "Line length exceeds 100 characters".to_string(),
                pattern: Regex::new(r"^.{101,}$").unwrap(),
                severity: Severity::Info,
                suggestion: Some("Break long lines for better readability".to_string()),
                rule_type: RuleType::LinePattern,
            },
            Rule {
                id: "no_magic_numbers".to_string(),
                description: "Avoid magic numbers".to_string(),
                pattern: Regex::new(r"\b\d{2,}\b").unwrap(),
                severity: Severity::Info,
                suggestion: Some("Use named constants instead".to_string()),
                rule_type: RuleType::LinePattern,
            },
            Rule {
                id: "missing_space_after_keyword".to_string(),
                description: "Missing space after control flow keyword".to_string(),
                pattern: Regex::new(r"\b(if|for|while|switch)\(").unwrap(),
                severity: Severity::Warning,
                suggestion: Some("Add space after keyword: 'if ('".to_string()),
                rule_type: RuleType::LinePattern,
            },
        ]
    }
    
    pub fn check_line(&self, line: &str, line_num: usize) -> Vec<Issue> {
        let mut issues = Vec::new();
        
        for rule in &self.rules {
            if matches!(rule.rule_type, RuleType::LinePattern) {
                if rule.pattern.is_match(line) {
                    issues.push(Issue {
                        rule_id: rule.id.clone(),
                        line: line_num,
                        column: 0, // TODO: calculate actual column
                        severity: rule.severity.clone(),
                        message: rule.description.clone(),
                        suggestion: rule.suggestion.clone(),
                    });
                }
            }
        }
        
        issues
    }
    
    pub fn check_functions(&self, content: &str) -> Vec<Issue> {
        let mut issues = Vec::new();
        
        // Find function definitions and check their length
        let function_regex = Regex::new(r"(?m)^[a-zA-Z_][a-zA-Z0-9_]*\s+[a-zA-Z_][a-zA-Z0-9_]*\s*\([^)]*\)\s*\{").unwrap();
        
        for mat in function_regex.find_iter(content) {
            let start_line = content[..mat.start()].lines().count();
            let function_length = self.estimate_function_length(content, mat.start());
            
            if function_length > 50 {
                issues.push(Issue {
                    rule_id: "function_too_long".to_string(),
                    line: start_line,
                    column: 0,
                    severity: Severity::Warning,
                    message: format!("Function is {} lines long, consider breaking it down", function_length),
                    suggestion: Some("Split large functions into smaller, focused functions".to_string()),
                });
            }
        }
        
        issues
    }
    
    fn estimate_function_length(&self, content: &str, start_pos: usize) -> usize {
        let remaining = &content[start_pos..];
        let mut brace_count = 0;
        let mut lines = 0;
        
        for ch in remaining.chars() {
            if ch == '\n' {
                lines += 1;
            }
            match ch {
                '{' => brace_count += 1,
                '}' => {
                    brace_count -= 1;
                    if brace_count == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
        
        lines
    }
    
    pub fn get_rule(&self, rule_id: &str) -> Option<&Rule> {
        self.rule_map.get(rule_id)
    }
    
    pub fn list_rules(&self) -> &[Rule] {
        &self.rules
    }
}