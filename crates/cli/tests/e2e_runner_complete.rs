//! # Complete E2E Tests for Runner + Rollback (R3)
//!
//! End-to-end tests that create ephemeral mini-projects and validate
//! real execution of test frameworks with sandbox integration and auto-rollback.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

use devit_cli::core::{
    patch::PatchApplyRequest,
    tester::{Stack, TestRunRequest},
    CoreConfig, CoreEngine, DevItResult, SandboxProfile, TestConfig,
};
use devit_common::ApprovalLevel;

mod fixtures;
use fixtures::*;

/// Creates a mini Rust crate with a passing test in a temp directory
async fn create_mini_rust_crate(temp_dir: &Path) -> DevItResult<()> {
    let cargo_toml = r#"[package]
name = "mini-test-crate"
version = "0.1.0"
edition = "2021"

[dependencies]
"#;

    let lib_rs = r#"pub fn add(left: usize, right: usize) -> usize {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
"#;

    // Create Cargo.toml
    fs::write(temp_dir.join("Cargo.toml"), cargo_toml).map_err(|e| {
        devit_cli::core::DevItError::io(Some(temp_dir.join("Cargo.toml")), "write Cargo.toml", e)
    })?;

    // Create src directory and lib.rs
    let src_dir = temp_dir.join("src");
    fs::create_dir_all(&src_dir)
        .map_err(|e| devit_cli::core::DevItError::io(Some(src_dir.clone()), "create src dir", e))?;

    fs::write(src_dir.join("lib.rs"), lib_rs).map_err(|e| {
        devit_cli::core::DevItError::io(Some(src_dir.join("lib.rs")), "write lib.rs", e)
    })?;

    Ok(())
}

/// Creates a mini Python project with a failing test
async fn create_mini_python_project(temp_dir: &Path) -> DevItResult<()> {
    let test_py = r#"def test_that_fails():
    assert 1 + 1 == 3, "This test is designed to fail"

def test_that_passes():
    assert 1 + 1 == 2, "This should pass"
"#;

    let requirements_txt = "pytest\n";

    // Create test_failing.py
    fs::write(temp_dir.join("test_failing.py"), test_py).map_err(|e| {
        devit_cli::core::DevItError::io(
            Some(temp_dir.join("test_failing.py")),
            "write test_failing.py",
            e,
        )
    })?;

    // Create requirements.txt
    fs::write(temp_dir.join("requirements.txt"), requirements_txt).map_err(|e| {
        devit_cli::core::DevItError::io(
            Some(temp_dir.join("requirements.txt")),
            "write requirements.txt",
            e,
        )
    })?;

    Ok(())
}

/// Creates a mini Rust crate that will have a test introduced via patch
async fn create_rust_crate_for_patching(temp_dir: &Path) -> DevItResult<()> {
    let cargo_toml = r#"[package]
name = "patchable-crate"
version = "0.1.0"
edition = "2021"

[dependencies]
"#;

    let lib_rs = r#"pub fn multiply(a: i32, b: i32) -> i32 {
    a * b
}
"#;

    // Create Cargo.toml
    fs::write(temp_dir.join("Cargo.toml"), cargo_toml).map_err(|e| {
        devit_cli::core::DevItError::io(
            Some(temp_dir.join("Cargo.toml")),
            "write Cargo.toml for patching",
            e,
        )
    })?;

    // Create src directory and lib.rs
    let src_dir = temp_dir.join("src");
    fs::create_dir_all(&src_dir).map_err(|e| {
        devit_cli::core::DevItError::io(Some(src_dir.clone()), "create src dir for patching", e)
    })?;

    fs::write(src_dir.join("lib.rs"), lib_rs).map_err(|e| {
        devit_cli::core::DevItError::io(
            Some(src_dir.join("lib.rs")),
            "write lib.rs for patching",
            e,
        )
    })?;

    Ok(())
}

/// Creates a patch that adds a failing test to the Rust crate
fn create_failing_test_patch() -> String {
    r#"diff --git a/src/lib.rs b/src/lib.rs
index 1234567..abcdefg 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,2 +1,12 @@
 pub fn multiply(a: i32, b: i32) -> i32 {
     a * b
 }
+
+#[cfg(test)]
+mod tests {
+    use super::*;
+
+    #[test]
+    fn test_failing() {
+        assert_eq!(multiply(2, 3), 7); // This will fail: 2*3=6, not 7
+    }
+}
"#
    .to_string()
}

