//! # DevIt Core Configuration
//!
//! Configuration structures for the DevIt core engine.
//! Handles backend configuration, policy settings, and runtime options.
//!
//! ## Architecture
//!
//! The configuration system provides comprehensive control over all aspects
//! of DevIt operation:
//!
//! - **Backend Configuration**: LLM integration and API settings
//! - **Policy Configuration**: Security policies and approval workflows
//! - **Sandbox Configuration**: Process isolation and resource limits
//! - **Git Configuration**: Version control integration settings
//! - **Testing Configuration**: Test execution and framework settings
//! - **Journal Configuration**: Audit logging and compliance settings
//! - **Runtime Configuration**: General behavior and performance settings
//!
//! ## Configuration Loading
//!
//! Configuration is loaded from multiple sources in order of precedence:
//! 1. Environment variables (DEVIT_*)
//! 2. Configuration files (devit.toml, .devit/devit.toml)
//! 3. Built-in defaults

use std::collections::HashMap;
use std::env;
use std::fs;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as AnyhowContext, Result as AnyhowResult};
use once_cell::sync::Lazy;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;
use toml::Value;

use devit_common::{ApprovalLevel, SandboxProfile};

const DEFAULT_PROTECTED_PATHS: &[&str] = &[".git", "Cargo.toml", "package.json", ".env"];

const DEFAULT_SMALL_BINARY_EXTS: &[&str] = &["png", "jpg", "jpeg", "ico", "woff", "woff2"];

const DEFAULT_MAX_FILES_MODERATE: usize = 10;
const DEFAULT_MAX_LINES_MODERATE: usize = 400;
const DEFAULT_SMALL_BINARY_MAX_BYTES: u64 = 1_048_576; // 1 MiB

