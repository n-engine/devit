//! Minimal patch subsystem placeholder.
//!
//! The original patch engine is being replaced by a shared implementation in
//! `patch_parser` and `patch_preview`. This module now provides the structures
//! required by the surrounding code so the CLI continues to compile while the
//! new flow is rolled out.

use std::path::PathBuf;

use crate::core::errors::DevItResult;
use crate::core::patch_parser::{ParsedPatch, PatchLine};
use crate::core::tester::TestRunRequest;
use devit_common::ApprovalLevel;
use uuid::Uuid;

/// Risk classification used by the preview module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

/// Types of file changes detected in a patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileChangeType {
    Created,
    Modified,
    Deleted,
    Renamed,
    Copied,
}

/// Minimal description of a file level change extracted from a diff.
#[derive(Debug, Clone)]
pub struct FileChange {
    pub file_path: PathBuf,
    pub change_type: FileChangeType,
    pub lines_added: usize,
    pub lines_removed: usize,
    pub is_binary: bool,
    pub adds_exec_bit: bool,
    pub is_symlink: bool,
    pub symlink_target: Option<PathBuf>,
    pub is_submodule: bool,
    pub touches_gitmodules: bool,
}

/// Configuration for the legacy patch pipeline.
#[derive(Debug, Clone)]
pub struct PatchConfig {
    pub working_directory: PathBuf,
    pub create_backups: bool,
    pub max_patch_size: usize,
    pub max_affected_files: usize,
    pub enable_security_analysis: bool,
    pub cache_size: usize,
}

impl Default for PatchConfig {
    fn default() -> Self {
        Self {
            working_directory: PathBuf::from("."),
            create_backups: false,
            max_patch_size: 1024 * 1024,
            max_affected_files: 100,
            enable_security_analysis: true,
            cache_size: 32,
        }
    }
}

/// Lightweight manager kept for backwards compatibility.
pub struct PatchManager {
    config: PatchConfig,
}

impl PatchManager {
    pub fn new(config: PatchConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &PatchConfig {
        &self.config
    }

    /// Update the base working directory for patch application.
    pub fn set_working_directory<P: Into<PathBuf>>(&mut self, path: P) {
        self.config.working_directory = path.into();
    }

    /// Helper to map a risk level to the recommended approval.
    pub fn recommended_approval(risk: RiskLevel) -> ApprovalLevel {
        match risk {
            RiskLevel::Low => ApprovalLevel::Untrusted,
            RiskLevel::Medium => ApprovalLevel::Ask,
            RiskLevel::High => ApprovalLevel::Moderate,
            RiskLevel::Critical => ApprovalLevel::Trusted,
        }
    }
}

/// Input payload for the enhanced patch workflow (R2).
#[derive(Debug, Clone)]
pub struct PatchApplyRequest {
    pub diff: String,
    pub idempotency_key: Option<Uuid>,
    pub post_tests: Option<TestRunRequest>,
}

/// Classify file-level changes contained in a unified diff.
///
/// This lightweight implementation uses the shared `ParsedPatch` structure and
/// extracts aggregate information required by policy evaluation.
pub fn classify_changes(diff_content: &str) -> DevItResult<Vec<FileChange>> {
    let parsed = ParsedPatch::from_diff(diff_content)?;
    let mut changes = Vec::new();

    for file in parsed.files {
        let primary_path = file
            .new_path
            .clone()
            .or(file.old_path.clone())
            .unwrap_or_else(|| PathBuf::from("<unknown>"));

        let change_type = if file.is_new_file {
            FileChangeType::Created
        } else if file.is_deleted_file {
            FileChangeType::Deleted
        } else if file.old_path != file.new_path
            && file.old_path.is_some()
            && file.new_path.is_some()
        {
            FileChangeType::Renamed
        } else {
            FileChangeType::Modified
        };

        let mut lines_added = 0usize;
        let mut lines_removed = 0usize;
        for hunk in &file.hunks {
            for line in &hunk.lines {
                match line {
                    PatchLine::Add(_) => lines_added += 1,
                    PatchLine::Remove(_) => lines_removed += 1,
                    PatchLine::Context(_) => {}
                }
            }
        }

        let touches_gitmodules = primary_path == PathBuf::from(".gitmodules");

        changes.push(FileChange {
            file_path: primary_path,
            change_type,
            lines_added,
            lines_removed,
            is_binary: file.is_binary,
            adds_exec_bit: file.adds_exec_bit,
            is_symlink: false,
            symlink_target: None,
            is_submodule: false,
            touches_gitmodules,
        });
    }

    Ok(changes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_exec_bit_change() {
        let patch = r#"diff --git a/scripts/deploy.sh b/scripts/deploy.sh
old mode 100644
new mode 100755
index 1234567..abcdefg
--- a/scripts/deploy.sh
+++ b/scripts/deploy.sh
@@ -1,3 +1,4 @@
 #!/bin/bash
 echo \"Starting deployment...\"
 # Deploy logic here
+chmod +x other_script.sh
"#;

        let changes = classify_changes(patch).expect("classification works");
        assert_eq!(changes.len(), 1);
        assert!(changes[0].adds_exec_bit, "exec bit should be detected");
    }
}
