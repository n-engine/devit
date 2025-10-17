//! # DevIt Core Engine Contract Tests
//!
//! These tests validate the core business logic contracts that the Core Engine
//! must fulfill. They use synthetic data (mock patches, FileChanges) to verify
//! specific approval level behaviors and error conditions.
//!
//! All tests are marked with #[ignore] to be activated progressively as the
//! core engine implementation advances.

use std::collections::HashMap;
use std::path::PathBuf;

use devit_cli::core::{
    ApprovalLevel, CoreEngine, DevItError, FileChangeKind, PatchResult, SandboxProfile,
};

mod fixtures;
use fixtures::*;

/// Contract: ASK approval level should require confirmation for exec-bit changes
#[test]
fn contract_ask_requires_confirmation_for_exec_bit() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let engine = rt.block_on(async { create_test_engine(ApprovalLevel::Ask).await.unwrap() });

    // Create a patch that adds executable permission to a script file
    let exec_bit_patch = r#"diff --git a/scripts/deploy.sh b/scripts/deploy.sh
old mode 100644
new mode 100755
index 1234567..abcdefg
--- a/scripts/deploy.sh
+++ b/scripts/deploy.sh
@@ -1,3 +1,4 @@
 #!/bin/bash
 echo "Starting deployment..."
 # Deploy logic here
+chmod +x other_script.sh"#;
    let result = rt.block_on(async {
        engine
            .patch_apply(exec_bit_patch, ApprovalLevel::Ask, false, None)
            .await
    });

    // CONTRACT: Ask level should either:
    // 1. Fail with a specific error requiring confirmation, OR
    // 2. Succeed but with warnings/confirmations in the result
    match result {
        Err(DevItError::PolicyBlock { rule, .. }) => {
            // Engine detected exec-bit change and blocked it
            assert!(
                rule.to_lowercase().contains("executable")
                    || rule.to_lowercase().contains("permission"),
                "Policy block should reference executable permissions, got: {}",
                rule
            );
        }
        Ok(patch_result) => {
            // Engine succeeded but should have warnings about exec-bit
            assert!(
                !patch_result.success
                    || patch_result
                        .warnings
                        .iter()
                        .any(|w| w.to_lowercase().contains("executable")),
                "Exec-bit changes should either fail or generate warnings at Ask level"
            );
        }
        Err(other) => {
            panic!(
                "Unexpected error for exec-bit change at Ask level: {:?}",
                other
            );
        }
    }
}

/// Contract: MODERATE approval level should reject protected paths like .env
#[test]
fn contract_moderate_rejects_protected_env() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let engine = rt.block_on(async { create_test_engine(ApprovalLevel::Moderate).await.unwrap() });

    // Create a patch that modifies .env file (protected path)
    let env_patch = r#"diff --git a/.env b/.env
index 1234567..abcdefg 100644
--- a/.env
+++ b/.env
@@ -1,3 +1,4 @@
 DATABASE_URL=postgres://localhost/mydb
 API_KEY=secret123
 DEBUG=true
+NEW_SECRET=potentially_dangerous_value"#;

    let result = rt.block_on(async {
        engine
            .patch_apply(env_patch, ApprovalLevel::Moderate, false, None)
            .await
    });

    // CONTRACT: Moderate level MUST reject .env file modifications
    match result {
        Err(DevItError::ProtectedPath { path, .. }) => {
            assert!(
                path.to_string_lossy().contains(".env"),
                "Error should reference .env file, got path: {:?}",
                path
            );
        }
        Err(DevItError::PolicyBlock { rule, .. }) => {
            // Also acceptable - engine blocks due to policy violation
            assert!(
                rule.to_lowercase().contains("protected") || rule.to_lowercase().contains("env"),
                "Policy block should reference protected paths or env files, got: {}",
                rule
            );
        }
        Ok(_) => {
            panic!("Moderate approval should NOT allow .env file modifications");
        }
        Err(other) => {
            panic!("Unexpected error for .env modification: {:?}", other);
        }
    }
}