/// Core configuration for the DevIt engine.
///
/// Contains all settings needed to initialize and operate the core engine,
/// including backend settings, policies, and operational parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CoreConfig {
    /// Backend configuration for LLM integration
    pub backend: BackendConfig,

    /// Policy configuration for approval and security
    pub policy: PolicyConfig,

    /// Sandbox configuration for process isolation
    pub sandbox: SandboxConfig,

    /// Git and VCS configuration
    pub git: GitConfig,

    /// Test execution configuration
    pub testing: TestConfig,

    /// Journal and audit configuration
    pub journal: JournalSettings,

    /// Runtime behavior configuration
    pub runtime: RuntimeConfig,

    /// Workspace sandbox configuration
    pub workspace: WorkspaceConfig,

    /// AI orchestration configuration
    pub orchestration: OrchestrationSettings,

    /// Tools configuration (MCP helpers, utilities)
    pub tools: ToolsConfig,

    /// Compat: project metadata for legacy CLI chemins
    #[serde(default)]
    pub project: Option<ProjectCfg>,

    /// Compat: approvals (legacy)
    #[serde(default)]
    pub approvals: Option<ApprovalsCfg>,

    /// Compat: test preferences (legacy)
    #[serde(default)]
    pub tests: Option<TestsCfg>,

    /// Compat: monitoring section (legacy)
    #[serde(default)]
    pub monitoring: Option<MonitoringCfg>,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            backend: BackendConfig::default(),
            policy: PolicyConfig::default(),
            sandbox: SandboxConfig::default(),
            git: GitConfig::default(),
            testing: TestConfig::default(),
            journal: JournalSettings::default(),
            runtime: RuntimeConfig::default(),
            workspace: WorkspaceConfig::default(),
            orchestration: OrchestrationSettings::default(),
            tools: ToolsConfig::default(),
            project: Some(ProjectCfg::default()),
            approvals: Some(ApprovalsCfg::default()),
            tests: Some(TestsCfg::default()),
            monitoring: Some(MonitoringCfg::default()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct OrchestrationSettings {
    pub base: devit_common::orchestration::OrchestrationConfig,
}

impl Default for OrchestrationSettings {
    fn default() -> Self {
        Self {
            base: devit_common::orchestration::OrchestrationConfig::default(),
        }
    }
}

impl Deref for OrchestrationSettings {
    type Target = devit_common::orchestration::OrchestrationConfig;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl DerefMut for OrchestrationSettings {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}

impl From<OrchestrationSettings> for devit_common::orchestration::OrchestrationConfig {
    fn from(value: OrchestrationSettings) -> Self {
        value.base
    }
}

impl From<devit_common::orchestration::OrchestrationConfig> for OrchestrationSettings {
    fn from(base: devit_common::orchestration::OrchestrationConfig) -> Self {
        Self { base }
    }
}

#[derive(Serialize, Deserialize)]
struct OrchestrationConfigSerde {
    #[serde(default)]
    max_concurrent_tasks: usize,
    #[serde(default)]
    default_timeout_secs: u64,
    #[serde(default)]
    default_watch_patterns: Vec<String>,
    #[serde(default = "serde_default_mode")]
    mode: devit_common::orchestration::OrchestrationMode,
    #[serde(default)]
    daemon_socket: Option<String>,
    #[serde(default = "serde_default_auto_start")]
    auto_start_daemon: bool,
    #[serde(default)]
    daemon_start_timeout_ms: u64,
    #[serde(default)]
    capabilities: devit_common::orchestration::OrchestrationCapabilities,
}

const fn serde_default_mode() -> devit_common::orchestration::OrchestrationMode {
    devit_common::orchestration::OrchestrationMode::Auto
}

const fn serde_default_auto_start() -> bool {
    true
}

impl Serialize for OrchestrationSettings {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let helper = OrchestrationConfigSerde {
            max_concurrent_tasks: self.base.max_concurrent_tasks,
            default_timeout_secs: self.base.default_timeout_secs,
            default_watch_patterns: self.base.default_watch_patterns.clone(),
            mode: self.base.mode,
            daemon_socket: self.base.daemon_socket.clone(),
            auto_start_daemon: self.base.auto_start_daemon,
            daemon_start_timeout_ms: self.base.daemon_start_timeout_ms,
            capabilities: self.base.capabilities.clone(),
        };
        helper.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for OrchestrationSettings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let helper = OrchestrationConfigSerde::deserialize(deserializer)?;
        Ok(Self {
            base: devit_common::orchestration::OrchestrationConfig {
                max_concurrent_tasks: helper.max_concurrent_tasks,
                default_timeout_secs: if helper.default_timeout_secs == 0 {
                    devit_common::orchestration::DEFAULT_TIMEOUT_SECS
                } else {
                    helper.default_timeout_secs
                },
                default_watch_patterns: if helper.default_watch_patterns.is_empty() {
                    devit_common::orchestration::OrchestrationConfig::default()
                        .default_watch_patterns
                } else {
                    helper.default_watch_patterns
                },
                mode: helper.mode,
                daemon_socket: helper.daemon_socket.or_else(|| {
                    Some(devit_common::orchestration::DEFAULT_DAEMON_SOCKET.to_string())
                }),
                auto_start_daemon: helper.auto_start_daemon,
                daemon_start_timeout_ms: if helper.daemon_start_timeout_ms == 0 {
                    devit_common::orchestration::default_daemon_start_timeout_ms()
                } else {
                    helper.daemon_start_timeout_ms
                },
                capabilities: helper.capabilities,
            },
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolsConfig {
    pub screenshot: ScreenshotToolConfig,
    pub exec: ExecToolConfig,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            screenshot: ScreenshotToolConfig::default(),
            exec: ExecToolConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ScreenshotToolConfig {
    pub enabled: bool,
    #[serde(default)]
    pub backend: ScreenshotBackend,
    #[serde(default = "default_screenshot_format")]
    pub format: String,
    #[serde(default)]
    pub output_dir: Option<String>,
}

impl Default for ScreenshotToolConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            backend: ScreenshotBackend::default(),
            format: default_screenshot_format(),
            output_dir: None,
        }
    }
}

fn default_screenshot_format() -> String {
    "png".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenshotBackend {
    Scrot,
    Imagemagick,
    Native,
}

impl Default for ScreenshotBackend {
    fn default() -> Self {
        #[cfg(windows)]
        {
            ScreenshotBackend::Native
        }
        #[cfg(not(windows))]
        {
            ScreenshotBackend::Scrot
        }
    }
}

/// Exec tool configuration (devit_exec MVP)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ExecToolConfig {
    /// RLIMIT_NOFILE (max open file descriptors)
    pub rlimit_nofile: u64,
    /// RLIMIT_NPROC (max processes)
    pub rlimit_nproc: u64,
    /// RLIMIT_AS (address space in GB)
    pub rlimit_as_gb: u64,
    /// RLIMIT_CPU (CPU time in seconds)
    pub rlimit_cpu_secs: u64,
    /// Max lifetime per process (seconds)
    pub max_lifetime_secs: u64,
    /// Allowlist of binary paths
    pub binary_allowlist: Vec<String>,
}

impl Default for ExecToolConfig {
    fn default() -> Self {
        Self {
            rlimit_nofile: 1024,
            rlimit_nproc: 128,
            rlimit_as_gb: 2,         // 2GB
            rlimit_cpu_secs: 300,    // 5 minutes
            max_lifetime_secs: 3600, // 1 hour
            binary_allowlist: vec!["/usr/bin/*".to_string(), "/bin/*".to_string()],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectCfg {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub default_language: String,
}

impl ProjectCfg {
    pub fn with_defaults() -> Self {
        Self {
            name: "default".to_string(),
            default_language: "en".to_string(),
        }
    }
}

impl Default for ProjectCfg {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalsCfg {
    #[serde(default)]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub max_auto_approve_size: usize,
}

impl ApprovalsCfg {
    pub fn with_defaults() -> Self {
        Self {
            timeout_seconds: 300,
            max_auto_approve_size: 10_000,
        }
    }
}

impl Default for ApprovalsCfg {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestsCfg {
    #[serde(default)]
    pub default_timeout: u64,
    #[serde(default)]
    pub parallel_jobs: usize,
}

impl TestsCfg {
    pub fn with_defaults() -> Self {
        Self {
            default_timeout: 300,
            parallel_jobs: 4,
        }
    }
}

impl Default for TestsCfg {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoringCfg {
    #[serde(default)]
    pub metrics_enabled: bool,
    #[serde(default)]
    pub metrics_port: u16,
}

impl MonitoringCfg {
    pub fn with_defaults() -> Self {
        Self {
            metrics_enabled: false,
            metrics_port: 9090,
        }
    }
}

impl Default for MonitoringCfg {
    fn default() -> Self {
        Self::with_defaults()
    }
}

/// Backend configuration for LLM integration.
///
/// Configures how the core engine communicates with language models
/// for patch generation and other AI-assisted operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendConfig {
    /// Type of backend (openai_like, ollama, etc.)
    pub kind: String,

    /// Base URL for the backend API
    pub base_url: String,

    /// Model identifier to use
    pub model: String,

    /// API key for authentication (optional)
    pub api_key: Option<String>,

    /// Request timeout in seconds
    pub timeout_secs: u64,

    /// Maximum retries for failed requests
    pub max_retries: u32,

    /// Additional headers for requests
    pub headers: HashMap<String, String>,

    /// Custom parameters for the backend
    pub parameters: HashMap<String, serde_json::Value>,
}

impl Default for BackendConfig {
    fn default() -> Self {
        Self {
            kind: "openai_like".to_string(),
            base_url: "http://localhost:11434/v1".to_string(),
            model: "llama3.1:8b".to_string(),
            api_key: None,
            timeout_secs: 120,
            max_retries: 3,
            headers: HashMap::new(),
            parameters: HashMap::new(),
        }
    }
}

/// Policy configuration for approval and security.
///
/// Values are loaded from `devit.toml` when available and can be overwritten
/// with `DEVIT_*` environment variables. Missing values fall back to the
/// built-in defaults defined here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyConfig {
    /// Default approval level for operations
    pub default_approval_level: ApprovalLevel,

    /// Maximum number of files that can be handled at moderate level
    pub max_files_moderate: usize,

    /// Maximum number of touched lines for moderate level operations
    pub max_lines_moderate: usize,

    /// Paths that are considered protected and require elevated approval
    pub protected_paths: Vec<PathBuf>,

    /// Maximum size (in bytes) for a binary to be considered "small"
    pub small_binary_max_bytes: u64,

    /// Extensions allowed for small binaries under relaxed rules
    pub small_binary_ext_whitelist: Vec<String>,

    /// Default sandbox profile for operations
    pub sandbox_profile_default: SandboxProfile,

    /// Whether to automatically revert patches if post-tests fail
    pub auto_revert_on_test_fail: bool,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self::from_sources(None)
    }
}

/// Errors that can occur while loading the policy configuration.
#[derive(Debug, Error)]
pub enum PolicyConfigError {
    /// Raised when the configuration file cannot be read.
    #[error("failed to read policy config at {path:?}")]
    Io {
        /// Path that failed to load
        path: PathBuf,
        /// Source I/O error
        #[source]
        source: std::io::Error,
    },

    /// Raised when the configuration file cannot be parsed as TOML.
    #[error("failed to parse policy config at {path:?}")]
    Parse {
        /// Path that failed to parse
        path: PathBuf,
        /// Underlying TOML parser error
        #[source]
        source: toml::de::Error,
    },
}

impl PolicyConfig {
    /// Returns the base defaults without reading disk or environment.
    pub fn builtin_defaults() -> Self {
        Self {
            default_approval_level: ApprovalLevel::Ask,
            max_files_moderate: DEFAULT_MAX_FILES_MODERATE,
            max_lines_moderate: DEFAULT_MAX_LINES_MODERATE,
            protected_paths: DEFAULT_PROTECTED_PATHS.iter().map(PathBuf::from).collect(),
            small_binary_max_bytes: DEFAULT_SMALL_BINARY_MAX_BYTES,
            small_binary_ext_whitelist: DEFAULT_SMALL_BINARY_EXTS
                .iter()
                .map(|ext| ext.to_string())
                .collect(),
            sandbox_profile_default: SandboxProfile::Strict,
            auto_revert_on_test_fail: true, // Enable by default for safety
        }
    }

    /// Loads configuration from the given path, applying environment overrides.
    pub fn load_from_path<P: AsRef<Path>>(path: P) -> Result<Self, PolicyConfigError> {
        let path_ref = path.as_ref();
        let contents = fs::read_to_string(path_ref).map_err(|source| PolicyConfigError::Io {
            path: path_ref.to_path_buf(),
            source,
        })?;

        let root: Value = toml::from_str(&contents).map_err(|source| PolicyConfigError::Parse {
            path: path_ref.to_path_buf(),
            source,
        })?;

        let mut config = Self::builtin_defaults();
        if let Some(policy_table) = root.get("policy").and_then(Value::as_table) {
            config.apply_policy_table(policy_table);
        }

        config.apply_env_overrides();
        Ok(config)
    }

    /// Loads configuration from standard sources (path override, env, defaults).
    pub fn from_sources(path_override: Option<PathBuf>) -> Self {
        if let Some(path) = Self::discover_config_path(path_override) {
            if let Ok(config) = Self::load_from_path(&path) {
                return config;
            }
        }

        let mut config = Self::builtin_defaults();
        config.apply_env_overrides();
        config
    }

    /// Returns the configured default approval level.
    pub fn default_approval(&self) -> &ApprovalLevel {
        &self.default_approval_level
    }

    /// Returns the configured protected paths list.
    pub fn protected_paths(&self) -> &[PathBuf] {
        &self.protected_paths
    }

    /// Returns the maximum number of files for moderate approval level.
    pub fn max_files_moderate(&self) -> usize {
        self.max_files_moderate
    }

    /// Returns the maximum number of lines for moderate approval level.
    pub fn max_lines_moderate(&self) -> usize {
        self.max_lines_moderate
    }

    /// Returns the maximum size in bytes for small binaries.
    pub fn small_binary_max_bytes(&self) -> u64 {
        self.small_binary_max_bytes
    }

    /// Returns the whitelist of extensions for small binaries.
    pub fn small_binary_ext_whitelist(&self) -> &[String] {
        &self.small_binary_ext_whitelist
    }

    /// Returns the default sandbox profile.
    pub fn sandbox_profile_default(&self) -> &SandboxProfile {
        &self.sandbox_profile_default
    }

    /// Returns whether auto-revert on test failure is enabled.
    pub fn auto_revert_on_test_fail(&self) -> bool {
        self.auto_revert_on_test_fail
    }

    /// Applies values from a `[policy]` table onto the configuration.
    fn apply_policy_table(&mut self, table: &toml::value::Table) {
        if let Some(value) = table
            .get("default_approval")
            .and_then(Value::as_str)
            .and_then(Self::parse_approval_level)
        {
            self.default_approval_level = value;
        }

        if let Some(value) = table
            .get("max_files_moderate")
            .and_then(Self::value_as_usize)
        {
            self.max_files_moderate = value;
        }

        if let Some(value) = table
            .get("max_lines_moderate")
            .and_then(Self::value_as_usize)
        {
            self.max_lines_moderate = value;
        }

        if let Some(array) = table.get("protected_paths").and_then(Value::as_array) {
            let mut paths = Vec::new();
            for entry in array {
                if let Some(raw) = entry.as_str() {
                    let trimmed = raw.trim();
                    if !trimmed.is_empty() {
                        paths.push(PathBuf::from(trimmed));
                    }
                }
            }
            self.protected_paths = paths;
        }

        if let Some(value) = table
            .get("small_binary_max_bytes")
            .and_then(Self::value_as_u64)
        {
            self.small_binary_max_bytes = value;
        }

        if let Some(array) = table
            .get("small_binary_ext_whitelist")
            .and_then(Value::as_array)
        {
            let mut extensions = Vec::new();
            for entry in array {
                if let Some(raw) = entry.as_str() {
                    let cleaned = Self::clean_extension(raw);
                    if !cleaned.is_empty() {
                        extensions.push(cleaned);
                    }
                }
            }

            Self::dedup_extensions(&mut extensions);
            self.small_binary_ext_whitelist = extensions;
        }

        if let Some(value) = table
            .get("sandbox_profile_default")
            .and_then(Value::as_str)
            .and_then(Self::parse_sandbox_profile)
        {
            self.sandbox_profile_default = value;
        }

        if let Some(value) = table
            .get("auto_revert_on_test_fail")
            .and_then(Value::as_bool)
        {
            self.auto_revert_on_test_fail = value;
        }
    }

    /// Applies environment variable overrides using the `DEVIT_*` namespace.
    fn apply_env_overrides(&mut self) {
        if let Ok(value) = env::var("DEVIT_DEFAULT_APPROVAL") {
            if let Some(level) = Self::parse_approval_level(value.trim()) {
                self.default_approval_level = level;
            }
        }

        if let Ok(value) = env::var("DEVIT_MAX_FILES_MODERATE") {
            if let Ok(parsed) = value.trim().parse::<usize>() {
                self.max_files_moderate = parsed;
            }
        }

        if let Ok(value) = env::var("DEVIT_MAX_LINES_MODERATE") {
            if let Ok(parsed) = value.trim().parse::<usize>() {
                self.max_lines_moderate = parsed;
            }
        }

        if let Ok(value) = env::var("DEVIT_PROTECTED_PATHS") {
            let trimmed = value.trim();
            let mut paths = Vec::new();

            if !trimmed.is_empty() {
                if trimmed.contains(',') {
                    paths = trimmed
                        .split(',')
                        .map(str::trim)
                        .filter(|segment| !segment.is_empty())
                        .map(PathBuf::from)
                        .collect();
                }

                if paths.is_empty() {
                    paths = env::split_paths(trimmed)
                        .filter(|p| !p.as_os_str().is_empty())
                        .collect();

                    if paths.is_empty() {
                        paths.push(PathBuf::from(trimmed));
                    }
                }
            }

            self.protected_paths = paths;
        }

        if let Ok(value) = env::var("DEVIT_SMALL_BINARY_MAX_BYTES") {
            if let Ok(parsed) = value.trim().parse::<u64>() {
                self.small_binary_max_bytes = parsed;
            }
        }

        if let Ok(value) = env::var("DEVIT_SMALL_BINARY_EXT_WHITELIST") {
            let mut extensions = value
                .split(',')
                .map(Self::clean_extension)
                .filter(|ext| !ext.is_empty())
                .collect::<Vec<_>>();

            if extensions.is_empty() {
                extensions = value
                    .split_whitespace()
                    .map(Self::clean_extension)
                    .filter(|ext| !ext.is_empty())
                    .collect();
            }

            Self::dedup_extensions(&mut extensions);
            self.small_binary_ext_whitelist = extensions;
        }

        if let Ok(value) = env::var("DEVIT_SANDBOX_PROFILE_DEFAULT") {
            if let Some(profile) = Self::parse_sandbox_profile(value.trim()) {
                self.sandbox_profile_default = profile;
            }
        }
    }

    /// Discovers the configuration path to use.
    fn discover_config_path(path_override: Option<PathBuf>) -> Option<PathBuf> {
        if let Some(path) = path_override {
            return Some(path);
        }

        if let Ok(from_env) = env::var("DEVIT_CONFIG") {
            let trimmed = from_env.trim();
            if !trimmed.is_empty() {
                return Some(PathBuf::from(trimmed));
            }
        }

        let candidates = [
            PathBuf::from("devit.toml"),
            Path::new(".devit").join("devit.toml"),
        ];

        for candidate in candidates {
            if candidate.exists() {
                return Some(candidate);
            }
        }

        None
    }

    /// Parses a string into an approval level.
    fn parse_approval_level(value: &str) -> Option<ApprovalLevel> {
        match value.trim().to_ascii_lowercase().as_str() {
            "untrusted" => Some(ApprovalLevel::Untrusted),
            "ask" => Some(ApprovalLevel::Ask),
            "moderate" => Some(ApprovalLevel::Moderate),
            "trusted" => Some(ApprovalLevel::Trusted),
            _ => None,
        }
    }

    /// Parses a string into a sandbox profile.
    fn parse_sandbox_profile(value: &str) -> Option<SandboxProfile> {
        match value.trim().to_ascii_lowercase().as_str() {
            "permissive" => Some(SandboxProfile::Permissive),
            "strict" => Some(SandboxProfile::Strict),
            _ => None,
        }
    }

    /// Attempts to parse a TOML value as usize.
    fn value_as_usize(value: &Value) -> Option<usize> {
        match value {
            Value::Integer(int) if *int >= 0 => (*int).try_into().ok(),
            Value::String(s) => s.trim().parse::<usize>().ok(),
            _ => None,
        }
    }

    /// Attempts to parse a TOML value as u64.
    fn value_as_u64(value: &Value) -> Option<u64> {
        match value {
            Value::Integer(int) if *int >= 0 => (*int).try_into().ok(),
            Value::String(s) => s.trim().parse::<u64>().ok(),
            _ => None,
        }
    }

    /// Normalises an extension by trimming whitespace, removing leading dots and lowering case.
    fn clean_extension(raw: &str) -> String {
        raw.trim().trim_start_matches('.').to_ascii_lowercase()
    }

    /// Deduplicates and sorts extension values for deterministic output.
    fn dedup_extensions(exts: &mut Vec<String>) {
        exts.sort_unstable();
        exts.dedup();
    }
}

/// Custom policy rule definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRuleConfig {
    /// Unique identifier for the rule
    pub id: String,

    /// Human-readable name for the rule
    pub name: String,

    /// Pattern to match against (glob or regex)
    pub pattern: String,

    /// Type of pattern matching
    pub pattern_type: PatternType,

    /// Required approval level for matches
    pub required_approval: ApprovalLevel,

    /// Whether this rule blocks the operation entirely
    pub blocking: bool,

    /// Optional description of the rule
    pub description: Option<String>,
}

/// Type of pattern matching for policy rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PatternType {
    /// Glob pattern matching
    Glob,
    /// Regular expression matching
    Regex,
    /// Exact string matching
    Exact,
}

/// Sandbox configuration for process isolation.
///
/// Controls how operations are isolated and what resources
/// they can access during execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    /// Whether sandbox is enabled by default
    pub enabled: bool,

    /// Default sandbox profile to use
    pub default_profile: SandboxProfile,

    /// CPU limit in seconds per operation
    pub cpu_limit_secs: Option<u64>,

    /// Memory limit in megabytes
    pub memory_limit_mb: Option<u64>,

    /// Network access policy
    pub network_access: NetworkAccess,

    /// Allowed directories for file access
    pub allowed_directories: Vec<PathBuf>,

    /// Forbidden directories that should never be accessed
    pub forbidden_directories: Vec<PathBuf>,

    /// Environment variables to preserve in sandbox
    pub preserved_env_vars: Vec<String>,

    /// Additional sandbox restrictions
    pub custom_restrictions: HashMap<String, serde_json::Value>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            default_profile: SandboxProfile::Strict,
            cpu_limit_secs: Some(300),   // 5 minutes
            memory_limit_mb: Some(2048), // 2GB
            network_access: NetworkAccess::Disabled,
            allowed_directories: vec![PathBuf::from(".")],
            forbidden_directories: vec![
                PathBuf::from("/etc"),
                PathBuf::from("/usr"),
                PathBuf::from("/bin"),
                PathBuf::from("/sbin"),
            ],
            preserved_env_vars: vec!["PATH".to_string(), "HOME".to_string(), "USER".to_string()],
            custom_restrictions: HashMap::new(),
        }
    }
}

/// Network access policies for sandbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NetworkAccess {
    /// No network access allowed
    Disabled,
    /// Only localhost access allowed
    LocalhostOnly,
    /// Full network access
    Full,
    /// Custom network restrictions
    Custom { allowed_hosts: Vec<String> },
}

