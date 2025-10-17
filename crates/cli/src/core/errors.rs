//! # DevIt Core Engine Error Types
//!
//! This module defines all error types that can be returned by the Core Engine.
//! Each error variant corresponds to a specific failure mode and includes
//! contextual information for debugging and user feedback.
//!
//! ## Error Categories
//!
//! - **Validation Errors**: Invalid input or state
//! - **Policy Violations**: Security or approval violations
//! - **System Errors**: I/O, resource, or internal failures
//! - **Operation Failures**: Test, VCS, or sandbox failures

use std::borrow::Cow;
use std::path::PathBuf;
use thiserror::Error;
use uuid::Uuid;

/// Error categories for comprehensive taxonomy and monitoring.
///
/// Categories provide a higher-level classification than individual error codes,
/// enabling systematic error handling, metrics collection, and automated responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ErrorCategory {
    /// Input validation failures and malformed data
    Validation,
    /// Security policy violations and access control failures
    Security,
    /// System state inconsistencies and requirements
    State,
    /// Version control system issues and conflicts
    Version,
    /// Operation execution failures (tests, builds, etc.)
    Operation,
    /// Resource exhaustion and capacity limits
    Resource,
    /// System-level errors (I/O, internal failures)
    System,
}

/// Result type for all Core Engine operations.
///
/// This is a convenience type alias that uses [`DevItError`] as the error type
/// for all fallible operations in the Core Engine.
pub type DevItResult<T> = Result<T, DevItError>;

/// Comprehensive error enumeration for DevIt Core Engine operations.
///
/// Each variant represents a specific failure mode that can occur during
/// Core Engine operations. Error variants are designed to provide both
/// machine-readable error codes and human-readable context.
#[derive(Debug, Error)]
pub enum DevItError {
    /// E_INVALID_DIFF - Patch format is invalid or corrupted
    ///
    /// This error occurs when the provided patch cannot be parsed or contains
    /// malformed diff syntax that prevents further processing.
    #[error("Invalid patch format: {reason}")]
    InvalidDiff {
        /// Human-readable explanation of why the diff is invalid
        reason: String,
        /// Line number where parsing failed (if applicable)
        line_number: Option<usize>,
    },

    /// E_SNAPSHOT_REQUIRED - Operation requires a valid snapshot
    ///
    /// Indicates that the requested operation cannot proceed without a valid
    /// snapshot being available or specified.
    #[error("Snapshot required for operation '{operation}': {expected}")]
    SnapshotRequired {
        /// Name of the operation that was attempted
        operation: String,
        /// Description of what snapshot is expected
        expected: String,
    },

    /// E_SNAPSHOT_STALE - Snapshot is outdated relative to current state
    ///
    /// This error is returned when the working directory has changed since
    /// the snapshot was created, making the snapshot invalid for operations.
    #[error("Snapshot {snapshot_id} is stale and no longer valid")]
    SnapshotStale {
        /// ID of the stale snapshot
        snapshot_id: String,
        /// Timestamp when the snapshot was created
        created_at: Option<chrono::DateTime<chrono::Utc>>,
        /// Description of what has changed since creation
        staleness_reason: Option<String>,
    },

