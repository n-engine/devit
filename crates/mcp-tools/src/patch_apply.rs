use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use mcp_core::{McpResult, McpTool};
use serde_json::{json, Value};

use crate::atomic_patcher::{AtomicPatcher, FileChangeSummary, PatchStats};
use crate::errors::{
    empty_patch_error, internal_error, invalid_diff_error, unsupported_format_error,
};
use chrono::{SecondsFormat, Utc};

const MAX_PATCH_SIZE: usize = 1024 * 1024; // 1 MB

pub struct PatchApplyTool {
    context: Arc<PatchContext>,
}

impl PatchApplyTool {
    pub fn new(context: Arc<PatchContext>) -> Self {
        Self { context }
    }
}

#[async_trait]
impl McpTool for PatchApplyTool {
    fn name(&self) -> &str {
        "devit_patch_apply"
    }

    fn description(&self) -> &str {
        "Apply git-style unified diff patches (--- a/ +++ b/).

Example:
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,4 @@
 fn add(a: i32, b: i32) -> i32 {
     a + b
+        // extra logic
 }

Tip: generate patches with `git diff` to ensure proper headers."
    }

    async fn execute(&self, params: Value) -> McpResult<Value> {
        let diff = params
            .get("diff")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid_diff_error("Parameter 'diff' is required", None))?;

        let dry_run = params
            .get("dry_run")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        ensure_supported_format(diff)?;

        match self.context.apply_patch(diff, dry_run) {
            Ok(result) => Ok(build_response(dry_run, &result)),
            Err(err) => Err(err),
        }
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "diff": {"type": "string"},
                "dry_run": {"type": "boolean"}
            },
            "required": ["diff"]
        })
    }
}

pub struct PatchContext {
    root_path: PathBuf,
}

pub struct PatchExecutionResult {
    pub files: Vec<FileChangeSummary>,
    pub stats: PatchStats,
}

impl PatchContext {
    pub fn new(root_path: PathBuf) -> McpResult<Self> {
        let canonical = root_path.canonicalize().map_err(|err| {
            internal_error(format!("Impossible de résoudre le répertoire: {}", err))
        })?;
        Ok(Self {
            root_path: canonical,
        })
    }

    pub fn apply_patch(&self, diff: &str, dry_run: bool) -> McpResult<PatchExecutionResult> {
        if diff.trim().is_empty() {
            return Err(empty_patch_error());
        }

        if diff.len() > MAX_PATCH_SIZE {
            return Err(invalid_diff_error(
                format!(
                    "Le diff dépasse la taille maximale autorisée ({} octets)",
                    MAX_PATCH_SIZE
                ),
                None,
            ));
        }

        let patcher = AtomicPatcher::new(self.root_path.clone(), dry_run);
        let (stats, summaries) = patcher.apply_patch(diff)?;

        Ok(PatchExecutionResult {
            files: summaries,
            stats,
        })
    }
}

fn build_response(dry_run: bool, result: &PatchExecutionResult) -> Value {
    let stats = &result.stats;
    let status_icon = if dry_run { "🔍" } else { "✅" };
    let action_text = if dry_run { "Preview" } else { "Applied" };

    let mut lines = Vec::new();
    lines.push(format!(
        "{} Patch {} successfully — {} file(s), {} hunks, +{} / -{} lines",
        status_icon,
        action_text.to_lowercase(),
        result.files.len(),
        stats.hunks_applied,
        stats.lines_added,
        stats.lines_removed
    ));

    if !result.files.is_empty() {
        lines.push(String::new());
        for file in &result.files {
            lines.push(format!(
                "- {} {} (hunks: {}, +{} / -{})",
                file.action, file.path, file.hunks, file.lines_added, file.lines_removed
            ));
        }
    }

    let structured = json!({
        "patch": {
            "success": true,
            "dryRun": dry_run,
            "timestamp": Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            "summary": {
                "files": result.files.len(),
                "files_modified": stats.files_modified,
                "files_created": stats.files_created,
                "files_deleted": stats.files_deleted,
                "hunks": stats.hunks_applied,
                "lines_added": stats.lines_added,
                "lines_removed": stats.lines_removed
            },
            "files": result.files.iter().map(|file| {
                json!({
                    "path": file.path,
                    "action": file.action.to_string(),
                    "hunks": file.hunks,
                    "lines_added": file.lines_added,
                    "lines_removed": file.lines_removed
                })
            }).collect::<Vec<_>>()
        }
    });

    json!({
        "content": [
            {
                "type": "text",
                "text": lines.join("\n")
            }
        ],
        "structuredContent": structured
    })
}

