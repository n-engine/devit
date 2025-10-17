use std::fmt;
use std::path::{Path, PathBuf};

use devit_cli::core::atomic_patcher::{
    AtomicPatcher as CoreAtomicPatcher, PatchStats as CorePatchStats,
};
use devit_cli::core::errors::DevItError;
use devit_cli::core::patch_parser::{FilePatch, ParsedPatch, PatchLine};
use mcp_core::{McpError, McpResult};

use crate::errors::{
    file_not_found_error, git_dirty_error, internal_error, invalid_diff_error, io_error,
    policy_block_error, resource_limit_error, test_fail_error, test_timeout_error,
    vcs_conflict_error,
};

pub type PatchStats = CorePatchStats;

pub(crate) struct AtomicPatcher {
    working_dir: PathBuf,
    dry_run: bool,
}

impl AtomicPatcher {
    pub fn new(working_dir: PathBuf, dry_run: bool) -> Self {
        Self {
            working_dir,
            dry_run,
        }
    }

    pub fn apply_patch(&self, diff: &str) -> McpResult<(PatchStats, Vec<FileChangeSummary>)> {
        let parsed = ParsedPatch::from_diff(diff).map_err(map_core_error)?;
        if parsed.files.is_empty() {
            return Err(invalid_diff_error("No file changes detected", None));
        }

        if let Some(err) = validate_workspace_state(&parsed, &self.working_dir, self.dry_run) {
            return Err(err);
        }

        let patcher = CoreAtomicPatcher::new(self.working_dir.clone(), self.dry_run);
        let stats = match patcher.apply_patch(diff) {
            Ok(stats) => stats,
            Err(err) => return Err(map_core_error(err)),
        };
        let summaries = build_summaries(&parsed);

        Ok((stats, summaries))
    }
}

#[derive(Debug, Clone, Copy)]
pub enum FileAction {
    Created,
    Modified,
    Deleted,
}

impl fmt::Display for FileAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FileAction::Created => write!(f, "created"),
            FileAction::Modified => write!(f, "modified"),
            FileAction::Deleted => write!(f, "deleted"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FileChangeSummary {
    pub path: String,
    pub action: FileAction,
    pub hunks: usize,
    pub lines_added: usize,
    pub lines_removed: usize,
}

fn build_summaries(parsed: &ParsedPatch) -> Vec<FileChangeSummary> {
    parsed
        .files
        .iter()
        .map(|file_patch| {
            let (display_path, action) = determine_action(file_patch);
            let (added, removed) = count_line_changes(file_patch);
            FileChangeSummary {
                path: display_path,
                action,
                hunks: file_patch.hunks.len(),
                lines_added: added,
                lines_removed: removed,
            }
        })
        .collect()
}

fn determine_action(file_patch: &FilePatch) -> (String, FileAction) {
    if file_patch.is_new_file || file_patch.old_path.is_none() {
        let path = file_patch
            .new_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "<unknown>".to_string());
        (path, FileAction::Created)
    } else if file_patch.is_deleted_file || file_patch.new_path.is_none() {
        let path = file_patch
            .old_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "<unknown>".to_string());
        (path, FileAction::Deleted)
    } else {
        let path = file_patch
            .new_path
            .as_ref()
            .or(file_patch.old_path.as_ref())
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "<unknown>".to_string());
        (path, FileAction::Modified)
    }
}

fn count_line_changes(file_patch: &FilePatch) -> (usize, usize) {
    let mut added = 0usize;
    let mut removed = 0usize;

    for hunk in &file_patch.hunks {
        for line in &hunk.lines {
            match line {
                PatchLine::Add(_) => added += 1,
                PatchLine::Remove(_) => removed += 1,
                PatchLine::Context(_) => {}
            }
        }
    }

    (added, removed)
}

fn validate_workspace_state(
    parsed: &ParsedPatch,
    working_dir: &Path,
    dry_run: bool,
) -> Option<McpError> {
    if dry_run {
        return None;
    }

    for file_patch in &parsed.files {
        if file_patch.is_new_file {
            continue;
        }

        let candidate_path = file_patch
            .old_path
            .as_ref()
            .or(file_patch.new_path.as_ref());

        if let Some(path) = candidate_path {
            let full_path = working_dir.join(path);
            if !full_path.exists() {
                return Some(file_not_found_error(path));
            }
        }
    }

    None
}

fn map_core_error(err: DevItError) -> McpError {
    match err {
        DevItError::InvalidDiff {
            reason,
            line_number,
        } => invalid_diff_error(reason, line_number),
        DevItError::PolicyBlock {
            rule,
            required_level,
            current_level,
            context,
        } => policy_block_error(&rule, &required_level, &current_level, context),
        DevItError::ProtectedPath {
            path,
            protection_rule,
            attempted_operation,
        } => policy_block_error(
            &protection_rule,
            "privileged",
            "current",
            format!(
                "Protected path {} during {}",
                path.display(),
                attempted_operation
            ),
        ),
        DevItError::PrivilegeEscalation {
            attempted_privileges,
            current_privileges,
            escalation_type,
            ..
        } => policy_block_error(
            "privilege_escalation",
            attempted_privileges.as_str(),
            current_privileges.as_str(),
            format!("Escalation attempt ({}) blocked", escalation_type),
        ),
        DevItError::SandboxDenied {
            reason,
            active_profile,
            attempted_operation,
            ..
        } => policy_block_error(
            "sandbox_denied",
            active_profile.as_str(),
            attempted_operation.as_str(),
            reason,
        ),
        DevItError::GitDirty {
            dirty_files,
            modified_files,
            branch,
        } => git_dirty_error(dirty_files, &modified_files, branch.as_deref()),
        DevItError::VcsConflict {
            location,
            conflict_type,
            conflicted_files,
            resolution_hint,
        } => vcs_conflict_error(
            &location,
            &conflict_type,
            &conflicted_files,
            resolution_hint.as_deref(),
        ),
        DevItError::Io {
            operation,
            path,
            source,
        } => io_error(&operation, path.as_deref(), source.to_string()),
        DevItError::InvalidFormat { format, supported } => invalid_diff_error(
            format!(
                "Unsupported diff format '{}' (supported: {})",
                format,
                supported.join(", ")
            ),
            None,
        ),
        DevItError::ResourceLimit {
            resource_type,
            current_usage,
            limit,
            unit,
        } => resource_limit_error(&resource_type, current_usage, limit, &unit),
        DevItError::TestFail {
            failed_count,
            total_count,
            test_framework,
            ..
        } => test_fail_error(failed_count, total_count, &test_framework),
        DevItError::TestTimeout {
            timeout_secs,
            test_framework,
            ..
        } => test_timeout_error(timeout_secs, &test_framework),
        DevItError::SnapshotRequired {
            operation,
            expected,
        } => internal_error(format!(
            "Snapshot required for '{}' (expected {})",
            operation, expected
        )),
        DevItError::SnapshotStale { snapshot_id, .. } => internal_error(format!(
            "Snapshot {} is stale; refresh snapshot before applying patch",
            snapshot_id
        )),
        DevItError::InvalidTestConfig {
            field,
            value,
            reason,
        } => internal_error(format!(
            "Invalid test configuration for '{}': {} ({})",
            field, value, reason
        )),
        DevItError::Internal {
            component, message, ..
        } => internal_error(format!("{}: {}", component, message)),
    }
}
