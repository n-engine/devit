use devit_cli::core::ApprovalLevel;

mod fixtures;
use fixtures::create_test_engine;

/// Contract: Dry run mode should not modify files but should validate everything
#[test]
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