    /// E_POLICY_BLOCK - Operation blocked by security policy
    ///
    /// Returned when an operation violates security policies or approval
    /// requirements. The operation may be retried with higher approval.
    #[error(
        "Operation blocked by policy rule '{rule}': {current_level} \
             insufficient, requires {required_level}"
    )]
    PolicyBlock {
        /// Policy rule that was violated
        rule: String,
        /// Required approval level for operation
        required_level: String,
        /// Current approval level provided
        current_level: String,
        /// Additional context about the violation
        context: String,
    },

    /// E_PROTECTED_PATH - Attempt to modify protected file or directory
    ///
    /// This error occurs when an operation attempts to modify a path that
    /// is marked as protected in the security configuration.
    #[error("Access denied to protected path: {path}")]
    ProtectedPath {
        /// Path that was attempted to be accessed
        path: PathBuf,
        /// Rule that marks this path as protected
        protection_rule: String,
        /// Operation that was attempted
        attempted_operation: String,
    },

    /// E_PRIV_ESCALATION - Detected privilege escalation attempt
    ///
    /// Returned when the system detects an attempt to escalate privileges
    /// beyond what is allowed by the current security context.
    #[error("Privilege escalation detected: {escalation_type}")]
    PrivilegeEscalation {
        /// Type of privilege escalation detected
        escalation_type: String,
        /// Current privilege level
        current_privileges: String,
        /// Attempted privilege level
        attempted_privileges: String,
        /// Additional security context
        security_context: String,
    },

    /// E_GIT_DIRTY - Git working directory has uncommitted changes
    ///
    /// This error is returned when an operation requires a clean Git working
    /// directory but uncommitted changes are present.
    #[error("Git working directory is dirty: {dirty_files} uncommitted files")]
    GitDirty {
        /// Number of files with uncommitted changes
        dirty_files: usize,
        /// List of modified files (limited for readability)
        modified_files: Vec<PathBuf>,
        /// Branch name where changes are present
        branch: Option<String>,
    },

    /// E_VCS_CONFLICT - Version control system conflict detected
    ///
    /// Indicates that the operation cannot proceed due to merge conflicts
    /// or other VCS-related issues that require manual resolution.
    #[error("VCS conflict in {location}: {conflict_type}")]
    VcsConflict {
        /// Location where the conflict occurred
        location: String,
        /// Type of conflict (merge, rebase, etc.)
        conflict_type: String,
        /// Files involved in the conflict
        conflicted_files: Vec<PathBuf>,
        /// Suggested resolution steps
        resolution_hint: Option<String>,
    },

    /// E_TEST_FAIL - Test execution failed
    ///
    /// This error is returned when test execution completes but one or more
    /// tests fail, preventing the operation from proceeding.
    #[error(
        "Test execution failed: {failed_count} of {total_count} tests \
             failed"
    )]
    TestFail {
        /// Number of tests that failed
        failed_count: u32,
        /// Total number of tests executed
        total_count: u32,
        /// Framework used for testing
        test_framework: String,
        /// Detailed failure information
        failure_details: Vec<String>,
    },

    /// E_TEST_TIMEOUT - Test execution exceeded time limit
    ///
    /// Returned when test execution is terminated due to exceeding the
    /// configured timeout period.
    #[error("Test execution timed out after {timeout_secs} seconds")]
    TestTimeout {
        /// Timeout period that was exceeded (in seconds)
        timeout_secs: u64,
        /// Framework that was executing tests
        test_framework: String,
        /// Tests that were running when timeout occurred
        running_tests: Vec<String>,
    },

    /// E_SANDBOX_DENIED - Sandbox denied operation
    ///
    /// This error occurs when the sandbox security mechanism blocks an
    /// operation due to policy violations or resource restrictions.
    #[error("Sandbox denied operation: {reason}")]
    SandboxDenied {
        /// Reason why the sandbox denied the operation
        reason: String,
        /// Sandbox profile that was active
        active_profile: String,
        /// Operation that was attempted
        attempted_operation: String,
        /// Sandbox policy that was violated
        violated_policy: Option<String>,
    },

    /// E_RESOURCE_LIMIT - System resource limit exceeded
    ///
    /// Returned when an operation cannot complete due to insufficient system
    /// resources such as memory, disk space, or file descriptors.
    #[error("Resource limit exceeded: {resource_type}")]
    ResourceLimit {
        /// Type of resource that was exhausted
        resource_type: String,
        /// Current usage level
        current_usage: u64,
        /// Maximum allowed usage
        limit: u64,
        /// Unit of measurement (bytes, count, etc.)
        unit: String,
    },

    /// E_IO - I/O operation failed
    ///
    /// This error wraps standard I/O errors that occur during file system
    /// operations, network communications, or other I/O activities.
    #[error("I/O error in {operation}: {source}")]
    Io {
        /// Operation that was being performed
        operation: String,
        /// Path involved in the operation (if applicable)
        path: Option<PathBuf>,
        /// Underlying I/O error
        #[source]
        source: std::io::Error,
    },

    /// E_INVALID_TEST_CONFIG - Test configuration validation failed
    ///
    /// This error occurs when test configuration parameters fail validation,
    /// such as invalid timeout values, memory limits, or inconsistent settings.
    #[error("Invalid test configuration for field '{field}': {reason}")]
    InvalidTestConfig {
        /// Field name that failed validation
        field: String,
        /// Value that was provided
        value: String,
        /// Reason why the value is invalid
        reason: String,
    },

    /// E_INTERNAL - Internal system error
    ///
    /// Represents an unexpected internal error that should not occur during
    /// normal operation. These typically indicate bugs or system corruption.
    #[error("Internal error in {component}: {message}")]
    Internal {
        /// Component where the error occurred
        component: String,
        /// Descriptive error message
        message: String,
        /// Optional cause chain for debugging
        cause: Option<String>,
        /// Error correlation ID for support
        correlation_id: String,
    },

    /// E_INVALID_FORMAT - Unsupported output format specified
    ///
    /// This error occurs when a client requests an output format that is not
    /// supported by the current tool implementation.
    #[error("Invalid format '{format}'. Supported formats: {}", supported.join(", "))]
    InvalidFormat {
        /// The format that was requested
        format: String,
        /// List of supported formats
        supported: Vec<String>,
    },
}

