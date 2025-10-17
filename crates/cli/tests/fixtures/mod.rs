//! # Test fixtures and mocks
//!
//! Provides abstract fixtures for integration tests without real I/O operations.
//! All file system, VCS, and policy operations are mocked.

use devit_cli::core::{errors::DevItResult, ApprovalLevel, CoreConfig, CoreEngine, SnapshotId};
use devit_common::SandboxProfile;
use std::fs;
use std::path::PathBuf;
use tempfile;

mod config;
mod mcp;
mod patches;
mod snapshots;

pub use config::*;
pub use mcp::*;
pub use patches::*;
pub use snapshots::*;

/// Creates a test CoreEngine with the specified default approval level
pub async fn create_test_engine(default_approval: ApprovalLevel) -> DevItResult<CoreEngine> {
    let mut config = create_test_config(default_approval);

    let temp = tempfile::tempdir().expect("unable to create test workspace");
    let root = temp.into_path();

    fs::create_dir_all(root.join("scripts"))
        .expect("unable to create scripts directory for test workspace");
    let deploy_path = root.join("scripts/deploy.sh");
    fs::write(
        &deploy_path,
        "#!/bin/bash\necho \"Starting deployment...\"\n# Deploy logic here\n",
    )
    .expect("unable to write deploy.sh fixture");

    // Seed baseline files referenced by patch fixtures so atomic patching succeeds
    fs::write(
        root.join("README.md"),
        "# Project\n\nThis is a test project.\n",
    )
    .expect("unable to write README.md fixture");

    fs::write(root.join(".env"), "API_KEY=secret123\nDEBUG=true\n")
        .expect("unable to write .env fixture");

    fs::write(root.join("script.sh"), "#!/bin/bash\necho \"test\"\n")
        .expect("unable to write script.sh fixture");

    fs::create_dir_all(root.join("src")).expect("unable to create src directory");
    fs::write(
        root.join("src/large_file.rs"),
        "// Original file content\nuse std::collections::HashMap;\nfn main() {\n    println!(\"Hello\");\n}\n",
    )
    .expect("unable to seed src/large_file.rs fixture");

    config.workspace.sandbox_root = Some(root.clone());
    config.runtime.working_directory = Some(root.clone());
    config.sandbox.allowed_directories = vec![root.clone()];

    CoreEngine::new(config).await
}

/// Creates test configuration with specific approval and sandbox settings
pub fn create_test_config(default_approval: ApprovalLevel) -> CoreConfig {
    use devit_cli::core::config::*;
    use std::collections::HashMap;

    let mut config = CoreConfig::default();
    config.backend = BackendConfig {
        kind: "mock".to_string(),
        base_url: "http://localhost:8080".to_string(),
        model: "test-model".to_string(),
        api_key: Some("test-key".to_string()),
        timeout_secs: 30,
        max_retries: 3,
        headers: HashMap::new(),
        parameters: HashMap::new(),
    };
    config.policy = PolicyConfig {
        default_approval_level: default_approval,
        max_files_moderate: 10,
        max_lines_moderate: 100,
        auto_revert_on_test_fail: false,
        protected_paths: vec![
            PathBuf::from(".env"),
            PathBuf::from(".env.local"),
            PathBuf::from(".secrets"),
            PathBuf::from(".gitmodules"),
            PathBuf::from("Dockerfile"),
            PathBuf::from("docker-compose.yml"),
        ],
        small_binary_max_bytes: 1024 * 1024,
        small_binary_ext_whitelist: vec![
            "png".to_string(),
            "jpg".to_string(),
            "gif".to_string(),
            "ico".to_string(),
        ],
        sandbox_profile_default: SandboxProfile::Strict,
    };
    config.sandbox = SandboxConfig {
        enabled: true,
        default_profile: SandboxProfile::Strict,
        cpu_limit_secs: Some(300),
        memory_limit_mb: Some(512),
        network_access: devit_cli::core::config::NetworkAccess::Disabled,
        allowed_directories: vec![PathBuf::from("/tmp")],
        forbidden_directories: vec![PathBuf::from("/etc"), PathBuf::from("/root")],
        preserved_env_vars: vec!["PATH".to_string(), "HOME".to_string()],
        custom_restrictions: HashMap::new(),
    };
    config.git = GitConfig {
        conventional_commits: true,
        max_staged_files: 50,
        auto_commit: false,
        commit_message_template: None,
        sign_commits: false,
        gpg_key_id: None,
        use_git_notes: false,
        protected_branches: vec!["main".to_string(), "master".to_string()],
    };
    config.testing = TestConfig {
        enabled: true,
        default_timeout: std::time::Duration::from_secs(300),
        parallel_execution: true,
        max_parallel_jobs: Some(4),
        auto_detect_frameworks: vec!["cargo".to_string(), "npm".to_string()],
        custom_test_commands: HashMap::new(),
        fail_fast: false,
        test_env_vars: HashMap::new(),
        excluded_paths: vec![],
    };
    config.journal = JournalSettings {
        enabled: true,
        journal_path: PathBuf::from(".devit/journal.log"),
        sign_entries: false,
        signing_key: None,
        max_file_size_mb: 10,
        max_rotated_files: 5,
        include_sensitive_data: false,
        log_levels: vec![devit_cli::core::config::LogLevel::Info],
        custom_fields: HashMap::new(),
    };
    config.runtime = RuntimeConfig {
        colored_output: true,
        verbosity_level: 1,
        show_progress: true,
        working_directory: None,
        validate_config_on_startup: true,
        performance: devit_cli::core::config::PerformanceConfig::default(),
        feature_flags: HashMap::new(),
    };
    config.orchestration = devit_cli::core::config::OrchestrationSettings::default();
    config.orchestration.mode = devit_common::orchestration::OrchestrationMode::Local;
    config.orchestration.auto_start_daemon = false;
    config
}

/// Helper to create test paths
pub fn test_path(path: &str) -> PathBuf {
    PathBuf::from(format!("/workspace/test/{}", path))
}

/// Creates a mock snapshot ID for testing
pub fn create_test_snapshot_id(suffix: &str) -> SnapshotId {
    SnapshotId(format!("snapshot_test_{}", suffix))
}