/// Git and VCS configuration.
///
/// Settings for version control integration and Git operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitConfig {
    /// Whether to enforce conventional commits
    pub conventional_commits: bool,

    /// Maximum number of files to stage at once
    pub max_staged_files: u32,

    /// Whether to automatically commit after successful patches
    pub auto_commit: bool,

    /// Default commit message template
    pub commit_message_template: Option<String>,

    /// Whether to create signed commits
    pub sign_commits: bool,

    /// GPG key ID for signing (if sign_commits is true)
    pub gpg_key_id: Option<String>,

    /// Whether to use git notes for metadata
    pub use_git_notes: bool,

    /// Branch patterns to protect from direct commits
    pub protected_branches: Vec<String>,
}

impl Default for GitConfig {
    fn default() -> Self {
        Self {
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
}

/// Test execution configuration.
///
/// Settings for running tests and validating changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestConfig {
    /// Whether tests are enabled by default
    pub enabled: bool,

    /// Default timeout for test execution
    pub default_timeout: Duration,

    /// Whether to run tests in parallel
    pub parallel_execution: bool,

    /// Maximum number of parallel test jobs
    pub max_parallel_jobs: Option<u32>,

    /// Test frameworks to auto-detect
    pub auto_detect_frameworks: Vec<String>,

    /// Custom test commands for different file types
    pub custom_test_commands: HashMap<String, String>,

    /// Whether to fail fast on first test failure
    pub fail_fast: bool,

    /// Environment variables for test execution
    pub test_env_vars: HashMap<String, String>,

    /// Paths to exclude from test discovery
    pub excluded_paths: Vec<PathBuf>,
}

impl Default for TestConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            default_timeout: Duration::from_secs(300), // 5 minutes
            parallel_execution: true,
            max_parallel_jobs: None, // Use system default
            auto_detect_frameworks: vec![
                "cargo".to_string(),
                "npm".to_string(),
                "pytest".to_string(),
                "jest".to_string(),
            ],
            custom_test_commands: HashMap::new(),
            fail_fast: false,
            test_env_vars: HashMap::new(),
            excluded_paths: vec![
                PathBuf::from("target"),
                PathBuf::from("node_modules"),
                PathBuf::from(".git"),
            ],
        }
    }
}

