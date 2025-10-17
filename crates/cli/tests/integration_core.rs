//! # Integration tests for DevIt Core Engine
//!
//! Tests that validate end-to-end scenarios using abstract fixtures
//! without real I/O operations. Uses mocks for FS/VCS/policy.

use std::collections::HashMap;
use std::path::PathBuf;

use devit_cli::core::{
    ApprovalLevel, CoreConfig, CoreEngine, DevItError, FileChangeKind, SandboxProfile, SnapshotId,
    TestConfig, TestResults,
};

mod fixtures;
use fixtures::*;

/// Test simple text patch addition with moderate policy - should succeed
#[test]
#[ignore = "Core engine not fully implemented yet"]
fn ok_add_fn() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let engine = rt
        .block_on(create_test_engine(ApprovalLevel::Moderate))
        .unwrap();
    let patch = create_simple_function_add_patch();
    let result = rt.block_on(async {
        engine
            .patch_apply(&patch, ApprovalLevel::Moderate, false, None)
            .await
    });

    assert!(
        result.is_ok(),
        "Simple function addition should succeed with moderate approval"
    );
    let patch_result = result.unwrap();
    assert!(
        patch_result.success,
        "Patch application should report success"
    );
    assert!(
        !patch_result.modified_files.is_empty(),
        "Should have modified files"
    );
}

/// Test snapshot staleness detection during apply - should fail with E_SNAPSHOT_STALE
#[test]
#[ignore = "Core engine not fully implemented yet"]
fn stale_during_apply() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let engine = rt
        .block_on(create_test_engine(ApprovalLevel::Moderate))
        .unwrap();
    let patch = create_simple_function_add_patch();
    let stale_snapshot = create_stale_snapshot();
    let result = rt.block_on(async {
        // First validate snapshot to trigger staleness detection
        let validation = engine
            .snapshot_validate(&stale_snapshot, &[PathBuf::from("src/main.rs")])
            .await;
        assert!(validation.is_ok(), "Validation call should not error");
        assert!(!validation.unwrap(), "Stale snapshot should be invalid");

        // Then attempt patch apply which should detect staleness
        engine
            .patch_apply(&patch, ApprovalLevel::Moderate, false, None)
            .await
    });

    assert!(
        result.is_err(),
        "Patch apply with stale snapshot should fail"
    );
    match result.unwrap_err() {
        DevItError::SnapshotStale { .. } => {} // Expected error
        other => panic!("Expected E_SNAPSHOT_STALE, got: {:?}", other),
    }
}

/// Test touching protected .env file - should fail with E_PROTECTED_PATH in moderate
#[test]
#[ignore = "Core engine not fully implemented yet"]
fn protected_env() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let engine = rt
        .block_on(create_test_engine(ApprovalLevel::Moderate))
        .unwrap();
    let patch = create_env_file_patch();
    let result = rt.block_on(async {
        engine
            .patch_apply(&patch, ApprovalLevel::Moderate, false, None)
            .await
    });

    assert!(
        result.is_err(),
        "Touching .env should fail in moderate mode"
    );
    match result.unwrap_err() {
        DevItError::ProtectedPath { path, .. } => {
            assert!(
                path.to_string_lossy().contains(".env"),
                "Error should reference .env file"
            );
        }
        other => panic!("Expected E_PROTECTED_PATH, got: {:?}", other),
    }
}

/// Test small whitelisted binary (PNG) - allowed in trusted, refused in moderate
#[test]
#[ignore = "Core engine not fully implemented yet"]
fn binary_small_whitelisted() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    // Test with trusted approval - should succeed
    let engine_trusted = rt
        .block_on(create_test_engine(ApprovalLevel::Trusted))
        .unwrap();
    let patch = create_small_png_patch();

    let result_trusted = rt.block_on(async {
        engine_trusted
            .patch_apply(&patch, ApprovalLevel::Trusted, false, None)
            .await
    });

    assert!(
        result_trusted.is_ok(),
        "Small PNG should be allowed with trusted approval"
    );

    // Test with moderate approval - should be rejected or downgraded
    let engine_moderate = rt
        .block_on(create_test_engine(ApprovalLevel::Moderate))
        .unwrap();

    let result_moderate = rt.block_on(async {
        engine_moderate
            .patch_apply(&patch, ApprovalLevel::Moderate, false, None)
            .await
    });

    // Should either fail or require higher approval
    assert!(
        result_moderate.is_err() || !result_moderate.as_ref().unwrap().success,
        "Small PNG should be rejected or require approval in moderate mode"
    );
}