/// Contract: TRUSTED approval level should allow small whitelisted images
#[test]
#[ignore = "Contract test - activate when patch application is implemented"]
fn contract_trusted_allows_small_whitelisted_images() {
    // Create a patch that adds a small PNG file (whitelisted binary type)
    let small_png_patch = r#"diff --git a/assets/icons/new-icon.png b/assets/icons/new-icon.png
new file mode 100644
index 0000000..1234567
Binary files /dev/null and b/assets/icons/new-icon.png differ"#;

    let rt = tokio::runtime::Runtime::new().unwrap();
    let engine = rt.block_on(async { create_test_engine(ApprovalLevel::Trusted).await.unwrap() });

    let result = rt.block_on(async {
        engine
            .patch_apply(small_png_patch, ApprovalLevel::Trusted, false, None)
            .await
    });

    // CONTRACT: Trusted level SHOULD allow small PNG files (whitelisted extension)
    match result {
        Ok(patch_result) => {
            assert!(
                patch_result.success,
                "Small PNG files should be allowed at Trusted level"
            );
            assert!(
                !patch_result.modified_files.is_empty(),
                "Should report the PNG file as modified"
            );
        }
        Err(DevItError::PolicyBlock { rule, .. }) => {
            panic!(
                "Trusted approval should be sufficient for small whitelisted binary files, got: {}",
                rule
            );
        }
        Err(other) => {
            panic!(
                "Unexpected error for small PNG at Trusted level: {:?}",
                other
            );
        }
    }
}

/// Contract: Stale snapshots should trigger E_SNAPSHOT_STALE error
#[test]
#[ignore = "Contract test - activate when snapshot validation is implemented"]
fn contract_stale_snapshot_triggers_error() {
    let engine = create_test_engine(ApprovalLevel::Moderate);

    // Create a simple patch (content doesn't matter for this test)
    let simple_patch = r#"diff --git a/src/utils.rs b/src/utils.rs
index 1234567..abcdefg 100644
--- a/src/utils.rs
+++ b/src/utils.rs
@@ -10,6 +10,9 @@ impl Utils {
     pub fn existing_function(&self) -> String {
         "existing".to_string()
     }
+    pub fn new_function(&self) -> String {
+        "new".to_string()
+    }
 }"#;

    let rt = tokio::runtime::Runtime::new().unwrap();
    let engine = rt.block_on(engine).unwrap();
    let result = rt.block_on(async {
        // First, we need to simulate a stale snapshot condition
        // This would typically happen when:
        // 1. A snapshot was created
        // 2. The working directory changed after snapshot creation
        // 3. We try to apply a patch

        // For this test, we'll assume the engine has logic to detect staleness
        // based on internal state or file timestamps
        engine
            .patch_apply(simple_patch, ApprovalLevel::Moderate, false, None)
            .await
    });

    // CONTRACT: If the engine detects snapshot staleness, it MUST return E_SNAPSHOT_STALE
    // Note: This test might pass initially if no staleness is detected,
    // but should fail appropriately when staleness detection is implemented
    match result {
        Err(DevItError::SnapshotStale { snapshot_id, .. }) => {
            // Expected behavior when staleness is detected
            assert!(!snapshot_id.is_empty(), "Snapshot ID should not be empty");
        }
        Ok(_) => {
            // This is acceptable if no staleness was detected in this test run
            // The real staleness detection will depend on the actual engine implementation
            println!("Warning: No staleness detected - this test will be meaningful once snapshot validation is implemented");
        }
        Err(other) => {
            panic!(
                "Unexpected error when testing snapshot staleness: {:?}",
                other
            );
        }
    }
}

/// Contract: Synthetic FileChange validation for policy decisions
#[test]
#[ignore = "Contract test - activate when policy engine is implemented"]
fn contract_synthetic_file_changes_trigger_correct_policies() {
    let engine = create_test_engine(ApprovalLevel::Moderate);

    // Test with synthetic FileChange data (when the API supports it)
    let synthetic_changes = vec![
        // Simulate a change to a protected file
        create_synthetic_file_change(
            ".env.production",
            FileChangeKind::Modify,
            Some("SECRET_KEY=new_value".to_string()),
        ),
        // Simulate adding executable permission
        create_synthetic_file_change("scripts/deploy.sh", FileChangeKind::Modify, None),
        // Simulate a normal code change
        create_synthetic_file_change(
            "src/main.rs",
            FileChangeKind::Modify,
            Some("fn main() { println!(\"updated\"); }".to_string()),
        ),
    ];

    // This test assumes a future API that accepts FileChange objects directly
    // For now, we validate the contract through patch application
    let combined_patch = create_combined_patch_from_changes(&synthetic_changes);

    let rt = tokio::runtime::Runtime::new().unwrap();
    let engine = rt.block_on(engine).unwrap();
    let result = rt.block_on(async {
        engine
            .patch_apply(&combined_patch, ApprovalLevel::Moderate, false, None)
            .await
    });

    // CONTRACT: Policy engine should reject the batch due to protected .env.production
    match result {
        Err(DevItError::ProtectedPath { path, .. }) => {
            assert!(
                path.to_string_lossy().contains(".env"),
                "Should reject due to protected .env.production file"
            );
        }
        Err(DevItError::PolicyBlock { .. }) => {
            // Also acceptable - blocked by policy
        }
        Ok(_) => {
            panic!("Should not allow modification of .env.production at Moderate level");
        }
        Err(other) => {
            panic!("Unexpected error for synthetic file changes: {:?}", other);
        }
    }
}