impl DevItError {
    /// Helper pour créer une erreur I/O avec un chemin optionnel et un nom d'opération.
    pub fn io<P, S>(path: P, operation: S, source: std::io::Error) -> Self
    where
        P: Into<Option<PathBuf>>,
        S: Into<String>,
    {
        Self::Io {
            operation: operation.into(),
            path: path.into(),
            source,
        }
    }

    /// Constructeur compatibilité pour les erreurs internes simples.

    pub fn internal<M: Into<Cow<'static, str>>>(message: M) -> Self {
        DevItError::Internal {
            component: "cli".to_string(),
            message: message.into().into_owned(),
            cause: None,
            correlation_id: Uuid::new_v4().to_string(),
        }
    }

    /// Returns the error code for this error variant.
    ///
    /// Error codes are standardized identifiers that can be used by clients
    /// to programmatically handle specific error conditions.
    ///
    /// # Returns
    ///
    /// A string slice containing the error code (e.g., "E_INVALID_DIFF").
    pub fn error_code(&self) -> &'static str {
        match self {
            DevItError::InvalidDiff { .. } => "E_INVALID_DIFF",
            DevItError::SnapshotRequired { .. } => "E_SNAPSHOT_REQUIRED",
            DevItError::SnapshotStale { .. } => "E_SNAPSHOT_STALE",
            DevItError::PolicyBlock { .. } => "E_POLICY_BLOCK",
            DevItError::ProtectedPath { .. } => "E_PROTECTED_PATH",
            DevItError::PrivilegeEscalation { .. } => "E_PRIV_ESCALATION",
            DevItError::GitDirty { .. } => "E_GIT_DIRTY",
            DevItError::VcsConflict { .. } => "E_VCS_CONFLICT",
            DevItError::TestFail { .. } => "E_TEST_FAIL",
            DevItError::TestTimeout { .. } => "E_TEST_TIMEOUT",
            DevItError::SandboxDenied { .. } => "E_SANDBOX_DENIED",
            DevItError::ResourceLimit { .. } => "E_RESOURCE_LIMIT",
            Self::Io { .. } => "E_IO",
            DevItError::InvalidTestConfig { .. } => "E_INVALID_TEST_CONFIG",
            DevItError::Internal { .. } => "E_INTERNAL",
            DevItError::InvalidFormat { .. } => "E_INVALID_FORMAT",
        }
    }

    /// Returns the error category for taxonomic classification.
    ///
    /// Categories help group related errors for systematic handling and
    /// provide a higher-level classification than individual error codes.
    ///
    /// # Returns
    ///
    /// The category that this error belongs to.
    pub fn category(&self) -> ErrorCategory {
        match self {
            DevItError::InvalidDiff { .. } => ErrorCategory::Validation,
            DevItError::SnapshotRequired { .. } => ErrorCategory::State,
            DevItError::SnapshotStale { .. } => ErrorCategory::State,
            DevItError::PolicyBlock { .. } => ErrorCategory::Security,
            DevItError::ProtectedPath { .. } => ErrorCategory::Security,
            DevItError::PrivilegeEscalation { .. } => ErrorCategory::Security,
            DevItError::GitDirty { .. } => ErrorCategory::Version,
            DevItError::VcsConflict { .. } => ErrorCategory::Version,
            DevItError::TestFail { .. } => ErrorCategory::Operation,
            DevItError::TestTimeout { .. } => ErrorCategory::Operation,
            DevItError::SandboxDenied { .. } => ErrorCategory::Resource,
            DevItError::ResourceLimit { .. } => ErrorCategory::Resource,
            Self::Io { .. } => ErrorCategory::System,
            DevItError::InvalidTestConfig { .. } => ErrorCategory::Validation,
            DevItError::Internal { .. } => ErrorCategory::System,
            DevItError::InvalidFormat { .. } => ErrorCategory::Validation,
        }
    }

    /// Returns actionable recovery hints for resolving this error.
    ///
    /// Recovery hints provide specific, actionable steps that users can take
    /// to resolve the error condition. Each hint is a complete sentence that
    /// describes a concrete action.
    ///
    /// # Returns
    ///
    /// A vector of recovery hint strings, ordered by likelihood of success.
    pub fn recovery_hints(&self) -> Vec<String> {
        match self {
            DevItError::InvalidDiff { line_number, .. } => {
                let mut hints = vec![
                    "Verify the patch format follows unified diff syntax (diff -u)".to_string(),
                    "Check if the patch was corrupted during transfer or storage".to_string(),
                    "Regenerate the patch from the source repository".to_string(),
                ];
                if line_number.is_some() {
                    hints.insert(
                        0,
                        format!(
                            "Check line {} in the patch for syntax errors",
                            line_number.unwrap()
                        ),
                    );
                }
                hints
            }
            DevItError::SnapshotRequired { operation, .. } => vec![
                format!(
                    "Create a snapshot before running the '{}' operation",
                    operation
                ),
                "Use 'devit snapshot' to capture the current project state".to_string(),
                "Verify that snapshot creation completed successfully".to_string(),
            ],
            DevItError::SnapshotStale { .. } => vec![
                "Create a new snapshot to capture current file states".to_string(),
                "Use 'devit snapshot' to refresh the project snapshot".to_string(),
                "Verify no external processes have modified project files".to_string(),
                "Check if Git working directory has uncommitted changes".to_string(),
            ],
            DevItError::PolicyBlock {
                rule,
                required_level,
                current_level,
                ..
            } => {
                let mut hints = vec![
                    format!(
                        "Increase approval level from '{}' to '{}'",
                        current_level, required_level
                    ),
                    "Contact an administrator if higher privileges are needed".to_string(),
                ];

                // Add rule-specific hints
                match rule.as_str() {
                    "path_security_repo_boundary" => {
                        hints.insert(
                            0,
                            "Ensure all file paths stay within the project directory".to_string(),
                        );
                        hints.push(
                            "Check for '../' patterns that might escape the repository".to_string(),
                        );
                    }
                    "symlink_security_repo_boundary" => {
                        hints.insert(
                            0,
                            "Verify symlink targets point to files within the project".to_string(),
                        );
                        hints.push(
                            "Remove or modify symlinks that point outside the repository"
                                .to_string(),
                        );
                    }
                    "policy_protected_path" => {
                        hints.insert(
                            0,
                            "Avoid modifying files flagged as protected by project policy"
                                .to_string(),
                        );
                        hints
                            .push("Check policy configuration for permitted locations".to_string());
                    }
                    "policy_exec_permission" => {
                        hints.insert(
                            0,
                            "Review executable permission changes with a project maintainer"
                                .to_string(),
                        );
                        hints.push(
                            "Consider handling permission updates outside the patch workflow"
                                .to_string(),
                        );
                    }
                    "policy_binary_restriction" => {
                        hints.insert(
                            0,
                            "Validate that binary files comply with size and extension policies"
                                .to_string(),
                        );
                        hints.push(
                            "Use approved formats or request an exemption before retrying"
                                .to_string(),
                        );
                    }
                    "policy_gitmodules" => {
                        hints.insert(
                            0,
                            "Coordinate submodule updates with repository maintainers".to_string(),
                        );
                        hints.push(
                            "Update submodules manually after receiving the appropriate approval"
                                .to_string(),
                        );
                    }
                    "policy_symlink_restriction" => {
                        hints.insert(
                            0,
                            "Confirm that symlink targets stay within allowed directories"
                                .to_string(),
                        );
                        hints.push(
                            "Replace the symlink with a regular file if cross-boundary access is required"
                                .to_string(),
                        );
                    }
                    "policy_evaluation" => {
                        hints.insert(
                            0,
                            "Review the proposed changes for potential security issues".to_string(),
                        );
                        hints.push(
                            "Consider breaking large changes into smaller, safer patches"
                                .to_string(),
                        );
                    }
                    _ => {}
                }
                hints
            }
            DevItError::ProtectedPath {
                path,
                protection_rule,
                ..
            } => vec![
                format!("Avoid modifying protected path: {:?}", path),
                "Use a different approach that doesn't affect protected files".to_string(),
                format!(
                    "Request permission to modify files covered by rule '{}'",
                    protection_rule
                ),
                "Check if the operation can be performed in a different location".to_string(),
            ],
            DevItError::PrivilegeEscalation {
                current_privileges,
                attempted_privileges,
                ..
            } => vec![
                format!(
                    "Operate within current privilege level: {}",
                    current_privileges
                ),
                format!(
                    "Request {} privileges through proper authorization",
                    attempted_privileges
                ),
                "Contact a system administrator for privilege escalation".to_string(),
                "Review the operation to eliminate privilege requirements".to_string(),
            ],
            DevItError::GitDirty {
                dirty_files,
                modified_files,
                ..
            } => {
                let mut hints = vec![
                    "Commit or stash current changes before proceeding".to_string(),
                    "Use 'git add -A && git commit -m \"WIP\"' to commit changes".to_string(),
                    "Use 'git stash' to temporarily save uncommitted changes".to_string(),
                ];
                if *dirty_files > 0 && !modified_files.is_empty() {
                    hints.insert(
                        0,
                        format!(
                            "Review {} modified files: {:?}",
                            dirty_files,
                            modified_files.iter().take(3).collect::<Vec<_>>()
                        ),
                    );
                }
                hints
            }
            DevItError::VcsConflict {
                conflict_type,
                conflicted_files,
                resolution_hint,
                ..
            } => {
                let mut hints = vec![
                    format!("Resolve {} conflicts before proceeding", conflict_type),
                    "Use 'git status' to see conflicted files".to_string(),
                    "Edit conflict markers in affected files".to_string(),
                    "Run 'git add' after resolving conflicts".to_string(),
                ];
                if !conflicted_files.is_empty() {
                    hints.insert(
                        1,
                        format!(
                            "Focus on files: {:?}",
                            conflicted_files.iter().take(3).collect::<Vec<_>>()
                        ),
                    );
                }
                if let Some(hint) = resolution_hint {
                    hints.insert(0, hint.clone());
                }
                hints
            }
            DevItError::TestFail {
                failed_count,
                total_count,
                test_framework,
                failure_details,
            } => {
                let mut hints = vec![
                    format!("Fix {} failing tests out of {}", failed_count, total_count),
                    format!(
                        "Run '{} test' to see detailed failure output",
                        test_framework
                    ),
                    "Review test failure output for specific error messages".to_string(),
                    "Fix code issues causing test failures".to_string(),
                ];
                if !failure_details.is_empty() {
                    hints.insert(
                        1,
                        format!("Address these issues: {}", failure_details.join(", ")),
                    );
                }
                hints
            }
            DevItError::TestTimeout {
                timeout_secs,
                test_framework,
                running_tests,
            } => {
                let mut hints = vec![
                    format!("Increase timeout beyond {} seconds", timeout_secs),
                    format!("Use --timeout option with {} test", test_framework),
                    "Optimize slow tests to run more efficiently".to_string(),
                    "Run tests in smaller subsets to isolate slow tests".to_string(),
                ];
                if !running_tests.is_empty() {
                    hints.insert(
                        2,
                        format!(
                            "Investigate slow tests: {:?}",
                            running_tests.iter().take(3).collect::<Vec<_>>()
                        ),
                    );
                }
                hints
            }
            DevItError::SandboxDenied {
                reason,
                active_profile,
                attempted_operation,
                ..
            } => vec![
                format!(
                    "Modify the operation '{}' to comply with sandbox restrictions",
                    attempted_operation
                ),
                format!(
                    "Switch from '{}' profile to a more permissive sandbox",
                    active_profile
                ),
                "Review sandbox policies and adjust operation accordingly".to_string(),
                format!("Sandbox denial reason: {}", reason),
            ],
            DevItError::ResourceLimit {
                resource_type,
                current_usage,
                limit,
                unit,
            } => vec![
                format!(
                    "Reduce {} usage from {} {} to below {} {}",
                    resource_type, current_usage, unit, limit, unit
                ),
                "Free up system resources before retrying".to_string(),
                "Increase resource limits if administratively possible".to_string(),
                "Optimize the operation to use fewer resources".to_string(),
            ],
            Self::Io {
                operation,
                path,
                source,
            } => {
                let mut hints = vec![
                    "Check file and directory permissions".to_string(),
                    "Verify sufficient disk space is available".to_string(),
                    "Ensure the file system is not read-only".to_string(),
                    format!("Retry the '{}' operation", operation),
                ];
                if let Some(p) = path {
                    hints.insert(0, format!("Check access to path: {:?}", p));
                }
                // Add specific hints based on I/O error kind
                match source.kind() {
                    std::io::ErrorKind::PermissionDenied => {
                        hints.insert(
                            0,
                            "Fix file permissions or run with appropriate privileges".to_string(),
                        );
                    }
                    std::io::ErrorKind::NotFound => {
                        hints.insert(0, "Create missing files or directories".to_string());
                    }
                    std::io::ErrorKind::AlreadyExists => {
                        hints.insert(0, "Remove or rename conflicting files".to_string());
                    }
                    _ => {}
                }
                hints
            }
            DevItError::InvalidTestConfig {
                field,
                value,
                reason,
            } => vec![
                format!("Fix the '{}' configuration field", field),
                format!("Current invalid value '{}' - {}", value, reason),
                "Check configuration documentation for valid values".to_string(),
                "Use default configuration if unsure about correct values".to_string(),
            ],
            DevItError::Internal {
                component,
                correlation_id,
                ..
            } => vec![
                format!(
                    "Report this internal error to support with ID: {}",
                    correlation_id
                ),
                format!(
                    "Component '{}' encountered an unexpected condition",
                    component
                ),
                "Retry the operation after a brief delay".to_string(),
                "Check system logs for additional error details".to_string(),
                "Consider restarting the application if errors persist".to_string(),
            ],
            DevItError::InvalidFormat { supported, .. } => vec![
                format!("Use one of the supported formats: {}", supported.join(", ")),
                "Check the tool's help documentation for format examples".to_string(),
                "The 'json' format is always supported as a fallback".to_string(),
                "Consider using 'compact' format for better performance".to_string(),
            ],
        }
    }

    /// Indicates whether this error is recoverable through user action.
    ///
    /// Recoverable errors can potentially be resolved by the user (e.g., by
    /// providing higher approval, cleaning the working directory, or fixing
    /// test failures). Non-recoverable errors typically require system
    /// administrator intervention or code changes.
    ///
    /// # Returns
    ///
    /// `true` if the error condition might be resolvable by user action.
    pub fn is_recoverable(&self) -> bool {
        match self {
            DevItError::InvalidDiff { .. } => false,
            DevItError::SnapshotRequired { .. } => true,
            DevItError::SnapshotStale { .. } => true,
            DevItError::PolicyBlock { .. } => true,
            DevItError::ProtectedPath { .. } => true,
            DevItError::PrivilegeEscalation { .. } => false,
            DevItError::GitDirty { .. } => true,
            DevItError::VcsConflict { .. } => true,
            DevItError::TestFail { .. } => true,
            DevItError::TestTimeout { .. } => true,
            DevItError::SandboxDenied { .. } => true,
            DevItError::ResourceLimit { .. } => true,
            Self::Io { .. } => false,
            DevItError::InvalidTestConfig { .. } => true,
            DevItError::Internal { .. } => false,
            DevItError::InvalidFormat { .. } => true,
        }
    }

    /// Returns the severity level of this error.
    ///
    /// Severity levels help in determining appropriate logging levels and
    /// user notification strategies.
    pub fn severity(&self) -> ErrorSeverity {
        match self {
            DevItError::InvalidDiff { .. } => ErrorSeverity::Error,
            DevItError::SnapshotRequired { .. } => ErrorSeverity::Warning,
            DevItError::SnapshotStale { .. } => ErrorSeverity::Warning,
            DevItError::PolicyBlock { .. } => ErrorSeverity::Warning,
            DevItError::ProtectedPath { .. } => ErrorSeverity::Error,
            DevItError::PrivilegeEscalation { .. } => ErrorSeverity::Critical,
            DevItError::GitDirty { .. } => ErrorSeverity::Warning,
            DevItError::VcsConflict { .. } => ErrorSeverity::Error,
            DevItError::TestFail { .. } => ErrorSeverity::Error,
            DevItError::TestTimeout { .. } => ErrorSeverity::Warning,
            DevItError::SandboxDenied { .. } => ErrorSeverity::Error,
            DevItError::ResourceLimit { .. } => ErrorSeverity::Error,
            Self::Io { .. } => ErrorSeverity::Error,
            DevItError::InvalidTestConfig { .. } => ErrorSeverity::Warning,
            DevItError::Internal { .. } => ErrorSeverity::Critical,
            DevItError::InvalidFormat { .. } => ErrorSeverity::Warning,
        }
    }

    /// Creates a detailed error context for debugging and logging.
    ///
    /// This method extracts relevant context information from the error
    /// variant and formats it for structured logging or debugging output.
    ///
    /// # Returns
    ///
    /// A string containing formatted context information.
    pub fn debug_context(&self) -> String {
        match self {
            DevItError::PolicyBlock {
                rule,
                required_level,
                current_level,
                context,
            } => {
                format!(
                    "Policy Violation - Rule: {}, Current: {}, Required: {}, \
                     Context: {}",
                    rule, current_level, required_level, context
                )
            }
            DevItError::ProtectedPath {
                path,
                protection_rule,
                attempted_operation,
            } => {
                format!(
                    "Protected Path Access - Path: {:?}, Rule: {}, \
                     Operation: {}",
                    path, protection_rule, attempted_operation
                )
            }
            DevItError::TestFail {
                failed_count,
                total_count,
                test_framework,
                failure_details,
            } => {
                format!(
                    "Test Failure - {}/{} failed using {}, Details: {:?}",
                    failed_count, total_count, test_framework, failure_details
                )
            }
            DevItError::InvalidTestConfig {
                field,
                value,
                reason,
            } => {
                format!(
                    "Invalid Test Config - Field: {}, Value: {}, Reason: {}",
                    field, value, reason
                )
            }
            _ => format!("{:?}", self),
        }
    }
}