/// Journal and audit configuration.
///
/// Settings for operation logging and audit trails.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalSettings {
    /// Whether journaling is enabled
    pub enabled: bool,

    /// Path to the journal file
    pub journal_path: PathBuf,

    /// Whether to sign journal entries
    pub sign_entries: bool,

    /// HMAC key for signing (if sign_entries is true)
    pub signing_key: Option<String>,

    /// Maximum size of journal file before rotation
    pub max_file_size_mb: u64,

    /// Number of rotated journal files to keep
    pub max_rotated_files: u32,

    /// Whether to include sensitive data in journal
    pub include_sensitive_data: bool,

    /// Log levels to include in journal
    pub log_levels: Vec<LogLevel>,

    /// Custom fields to include in journal entries
    pub custom_fields: HashMap<String, String>,
}

impl Default for JournalSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            journal_path: PathBuf::from(".devit/journal.jsonl"),
            sign_entries: true,
            signing_key: None, // Generated automatically if None
            max_file_size_mb: 100,
            max_rotated_files: 5,
            include_sensitive_data: false,
            log_levels: vec![LogLevel::Info, LogLevel::Warning, LogLevel::Error],
            custom_fields: HashMap::new(),
        }
    }
}

/// Log levels for journal entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LogLevel {
    Debug,
    Info,
    Warning,
    Error,
    Critical,
}