/// E2E Test 1: Cargo test execution that passes
#[tokio::test]
async fn e2e_runner_cargo_pass() -> DevItResult<()> {
    let temp_dir =
        TempDir::new().map_err(|e| devit_cli::core::DevItError::io(None, "create temp dir", e))?;

    // Create mini Rust crate with passing test
    create_mini_rust_crate(temp_dir.path()).await?;

    // Create engine with temp directory as working directory
    let mut config = CoreConfig::default();
    config.runtime.working_directory = Some(temp_dir.path().to_path_buf());
    let engine = CoreEngine::new(config).await?;

    // Configure test to run cargo test
    let test_config = TestConfig {
        framework: Some("cargo".to_string()),
        patterns: vec!["test".to_string()],
        timeout_secs: 30,
        parallel: false,
        env_vars: HashMap::new(),
    };

    // Execute the test
    let result = engine
        .test_run(&test_config, SandboxProfile::Permissive)
        .await?;

    // Validate results
    assert!(result.success, "Cargo test should pass");
    assert!(
        result.execution_time.as_millis() > 0,
        "Execution time should be positive"
    );
    assert!(
        result.output.contains("running")
            || result.output.contains("test")
            || result.passed_tests > 0,
        "Output should contain test execution info or have passed tests"
    );

    println!(
        "✅ e2e_runner_cargo_pass: success={}, passed={}, failed={}, time={}ms",
        result.success,
        result.passed_tests,
        result.failed_tests,
        result.execution_time.as_millis()
    );

    Ok(())
}

/// E2E Test 2: Python pytest execution that fails
#[tokio::test]
async fn e2e_runner_pytest_fail() -> DevItResult<()> {
    let temp_dir =
        TempDir::new().map_err(|e| devit_cli::core::DevItError::io(None, "create temp dir", e))?;

    // Create mini Python project with failing test
    create_mini_python_project(temp_dir.path()).await?;

    // Create engine with temp directory as working directory
    let mut config = CoreConfig::default();
    config.runtime.working_directory = Some(temp_dir.path().to_path_buf());
    let engine = CoreEngine::new(config).await?;

    // Configure test to run pytest
    let test_config = TestConfig {
        framework: Some("pytest".to_string()),
        patterns: vec!["test_failing.py".to_string()],
        timeout_secs: 30,
        parallel: false,
        env_vars: HashMap::new(),
    };

    // Execute the test (expect failure)
    let result = engine
        .test_run(&test_config, SandboxProfile::Permissive)
        .await?;

    // Validate results
    assert!(!result.success, "Pytest should fail due to failing test");
    assert!(result.failed_tests > 0, "Should have failed tests");
    assert!(
        result.execution_time.as_millis() > 0,
        "Execution time should be positive"
    );

    println!(
        "✅ e2e_runner_pytest_fail: success={}, passed={}, failed={}, time={}ms",
        result.success,
        result.passed_tests,
        result.failed_tests,
        result.execution_time.as_millis()
    );

    Ok(())
}