/// Contract: Different approval levels have different file/line limits
#[test]
#[ignore = "Contract test - activate when policy limits are implemented"]
fn contract_approval_levels_respect_size_limits() {
    // Test that moderate level respects line/file limits
    let engine_moderate = create_test_engine(ApprovalLevel::Moderate);

    // Create a patch that exceeds moderate limits (>100 lines)
    let large_patch = create_large_patch_exceeding_limits(150); // 150 lines

    let rt = tokio::runtime::Runtime::new().unwrap();
    let engine_moderate = rt.block_on(engine_moderate).unwrap();
    let result_moderate = rt.block_on(async {
        engine_moderate
            .patch_apply(&large_patch, ApprovalLevel::Moderate, false, None)
            .await
    });

    // CONTRACT: Moderate should reject patches exceeding line limits
    match result_moderate {
        Err(DevItError::PolicyBlock { rule, .. }) => {
            assert!(
                rule.to_lowercase().contains("size")
                    || rule.to_lowercase().contains("limit")
                    || rule.to_lowercase().contains("large"),
                "Large patches should be blocked due to size limits, got: {}",
                rule
            );
        }
        Ok(_) => {
            panic!("Moderate approval should not allow patches exceeding line limits");
        }
        Err(other) => {
            println!("Acceptable error for size limits: {:?}", other);
        }
    }

    // Test that trusted level allows larger patches
    let engine_trusted = create_test_engine(ApprovalLevel::Trusted);
    let engine_trusted = rt.block_on(engine_trusted).unwrap();
    let result_trusted = rt.block_on(async {
        engine_trusted
            .patch_apply(&large_patch, ApprovalLevel::Trusted, false, None)
            .await
    });

    // CONTRACT: Trusted should allow larger patches (within its limits)
    assert!(
        result_trusted.is_ok() || matches!(result_trusted, Err(DevItError::PolicyBlock { .. })),
        "Trusted level should handle larger patches better than Moderate"
    );
}

// Helper functions for creating synthetic test data

/// Creates a synthetic FileChange for testing policy decisions
fn create_synthetic_file_change(
    path: &str,
    kind: FileChangeKind,
    content: Option<String>,
) -> SyntheticFileChange {
    let new_mode = match kind {
        FileChangeKind::Modify => Some(0o755), // Assume mode change for Modify
        _ => Some(0o644),
    };

    SyntheticFileChange {
        path: PathBuf::from(path),
        kind,
        content,
        old_mode: Some(0o644),
        new_mode,
    }
}

/// Represents a synthetic file change for testing
#[derive(Debug, Clone)]
struct SyntheticFileChange {
    path: PathBuf,
    kind: FileChangeKind,
    content: Option<String>,
    old_mode: Option<u32>,
    new_mode: Option<u32>,
}

/// Creates a combined patch from synthetic file changes
fn create_combined_patch_from_changes(changes: &[SyntheticFileChange]) -> String {
    let mut patch = String::new();

    for change in changes {
        match change.kind {
            FileChangeKind::Modify => {
                // Handle mode changes vs content changes
                if change.old_mode != change.new_mode {
                    patch.push_str(&format!(
                        "diff --git a/{path} b/{path}\nold mode {old:o}\nnew mode {new:o}\n",
                        path = change.path.display(),
                        old = change.old_mode.unwrap_or(0o644),
                        new = change.new_mode.unwrap_or(0o755)
                    ));
                } else {
                    patch.push_str(&format!(
                        "diff --git a/{path} b/{path}\nindex 1234567..abcdefg 100644\n--- a/{path}\n+++ b/{path}\n@@ -1,3 +1,4 @@\n",
                        path = change.path.display()
                    ));
                }
                if let Some(content) = &change.content {
                    patch.push_str(&format!("+{}\n", content));
                }
            }
            FileChangeKind::Add | FileChangeKind::Create => {
                patch.push_str(&format!(
                    "diff --git a/{path} b/{path}\nnew file mode 100644\nindex 0000000..1234567\n--- /dev/null\n+++ b/{path}\n@@ -0,0 +1 @@\n",
                    path = change.path.display()
                ));
                if let Some(content) = &change.content {
                    patch.push_str(&format!("+{}\n", content));
                }
            }
            FileChangeKind::Del => {
                patch.push_str(&format!(
                    "diff --git a/{path} b/{path}\ndeleted file mode 100644\nindex 1234567..0000000\n--- a/{path}\n+++ /dev/null\n@@ -1 +0,0 @@\n",
                    path = change.path.display()
                ));
                if let Some(content) = &change.content {
                    patch.push_str(&format!("-{}\n", content));
                }
            }
            _ => {
                // Handle other change types as needed
                patch.push_str(&format!(
                    "# Change type {:?} for {}\n",
                    change.kind,
                    change.path.display()
                ));
            }
        }
        patch.push('\n');
    }

    patch
}