/// Runtime behavior configuration.
///
/// Settings that control general runtime behavior of the core engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    /// Whether to use colored output
    pub colored_output: bool,

    /// Verbosity level (0-3)
    pub verbosity_level: u8,

    /// Whether to show progress indicators
    pub show_progress: bool,

    /// Default working directory
    pub working_directory: Option<PathBuf>,

    /// Whether to validate configuration on startup
    pub validate_config_on_startup: bool,

    /// Performance optimization settings
    pub performance: PerformanceConfig,

    /// Feature flags for experimental features
    pub feature_flags: HashMap<String, bool>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            colored_output: true,
            verbosity_level: 1,
            show_progress: true,
            working_directory: None,
            validate_config_on_startup: true,
            performance: PerformanceConfig::default(),
            feature_flags: HashMap::new(),
        }
    }
}

/// Workspace sandbox configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    /// Sandbox root directory (jail boundary)
    #[serde(default)]
    pub sandbox_root: Option<PathBuf>,
    /// Default project (relative path) to switch to on startup
    #[serde(default)]
    pub default_project: Option<String>,
    /// Allow-list of project globs relative to sandbox root
    #[serde(default)]
    pub allowed_projects: Vec<String>,
    /// Optional quota on total size in megabytes
    #[serde(default)]
    pub max_size_mb: Option<u64>,
    /// Optional quota on number of files
    #[serde(default)]
    pub max_files: Option<u64>,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            sandbox_root: None,
            default_project: None,
            allowed_projects: Vec::new(),
            max_size_mb: None,
            max_files: None,
        }
    }
}