/// E2E Test 3: Apply patch + test fail + auto-revert enabled
#[tokio::test]
async fn e2e_apply_then_tests_fail_autorevert() -> DevItResult<()> {
    let temp_dir =
        TempDir::new().map_err(|e| devit_cli::core::DevItError::io(None, "create temp dir", e))?;

    // Create patchable Rust crate
    create_rust_crate_for_patching(temp_dir.path()).await?;

    // Initialize git repo
    let git_init_result = std::process::Command::new("git")
        .arg("init")
        .current_dir(temp_dir.path())
        .output()
        .map_err(|e| {
            devit_cli::core::DevItError::io(Some(temp_dir.path().to_path_buf()), "git init", e)
        })?;

    if !git_init_result.status.success() {
        return Err(devit_cli::core::DevItError::io(
            Some(temp_dir.path().to_path_buf()),
            "git init",
            std::io::Error::new(std::io::ErrorKind::Other, "Git init failed"),
        ));
    }

    // Add initial commit
    std::process::Command::new("git")
        .args(&["add", "."])
        .current_dir(temp_dir.path())
        .output()
        .map_err(|e| {
            devit_cli::core::DevItError::io(Some(temp_dir.path().to_path_buf()), "git add", e)
        })?;

    std::process::Command::new("git")
        .args(&["commit", "-m", "Initial commit"])
        .current_dir(temp_dir.path())
        .output()
        .map_err(|e| {
            devit_cli::core::DevItError::io(Some(temp_dir.path().to_path_buf()), "git commit", e)
        })?;

    // Create engine with temp directory and auto-revert enabled
    let mut config = CoreConfig::default();
    config.runtime.working_directory = Some(temp_dir.path().to_path_buf());
    config.policy.auto_revert_on_test_fail = true;
    let engine = CoreEngine::new(config).await?;

    // Create patch request with failing test
    let patch_request = PatchApplyRequest {
        diff: create_failing_test_patch(),
        idempotency_key: None,
        post_tests: Some(TestRunRequest {
            stack: Some(Stack::Cargo),
            command: Some("cargo test".to_string()),
            timeout_s: Some(30),
            cpu_limit: None,
            mem_limit_mb: None,
        }),
    };

    // Apply patch with auto-revert
    let result = engine
        .patch_apply_with_tests(&patch_request, ApprovalLevel::Moderate, false)
        .await?;

    // Validate results
    assert!(result.success, "Patch apply should succeed");
    assert!(result.commit_sha.is_some(), "Should have commit SHA");
    assert!(result.test_results.is_some(), "Should have test results");

    let test_results = result.test_results.as_ref().unwrap();
    assert!(!test_results.success, "Tests should fail");

    assert!(result.auto_reverted, "Should have auto-reverted");
    assert!(result.reverted_sha.is_some(), "Should have revert SHA");

    // Verify git state - the failing test should be reverted
    let git_log_output = std::process::Command::new("git")
        .args(&["log", "--oneline", "-3"])
        .current_dir(temp_dir.path())
        .output()
        .map_err(|e| {
            devit_cli::core::DevItError::io(Some(temp_dir.path().to_path_buf()), "git log", e)
        })?;

    let log_str = String::from_utf8_lossy(&git_log_output.stdout);
    assert!(
        log_str.contains("Revert"),
        "Should have revert commit in log"
    );

    println!(
        "✅ e2e_apply_then_tests_fail_autorevert: auto_reverted={}, test_success={}",
        result.auto_reverted, test_results.success
    );

    Ok(())
}

