//! # Configuration fixtures
//!
//! Mock configurations for testing different approval levels and policies

use devit_cli::core::config::*;
use devit_cli::core::ApprovalLevel;
use devit_common::SandboxProfile;
use std::collections::HashMap;
use std::path::PathBuf;

/// Creates test backend configuration
pub fn create_test_backend_config() -> BackendConfig {
    BackendConfig {
        kind: "mock".to_string(),
        base_url: "http://localhost:8080".to_string(),
        model: "test-model-v1".to_string(),
        api_key: Some("test_api_key_12345".to_string()),
        timeout_secs: 30,
        max_retries: 3,
        headers: HashMap::new(),
        parameters: HashMap::new(),
    }
}

/// Creates policy configuration for testing different approval scenarios
pub fn create_test_policy_config(approval_level: ApprovalLevel) -> PolicyConfig {
    PolicyConfig {
        default_approval_level: approval_level,
        max_files_moderate: 10,
        max_lines_moderate: 100,
        auto_revert_on_test_fail: false,
        protected_paths: vec![
            PathBuf::from(".env"),
            PathBuf::from(".env.local"),
            PathBuf::from(".env.production"),
            PathBuf::from(".secrets"),
            PathBuf::from(".gitmodules"),
            PathBuf::from("Dockerfile"),
            PathBuf::from("docker-compose.yml"),
            PathBuf::from("docker-compose.*.yml"),
            PathBuf::from("kubernetes/"),
            PathBuf::from("k8s/"),
            PathBuf::from(".github/workflows/"),
            PathBuf::from("scripts/deploy/"),
        ],
        small_binary_max_bytes: 1024 * 1024, // 1MB
        small_binary_ext_whitelist: vec![
            "png".to_string(),
            "jpg".to_string(),
            "jpeg".to_string(),
            "gif".to_string(),
            "ico".to_string(),
            "svg".to_string(),
            "webp".to_string(),
            "ttf".to_string(),
            "woff".to_string(),
            "woff2".to_string(),
        ],
        sandbox_profile_default: SandboxProfile::Strict,
    }
}

/// Creates sandbox configuration for testing
pub fn create_test_sandbox_config() -> SandboxConfig {
    SandboxConfig {
        enabled: true,
        default_profile: SandboxProfile::Strict,
        cpu_limit_secs: Some(300),
        memory_limit_mb: Some(512),
        network_access: NetworkAccess::Disabled,
        allowed_directories: vec![PathBuf::from("/tmp")],
        forbidden_directories: vec![PathBuf::from("/etc")],
        preserved_env_vars: vec!["PATH".to_string()],
        custom_restrictions: HashMap::new(),
    }
}

/// Creates git configuration for testing
pub fn create_test_git_config() -> GitConfig {
    GitConfig {
        conventional_commits: true,
        max_staged_files: 50,
        auto_commit: false,
        commit_message_template: None,
        sign_commits: false,
        gpg_key_id: None,
        use_git_notes: false,
        protected_branches: vec!["main".to_string(), "master".to_string()],
    }
}

/// Creates testing configuration for testing
pub fn create_test_testing_config() -> TestConfig {
    TestConfig {
        enabled: true,
        default_timeout: std::time::Duration::from_secs(300),
        parallel_execution: true,
        max_parallel_jobs: Some(4),
        auto_detect_frameworks: vec!["cargo".to_string()],
        custom_test_commands: HashMap::new(),
        fail_fast: false,
        test_env_vars: HashMap::new(),
        excluded_paths: vec![],
    }
}

/// Creates journal configuration for testing
pub fn create_test_journal_settings() -> JournalSettings {
    JournalSettings {
        enabled: true,
        journal_path: PathBuf::from(".devit/journal.log"),
        sign_entries: false,
        signing_key: None,
        max_file_size_mb: 10,
        max_rotated_files: 5,
        include_sensitive_data: false,
        log_levels: vec![LogLevel::Info],
        custom_fields: HashMap::new(),
    }
}

/// Creates runtime configuration for testing
pub fn create_test_runtime_config() -> RuntimeConfig {
    RuntimeConfig {
        colored_output: true,
        verbosity_level: 1,
        show_progress: true,
        working_directory: None,
        validate_config_on_startup: true,
        performance: PerformanceConfig::default(),
        feature_flags: HashMap::new(),
    }
}