impl WorkspaceConfig {
    /// Resolve the sandbox root, expanding `~` and relative segments.
    pub fn resolve_root(&self) -> AnyhowResult<PathBuf> {
        if let Some(root) = &self.sandbox_root {
            return Ok(expand_tilde(root));
        }
        Ok(std::env::current_dir().context("Unable to determine current directory")?)
    }
}

fn expand_tilde(path: &PathBuf) -> PathBuf {
    if let Some(raw) = path.to_str() {
        if raw == "~" {
            return std::env::var("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|_| path.clone());
        } else if raw.starts_with("~/") {
            if let Ok(home) = std::env::var("HOME") {
                let mut expanded = PathBuf::from(home);
                expanded.push(&raw[2..]);
                return expanded;
            }
        }
    }
    path.clone()
}

/// Performance optimization configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceConfig {
    /// Number of worker threads for parallel operations
    pub worker_threads: Option<usize>,

    /// Buffer size for I/O operations
    pub io_buffer_size: usize,

    /// Whether to enable caching
    pub enable_caching: bool,

    /// Cache size in megabytes
    pub cache_size_mb: u64,

    /// Cache TTL in seconds
    pub cache_ttl_secs: u64,
}

impl Default for PerformanceConfig {
    fn default() -> Self {
        Self {
            worker_threads: None,      // Use system default
            io_buffer_size: 64 * 1024, // 64KB
            enable_caching: true,
            cache_size_mb: 100,
            cache_ttl_secs: 300, // 5 minutes
        }
    }
}

impl CoreConfig {
    /// Returns the policy configuration.
    pub fn policy_config(&self) -> &PolicyConfig {
        &self.policy
    }

    /// Loads configuration from a TOML file.
    ///
    /// # Arguments
    /// * `path` - Path to the configuration file
    ///
    /// # Returns
    /// * `Ok(config)` - Loaded configuration
    /// * `Err(error)` - If loading fails
    ///
    /// # Errors
    /// Returns an error if the file cannot be read or parsed.
    pub fn from_file<P: AsRef<std::path::Path>>(
        path: P,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        use std::fs;

        let contents = fs::read_to_string(path.as_ref())?;
        let config: CoreConfig = toml::from_str(&contents)?;
        config.validate().map_err(|errors| errors.join(", "))?;
        Ok(config)
    }

    /// Saves configuration to a TOML file.
    ///
    /// # Arguments
    /// * `path` - Path where to save the configuration
    ///
    /// # Returns
    /// * `Ok(())` - If saving succeeds
    /// * `Err(error)` - If saving fails
    ///
    /// # Errors
    /// Returns an error if the file cannot be written.
    pub fn project_or_default(&self) -> &ProjectCfg {
        static DEFAULT: Lazy<ProjectCfg> = Lazy::new(ProjectCfg::default);
        self.project.as_ref().unwrap_or(&DEFAULT)
    }