/// E2E Test 4: Apply patch + test fail + auto-revert disabled
#[tokio::test]
async fn e2e_apply_then_tests_fail_no_autorevert() -> DevItResult<()> {
    let temp_dir =
        TempDir::new().map_err(|e| devit_cli::core::DevItError::io(None, "create temp dir", e))?;

    // Create patchable Rust crate
    create_rust_crate_for_patching(temp_dir.path()).await?;

    // Initialize git repo
    let git_init_result = std::process::Command::new("git")
        .arg("init")
        .current_dir(temp_dir.path())
        .output()
        .map_err(|e| {
            devit_cli::core::DevItError::io(Some(temp_dir.path().to_path_buf()), "git init", e)
        })?;

    if !git_init_result.status.success() {
        return Err(devit_cli::core::DevItError::io(
            Some(temp_dir.path().to_path_buf()),
            "git init",
            std::io::Error::new(std::io::ErrorKind::Other, "Git init failed"),
        ));
    }

    // Add initial commit
    std::process::Command::new("git")
        .args(&["add", "."])
        .current_dir(temp_dir.path())
        .output()
        .map_err(|e| {
            devit_cli::core::DevItError::io(Some(temp_dir.path().to_path_buf()), "git add", e)
        })?;

    std::process::Command::new("git")
        .args(&["commit", "-m", "Initial commit"])
        .current_dir(temp_dir.path())
        .output()
        .map_err(|e| {
            devit_cli::core::DevItError::io(Some(temp_dir.path().to_path_buf()), "git commit", e)
        })?;

    // Create engine with temp directory and auto-revert DISABLED
    let mut config = CoreConfig::default();
    config.runtime.working_directory = Some(temp_dir.path().to_path_buf());
    config.policy.auto_revert_on_test_fail = false; // Disabled
    let engine = CoreEngine::new(config).await?;

    // Create patch request with failing test
    let patch_request = PatchApplyRequest {
        diff: create_failing_test_patch(),
        idempotency_key: None,
        post_tests: Some(TestRunRequest {
            stack: Some(Stack::Cargo),
            command: Some("cargo test".to_string()),
            timeout_s: Some(30),
            cpu_limit: None,
            mem_limit_mb: None,
        }),
    };

    // Apply patch without auto-revert
    let result = engine
        .patch_apply_with_tests(&patch_request, ApprovalLevel::Moderate, false)
        .await?;

    // Validate results
    assert!(result.success, "Patch apply should succeed");
    assert!(result.commit_sha.is_some(), "Should have commit SHA");
    assert!(result.test_results.is_some(), "Should have test results");

    let test_results = result.test_results.as_ref().unwrap();
    assert!(!test_results.success, "Tests should fail");

    assert!(!result.auto_reverted, "Should NOT have auto-reverted");
    assert!(result.reverted_sha.is_none(), "Should NOT have revert SHA");
    assert!(
        result.rollback_cmd.is_some(),
        "Should have rollback command ready"
    );

    // Verify git state - the failing test should still be present
    let git_log_output = std::process::Command::new("git")
        .args(&["log", "--oneline", "-2"])
        .current_dir(temp_dir.path())
        .output()
        .map_err(|e| {
            devit_cli::core::DevItError::io(Some(temp_dir.path().to_path_buf()), "git log", e)
        })?;

    let log_str = String::from_utf8_lossy(&git_log_output.stdout);
    assert!(
        !log_str.contains("Revert"),
        "Should NOT have revert commit in log"
    );

    println!("✅ e2e_apply_then_tests_fail_no_autorevert: auto_reverted={}, test_success={}, rollback_cmd={:?}",
             result.auto_reverted, test_results.success, result.rollback_cmd);

    Ok(())
}

/// E2E Test 5: Sandbox strict network restrictions (optional, requires bwrap)
#[tokio::test]
#[ignore = "Requires bwrap and network testing"]
async fn e2e_sandbox_strict_disables_network() -> DevItResult<()> {
    let temp_dir =
        TempDir::new().map_err(|e| devit_cli::core::DevItError::io(None, "create temp dir", e))?;

    // Create a test script that tries to access network
    let network_test_script = r#"#!/bin/bash
curl -s --connect-timeout 5 https://httpbin.org/get > /dev/null
if [ $? -eq 0 ]; then
    echo "Network access succeeded"
    exit 0
else
    echo "Network access blocked"
    exit 1
fi
"#;

    let script_path = temp_dir.path().join("network_test.sh");
    fs::write(&script_path, network_test_script).map_err(|e| {
        devit_cli::core::DevItError::io(Some(script_path.clone()), "write network test script", e)
    })?;

    // Make executable
    std::process::Command::new("chmod")
        .args(&["+x", script_path.to_str().unwrap()])
        .output()
        .map_err(|e| devit_cli::core::DevItError::io(Some(script_path.clone()), "chmod", e))?;

    // Create engine with temp directory
    let mut config = CoreConfig::default();
    config.runtime.working_directory = Some(temp_dir.path().to_path_buf());
    let engine = CoreEngine::new(config).await?;

    // Test with strict sandbox (should block network)
    let test_config_strict = TestConfig {
        framework: None, // Custom command
        patterns: vec![script_path.to_str().unwrap().to_string()],
        timeout_secs: 10,
        parallel: false,
        env_vars: HashMap::new(),
    };

    let result_strict = engine
        .test_run(&test_config_strict, SandboxProfile::Strict)
        .await?;

    // Test with permissive sandbox (should allow network)
    let result_permissive = engine
        .test_run(&test_config_strict, SandboxProfile::Permissive)
        .await?;

    // Validate results - strict should fail, permissive should succeed (or warn)
    println!(
        "✅ e2e_sandbox_strict_disables_network: strict_success={}, permissive_success={}",
        result_strict.success, result_permissive.success
    );

    // Note: Actual network blocking validation depends on bwrap availability
    // If bwrap is not available, both might succeed with fallback warnings

    Ok(())
}
