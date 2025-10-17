use devit_cli::core::{ApprovalLevel, DevItError};

mod fixtures;
use fixtures::create_test_engine;

/// Contract: TRUSTED approval level should allow small whitelisted images
#[test]
fn contract_trusted_allows_small_whitelisted_images() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let engine = rt.block_on(async { create_test_engine(ApprovalLevel::Trusted).await.unwrap() });

    // Create a patch that adds a small PNG file (whitelisted image type)
    let small_png_patch = r#"diff --git a/assets/icon.png b/assets/icon.png
new file mode 100644
index 0000000..1234567
--- /dev/null
+++ b/assets/icon.png
Binary files /dev/null and b/assets/icon.png differ"#;

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async {
        engine
            .patch_apply(small_png_patch, ApprovalLevel::Trusted, false, None)
            .await
    });

    // CONTRACT: Trusted level should allow small whitelisted binary files
    match result {
        Ok(patch_result) => {
            assert!(
                patch_result.success,
                "Trusted level should allow whitelisted images"
            );

            // Check that the PNG file was processed
            let has_png_info = patch_result.info_messages.iter().any(|msg| {
                msg.to_lowercase().contains("png")
                    || msg.to_lowercase().contains("whitelisted")
                    || msg.to_lowercase().contains("binary")
            });

            assert!(
                has_png_info,
                "Expected info message about whitelisted binary file, got: {:?}",
                patch_result.info_messages
            );
        }
        Err(DevItError::PolicyBlock { rule, .. }) => {
            panic!(
                "Trusted level should NOT block whitelisted images, blocked due to: {}",
                rule
            );
        }
        Err(other) => {
            panic!("Unexpected error for whitelisted image: {:?}", other);
        }
    }
}