    pub fn approvals_or_default(&self) -> &ApprovalsCfg {
        static DEFAULT: Lazy<ApprovalsCfg> = Lazy::new(ApprovalsCfg::default);
        self.approvals.as_ref().unwrap_or(&DEFAULT)
    }

    pub fn tests_or_default(&self) -> &TestsCfg {
        static DEFAULT: Lazy<TestsCfg> = Lazy::new(TestsCfg::default);
        self.tests.as_ref().unwrap_or(&DEFAULT)
    }

    pub fn monitoring_or_default(&self) -> &MonitoringCfg {
        static DEFAULT: Lazy<MonitoringCfg> = Lazy::new(MonitoringCfg::default);
        self.monitoring.as_ref().unwrap_or(&DEFAULT)
    }

    pub fn save_to_file<P: AsRef<std::path::Path>>(
        &self,
        path: P,
    ) -> Result<(), Box<dyn std::error::Error>> {
        use std::fs;
        use std::io::Write;

        // Validate before saving
        self.validate().map_err(|errors| errors.join(", "))?;

        // Ensure parent directory exists
        if let Some(parent) = path.as_ref().parent() {
            fs::create_dir_all(parent)?;
        }

        // Serialize to TOML
        let toml_string = toml::to_string_pretty(self)?;

        // Write to file
        let mut file = fs::File::create(path.as_ref())?;
        file.write_all(toml_string.as_bytes())?;

        Ok(())
    }

    /// Validates the configuration for consistency and correctness.
    ///
    /// # Returns
    /// * `Ok(())` - If configuration is valid
    /// * `Err(errors)` - List of validation errors
    ///
    /// # Errors
    /// Returns validation errors if the configuration is invalid.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();

        // Validate project settings
        let project = self.project_or_default();
        if project.name.trim().is_empty() {
            errors.push("Project name cannot be empty".to_string());
        }
        if project.default_language.trim().is_empty() {
            errors.push("Default language cannot be empty".to_string());
        }

        // Validate approval settings
        let approvals = self.approvals_or_default();
        if approvals.timeout_seconds == 0 {
            errors.push("Approval timeout must be greater than 0".to_string());
        }
        if approvals.max_auto_approve_size > 1_000_000 {
            errors.push("Max auto-approve size seems too large (>1MB)".to_string());
        }

        // Validate test settings
        let tests = self.tests_or_default();
        if tests.default_timeout == 0 {
            errors.push("Test timeout must be greater than 0".to_string());
        }
        if tests.parallel_jobs > 256 {
            errors.push("Parallel jobs seems too high (>256)".to_string());
        }

        // Validate sandbox settings
        if self.sandbox.enabled {
            if let Some(memory_limit) = self.sandbox.memory_limit_mb {
                if memory_limit < 64 {
                    errors.push("Sandbox memory limit too low (<64MB)".to_string());
                }
            }
            if let Some(cpu_limit) = self.sandbox.cpu_limit_secs {
                if cpu_limit == 0 {
                    errors.push("Sandbox CPU limit must be greater than 0".to_string());
                }
            }
        }

        // Validate journal settings
        if self.journal.enabled && self.journal.max_file_size_mb == 0 {
            errors.push("Journal max file size must be greater than 0".to_string());
        }

        // Validate monitoring settings
        let monitoring = self.monitoring_or_default();
        if monitoring.metrics_enabled && monitoring.metrics_port < 1024 {
            errors.push("Metrics port should be >= 1024 (non-privileged)".to_string());
        }