fn ensure_supported_format(diff: &str) -> McpResult<()> {
    let trimmed = diff.trim();
    if trimmed.starts_with("*** ") {
        return Err(unsupported_format_error("context diff"));
    }

    let has_git_header = trimmed.contains("diff --git");
    let has_unified_markers =
        trimmed.contains("\n@@") || trimmed.starts_with("@@") || trimmed.contains("\r\n@@");
    let has_file_headers =
        trimmed.contains("\n--- ") || trimmed.starts_with("--- ") || trimmed.contains("\r\n--- ");

    if has_git_header || (has_unified_markers && has_file_headers) {
        return Ok(());
    }

    Err(unsupported_format_error("unknown"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn sample_diff() -> &'static str {
        "diff --git a/hello.txt b/hello.txt
index e69de29..4b825dc 100644
--- a/hello.txt
+++ b/hello.txt
@@ -1 +1 @@
-old
+new
"
    }

    #[test]
    fn apply_patch_updates_file_and_summary() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("hello.txt");
        fs::write(&file_path, "old\n").unwrap();

        let context = PatchContext::new(temp.path().to_path_buf()).unwrap();
        let result = context.apply_patch(sample_diff(), false).unwrap();

        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content.trim_end(), "new");

        let response = build_response(false, &result);
        let text = response["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Patch applied successfully"));
        assert!(text.contains("hello.txt"));

        let structured = &response["structuredContent"]["patch"];
        assert!(structured["success"].as_bool().unwrap());
        assert!(!structured["dryRun"].as_bool().unwrap());
        assert_eq!(structured["files"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn dry_run_provides_preview_without_modifying() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("hello.txt");
        fs::write(&file_path, "old\n").unwrap();

        let context = PatchContext::new(temp.path().to_path_buf()).unwrap();
        let result = context.apply_patch(sample_diff(), true).unwrap();

        // File should remain unchanged
        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content.trim_end(), "old");

        let response = build_response(true, &result);
        let text = response["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Patch preview"));
        let structured = &response["structuredContent"]["patch"];
        assert!(structured["dryRun"].as_bool().unwrap());
    }

    #[test]
    fn invalid_format_rejected() {
        let err = ensure_supported_format("random text");
        assert!(err.is_err());
        let msg = err.err().unwrap().to_string();
        assert!(msg.contains("unsupported diff format"));
    }

    #[test]
    fn file_not_found_error() {
        let temp = tempdir().unwrap();
        let context = PatchContext::new(temp.path().to_path_buf()).unwrap();
        let diff = r#"diff --git a/missing.txt b/missing.txt
index e69de29..4b825dc 100644
--- a/missing.txt
+++ b/missing.txt
@@ -1 +1 @@
-old
+new
"#;
        let err = match context.apply_patch(diff, false) {
            Ok(_) => panic!("expected patch application to fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("file 'missing.txt' not found"));
    }

    #[test]
    fn security_violation_detected() {
        let temp = tempdir().unwrap();
        let context = PatchContext::new(temp.path().to_path_buf()).unwrap();
        let diff = r#"diff --git a/../evil.txt b/../evil.txt
index e69de29..4b825dc 100644
--- a/../evil.txt
+++ b/../evil.txt
@@ -1 +1 @@
-old
+new
"#;
        let err = match context.apply_patch(diff, true) {
            Ok(_) => panic!("expected security violation"),
            Err(err) => err,
        };
        let msg = err.to_string().to_lowercase();
        assert!(
            msg.contains("security violation") || msg.contains("path traversal"),
            "unexpected error message: {}",
            msg
        );
    }

    #[test]
    fn context_mismatch_is_reported() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("hello.txt");
        fs::write(&file_path, "other\nline2\n").unwrap();
        let context = PatchContext::new(temp.path().to_path_buf()).unwrap();

        let diff = r#"diff --git a/hello.txt b/hello.txt
index e69de29..4b825dc 100644
--- a/hello.txt
+++ b/hello.txt
@@ -1 +1 @@
-line1
+updated
"#;

        let err = match context.apply_patch(diff, false) {
            Ok(_) => panic!("expected context mismatch error"),
            Err(err) => err,
        };
        let msg = err.to_string();
        assert!(
            msg.contains("VCS conflict") && msg.contains("Expected to remove"),
            "unexpected error message: {}",
            msg
        );
    }
}