impl From<(Option<PathBuf>, std::io::Error)> for DevItError {
    fn from(value: (Option<PathBuf>, std::io::Error)) -> Self {
        DevItError::io(value.0, "io", value.1)
    }
}

impl From<anyhow::Error> for DevItError {
    fn from(error: anyhow::Error) -> Self {
        DevItError::internal(error.to_string())
    }
}

/// Error severity levels for categorizing error importance.
///
/// These levels can be used for determining logging verbosity, user
/// notification urgency, and automated response strategies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ErrorSeverity {
    /// Informational messages that don't indicate problems
    Info,
    /// Warning conditions that don't prevent operation completion
    Warning,
    /// Error conditions that prevent operation completion
    Error,
    /// Critical errors that may affect system stability
    Critical,
}

/// Convenience macros for creating common error variants with context.
///
/// These macros reduce boilerplate when creating errors in common scenarios.
/// Creates a PolicyBlock error with standardized formatting.
#[macro_export]
macro_rules! policy_block {
    ($rule:expr, $required:expr, $current:expr) => {
        DevItError::PolicyBlock {
            rule: $rule.to_string(),
            required_level: $required.to_string(),
            current_level: $current.to_string(),
            context: String::new(),
        }
    };
    ($rule:expr, $required:expr, $current:expr, $context:expr) => {
        DevItError::PolicyBlock {
            rule: $rule.to_string(),
            required_level: $required.to_string(),
            current_level: $current.to_string(),
            context: $context.to_string(),
        }
    };
}

/// Creates a ProtectedPath error with path and operation context.
#[macro_export]
macro_rules! protected_path {
    ($path:expr, $rule:expr, $operation:expr) => {
        DevItError::ProtectedPath {
            path: $path.into(),
            protection_rule: $rule.to_string(),
            attempted_operation: $operation.to_string(),
        }
    };
}

/// Creates an Internal error with component and correlation ID.
#[macro_export]
macro_rules! internal_error {
    ($component:expr, $message:expr) => {
        DevItError::Internal {
            component: $component.to_string(),
            message: $message.to_string(),
            cause: None,
            correlation_id: uuid::Uuid::new_v4().to_string(),
        }
    };
}