        if let Some(root) = &self.workspace.sandbox_root {
            let expanded = expand_tilde(root);
            if !expanded.exists() {
                errors.push(format!(
                    "Workspace sandbox_root does not exist: {}",
                    expanded.display()
                ));
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Merges this configuration with another, preferring values from other.
    ///
    /// # Arguments
    /// * `other` - Configuration to merge with
    ///
    /// # Returns
    /// New configuration with merged values
    pub fn merge(&self, other: &Self) -> Self {
        let mut merged = self.clone();

        merged.backend = other.backend.clone();
        merged.policy = other.policy.clone();
        merged.sandbox = other.sandbox.clone();
        merged.git = other.git.clone();
        merged.testing = other.testing.clone();
        merged.journal = other.journal.clone();
        merged.runtime = other.runtime.clone();
        merged.orchestration = other.orchestration.clone();
        merged.project = other.project.clone();
        merged.approvals = other.approvals.clone();
        merged.tests = other.tests.clone();
        merged.monitoring = other.monitoring.clone();

        merged
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use tempfile::tempdir;

    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    fn clear_policy_env() {
        for key in [
            "DEVIT_DEFAULT_APPROVAL",
            "DEVIT_MAX_FILES_MODERATE",
            "DEVIT_MAX_LINES_MODERATE",
            "DEVIT_PROTECTED_PATHS",
            "DEVIT_SMALL_BINARY_MAX_BYTES",
            "DEVIT_SMALL_BINARY_EXT_WHITELIST",
            "DEVIT_SANDBOX_PROFILE_DEFAULT",
            "DEVIT_CONFIG",
        ] {
            env::remove_var(key);
        }
    }

    #[test]
    fn loads_policy_config_from_toml() {
        let _guard = ENV_MUTEX.lock().unwrap();
        clear_policy_env();

        let dir = tempdir().expect("tempdir");
        let cfg_path = dir.path().join("devit.toml");

        let toml = r#"
            [policy]
            default_approval = "trusted"
            max_files_moderate = 7
            max_lines_moderate = 321
            protected_paths = ["src", "docs/readme.md"]
            small_binary_max_bytes = 2048
            small_binary_ext_whitelist = ["png", "gif"]
            sandbox_profile_default = "permissive"
        "#;

        fs::write(&cfg_path, toml).expect("write config");

        let policy = PolicyConfig::load_from_path(&cfg_path).expect("load policy");

        assert_eq!(*policy.default_approval(), ApprovalLevel::Trusted);
        assert_eq!(policy.max_files_moderate, 7);
        assert_eq!(policy.max_lines_moderate, 321);
        assert_eq!(
            policy.protected_paths(),
            &[PathBuf::from("src"), PathBuf::from("docs/readme.md")]
        );
        assert_eq!(policy.small_binary_max_bytes, 2048);
        assert_eq!(
            policy.small_binary_ext_whitelist,
            vec!["gif".to_string(), "png".to_string()]
        );
        assert_eq!(
            *policy.sandbox_profile_default(),
            SandboxProfile::Permissive
        );

        clear_policy_env();
    }

    #[test]
    fn env_overrides_take_precedence() {
        let _guard = ENV_MUTEX.lock().unwrap();
        clear_policy_env();

        let dir = tempdir().expect("tempdir");
        let cfg_path = dir.path().join("devit.toml");

        let toml = r#"
            [policy]
            default_approval = "ask"
            max_files_moderate = 5
            max_lines_moderate = 100
            protected_paths = ["src"]
            small_binary_max_bytes = 512
            small_binary_ext_whitelist = ["png"]
            sandbox_profile_default = "permissive"
        "#;

        fs::write(&cfg_path, toml).expect("write config");

        env::set_var("DEVIT_DEFAULT_APPROVAL", "moderate");
        env::set_var("DEVIT_MAX_FILES_MODERATE", "42");
        env::set_var("DEVIT_MAX_LINES_MODERATE", "4242");
        env::set_var("DEVIT_PROTECTED_PATHS", "policy,config/devit");
        env::set_var("DEVIT_SMALL_BINARY_MAX_BYTES", "8192");
        env::set_var("DEVIT_SMALL_BINARY_EXT_WHITELIST", "gif, svg ,PNG");
        env::set_var("DEVIT_SANDBOX_PROFILE_DEFAULT", "strict");

        let policy = PolicyConfig::load_from_path(&cfg_path).expect("load policy");

        assert_eq!(*policy.default_approval(), ApprovalLevel::Moderate);
        assert_eq!(policy.max_files_moderate, 42);
        assert_eq!(policy.max_lines_moderate, 4242);
        assert_eq!(
            policy.protected_paths(),
            &[PathBuf::from("policy"), PathBuf::from("config/devit")]
        );
        assert_eq!(policy.small_binary_max_bytes, 8192);
        assert_eq!(
            policy.small_binary_ext_whitelist,
            vec!["gif".to_string(), "png".to_string(), "svg".to_string()]
        );
        assert_eq!(*policy.sandbox_profile_default(), SandboxProfile::Strict);

        clear_policy_env();
    }

    #[test]
    fn fallback_to_defaults_when_file_missing() {
        let _guard = ENV_MUTEX.lock().unwrap();
        clear_policy_env();

        let dir = tempdir().expect("tempdir");
        let missing_path = dir.path().join("missing.toml");

        let policy = PolicyConfig::from_sources(Some(missing_path));

        let defaults = PolicyConfig::builtin_defaults();

        assert_eq!(*policy.default_approval(), *defaults.default_approval());
        assert_eq!(policy.max_files_moderate, defaults.max_files_moderate);
        assert_eq!(policy.max_lines_moderate, defaults.max_lines_moderate);
        assert_eq!(policy.protected_paths(), defaults.protected_paths());
        assert_eq!(
            policy.small_binary_max_bytes,
            defaults.small_binary_max_bytes
        );
        assert_eq!(
            policy.small_binary_ext_whitelist,
            defaults.small_binary_ext_whitelist
        );
        assert_eq!(
            *policy.sandbox_profile_default(),
            *defaults.sandbox_profile_default()
        );

        clear_policy_env();
    }

    #[test]
    fn test_policy_config_getters() {
        let _guard = ENV_MUTEX.lock().unwrap();
        clear_policy_env();

        let policy = PolicyConfig::builtin_defaults();

        // Test all getters
        assert_eq!(*policy.default_approval(), ApprovalLevel::Ask);
        assert_eq!(policy.max_files_moderate(), DEFAULT_MAX_FILES_MODERATE);
        assert_eq!(policy.max_lines_moderate(), DEFAULT_MAX_LINES_MODERATE);
        assert_eq!(
            policy.small_binary_max_bytes(),
            DEFAULT_SMALL_BINARY_MAX_BYTES
        );
        assert_eq!(
            policy.protected_paths().len(),
            DEFAULT_PROTECTED_PATHS.len()
        );
        assert_eq!(
            policy.small_binary_ext_whitelist().len(),
            DEFAULT_SMALL_BINARY_EXTS.len()
        );
        assert_eq!(*policy.sandbox_profile_default(), SandboxProfile::Strict);

        clear_policy_env();
    }

    #[test]
    fn test_sandbox_profile_parsing() {
        assert_eq!(
            PolicyConfig::parse_sandbox_profile("permissive"),
            Some(SandboxProfile::Permissive)
        );
        assert_eq!(
            PolicyConfig::parse_sandbox_profile("strict"),
            Some(SandboxProfile::Strict)
        );
        assert_eq!(
            PolicyConfig::parse_sandbox_profile("STRICT"),
            Some(SandboxProfile::Strict)
        );
        assert_eq!(
            PolicyConfig::parse_sandbox_profile(" permissive "),
            Some(SandboxProfile::Permissive)
        );
        assert_eq!(PolicyConfig::parse_sandbox_profile("disabled"), None);
        assert_eq!(PolicyConfig::parse_sandbox_profile("invalid"), None);
        assert_eq!(PolicyConfig::parse_sandbox_profile(""), None);
    }
}