/// Creates orchestration configuration for testing
pub fn create_test_orchestration_settings() -> OrchestrationSettings {
    OrchestrationSettings::default()
}

/// Creates configurations for different test scenarios
pub fn create_scenario_configs() -> HashMap<String, CoreConfig> {
    let mut configs = HashMap::new();

    // Strict configuration - high security
    let mut strict = CoreConfig::default();
    strict.backend = create_test_backend_config();
    strict.policy = create_test_policy_config(ApprovalLevel::Ask);
    strict.sandbox = SandboxConfig {
        enabled: true,
        default_profile: SandboxProfile::Strict,
        cpu_limit_secs: Some(60),
        memory_limit_mb: Some(256),
        network_access: NetworkAccess::Disabled,
        allowed_directories: vec![],
        forbidden_directories: vec![PathBuf::from("/etc"), PathBuf::from("/root")],
        preserved_env_vars: vec![],
        custom_restrictions: HashMap::new(),
    };
    strict.git = create_test_git_config();
    strict.testing = create_test_testing_config();
    strict.journal = create_test_journal_settings();
    strict.runtime = create_test_runtime_config();
    strict.orchestration = create_test_orchestration_settings();
    configs.insert("strict".to_string(), strict);

    // Permissive configuration - lower security for testing
    let mut permissive = CoreConfig::default();
    permissive.backend = create_test_backend_config();
    permissive.policy = PolicyConfig {
        default_approval_level: ApprovalLevel::Trusted,
        max_lines_moderate: 500,
        max_files_moderate: 50,
        auto_revert_on_test_fail: false,
        protected_paths: vec![PathBuf::from(".env")],
        small_binary_max_bytes: 10 * 1024 * 1024,
        small_binary_ext_whitelist: vec!["png".to_string(), "jpg".to_string(), "gif".to_string()],
        sandbox_profile_default: SandboxProfile::Permissive,
    };
    permissive.sandbox = SandboxConfig {
        enabled: true,
        default_profile: SandboxProfile::Permissive,
        cpu_limit_secs: Some(600),
        memory_limit_mb: Some(1024),
        network_access: NetworkAccess::LocalhostOnly,
        allowed_directories: vec![PathBuf::from("/tmp"), PathBuf::from(".")],
        forbidden_directories: vec![PathBuf::from("/etc")],
        preserved_env_vars: vec!["PATH".to_string(), "HOME".to_string()],
        custom_restrictions: HashMap::new(),
    };
    permissive.git = create_test_git_config();
    permissive.testing = create_test_testing_config();
    permissive.journal = create_test_journal_settings();
    permissive.runtime = create_test_runtime_config();
    permissive.orchestration = create_test_orchestration_settings();
    configs.insert("permissive".to_string(), permissive);

    // Privileged configuration - for infrastructure changes
    let mut privileged = CoreConfig::default();
    privileged.backend = create_test_backend_config();
    privileged.policy = create_test_policy_config(ApprovalLevel::Privileged {
        allowed_paths: vec![
            PathBuf::from(".gitmodules"),
            PathBuf::from("Dockerfile"),
            PathBuf::from("docker-compose.yml"),
            PathBuf::from("kubernetes/"),
            PathBuf::from(".github/workflows/"),
        ],
    });
    privileged.sandbox = SandboxConfig {
        enabled: true,
        default_profile: SandboxProfile::Strict,
        cpu_limit_secs: Some(900),
        memory_limit_mb: Some(2048),
        network_access: NetworkAccess::Full,
        allowed_directories: vec![PathBuf::from("/")],
        forbidden_directories: vec![],
        preserved_env_vars: vec!["PATH".to_string(), "HOME".to_string()],
        custom_restrictions: HashMap::new(),
    };
    privileged.git = create_test_git_config();
    privileged.testing = create_test_testing_config();
    privileged.journal = create_test_journal_settings();
    privileged.runtime = create_test_runtime_config();
    privileged.orchestration = create_test_orchestration_settings();
    configs.insert("privileged".to_string(), privileged);

    configs
}
