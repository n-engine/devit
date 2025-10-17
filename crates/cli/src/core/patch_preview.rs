use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::core::errors::{DevItError, DevItResult};
use crate::core::patch::RiskLevel;
use crate::core::patch_parser::{ParsedPatch, PatchLine};
use crate::core::path_security::PathSecurityContext;
use crate::core::{ApprovalLevel, PatchPreview, PermissionChange};

/// Context for patch preview analysis.
#[derive(Debug, Clone, Default)]
pub struct PreviewContext<'a> {
    /// Optional path security context used to validate affected paths.
    pub path_security: Option<&'a PathSecurityContext>,
    /// Repository root used for heuristics when path security is unavailable.
    pub repo_root: Option<&'a Path>,
}

/// Analyze a unified diff and build a preview summary.
///
/// Parses the diff, performs lightweight heuristics, validates affected paths,
/// and estimates the risk level of applying the patch. The preview never writes
/// to disk and can be called frequently by orchestration layers.
pub fn generate_preview(
    diff_content: &str,
    context: PreviewContext<'_>,
) -> DevItResult<PatchPreview> {
    let parsed = ParsedPatch::from_diff(diff_content)?;
    if parsed.files.is_empty() {
        return Err(DevItError::InvalidDiff {
            reason: "Patch does not contain any file changes".to_string(),
            line_number: None,
        });
    }

    let mut affected = BTreeSet::new();
    let mut affects_protected = false;
    let mut lines_added = 0usize;
    let mut lines_removed = 0usize;
    let mut policy_warnings = Vec::new();

    for file in &parsed.files {
        let logical_path = file
            .new_path
            .clone()
            .or_else(|| file.old_path.clone())
            .unwrap_or_else(|| PathBuf::from("<unknown>"));

        if let Some(path) = file.new_path.clone() {
            affected.insert(path);
        }

        if let Some(path) = file.old_path.clone() {
            affected.insert(path);
        }

        // Perform path security validation when available.
        if let Some(security) = context.path_security {
            if let Err(err) = security.validate_patch_path(&logical_path) {
                affects_protected = true;
                policy_warnings.push(format!(
                    "Chemin protégé ou invalide détecté: {} ({})",
                    logical_path.display(),
                    err
                ));
            }
        } else if is_protected_path(&logical_path, context.repo_root) {
            affects_protected = true;
            policy_warnings.push(format!(
                "Chemin potentiellement protégé: {}",
                logical_path.display()
            ));
        }

        for hunk in &file.hunks {
            for line in &hunk.lines {
                match line {
                    PatchLine::Add(_) => lines_added += 1,
                    PatchLine::Remove(_) => lines_removed += 1,
                    PatchLine::Context(_) => {}
                }
            }
        }
    }

    let total_lines_changed = lines_added + lines_removed;
    let risk_level = assess_risk(
        affects_protected,
        policy_warnings.is_empty(),
        total_lines_changed,
        affected.iter(),
    );
    let recommended_approval = match risk_level {
        RiskLevel::Low => ApprovalLevel::Untrusted,
        RiskLevel::Medium => ApprovalLevel::Ask,
        RiskLevel::High => ApprovalLevel::Moderate,
        RiskLevel::Critical => ApprovalLevel::Trusted,
    };

    Ok(PatchPreview {
        affected_files: affected.into_iter().collect(),
        affects_protected,
        affects_binaries: false,
        estimated_line_changes: total_lines_changed,
        policy_warnings,
        recommended_approval,
        permission_changes: Vec::<PermissionChange>::new(),
    })
}

fn assess_risk<'a, I>(
    affects_protected: bool,
    no_conflicts: bool,
    total_lines_changed: usize,
    files: I,
) -> RiskLevel
where
    I: Iterator<Item = &'a PathBuf>,
{
    if affects_protected {
        return RiskLevel::High;
    }

    if !no_conflicts {
        return RiskLevel::High;
    }

    let mut risk = if total_lines_changed > 200 {
        RiskLevel::High
    } else if total_lines_changed > 40 {
        RiskLevel::Medium
    } else {
        RiskLevel::Low
    };

    for path in files {
        if is_critical_file(path) {
            risk = match risk {
                RiskLevel::Low => RiskLevel::Medium,
                RiskLevel::Medium => RiskLevel::High,
                RiskLevel::High | RiskLevel::Critical => RiskLevel::High,
            };
        }
    }

    if total_lines_changed > 500 {
        risk = RiskLevel::Critical;
    }

    risk
}

fn is_critical_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };

    matches!(
        name,
        "Cargo.toml"
            | "Cargo.lock"
            | "Makefile"
            | "build.rs"
            | "package.json"
            | "requirements.txt"
            | "Dockerfile"
    )
}

fn is_protected_path(path: &Path, repo_root: Option<&Path>) -> bool {
    let segments: Vec<_> = path.components().collect();
    if segments.iter().any(|c| {
        matches!(
            c,
            std::path::Component::ParentDir | std::path::Component::RootDir
        )
    }) {
        return true;
    }

    if let Some(first) = segments.first() {
        if let std::path::Component::Normal(name) = first {
            if let Some(name_str) = name.to_str() {
                if name_str.starts_with('.') {
                    return true;
                }
            }
        }
    }

    if let Some(root) = repo_root {
        let candidate = root.join(path);
        if candidate.starts_with(root.join(".devit")) || candidate.starts_with(root.join(".git")) {
            return true;
        }
    }

    false
}
