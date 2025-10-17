use devit_cli::core::{ApprovalLevel, DevItError};

mod fixtures;
use fixtures::create_test_engine;

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

    let rt = tokio::runtime::Runtime::new().unwrap();
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
            // Engine allowed it but should have warnings about exec-bit changes
            assert!(
                !patch_result.warnings.is_empty(),
                "Ask level should produce warnings for executable permission changes"
            );

            let has_exec_warning = patch_result.warnings.iter().any(|w| {
                w.to_lowercase().contains("executable")
                    || w.to_lowercase().contains("permission")
                    || w.to_lowercase().contains("confirmation")
            });

            assert!(
                has_exec_warning,
                "Expected warning about executable permissions, got warnings: {:?}",
                patch_result.warnings
            );
        }
        Err(other) => {
            panic!("Unexpected error for exec-bit change: {:?}", other);
        }
    }
}