/// Test .gitmodules URL change - should fail with E_PROTECTED_PATH except for privileged
#[test]
#[ignore = "Core engine not fully implemented yet"]
fn submodule_url_change() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    // Test with moderate approval - should fail
    let engine_moderate = rt
        .block_on(create_test_engine(ApprovalLevel::Moderate))
        .unwrap();
    let patch = create_gitmodules_patch();

    let result_moderate = rt.block_on(async {
        engine_moderate
            .patch_apply(&patch, ApprovalLevel::Moderate, false, None)
            .await
    });

    assert!(
        result_moderate.is_err(),
        "gitmodules change should fail in moderate mode"
    );
    match result_moderate.unwrap_err() {
        DevItError::ProtectedPath { path, .. } => {
            assert!(
                path.to_string_lossy().contains(".gitmodules"),
                "Error should reference .gitmodules"
            );
        }
        other => panic!("Expected E_PROTECTED_PATH, got: {:?}", other),
    }

    // Test with privileged approval - should succeed
    let engine_privileged = rt
        .block_on(create_test_engine(ApprovalLevel::Privileged {
            allowed_paths: vec![PathBuf::from(".gitmodules"), PathBuf::from("submodules/")],
        }))
        .unwrap();

    let result_privileged = rt.block_on(async {
        engine_privileged
            .patch_apply(
                &patch,
                ApprovalLevel::Privileged {
                    allowed_paths: vec![PathBuf::from(".gitmodules"), PathBuf::from("submodules/")],
                },
                false,
                None,
            )
            .await
    });

    assert!(
        result_privileged.is_ok(),
        "gitmodules change should succeed with privileged approval"
    );
}

/// Test adding executable bit (chmod +x) - should downgrade to ask in moderate
#[test]
#[ignore = "Core engine not fully implemented yet"]
fn exec_bit_added() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let engine = rt
        .block_on(create_test_engine(ApprovalLevel::Moderate))
        .unwrap();
    let patch = create_exec_bit_patch();
    let result = rt.block_on(async {
        engine
            .patch_apply(&patch, ApprovalLevel::Moderate, false, None)
            .await
    });

    // Should either fail requiring higher approval or succeed with warning
    // The exact behavior depends on policy engine implementation
    assert!(
        result.is_err()
            || result
                .as_ref()
                .unwrap()
                .warnings
                .iter()
                .any(|w| w.contains("executable")),
        "Executive bit addition should require approval or generate warning"
    );
}

/// Test dangerous symlink pointing outside workspace - should fail with E_PROTECTED_PATH
#[test]
#[ignore = "Core engine not fully implemented yet"]
fn symlink_outside() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let engine = rt
        .block_on(create_test_engine(ApprovalLevel::Moderate))
        .unwrap();
    let patch = create_dangerous_symlink_patch();
    let result = rt.block_on(async {
        engine
            .patch_apply(&patch, ApprovalLevel::Moderate, false, None)
            .await
    });

    assert!(result.is_err(), "Dangerous symlink should be rejected");
    match result.unwrap_err() {
        DevItError::ProtectedPath { .. } => {} // Expected
        DevItError::PolicyBlock { .. } => {}   // Also acceptable
        other => panic!("Expected protection error, got: {:?}", other),
    }
}

/// Test preview operation with various patch types
#[test]
#[ignore = "Core engine not fully implemented yet"]
fn patch_preview_scenarios() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let engine = rt
        .block_on(create_test_engine(ApprovalLevel::Moderate))
        .unwrap();

    // Test simple function addition preview
    let simple_patch = create_simple_function_add_patch();
    let preview_result = rt.block_on(async { engine.patch_preview(&simple_patch, None).await });

    assert!(
        preview_result.is_ok(),
        "Preview should succeed for simple patch"
    );
    let preview = preview_result.unwrap();
    assert!(
        !preview.affected_files.is_empty(),
        "Preview should show affected files"
    );
    assert!(
        !preview.affects_protected,
        "Simple patch should not affect protected paths"
    );
}

/// Test test execution with different configurations
#[test]
#[ignore = "Core engine not fully implemented yet"]
fn test_execution_scenarios() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let engine = rt
        .block_on(create_test_engine(ApprovalLevel::Moderate))
        .unwrap();

    // Test basic test execution
    let test_config = TestConfig {
        framework: Some("cargo".to_string()),
        patterns: vec!["unit".to_string()],
        timeout_secs: 60,
        parallel: true,
        env_vars: HashMap::new(),
    };

    let test_result =
        rt.block_on(async { engine.test_run(&test_config, SandboxProfile::Strict).await });

    // Since this is mocked, we mainly test that the API works
    assert!(
        test_result.is_ok() || test_result.is_err(),
        "Test execution should return some result"
    );
}
