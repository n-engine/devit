// # -----------------------------
// # crates/common/src/lib.rs
// # -----------------------------
pub mod fs;
pub mod orchestration;
pub mod process_registry;
pub mod process_utils;
pub mod limits;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub backend: BackendCfg,
    pub policy: PolicyCfg,
    pub sandbox: SandboxCfg,
    pub git: GitCfg,
    #[serde(default)]
    pub provenance: ProvenanceCfg,
    #[serde(default)]
    pub precommit: Option<PrecommitCfg>,
    #[serde(default)]
    pub commit: Option<CommitCfg>,
    #[serde(default)]
    pub llm: Option<LlmCfg>,
    #[serde(default)]
    pub workspace: Option<WorkspaceCfg>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendCfg {
    pub kind: String,
    pub base_url: String,
    pub model: String,
    pub api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmCfg {
    pub backend: String,
    pub endpoint: String,
    pub model: String,
    #[serde(default)]
    pub timeout_s: Option<u64>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyCfg {
    pub approval: String,
    pub sandbox: String,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub approvals: Option<HashMap<String, String>>, // per-tool overrides: git|shell|test
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxCfg {
    pub cpu_limit: u32,
    pub mem_limit_mb: u32,
    pub net: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitCfg {
    pub conventional: bool,
    pub max_staged_files: u32,
    #[serde(default)]
    pub use_notes: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProvenanceCfg {
    #[serde(default)]
    pub footer: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkspaceCfg {
    #[serde(default)]
    pub sandbox_root: Option<PathBuf>,
    #[serde(default)]
    pub default_project: Option<String>,
    #[serde(default)]
    pub allowed_projects: Vec<String>,
    #[serde(default)]
    pub max_size_mb: Option<u64>,
    #[serde(default)]
    pub max_files: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QualityCfg {
    #[serde(default)]
    pub max_test_failures: u32,
    #[serde(default)]
    pub max_lint_errors: u32,
    #[serde(default = "default_true")]
    pub allow_lint_warnings: bool,
    #[serde(default)]
    pub fail_on_missing_reports: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommitCfg {
    #[serde(default = "default_max_subject")]
    pub max_subject: usize,
    #[serde(default)]
    pub scopes_alias: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub default_type: Option<String>,
    #[serde(default)]
    pub template_body: Option<String>,
}

fn default_max_subject() -> usize {
    72
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrecommitCfg {
    #[serde(default = "default_true")]
    pub rust: bool,
    #[serde(default = "default_true")]
    pub javascript: bool,
    #[serde(default = "default_true")]
    pub python: bool,
    #[serde(default)]
    pub additional: Vec<String>,
    #[serde(default = "default_fail_on")]
    pub fail_on: Vec<String>,
    #[serde(default)]
    pub allow_bypass_profiles: Vec<String>,
}

fn default_true() -> bool {
    true
}
fn default_fail_on() -> Vec<String> {
    vec!["rust".into(), "javascript".into(), "python".into()]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Event {
    ToolCall {
        name: String,
        args: serde_json::Value,
    },
    CommandOut {
        line: String,
    },
    Diff {
        unified: String,
    },
    AskApproval {
        summary: String,
    },
    Error {
        message: String,
    },
    Info {
        message: String,
    },
    Attest {
        hash: String,
    },
}

// DevIt Core Engine Types
use chrono::{DateTime, Utc};
use std::path::PathBuf;
use uuid::Uuid;

/// Approval levels for operation authorization.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub enum ApprovalLevel {
    /// Untrusted: Always requires explicit user approval
    Untrusted,
    /// Ask: Prompts for approval unless auto-approved
    #[default]
    Ask,
    /// Moderate: Requires approval for sensitive operations
    Moderate,
    /// Trusted: Most operations proceed without approval
    Trusted,
    /// Privileged: Restricted access to specific paths
    Privileged { allowed_paths: Vec<PathBuf> },
}

impl ApprovalLevel {
    /// Checks if this approval level can access the given path.
    pub fn can_access_path(&self, path: &PathBuf) -> bool {
        match self {
            ApprovalLevel::Privileged { allowed_paths } => allowed_paths
                .iter()
                .any(|allowed_path| path.starts_with(allowed_path) || path == allowed_path),
            _ => true, // Other levels can access any path (subject to other policies)
        }
    }

    /// Returns whether this approval level requires explicit user approval.
    pub fn requires_approval(&self) -> bool {
        matches!(self, ApprovalLevel::Untrusted | ApprovalLevel::Ask)
    }

    /// Returns the security rank of this approval level.
    pub fn security_rank(&self) -> u8 {
        match self {
            ApprovalLevel::Untrusted => 0,
            ApprovalLevel::Ask => 1,
            ApprovalLevel::Moderate => 2,
            ApprovalLevel::Trusted => 3,
            ApprovalLevel::Privileged { .. } => 4,
        }
    }

    /// Checks if this approval level satisfies the required level.
    pub fn satisfies(&self, required: &ApprovalLevel) -> bool {
        match (self, required) {
            // Privileged levels require exact path matching
            (
                ApprovalLevel::Privileged {
                    allowed_paths: our_paths,
                },
                ApprovalLevel::Privileged {
                    allowed_paths: req_paths,
                },
            ) => req_paths.iter().all(|req_path| {
                our_paths
                    .iter()
                    .any(|our_path| req_path.starts_with(our_path) || req_path == our_path)
            }),

            // Privileged can satisfy any non-privileged requirement
            (ApprovalLevel::Privileged { .. }, _) => true,

            // For other levels, compare security ranks
            _ => self.security_rank() >= required.security_rank(),
        }
    }
}

/// Sandbox profile configuration for isolation levels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SandboxProfile {
    /// Strict: Maximum isolation and restrictions
    Strict,
    /// Permissive: Moderate restrictions with more access
    Permissive,
}

/// Unique identifier for snapshots.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SnapshotId(pub String);

impl std::fmt::Display for SnapshotId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Type of file change in a patch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileChangeKind {
    /// Fichier ajouté
    Add,
    /// Fichier modifié
    Mod,
    /// Fichier supprimé
    Del,
    /// File content modified
    Modify,
    /// New file created
    Create,
    /// Existing file deleted
    Delete,
    /// File moved/renamed
    Rename,
    /// File copied
    Copy,
}

/// Standard JSON response wrapper.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StdResponse<T> {
    /// Whether the operation succeeded
    pub success: bool,
    /// Timestamp of the response
    pub timestamp: DateTime<Utc>,
    /// Optional request identifier for tracing
    pub request_id: Option<Uuid>,
    /// Error information if operation failed
    pub error: Option<StdError>,
    /// Response data if operation succeeded
    pub data: Option<T>,
}

impl<T> StdResponse<T> {
    pub fn success(data: T, request_id: Option<Uuid>) -> Self {
        Self {
            success: true,
            timestamp: Utc::now(),
            request_id,
            error: None,
            data: Some(data),
        }
    }

    pub fn error(error: StdError, request_id: Option<Uuid>) -> Self {
        Self {
            success: false,
            timestamp: Utc::now(),
            request_id,
            error: Some(error),
            data: None,
        }
    }
}

/// Standard error information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StdError {
    /// Error code for programmatic handling
    pub code: String,
    /// Human-readable error message
    pub message: String,
    /// Optional detailed error information
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    /// Hints for error resolution
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// Whether the error is actionable by the user
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actionable: Option<bool>,
}

impl StdError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
            hint: None,
            actionable: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn with_actionable(mut self, actionable: bool) -> Self {
        self.actionable = Some(actionable);
        self
    }
}