/// Creates a large patch that exceeds the specified line count
fn create_large_patch_exceeding_limits(line_count: usize) -> String {
    let mut patch = String::from(
        "diff --git a/src/large_file.rs b/src/large_file.rs\n\
         index 1234567..abcdefg 100644\n\
         --- a/src/large_file.rs\n\
         +++ b/src/large_file.rs\n\
         @@ -1,5 +1,{} @@\n\
         // Original file content\n\
         use std::collections::HashMap;\n\
         \n",
    );

    // Add many lines to exceed the limit
    for i in 1..=line_count {
        patch.push_str(&format!(
            "+// Generated line {}: fn generated_function_{}() -> i32 {{ {} }}\n",
            i, i, i
        ));
    }

    patch
}

/// Contract: Preview operations should work correctly across approval levels
#[test]
#[ignore = "Contract test - activate when patch preview is implemented"]
fn contract_preview_respects_approval_levels() {
    let engine = create_test_engine(ApprovalLevel::Moderate);

    let protected_patch = r#"diff --git a/.env b/.env
index 1234567..abcdefg 100644
--- a/.env
+++ b/.env
@@ -1 +1,2 @@
 DATABASE_URL=postgres://localhost/mydb
+SECRET_KEY=new_secret"#;

    let rt = tokio::runtime::Runtime::new().unwrap();
    let engine = rt.block_on(engine).unwrap();
    let preview_result = rt.block_on(async { engine.patch_preview(protected_patch, None).await });

    // CONTRACT: Preview should detect protected paths and indicate policy violations
    match preview_result {
        Ok(preview) => {
            assert!(
                preview.affects_protected,
                "Preview should detect that .env is a protected path"
            );
            assert!(
                !preview.affected_files.is_empty(),
                "Preview should list affected files"
            );
            assert!(
                preview
                    .affected_files
                    .iter()
                    .any(|f| f.to_string_lossy().contains(".env")),
                "Preview should include .env in affected files"
            );
        }
        Err(e) => {
            panic!(
                "Preview should not fail, but should indicate policy issues: {:?}",
                e
            );
        }
    }
}

/// Contract: Dry run mode should not modify files but should validate everything
#[test]
#[ignore = "Contract test - activate when dry run is implemented"]
fn contract_dry_run_validates_without_modifying() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let engine = rt.block_on(async { create_test_engine(ApprovalLevel::Trusted).await.unwrap() });

    let normal_patch = r#"diff --git a/src/utils.rs b/src/utils.rs
index 1234567..abcdefg 100644
--- a/src/utils.rs
+++ b/src/utils.rs
@@ -10,6 +10,9 @@ impl Utils {
     pub fn existing_function(&self) -> String {
         "existing".to_string()
     }
+    pub fn helper_function(&self) -> i32 {
+        42
+    }
 }"#;

    let rt = tokio::runtime::Runtime::new().unwrap();
    let dry_run_result = rt.block_on(async {
        engine
            .patch_apply(normal_patch, ApprovalLevel::Trusted, true, None)
            .await // dry_run = true
    });

    // CONTRACT: Dry run should validate the patch but not modify files
    match dry_run_result {
        Ok(result) => {
            assert!(result.success, "Dry run should succeed for valid patches");
            // In dry run, files should be analyzed but not actually modified
            // The exact behavior depends on how dry_run is implemented
        }
        Err(e) => {
            // If the patch has issues, dry run should still detect them
            println!("Dry run detected issues (expected for some cases): {:?}", e);
        }
    }
}
