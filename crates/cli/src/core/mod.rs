//! # DevIt Core Engine
//!
//! Central orchestration engine that coordinates patch application, snapshot
//! management, security policy enforcement, and test execution.
//!
//! ## Architecture
//!
//! The Core Engine follows a modular design where each subsystem can be
//! independently tested and configured:
//!
//! - **Patch Management**: Parse, validate, and apply unified diff patches
//! - **Snapshot System**: Track file states and detect changes over time
//! - **Policy Engine**: Enforce security policies and approval workflows
//! - **Test Runner**: Execute test suites with sandboxing and timeout
//! - **Journal System**: Audit trail for all operations and decisions
//!
//! ## Security Model
//!
//! All operations are subject to approval level checks:
//! - **Untrusted**: Minimal permissions, strict validation
//! - **Ask**: Interactive approval for sensitive operations
//! - **Moderate**: Limited automated operations
//! - **Trusted**: Extended permissions for known-safe operations
//! - **Privileged**: Infrastructure changes with explicit path allowlists

use std::collections::HashMap;
use std::fs::{create_dir_all, remove_file, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::warn;
use uuid::Uuid;

use crate::core::formats::Compressible;

// Module declarations
pub mod atomic_patcher;
pub mod config;
pub mod errors;
pub mod file_ops;
pub mod formats;
pub mod fs;
pub mod help_system;
pub mod journal;
pub mod num_compat;
pub mod orchestration;
pub mod patch;
pub mod patch_parser;
pub mod path_security;
pub mod policy;
mod request_id;
pub mod safe_write;
pub mod sandbox;
pub mod schema;
pub mod security;
pub mod serde_api;
pub mod snapshot;

// Re-export core types and errors for convenience
use atomic_patcher::AtomicPatcher;
pub use config::CoreConfig;
pub use devit_common::{ApprovalLevel, FileChangeKind, SandboxProfile, SnapshotId};
pub use errors::{DevItError, DevItResult, ErrorSeverity};
pub use path_security::PathSecurityContext;
pub use security::workspace::SecureWorkspace;

use crate::core::orchestration::{format_status, StatusFormat};

/// Represents a file change for policy evaluation.
#[derive(Debug, Clone)]
pub struct FileChange {
    /// Path to the file
    pub path: PathBuf,
    /// Kind of change (add, modify, delete, etc.)
    pub kind: FileChangeKind,
    /// Change in file size (positive = increase, negative = decrease)
    pub size_change: i64,
    /// Number of lines in the change
    pub line_count: usize,
    /// Whether this is a binary file
    pub binary: bool,
}
pub use journal::{
    JournalEntry, JournalManager, JournalResponse, JournalRuntimeConfig,
    OperationType as JournalOperationType,
};
pub use patch::PatchApplyRequest;
pub use policy::{PolicyContext, PolicyDecision, PolicyEngineConfig};
pub use request_id::resolve as resolve_request_id;
pub use sandbox::SandboxPlan;
pub use serde_api::{StdError, StdResponse};

/// Idempotency cache entry containing request results and timestamp
#[derive(Debug, Clone)]
struct IdempotencyEntry {
    /// Serialized response for replay
    response: String,
    /// Timestamp when the entry was created
    created_at: Instant,
    /// TTL for this entry
    ttl: Duration,
}

impl IdempotencyEntry {
    /// Check if this entry has expired
    fn is_expired(&self) -> bool {
        self.created_at.elapsed() > self.ttl
    }
}

/// In-process idempotency cache with TTL support
#[derive(Debug, Default)]
struct IdempotencyCache {
    /// Map of idempotency_key -> cached entry
    entries: HashMap<String, IdempotencyEntry>,
}

impl IdempotencyCache {
    /// Create a new idempotency cache
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Get an entry if it exists and hasn't expired
    fn get(&mut self, key: &str) -> Option<IdempotencyEntry> {
        if let Some(entry) = self.entries.get(key) {
            if entry.is_expired() {
                self.entries.remove(key);
                None
            } else {
                Some(entry.clone())
            }
        } else {
            None
        }
    }

    /// Insert a new entry with TTL
    fn insert(&mut self, key: String, response: String, ttl: Duration) {
        let entry = IdempotencyEntry {
            response,
            created_at: Instant::now(),
            ttl,
        };
        self.entries.insert(key, entry);
    }

    /// Clean up expired entries
    fn cleanup_expired(&mut self) {
        self.entries.retain(|_, entry| !entry.is_expired());
    }
}

/// Central orchestration engine for DevIt operations.
///
/// The CoreEngine coordinates all subsystems and enforces security policies
/// across patch application, snapshot management, testing, and journaling.
/// All operations are asynchronous and respect approval level constraints.
pub struct CoreEngine {
    /// Configuration settings for all subsystems
    config: CoreConfig,

    /// Patch management subsystem for handling unified diffs
    patch_manager: RwLock<patch::PatchManager>,

    /// Snapshot management for tracking file state changes
    snapshot_manager: RwLock<snapshot::SnapshotManager>,

    /// Security policy engine for approval and access control
    policy_engine: RwLock<policy::PolicyEngine>,

    /// Audit journal for operation tracking and compliance
    journal: RwLock<journal::Journal>,

    /// Path security context for validating file paths and symlinks
    path_security: RwLock<PathSecurityContext>,

    /// File operations manager for secure file exploration
    file_ops: RwLock<file_ops::FileOpsManager>,

    /// In-process idempotency cache for request deduplication
    idempotency_cache: RwLock<IdempotencyCache>,

    /// AI orchestration subsystem for task delegation and monitoring
    orchestration: RwLock<orchestration::OrchestrationManager>,

    /// Secure workspace manager enforcing sandbox boundaries
    workspace: RwLock<SecureWorkspace>,
}

impl CoreEngine {
    /// Creates a new CoreEngine instance with the specified configuration.
    ///
    /// Initializes all subsystems and validates the configuration for
    /// consistency and security requirements.
    ///
    /// # Arguments
    ///
    /// * `config` - Complete configuration for all subsystems
    ///
    /// # Returns
    ///
    /// A configured CoreEngine ready for operation, or an error if
    /// configuration validation fails.
    ///
    /// # Errors
    ///
    /// Returns `DevItError::Internal` if subsystem initialization fails.
    pub async fn new(config: CoreConfig) -> DevItResult<Self> {
        use crate::core::journal::Journal;
        use crate::core::patch::PatchManager;
        use crate::core::policy::PolicyEngine;
        use crate::core::snapshot::SnapshotManager;
        // Resolve sandbox root
        let mut config = config;

        let workspace_root =
            config
                .workspace
                .resolve_root()
                .map_err(|err| DevItError::Internal {
                    component: "workspace".to_string(),
                    message: err.to_string(),
                    cause: None,
                    correlation_id: Uuid::new_v4().to_string(),
                })?;

        let mut workspace =
            SecureWorkspace::new(workspace_root).map_err(|err| DevItError::Internal {
                component: "workspace".to_string(),
                message: err.to_string(),
                cause: None,
                correlation_id: Uuid::new_v4().to_string(),
            })?;
        workspace
            .set_allowed_projects(&config.workspace.allowed_projects)
            .map_err(|err| DevItError::Internal {
                component: "workspace".to_string(),
                message: err.to_string(),
                cause: None,
                correlation_id: Uuid::new_v4().to_string(),
            })?;

        if let Some(default_project) = &config.workspace.default_project {
            if let Err(err) = workspace.change_dir(default_project) {
                warn!(
                    "Failed to change to default project '{}': {}",
                    default_project, err
                );
            }
        }

        let working_dir = workspace.current_dir();
        config.runtime.working_directory = Some(working_dir.clone());

        let default_approval_level = config.policy.default_approval_level.clone();
        let allow_internal_symlinks = default_approval_level != ApprovalLevel::Untrusted;
        let orchestration_config = config.orchestration.clone().into();

        let orchestration_manager =
            orchestration::OrchestrationManager::new(orchestration_config).await?;

        Ok(Self {
            // Store the configuration
            config,
            // Initialize with default/mock implementations for testing
            patch_manager: RwLock::new(PatchManager::new(crate::core::patch::PatchConfig {
                working_directory: working_dir.clone(),
                create_backups: false,
                max_patch_size: 1024 * 1024,
                max_affected_files: 100,
                enable_security_analysis: true,
                cache_size: 50,
            })),
            snapshot_manager: RwLock::new(SnapshotManager::new(working_dir.clone(), 10)),
            policy_engine: RwLock::new(PolicyEngine::new(
                default_approval_level,
                devit_common::SandboxProfile::Strict,
            )),
            journal: RwLock::new(Journal::new(
                PathBuf::from(".devit/journal.log"),
                b"test-secret".to_vec(),
            )),
            path_security: RwLock::new(PathSecurityContext::new(
                &working_dir,
                allow_internal_symlinks,
            )?),
            file_ops: RwLock::new(file_ops::FileOpsManager::new(&working_dir)?),
            idempotency_cache: RwLock::new(IdempotencyCache::new()),
            orchestration: RwLock::new(orchestration_manager),
            workspace: RwLock::new(workspace),
        })
    }

    /// Retrieves the current snapshot of the working directory.
    ///
    /// Creates or updates a snapshot capturing the current state of all
    /// tracked files. This snapshot can be used for change detection and
    /// rollback operations.
    ///
    /// # Arguments
    ///
    /// * `paths` - Optional list of specific paths to include in snapshot.
    ///   If None, includes all tracked files.
    ///
    /// # Returns
    ///
    /// The ID of the created or updated snapshot.
    ///
    /// # Errors
    ///
    /// * `DevItError::Io` - If file system access fails
    /// * `DevItError::Internal` - If snapshot creation fails
    pub async fn snapshot_get(&self, paths: Option<&[PathBuf]>) -> DevItResult<SnapshotId> {
        // Create snapshot using SnapshotManager
        let root_path = self.workspace_current_dir().await?;
        let snapshot_id = {
            let manager = self.snapshot_manager.write().await;
            let description = format!("Snapshot created at {}", chrono::Utc::now().to_rfc3339());
            manager.create_snapshot(root_path.clone(), description, None)?
        };

        // Log snapshot creation to journal
        let details = std::collections::HashMap::from([
            ("snapshot_id".to_string(), snapshot_id.0.clone()),
            (
                "paths_count".to_string(),
                paths.map_or("all".to_string(), |p| p.len().to_string()),
            ),
        ]);
        self.journal_append("snapshot_create", &details, None)
            .await?;

        Ok(snapshot_id)
    }

    /// Validates whether a snapshot is still current relative to working
    /// directory state.
    ///
    /// Compares the snapshot against current file states to determine if
    /// the snapshot accurately represents the working directory.
    ///
    /// # Arguments
    ///
    /// * `snapshot_id` - ID of the snapshot to validate
    /// * `paths` - Paths to check for changes since snapshot
    ///
    /// # Returns
    ///
    /// `true` if the snapshot is current, `false` if stale.
    ///
    /// # Errors
    ///
    /// * `DevItError::SnapshotRequired` - If snapshot ID is invalid
    /// * `DevItError::Io` - If file system access fails
    pub async fn snapshot_validate(
        &self,
        snapshot_id: &SnapshotId,
        paths: &[PathBuf],
    ) -> DevItResult<bool> {
        let (is_valid, difference_count, diff_sample, created_at) = {
            let manager = self.snapshot_manager.read().await;
            let internal_id = SnapshotId(snapshot_id.0.clone());
            let snapshot = manager.get_snapshot(&internal_id)?;
            let reference = if paths.is_empty() { None } else { Some(paths) };
            let differences = snapshot.compare_with_current(reference)?;
            let sample = differences
                .first()
                .map(|diff| Self::format_snapshot_difference(diff));
            (
                differences.is_empty(),
                differences.len(),
                sample,
                snapshot.created_at,
            )
        };

        let mut details = HashMap::from([
            ("snapshot_id".to_string(), snapshot_id.0.clone()),
            (
                "paths_checked".to_string(),
                if paths.is_empty() {
                    "all".to_string()
                } else {
                    paths.len().to_string()
                },
            ),
            (
                "result".to_string(),
                if is_valid {
                    "valid".to_string()
                } else {
                    "stale".to_string()
                },
            ),
        ]);

        let timestamp: chrono::DateTime<chrono::Utc> = created_at.into();
        details.insert("created_at".to_string(), timestamp.to_rfc3339());

        if !is_valid {
            details.insert("differences".to_string(), difference_count.to_string());
            if let Some(sample) = diff_sample {
                details.insert("difference_sample".to_string(), sample);
            }
        }

        self.journal_append("snapshot_validate", &details, None)
            .await?;

        Ok(is_valid)
    }

    /// Creates a new snapshot with optional naming.
    ///
    /// Captures the current state of the working directory for later
    /// restoration or comparison.
    ///
    /// # Arguments
    ///
    /// * `name` - Optional human-readable name for the snapshot
    ///
    /// # Returns
    ///
    /// Unique identifier for the created snapshot.
    ///
    /// # Errors
    ///
    /// * `DevItError::Io` - If file system access fails
    /// * `DevItError::SnapshotLimit` - If storage quota exceeded
    pub async fn snapshot_create(&self, name: Option<&str>) -> DevItResult<SnapshotId> {
        let root_path = self.workspace_current_dir().await?;
        let description = name.unwrap_or("manual");

        let (snapshot_id, file_count, total_size, created_at) = {
            let manager = self.snapshot_manager.write().await;
            let snapshot_id =
                manager.create_snapshot(root_path.clone(), description.to_string(), None)?;
            let internal_id = SnapshotId(snapshot_id.0.clone());
            let snapshot = manager.get_snapshot(&internal_id)?;
            (
                snapshot_id,
                snapshot.files.len(),
                snapshot.total_size,
                snapshot.created_at,
            )
        };

        let mut details = HashMap::from([
            ("snapshot_id".to_string(), snapshot_id.0.clone()),
            ("name".to_string(), description.to_string()),
            ("file_count".to_string(), file_count.to_string()),
            ("size_bytes".to_string(), total_size.to_string()),
        ]);

        let timestamp: chrono::DateTime<chrono::Utc> = created_at.into();
        details.insert("created_at".to_string(), timestamp.to_rfc3339());

        self.journal_append("snapshot_create", &details, None)
            .await?;

        Ok(snapshot_id)
    }

    /// Restores the working directory to a previous snapshot state.
    ///
    /// Reverts all files to their state when the snapshot was created.
    /// This operation is atomic - either all files are restored or none.
    ///
    /// # Arguments
    ///
    /// * `snapshot_id` - ID or name of the snapshot to restore
    ///
    /// # Returns
    ///
    /// Success if restoration completed without errors.
    ///
    /// # Errors
    ///
    /// * `DevItError::SnapshotNotFound` - If snapshot ID is invalid
    /// * `DevItError::Io` - If file system access fails
    /// * `DevItError::PermissionDenied` - If restoration requires elevated permissions
    pub async fn snapshot_restore(&self, snapshot_id: &str) -> DevItResult<()> {
        // Restore snapshot using SnapshotManager
        {
            let manager = self.snapshot_manager.write().await;
            manager.restore_snapshot(&SnapshotId(snapshot_id.to_string()))?;
        }

        // Log snapshot restoration to journal
        let details = std::collections::HashMap::from([
            ("snapshot_id".to_string(), snapshot_id.to_string()),
            ("operation".to_string(), "restore".to_string()),
        ]);
        self.journal_append("snapshot_restore", &details, None)
            .await?;

        Ok(())
    }

    /// Analyzes a patch without applying it to preview the changes.
    ///
    /// Performs security analysis, policy checking, and impact assessment
    /// to help users understand what changes would be made.
    ///
    /// # Arguments
    ///
    /// * `patch_content` - Unified diff content to analyze
    /// * `base_snapshot` - Optional snapshot to use as baseline
    ///
    /// # Returns
    ///
    /// Detailed analysis of the patch contents and policy implications.
    ///
    /// # Errors
    ///
    /// * `DevItError::InvalidDiff` - If patch format is malformed
    /// * `DevItError::SnapshotStale` - If base snapshot is outdated
    pub async fn patch_preview(
        &self,
        _patch_content: &str,
        _base_snapshot: Option<&SnapshotId>,
    ) -> DevItResult<PatchPreview> {
        if let Some(snapshot_id) = _base_snapshot {
            if !self.snapshot_validate(snapshot_id, &[]).await? {
                return Err(DevItError::SnapshotStale {
                    snapshot_id: snapshot_id.0.clone(),
                    created_at: None,
                    staleness_reason: Some(
                        "Base snapshot is stale relative to workspace state".to_string(),
                    ),
                });
            }
        }

        let file_changes = patch::classify_changes(_patch_content)?;
        if file_changes.is_empty() {
            return Err(DevItError::InvalidDiff {
                reason: "Patch does not contain any file changes".to_string(),
                line_number: None,
            });
        }

        let affected_files = Self::unique_paths(&file_changes);
        let estimated_line_changes: usize = file_changes
            .iter()
            .map(|change| change.lines_added + change.lines_removed)
            .sum();

        let affects_binaries = file_changes.iter().any(|change| change.is_binary);
        let permission_changes = Self::extract_permission_changes(_patch_content);

        let (mut affects_protected, mut policy_warnings) =
            self.evaluate_path_safety(&file_changes).await?;

        if !permission_changes.is_empty() {
            affects_protected = true;
        }

        let mut protected_rule_hits = self.protected_rule_hits(&affected_files);
        if !protected_rule_hits.is_empty() {
            affects_protected = true;
            policy_warnings.append(&mut protected_rule_hits);
        }

        if !policy_warnings.is_empty() {
            policy_warnings.sort();
            policy_warnings.dedup();
        }

        let recommended_approval = Self::recommended_approval(
            affects_protected,
            affects_binaries,
            estimated_line_changes,
            affected_files.len(),
        );

        let preview = PatchPreview {
            affected_files: affected_files.clone(),
            estimated_line_changes,
            affects_protected,
            affects_binaries,
            policy_warnings: policy_warnings.clone(),
            recommended_approval: recommended_approval.clone(),
            permission_changes,
        };

        let mut details = HashMap::from([
            ("files".to_string(), affected_files.len().to_string()),
            (
                "estimated_lines".to_string(),
                estimated_line_changes.to_string(),
            ),
            (
                "affects_protected".to_string(),
                affects_protected.to_string(),
            ),
            ("affects_binaries".to_string(), affects_binaries.to_string()),
            (
                "recommended_approval".to_string(),
                format!("{:?}", recommended_approval),
            ),
        ]);

        if !policy_warnings.is_empty() {
            details.insert("policy_warnings".to_string(), policy_warnings.join(" | "));
        }

        self.journal_append("patch_preview", &details, None).await?;

        Ok(preview)
    }

    /// Applies a patch to the working directory with security enforcement.
    ///
    /// This implements the complete E2E patch application workflow:
    /// 1. classify_changes(diff) - Parse and analyze patch content
    /// 2. policy.evaluate() - Security policy validation
    /// 3. git apply --check - Dry-run validation
    /// 4. Apply real + git add -A - File system changes
    /// 5. Optional commit - Version control integration
    /// 6. Journal append - Audit trail recording
    /// 7. Generate rollback_cmd - Recovery preparation
    ///
    /// # Arguments
    ///
    /// * `patch_content` - Unified diff content to apply
    /// * `approval_level` - Level of approval for this operation
    /// * `dry_run` - If true, validate but don't apply changes
    ///
    /// # Returns
    ///
    /// Detailed results of the patch application process.
    ///
    /// # Errors
    ///
    /// * `DevItError::InvalidDiff` - If patch is malformed
    /// * `DevItError::PolicyBlock` - If approval level insufficient
    /// * `DevItError::ProtectedPath` - If patch affects protected files
    /// * `DevItError::GitDirty` - If working directory has uncommitted changes
    /// * `DevItError::VcsConflict` - If Git conflicts detected
    /// * `DevItError::Io` - If file system operations fail
    pub async fn patch_apply(
        &self,
        patch_content: &str,
        approval_level: ApprovalLevel,
        dry_run: bool,
        idempotency_key: Option<&str>,
    ) -> DevItResult<PatchResult> {
        use std::process::Command;
        use std::time::Instant;

        // Check idempotency cache if key is provided
        if let Some(key) = idempotency_key {
            let mut cache = self.idempotency_cache.write().await;

            // Clean up expired entries
            cache.cleanup_expired();

            if let Some(entry) = cache.get(key) {
                // Deserialize cached response
                if let Ok(cached_result) = serde_json::from_str::<PatchResult>(&entry.response) {
                    return Ok(cached_result);
                }
            }
        }

        let start_time = Instant::now();
        let mut warnings = Vec::new();
        let mut info_messages = Vec::new();
        let required_elevation = false;

        // Step 1: classify_changes(diff) - Parse patch and analyze changes
        let file_changes = patch::classify_changes(patch_content)?;

        if file_changes.is_empty() {
            return Err(DevItError::InvalidDiff {
                reason: "No valid file changes found in patch".to_string(),
                line_number: None,
            });
        }

        // Extract affected file paths for further processing
        let affected_files = Self::unique_paths(&file_changes);

        info_messages.push(format!(
            "Analyzed {} file changes from patch",
            file_changes.len()
        ));

        let binary_files: Vec<String> = file_changes
            .iter()
            .filter(|change| change.is_binary)
            .map(|change| change.file_path.display().to_string())
            .collect();
        if !binary_files.is_empty() {
            info_messages.push(format!(
                "Detected binary file changes (whitelist enforced): {}",
                binary_files.join(", ")
            ));
        }

        // Step 1.5: Path security validation (C4 requirement)
        {
            let path_security = self.path_security.read().await;
            for change in &file_changes {
                // Validate the file path itself
                path_security.validate_patch_path(&change.file_path)?;

                // If this is a symlink, validate the symlink target
                if change.is_symlink {
                    if let Some(target) = &change.symlink_target {
                        path_security.validate_symlink(&change.file_path, target)?;
                    }
                }
            }
        }

        info_messages.push("Path security validation passed".to_string());

        // Step 2: policy.evaluate() - Security policy evaluation with PolicyBlock handling
        let policy_engine = self.policy_engine.read().await;

        // Convert patch file changes to policy file changes format
        let policy_file_changes: Vec<policy::FileChange> = file_changes
            .iter()
            .map(|change| policy::FileChange {
                path: change.file_path.clone(),
                kind: match change.change_type {
                    patch::FileChangeType::Created => FileChangeKind::Create,
                    patch::FileChangeType::Modified => FileChangeKind::Modify,
                    patch::FileChangeType::Deleted => FileChangeKind::Delete,
                    patch::FileChangeType::Renamed => FileChangeKind::Modify,
                    patch::FileChangeType::Copied => FileChangeKind::Create,
                },
                is_binary: change.is_binary,
                adds_exec_bit: change.adds_exec_bit,
                lines_added: change.lines_added,
                lines_deleted: change.lines_removed,
                is_symlink: change.is_symlink,
                symlink_target_abs: change.symlink_target.clone(),
                touches_protected: false, // Will be set by policy engine
                touches_submodule: change.is_submodule,
                touches_gitmodules: change.touches_gitmodules,
                file_size_bytes: Some(0), // Not available from patch analysis
            })
            .collect();

        let policy_context = PolicyContext {
            file_changes: policy_file_changes,
            requested_approval_level: approval_level.clone(),
            protected_paths: self.config.policy.protected_paths.clone(),
            config: policy_engine.config().clone(),
        };

        let policy_decision = policy_engine
            .evaluate_changes(&policy_context)
            .map_err(|e| DevItError::Internal {
                component: "policy_engine".to_string(),
                message: format!("Policy evaluation failed: {:?}", e),
                cause: None,
                correlation_id: uuid::Uuid::new_v4().to_string(),
            })?;

        if !policy_decision.allow {
            let policy_rule = Self::infer_policy_rule(&policy_decision.reason);
            return Err(DevItError::PolicyBlock {
                rule: policy_rule,
                required_level: if let Some(downgraded) = policy_decision.downgraded_to {
                    format!("{:?}", downgraded)
                } else {
                    "Higher".to_string()
                },
                current_level: format!("{:?}", approval_level),
                context: policy_decision.reason,
            });
        }

        if policy_decision.requires_confirmation {
            if approval_level == ApprovalLevel::Ask {
                warnings.push(format!(
                    "Policy requires confirmation: {}",
                    policy_decision.reason
                ));
            } else {
                return Err(DevItError::PolicyBlock {
                    rule: "confirmation_required".to_string(),
                    required_level: "Ask".to_string(),
                    current_level: format!("{:?}", approval_level),
                    context: policy_decision.reason,
                });
            }
        } else {
            info_messages.push(format!("Policy evaluation: {}", policy_decision.reason));
        }
        drop(policy_engine);

        // Step 3: Atomic patch application with security validation
        info_messages.push("Applying patch with atomic file operations".to_string());

        let working_dir = self
            .config
            .runtime
            .working_directory
            .clone()
            .unwrap_or_else(|| PathBuf::from("."));

        let patcher = AtomicPatcher::new(working_dir, dry_run);
        let patch_stats = patcher.apply_patch(patch_content)?;

        info_messages.push("Patch validation and application successful".to_string());

        // Early exit for dry-run mode - guaranteed no modifications
        if dry_run {
            info_messages.push("Dry run complete - no files were modified".to_string());
            let execution_time = start_time.elapsed();

            let result = PatchResult {
                success: true,
                modified_files: Vec::new(),
                warnings,
                info_messages,
                resulting_snapshot: None,
                execution_time,
                required_elevation,
                commit_sha: None,
                rollback_cmd: None,
                test_results: None,
                auto_reverted: false,
                reverted_sha: None,
            };

            // Cache the dry-run result if idempotency key is provided
            if let Some(key) = idempotency_key {
                if let Ok(serialized) = serde_json::to_string(&result) {
                    let mut cache = self.idempotency_cache.write().await;
                    cache.insert(
                        key.to_string(),
                        serialized,
                        Duration::from_secs(3600), // 1 hour TTL
                    );
                }
            }

            return Ok(result);
        }

        // Update affected files based on actual patch statistics
        let modified_files = affected_files.clone();
        info_messages.push(format!(
            "Successfully applied patch: {} files modified, {} hunks, +{} -{} lines",
            patch_stats.files_modified + patch_stats.files_created,
            patch_stats.hunks_applied,
            patch_stats.lines_added,
            patch_stats.lines_removed
        ));

        // Step 4.5: Pre-commit path security validation (C4 TOCTOU protection)
        {
            let path_security = self.path_security.read().await;
            path_security.pre_commit_validation(&affected_files)?;
        }
        info_messages.push("Pre-commit security validation passed".to_string());

        // Step 5: Optional commit with conventional message handling
        let commit_sha = if self.config.git.auto_commit {
            info_messages.push("Creating commit for applied patch".to_string());

            let commit_message = format!("feat(patch): apply unified diff patch\n\nModified {} files through patch application\n\nGenerated with DevIt patch_apply", modified_files.len());

            let working_dir = self.workspace_current_dir().await?;
            let mut git_commit = Command::new("git");
            git_commit.args(["commit", "-m", &commit_message]);
            git_commit.current_dir(&working_dir);

            let commit_output = git_commit
                .output()
                .map_err(|e| DevItError::io(Some(working_dir.clone()), "git commit", e))?;

            if commit_output.status.success() {
                // Extract commit SHA from git output
                let stdout = String::from_utf8_lossy(&commit_output.stdout);
                let sha = stdout
                    .lines()
                    .find_map(|line| {
                        if line.starts_with("[") {
                            line.split_whitespace().nth(1).map(|s| s.to_string())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| "unknown".to_string());

                info_messages.push(format!("Created commit: {}", sha));
                Some(sha)
            } else {
                let stderr = String::from_utf8_lossy(&commit_output.stderr);
                warnings.push(format!("Commit failed: {}", stderr));
                None
            }
        } else {
            info_messages.push("Auto-commit disabled - changes staged only".to_string());
            None
        };

        // Step 6: Journal append with idempotency_key
        let journal_entry = serde_json::json!({
            "operation": "patch_apply",
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "approval_level": format!("{:?}", approval_level),
            "dry_run": false,
            "files_modified": modified_files.len(),
            "affected_files": modified_files,
            "commit_sha": commit_sha,
            "execution_time_ms": start_time.elapsed().as_millis()
        });

        let mut journal = self.journal.write().await;
        let journal_response = journal.append(journal_entry, None)?;
        drop(journal);

        info_messages.push(format!(
            "Journal entry recorded: offset {}",
            journal_response.offset
        ));

        // Step 7: Generate rollback_cmd without execution
        let rollback_cmd = if let Some(ref sha) = commit_sha {
            Some(format!("git revert {}", sha))
        } else {
            Some("git reset --hard HEAD".to_string())
        };

        if let Some(ref cmd) = rollback_cmd {
            info_messages.push(format!("Rollback command available: {}", cmd));
        }

        let execution_time = start_time.elapsed();

        let result = PatchResult {
            success: true,
            modified_files,
            warnings,
            info_messages,
            resulting_snapshot: None, // Would create snapshot in real implementation
            execution_time,
            required_elevation,
            commit_sha,
            rollback_cmd,
            test_results: None,
            auto_reverted: false,
            reverted_sha: None,
        };

        // Cache the result if idempotency key is provided
        if let Some(key) = idempotency_key {
            if let Ok(serialized) = serde_json::to_string(&result) {
                let mut cache = self.idempotency_cache.write().await;
                cache.insert(
                    key.to_string(),
                    serialized,
                    Duration::from_secs(3600), // 1 hour TTL
                );
            }
        }

        Ok(result)
    }

    /// Enhanced patch application with optional post-test execution and auto-revert.
    ///
    /// This method implements the full R2 workflow:
    /// 1. Apply patch as usual
    /// 2. If post_tests are specified, run them after commit
    /// 3. If tests fail and policy allows, automatically revert the patch
    ///
    /// # Arguments
    /// * `request` - Complete patch apply request including optional post-tests
    /// * `approval_level` - Security approval level for the operation
    /// * `dry_run` - Whether to perform a dry-run without actual changes
    ///
    /// # Returns
    /// Enhanced patch result including test results and revert information
    pub async fn patch_apply_with_tests(
        &self,
        request: &patch::PatchApplyRequest,
        approval_level: ApprovalLevel,
        dry_run: bool,
    ) -> DevItResult<PatchResult> {
        use std::time::Instant;

        let start_time = Instant::now();

        // Step 1: Apply the patch normally
        let idempotency_key_str = request
            .idempotency_key
            .as_ref()
            .map(|uuid| uuid.to_string());
        let mut patch_result = self
            .patch_apply(
                &request.diff,
                approval_level.clone(),
                dry_run,
                idempotency_key_str.as_deref(),
            )
            .await?;

        // If dry-run or no post-tests specified, return early
        if dry_run || request.post_tests.is_none() {
            return Ok(patch_result);
        }

        let post_tests = request.post_tests.as_ref().unwrap();

        // Step 2: Convert TestRunRequest to TestConfig
        let test_config = self.convert_test_run_request_to_config(post_tests)?;

        // Step 3: Run tests using the sandboxed executor
        let test_results = self.test_run(&test_config, SandboxProfile::Strict).await?;

        // Update patch result with test information
        patch_result.test_results = Some(test_results.clone());

        // Step 4: Check if auto-revert should be triggered
        let should_revert = self
            .should_auto_revert(&test_results, &approval_level)
            .await?;

        if should_revert {
            // Step 5: Perform auto-revert
            let revert_result = self.perform_auto_revert(&patch_result).await?;

            patch_result.auto_reverted = true;
            patch_result.reverted_sha = revert_result.commit_sha;

            // Update warnings and info messages
            patch_result
                .warnings
                .push("Tests failed - patch was automatically reverted".to_string());
            if let Some(ref cmd) = revert_result.rollback_cmd {
                patch_result
                    .info_messages
                    .push(format!("Auto-revert executed: {}", cmd));
            }

            // Log the revert operation
            self.log_revert_operation(&patch_result, &test_results)
                .await?;
        }

        patch_result.execution_time = start_time.elapsed();
        Ok(patch_result)
    }

    /// Executes tests with the specified configuration and sandboxing.
    ///
    /// Runs test suites in an isolated environment with resource constraints
    /// and timeout handling to prevent system interference.
    ///
    /// # Arguments
    ///
    /// * `test_config` - Configuration for test execution
    /// * `sandbox_profile` - Isolation profile to use
    ///
    /// # Returns
    ///
    /// Comprehensive results from test execution.
    ///
    /// # Errors
    ///
    /// * `DevItError::TestTimeout` - If tests exceed time limit
    /// * `DevItError::SandboxDenied` - If sandbox blocks execution
    /// * `DevItError::ResourceLimit` - If resource constraints exceeded
    pub async fn test_run(
        &self,
        test_config: &TestConfig,
        sandbox_profile: SandboxProfile,
    ) -> DevItResult<TestResults> {
        let start_time = std::time::Instant::now();

        // Detect framework if not specified
        let framework = test_config.framework.as_deref().unwrap_or("auto");
        let detected_stack = if framework == "auto" {
            self.detect_test_framework().await?
        } else {
            framework.to_string()
        };

        // Build sandbox plan
        let sandbox_plan = self.build_test_sandbox_plan(&sandbox_profile).await?;

        // Construct test command based on framework
        let command = self.build_test_command(&detected_stack, test_config)?;

        // Execute tests with sandbox and timeout
        let execution_result = self
            .execute_sandboxed_command(&command, &sandbox_plan, test_config.timeout_secs)
            .await?;

        // Parse test output and build results
        let test_results =
            self.parse_test_output(&detected_stack, &execution_result, start_time.elapsed())?;

        // Log test execution to journal
        let mut details = std::collections::HashMap::new();
        details.insert("framework".to_string(), detected_stack.clone());
        details.insert("success".to_string(), test_results.success.to_string());
        details.insert(
            "total_tests".to_string(),
            test_results.total_tests.to_string(),
        );
        details.insert(
            "duration_ms".to_string(),
            test_results.execution_time.as_millis().to_string(),
        );

        self.journal_append("test_run", &details, None).await?;

        Ok(test_results)
    }

    /// Appends an entry to the operation journal for audit purposes.
    ///
    /// Records operations, decisions, and outcomes for compliance and
    /// debugging. All entries are timestamped and optionally signed.
    ///
    /// # Arguments
    ///
    /// * `operation` - Description of the operation performed
    /// * `details` - Additional context and result information
    ///
    /// # Returns
    ///
    /// Success confirmation or error if journaling fails.
    ///
    /// # Errors
    ///
    /// * `DevItError::Io` - If journal file cannot be written
    /// * `DevItError::ResourceLimit` - If journal size limits exceeded
    pub async fn journal_append(
        &self,
        operation: &str,
        details: &HashMap<String, String>,
        idempotency_key: Option<&str>,
    ) -> DevItResult<JournalResponse> {
        // Generate request ID for this operation
        let request_id = Uuid::new_v4();

        // Check idempotency cache if key is provided
        if let Some(key) = idempotency_key {
            let mut cache = self.idempotency_cache.write().await;

            // Clean up expired entries
            cache.cleanup_expired();

            if let Some(entry) = cache.get(key) {
                // Deserialize cached response
                if let Ok(cached_result) = serde_json::from_str::<JournalResponse>(&entry.response)
                {
                    return Ok(cached_result);
                }
            }
        }

        // Create journal entry with proper HMAC signing
        let entry = serde_json::json!({
            "operation": operation,
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "request_id": request_id.to_string(),
            "details": details,
            "idempotency_key": idempotency_key
        });

        let journal_response = {
            let mut journal = self.journal.write().await;
            let idem_uuid = idempotency_key.and_then(|k| uuid::Uuid::parse_str(k).ok());
            journal.append(entry, idem_uuid)?
        };

        // Cache the result if idempotency key is provided
        if let Some(key) = idempotency_key {
            if let Ok(serialized) = serde_json::to_string(&journal_response) {
                let mut cache = self.idempotency_cache.write().await;
                cache.insert(
                    key.to_string(),
                    serialized,
                    Duration::from_secs(3600), // 1 hour TTL
                );
            }
        }

        Ok(journal_response)
    }

    /// Generates a request identifier, reusing the same UUID when the same
    /// idempotency key is provided within the process.
    pub fn request_id_for(&self, idempotency_key: Option<Uuid>) -> Uuid {
        request_id::resolve(idempotency_key)
    }

    /// Retrieves the current configuration for inspection or modification.
    ///
    /// Provides read-only access to the engine's configuration settings.
    /// Configuration changes require creating a new engine instance.
    ///
    /// # Returns
    ///
    /// Reference to the current configuration.
    pub fn get_config(&self) -> &config::CoreConfig {
        &self.config
    }

    /// Validates the current system state and configuration.
    ///
    /// Performs comprehensive health checks across all subsystems to
    /// identify potential issues or misconfigurations.
    ///
    /// # Returns
    ///
    /// `true` if all systems are operational, `false` if issues detected.
    ///
    /// # Errors
    ///
    /// * `DevItError::Internal` - If critical system errors detected
    pub async fn health_check(&self) -> DevItResult<bool> {
        use chrono::Utc;

        let workspace_dir = match self.workspace_current_dir().await {
            Ok(dir) => dir,
            Err(err) => {
                let section = HealthSectionStatus {
                    name: "workspace".to_string(),
                    ok: false,
                    details: Some(err.to_string()),
                };
                let report = HealthReport {
                    overall_ok: false,
                    timestamp: Utc::now(),
                    sections: vec![section.clone()],
                };

                let mut details = HashMap::new();
                details.insert("overall_ok".to_string(), "false".to_string());
                if let Ok(serialized) = serde_json::to_string(&report.sections) {
                    details.insert("sections".to_string(), serialized);
                }

                self.journal_append("health_check", &details, None).await?;

                return Ok(false);
            }
        };

        let mut sections = Vec::new();
        let mut overall_ok = true;

        let workspace_section = self.check_workspace(&workspace_dir);
        overall_ok &= workspace_section.ok;
        sections.push(workspace_section);

        let journal_section = self.check_journal(&workspace_dir);
        overall_ok &= journal_section.ok;
        sections.push(journal_section);

        let snapshot_section = self.check_snapshots().await;
        overall_ok &= snapshot_section.ok;
        sections.push(snapshot_section);

        let sandbox_section = self.check_sandbox(&workspace_dir);
        overall_ok &= sandbox_section.ok;
        sections.push(sandbox_section);

        let report = HealthReport {
            overall_ok,
            timestamp: Utc::now(),
            sections: sections.clone(),
        };

        let mut details = HashMap::new();
        details.insert("overall_ok".to_string(), overall_ok.to_string());
        if let Ok(serialized) = serde_json::to_string(&report.sections) {
            details.insert("sections".to_string(), serialized);
        }

        self.journal_append("health_check", &details, None).await?;

        Ok(overall_ok)
    }

    fn format_snapshot_difference(diff: &snapshot::FileDifference) -> String {
        use snapshot::FileDifference;

        match diff {
            FileDifference::Missing { path } => format!("missing {}", path.display()),
            FileDifference::Added { path } => format!("added {}", path.display()),
            FileDifference::Modified { path, .. } => format!("modified {}", path.display()),
            FileDifference::PermissionsChanged { path, .. } => {
                format!("permissions {}", path.display())
            }
            FileDifference::TimestampChanged { path, .. } => {
                format!("timestamp {}", path.display())
            }
        }
    }

    async fn evaluate_path_safety(
        &self,
        changes: &[patch::FileChange],
    ) -> DevItResult<(bool, Vec<String>)> {
        let mut affects_protected = false;
        let mut warnings = Vec::new();
        let path_security = self.path_security.read().await;

        for change in changes {
            if let Err(err) = path_security.validate_patch_path(&change.file_path) {
                affects_protected = true;
                warnings.push(err.to_string());
            }

            if change.is_symlink {
                if let Some(target) = &change.symlink_target {
                    if let Err(err) = path_security.validate_symlink(&change.file_path, target) {
                        affects_protected = true;
                        warnings.push(err.to_string());
                    }
                }
            }
        }

        Ok((affects_protected, warnings))
    }

    fn protected_rule_hits(&self, paths: &[PathBuf]) -> Vec<String> {
        let mut warnings = Vec::new();
        for path in paths {
            for rule in &self.config.policy.protected_paths {
                if Self::path_matches_rule(path, rule) {
                    warnings.push(format!(
                        "Path {} matches protected rule {}",
                        path.display(),
                        rule.display()
                    ));
                }
            }
        }
        warnings
    }

    fn recommended_approval(
        affects_protected: bool,
        affects_binaries: bool,
        estimated_line_changes: usize,
        file_count: usize,
    ) -> ApprovalLevel {
        use patch::RiskLevel;

        let mut score = if affects_protected { 2 } else { 0 };
        if affects_binaries {
            score = score.max(1);
        }
        if estimated_line_changes > 1000 || file_count > 25 {
            score = 3;
        } else if estimated_line_changes > 400 || file_count > 10 {
            score = score.max(1);
        }

        let risk = match score {
            0 => RiskLevel::Low,
            1 => RiskLevel::Medium,
            2 => RiskLevel::High,
            _ => RiskLevel::Critical,
        };

        patch::PatchManager::recommended_approval(risk)
    }

    fn unique_paths(changes: &[patch::FileChange]) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = changes.iter().map(|c| c.file_path.clone()).collect();
        paths.sort();
        paths.dedup();
        paths
    }

    fn infer_policy_rule(reason: &str) -> String {
        let lower = reason.to_lowercase();
        if lower.contains(".env")
            || lower.contains("protected path")
            || lower.contains("chemin prot")
        {
            "policy_protected_path".to_string()
        } else if lower.contains("executable") || lower.contains("permission exécutable") {
            "policy_exec_permission".to_string()
        } else if lower.contains("binary") || lower.contains("binaire") {
            "policy_binary_restriction".to_string()
        } else if lower.contains("gitmodules") {
            "policy_gitmodules".to_string()
        } else if lower.contains("symlink") || lower.contains("lien symbolique") {
            "policy_symlink_restriction".to_string()
        } else {
            "policy_evaluation".to_string()
        }
    }

    fn extract_permission_changes(patch_content: &str) -> Vec<PermissionChange> {
        let mut changes = Vec::new();
        let mut current_path: Option<PathBuf> = None;
        let mut old_mode: Option<u32> = None;

        for line in patch_content.lines() {
            if let Some(path) = Self::extract_diff_path(line) {
                current_path = Some(path);
                old_mode = None;
                continue;
            }

            if let Some(mode) = Self::parse_mode_line(line, "old mode") {
                old_mode = Some(mode);
                continue;
            }

            if let Some(new_mode) = Self::parse_mode_line(line, "new mode") {
                if let (Some(path), Some(previous_mode)) = (current_path.clone(), old_mode.take()) {
                    if previous_mode != new_mode {
                        changes.push(PermissionChange {
                            file_path: path.clone(),
                            current_mode: previous_mode,
                            new_mode,
                            description: format!(
                                "mode change {:o} → {:o}",
                                previous_mode, new_mode
                            ),
                        });
                    }
                }
            }
        }

        changes
    }

    fn extract_diff_path(line: &str) -> Option<PathBuf> {
        if !line.starts_with("diff --git") {
            return None;
        }
        let mut parts = line.split_whitespace();
        let _ = parts.next(); // diff
        let _ = parts.next(); // --git
        let _ = parts.next(); // a/path
        if let Some(b_path) = parts.next() {
            let normalized = b_path.strip_prefix("b/").unwrap_or(b_path);
            return Some(PathBuf::from(normalized));
        }
        None
    }

    fn parse_mode_line(line: &str, prefix: &str) -> Option<u32> {
        if !line.starts_with(prefix) {
            return None;
        }
        line.split_whitespace()
            .nth(2)
            .and_then(|mode| u32::from_str_radix(mode, 8).ok())
    }

    fn path_matches_rule(path: &PathBuf, rule: &PathBuf) -> bool {
        if path == rule {
            return true;
        }
        path.starts_with(rule)
    }

    fn check_workspace(&self, workspace_dir: &PathBuf) -> HealthSectionStatus {
        let probe_dir = workspace_dir.join(".devit");
        let probe_file = probe_dir.join("health_probe.tmp");
        let mut ok = true;
        let mut details = None;

        if let Err(err) = create_dir_all(&probe_dir) {
            ok = false;
            details = Some(format!("Failed to create {}: {}", probe_dir.display(), err));
        } else {
            match OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&probe_file)
            {
                Ok(mut file) => {
                    if let Err(err) = file.write_all(b"ok") {
                        ok = false;
                        details =
                            Some(format!("Failed to write {}: {}", probe_file.display(), err));
                    }
                }
                Err(err) => {
                    ok = false;
                    details = Some(format!("Failed to open {}: {}", probe_file.display(), err));
                }
            }
        }

        if ok {
            let _ = remove_file(&probe_file);
        }

        HealthSectionStatus {
            name: "workspace".to_string(),
            ok,
            details,
        }
    }

    fn check_journal(&self, workspace_dir: &PathBuf) -> HealthSectionStatus {
        let journal_path = workspace_dir.join(".devit/journal.log");
        let mut ok = true;
        let mut details = None;

        if let Some(parent) = journal_path.parent() {
            if let Err(err) = create_dir_all(parent) {
                ok = false;
                details = Some(format!("Failed to prepare {}: {}", parent.display(), err));
            }
        }

        if ok {
            match OpenOptions::new()
                .create(true)
                .append(true)
                .open(&journal_path)
            {
                Ok(file) => {
                    if let Err(err) = file.sync_all() {
                        ok = false;
                        details = Some(format!(
                            "Failed to sync journal {}: {}",
                            journal_path.display(),
                            err
                        ));
                    }
                }
                Err(err) => {
                    ok = false;
                    details = Some(format!(
                        "Failed to open journal {}: {}",
                        journal_path.display(),
                        err
                    ));
                }
            }
        }

        HealthSectionStatus {
            name: "journal".to_string(),
            ok,
            details,
        }
    }

    async fn check_snapshots(&self) -> HealthSectionStatus {
        let result = {
            let manager = self.snapshot_manager.read().await;
            manager.list_snapshots()
        };

        match result {
            Ok(_) => HealthSectionStatus {
                name: "snapshots".to_string(),
                ok: true,
                details: None,
            },
            Err(err) => HealthSectionStatus {
                name: "snapshots".to_string(),
                ok: false,
                details: Some(err.to_string()),
            },
        }
    }

    fn check_sandbox(&self, workspace_dir: &PathBuf) -> HealthSectionStatus {
        if let Some(root) = &self.config.workspace.sandbox_root {
            if workspace_dir.starts_with(root) {
                HealthSectionStatus {
                    name: "sandbox".to_string(),
                    ok: true,
                    details: None,
                }
            } else {
                HealthSectionStatus {
                    name: "sandbox".to_string(),
                    ok: false,
                    details: Some(format!(
                        "Workspace {} is outside sandbox root {}",
                        workspace_dir.display(),
                        root.display()
                    )),
                }
            }
        } else {
            HealthSectionStatus {
                name: "sandbox".to_string(),
                ok: true,
                details: None,
            }
        }
    }

    /// Detect the test framework for the current project
    async fn detect_test_framework(&self) -> DevItResult<String> {
        // Check for Cargo.toml (Rust)
        if tokio::fs::metadata("Cargo.toml").await.is_ok() {
            return Ok("cargo".to_string());
        }

        // Check for package.json (Node.js)
        if tokio::fs::metadata("package.json").await.is_ok() {
            return Ok("npm".to_string());
        }

        // Check for pytest.ini or pyproject.toml (Python)
        if tokio::fs::metadata("pytest.ini").await.is_ok()
            || tokio::fs::metadata("pyproject.toml").await.is_ok()
            || tokio::fs::metadata("setup.py").await.is_ok()
        {
            return Ok("pytest".to_string());
        }

        // Default fallback
        Ok("cargo".to_string())
    }

    /// Build sandbox plan for test execution based on profile
    async fn build_test_sandbox_plan(
        &self,
        profile: &SandboxProfile,
    ) -> DevItResult<sandbox::SandboxPlan> {
        use sandbox::SandboxPlan;

        let current_dir = std::env::current_dir().map_err(|e| DevItError::Internal {
            component: "sandbox".to_string(),
            message: format!("Failed to get current directory: {}", e),
            cause: Some(e.to_string()),
            correlation_id: uuid::Uuid::new_v4().to_string(),
        })?;

        let plan = match profile {
            SandboxProfile::Strict => SandboxPlan {
                bind_ro: vec![
                    PathBuf::from("/usr"),
                    PathBuf::from("/lib"),
                    PathBuf::from("/lib64"),
                    PathBuf::from("/bin"),
                    PathBuf::from("/sbin"),
                ],
                bind_rw: vec![current_dir.clone(), PathBuf::from("/tmp")],
                net: false,
                seccomp_profile: Some("test-strict".to_string()),
            },
            SandboxProfile::Permissive => SandboxPlan {
                bind_ro: vec![
                    PathBuf::from("/usr"),
                    PathBuf::from("/lib"),
                    PathBuf::from("/lib64"),
                    PathBuf::from("/bin"),
                    PathBuf::from("/sbin"),
                ],
                bind_rw: vec![
                    current_dir.clone(),
                    PathBuf::from("/tmp"),
                    PathBuf::from("/home"),
                ],
                net: true,
                seccomp_profile: None,
            },
        };

        Ok(plan)
    }

    /// Build test command based on framework and configuration
    fn build_test_command(&self, framework: &str, config: &TestConfig) -> DevItResult<Vec<String>> {
        let mut command = Vec::new();

        match framework {
            "cargo" => {
                command.push("cargo".to_string());
                command.push("test".to_string());

                // Add patterns if specified
                for pattern in &config.patterns {
                    command.push(pattern.clone());
                }

                // Add parallel flag if disabled
                if !config.parallel {
                    command.push("--".to_string());
                    command.push("--test-threads=1".to_string());
                }
            }
            "npm" => {
                command.push("npm".to_string());
                command.push("test".to_string());

                // Add patterns via environment or npm script args
                if !config.patterns.is_empty() {
                    command.push("--".to_string());
                    for pattern in &config.patterns {
                        command.push(pattern.clone());
                    }
                }
            }
            "pytest" => {
                command.push("pytest".to_string());

                // Add patterns
                for pattern in &config.patterns {
                    command.push(pattern.clone());
                }

                // Add parallel options if enabled
                if config.parallel {
                    command.push("-n".to_string());
                    command.push("auto".to_string()); // Use automatic job detection
                }

                // Add verbose output
                command.push("-v".to_string());
            }
            _ => {
                return Err(DevItError::Internal {
                    component: "test_runner".to_string(),
                    message: format!("Unsupported test framework: {}", framework),
                    cause: None,
                    correlation_id: uuid::Uuid::new_v4().to_string(),
                });
            }
        }

        Ok(command)
    }

    /// Execute command with sandbox and timeout support
    async fn execute_sandboxed_command(
        &self,
        command: &[String],
        sandbox_plan: &sandbox::SandboxPlan,
        timeout_secs: u64,
    ) -> DevItResult<CommandExecutionResult> {
        use tokio::process::Command;
        use tokio::time::{timeout, Duration};

        // Check if bwrap is available for sandboxing
        let use_sandbox = self.check_bwrap_available().await;

        let mut cmd = if use_sandbox {
            self.build_bwrap_command(command, sandbox_plan)?
        } else {
            // Fallback to direct execution
            let mut direct_cmd = Command::new(&command[0]);
            if command.len() > 1 {
                direct_cmd.args(&command[1..]);
            }
            direct_cmd
        };

        // Set up process with stdio capture
        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        // Execute with timeout
        let timeout_duration = Duration::from_secs(timeout_secs);
        let child = cmd.spawn().map_err(|e| DevItError::Internal {
            component: "test_runner".to_string(),
            message: format!("Failed to spawn test command: {}", e),
            cause: Some(e.to_string()),
            correlation_id: uuid::Uuid::new_v4().to_string(),
        })?;

        let output = timeout(timeout_duration, child.wait_with_output())
            .await
            .map_err(|_| DevItError::TestTimeout {
                timeout_secs,
                test_framework: "unknown".to_string(),
                running_tests: Vec::new(),
            })?
            .map_err(|e| DevItError::Internal {
                component: "test_runner".to_string(),
                message: format!("Failed to wait for test completion: {}", e),
                cause: Some(e.to_string()),
                correlation_id: uuid::Uuid::new_v4().to_string(),
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();

        Ok(CommandExecutionResult {
            stdout,
            success: output.status.success(),
        })
    }

    /// Check if bwrap (bubblewrap) is available for sandboxing
    async fn check_bwrap_available(&self) -> bool {
        tokio::process::Command::new("which")
            .arg("bwrap")
            .output()
            .await
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    /// Build bwrap command for sandboxed execution
    fn build_bwrap_command(
        &self,
        command: &[String],
        sandbox_plan: &sandbox::SandboxPlan,
    ) -> DevItResult<tokio::process::Command> {
        let mut bwrap_cmd = tokio::process::Command::new("bwrap");

        // Add basic isolation
        bwrap_cmd.arg("--unshare-all");
        bwrap_cmd.arg("--die-with-parent");

        // Bind mount read-only paths
        for path in &sandbox_plan.bind_ro {
            if path.exists() {
                bwrap_cmd.arg("--ro-bind");
                bwrap_cmd.arg(path);
                bwrap_cmd.arg(path);
            }
        }

        // Bind mount read-write paths
        for path in &sandbox_plan.bind_rw {
            if path.exists() {
                bwrap_cmd.arg("--bind");
                bwrap_cmd.arg(path);
                bwrap_cmd.arg(path);
            }
        }

        // Network configuration
        if sandbox_plan.net {
            bwrap_cmd.arg("--share-net");
        }

        // Add the actual command to execute
        for arg in command {
            bwrap_cmd.arg(arg);
        }

        Ok(bwrap_cmd)
    }

    /// Parse test output and build results
    fn parse_test_output(
        &self,
        framework: &str,
        execution_result: &CommandExecutionResult,
        duration: Duration,
    ) -> DevItResult<TestResults> {
        match framework {
            "cargo" => self.parse_cargo_output(execution_result, duration),
            "npm" => self.parse_npm_output(execution_result, duration),
            "pytest" => self.parse_pytest_output(execution_result, duration),
            _ => {
                // Generic fallback
                Ok(TestResults {
                    success: execution_result.success,
                    total_tests: 0,
                    passed_tests: 0,
                    failed_tests: if execution_result.success { 0 } else { 1 },
                    skipped_tests: 0,
                    execution_time: duration,
                    failure_details: Vec::new(),
                    output: execution_result.stdout.clone(),
                    timed_out: false,
                })
            }
        }
    }

    /// Parse cargo test output
    fn parse_cargo_output(
        &self,
        result: &CommandExecutionResult,
        duration: Duration,
    ) -> DevItResult<TestResults> {
        let output = &result.stdout;
        let mut total_tests = 0;
        let mut passed_tests = 0;
        let mut failed_tests = 0;
        let mut skipped_tests = 0;

        // Parse cargo test output format
        for line in output.lines() {
            if line.contains("test result:") {
                // Example: "test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out"
                let parts: Vec<&str> = line.split_whitespace().collect();
                for (i, part) in parts.iter().enumerate() {
                    if part == &"passed;" && i > 0 {
                        if let Ok(count) = parts[i - 1].parse::<u32>() {
                            passed_tests = count;
                        }
                    } else if part == &"failed;" && i > 0 {
                        if let Ok(count) = parts[i - 1].parse::<u32>() {
                            failed_tests = count;
                        }
                    } else if part == &"ignored;" && i > 0 {
                        if let Ok(count) = parts[i - 1].parse::<u32>() {
                            skipped_tests = count;
                        }
                    }
                }
                total_tests = passed_tests + failed_tests + skipped_tests;
                break;
            }
        }

        Ok(TestResults {
            success: result.success,
            total_tests,
            passed_tests,
            failed_tests,
            skipped_tests,
            execution_time: duration,
            failure_details: Vec::new(),
            output: result.stdout.clone(),
            timed_out: false,
        })
    }

    /// Parse npm test output
    fn parse_npm_output(
        &self,
        result: &CommandExecutionResult,
        duration: Duration,
    ) -> DevItResult<TestResults> {
        Ok(TestResults {
            success: result.success,
            total_tests: 0, // NPM output parsing is framework-dependent
            passed_tests: if result.success { 1 } else { 0 },
            failed_tests: if result.success { 0 } else { 1 },
            skipped_tests: 0,
            execution_time: duration,
            failure_details: Vec::new(),
            output: result.stdout.clone(),
            timed_out: false,
        })
    }

    /// Parse pytest output
    fn parse_pytest_output(
        &self,
        result: &CommandExecutionResult,
        duration: Duration,
    ) -> DevItResult<TestResults> {
        let output = &result.stdout;
        let mut total_tests = 0;
        let mut passed_tests = 0;
        let mut failed_tests = 0;
        let skipped_tests = 0;

        // Parse pytest output format
        for line in output.lines() {
            if line.contains("=")
                && (line.contains("passed") || line.contains("failed") || line.contains("error"))
            {
                // Example: "======= 2 passed, 1 failed in 0.12s ======="
                if line.contains("passed") {
                    if let Some(count_str) = line
                        .split_whitespace()
                        .find(|s| s.chars().all(|c| c.is_ascii_digit()))
                    {
                        if let Ok(count) = count_str.parse::<u32>() {
                            passed_tests = count;
                        }
                    }
                }
                if line.contains("failed") {
                    for word in line.split_whitespace() {
                        if word.chars().all(|c| c.is_ascii_digit()) {
                            if let Ok(count) = word.parse::<u32>() {
                                failed_tests = count;
                                break;
                            }
                        }
                    }
                }
                total_tests = passed_tests + failed_tests + skipped_tests;
                break;
            }
        }

        Ok(TestResults {
            success: result.success,
            total_tests,
            passed_tests,
            failed_tests,
            skipped_tests,
            execution_time: duration,
            failure_details: Vec::new(),
            output: result.stdout.clone(),
            timed_out: false,
        })
    }

    /// Converts a TestRunRequest to a TestConfig for internal use
    fn convert_test_run_request_to_config(
        &self,
        request: &tester::TestRunRequest,
    ) -> DevItResult<TestConfig> {
        let framework = request.stack.as_ref().map(|stack| stack.to_string());

        let timeout_secs = request.timeout_s.unwrap_or(300);

        // Extract command patterns if provided
        let patterns: Vec<String> = if let Some(ref command) = request.command {
            // Parse command into patterns - this is a simplified approach
            command
                .split_whitespace()
                .skip(2) // Skip "cargo test" or equivalent
                .map(|s| s.to_string())
                .collect()
        } else {
            Vec::new()
        };

        Ok(TestConfig {
            framework,
            patterns,
            timeout_secs,
            parallel: true, // Default to parallel
            env_vars: std::collections::HashMap::new(),
        })
    }

    /// Determines if auto-revert should be triggered based on test results and policy
    async fn should_auto_revert(
        &self,
        test_results: &TestResults,
        approval_level: &ApprovalLevel,
    ) -> DevItResult<bool> {
        // Only revert if tests failed
        if test_results.success {
            return Ok(false);
        }

        // Check policy configuration
        let policy_config = &self.config.policy;
        if !policy_config.auto_revert_on_test_fail() {
            return Ok(false);
        }

        // Only allow auto-revert for moderate and trusted approval levels
        // Ask level requires human confirmation for revert decisions
        match approval_level {
            ApprovalLevel::Moderate | ApprovalLevel::Trusted => Ok(true),
            ApprovalLevel::Ask | ApprovalLevel::Untrusted => Ok(false),
            ApprovalLevel::Privileged { .. } => Ok(true), // Privileged can auto-revert
        }
    }

    /// Performs the actual auto-revert operation
    async fn perform_auto_revert(&self, patch_result: &PatchResult) -> DevItResult<PatchResult> {
        use std::process::Command;

        if let Some(ref rollback_cmd) = patch_result.rollback_cmd {
            // Parse and execute the rollback command
            let cmd_parts: Vec<&str> = rollback_cmd.split_whitespace().collect();
            if cmd_parts.is_empty() {
                return Err(DevItError::Internal {
                    component: "auto_revert".to_string(),
                    message: "Empty rollback command".to_string(),
                    cause: None,
                    correlation_id: uuid::Uuid::new_v4().to_string(),
                });
            }

            let output = Command::new(cmd_parts[0])
                .args(&cmd_parts[1..])
                .output()
                .map_err(|e| DevItError::Internal {
                    component: "auto_revert".to_string(),
                    message: format!("Failed to execute rollback command: {}", e),
                    cause: Some(e.to_string()),
                    correlation_id: uuid::Uuid::new_v4().to_string(),
                })?;

            if !output.status.success() {
                return Err(DevItError::Internal {
                    component: "auto_revert".to_string(),
                    message: format!(
                        "Rollback command failed with exit code: {:?}",
                        output.status.code()
                    ),
                    cause: Some(String::from_utf8_lossy(&output.stderr).to_string()),
                    correlation_id: uuid::Uuid::new_v4().to_string(),
                });
            }

            // Extract commit SHA from output if possible
            let output_str = String::from_utf8_lossy(&output.stdout);
            let commit_sha = if output_str.contains("revert") {
                // Extract commit SHA from git revert output
                output_str
                    .lines()
                    .find(|line| line.contains("revert"))
                    .and_then(|line| line.split_whitespace().last())
                    .map(|sha| sha.to_string())
            } else {
                None
            };

            Ok(PatchResult {
                success: true,
                modified_files: Vec::new(),
                warnings: Vec::new(),
                info_messages: vec![format!("Auto-revert executed: {}", rollback_cmd)],
                resulting_snapshot: None,
                execution_time: std::time::Duration::from_millis(1),
                required_elevation: false,
                commit_sha,
                rollback_cmd: None, // No further rollback needed
                test_results: None,
                auto_reverted: false, // This is the revert operation itself
                reverted_sha: None,
            })
        } else {
            Err(DevItError::Internal {
                component: "auto_revert".to_string(),
                message: "No rollback command available for revert".to_string(),
                cause: None,
                correlation_id: uuid::Uuid::new_v4().to_string(),
            })
        }
    }

    /// Logs the revert operation to the journal
    async fn log_revert_operation(
        &self,
        patch_result: &PatchResult,
        test_results: &TestResults,
    ) -> DevItResult<()> {
        let mut details = std::collections::HashMap::new();
        details.insert("reason".to_string(), "test_failure".to_string());
        details.insert(
            "original_commit".to_string(),
            patch_result
                .commit_sha
                .clone()
                .unwrap_or_else(|| "unknown".to_string()),
        );
        details.insert(
            "revert_commit".to_string(),
            patch_result
                .reverted_sha
                .clone()
                .unwrap_or_else(|| "unknown".to_string()),
        );
        details.insert(
            "failed_tests".to_string(),
            test_results.failed_tests.to_string(),
        );
        details.insert(
            "total_tests".to_string(),
            test_results.total_tests.to_string(),
        );

        self.journal_append("auto_revert", &details, None).await?;
        Ok(())
    }

    /// Read file content with security validation and optional line numbers
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the file to read
    /// * `line_numbers` - Whether to include line numbers in output
    /// * `offset` - Optional starting line offset
    /// * `limit` - Optional maximum number of lines to read
    ///
    /// # Returns
    ///
    /// File content with metadata and optional line numbers
    pub async fn file_read<P: AsRef<std::path::Path>>(
        &self,
        path: P,
        line_numbers: bool,
        offset: Option<usize>,
        limit: Option<usize>,
    ) -> DevItResult<file_ops::FileContent> {
        let file_ops = self.file_ops.read().await;
        file_ops.file_read(path, line_numbers, offset, limit).await
    }

    /// List files and directories with metadata and optional recursion
    ///
    /// # Arguments
    ///
    /// * `path` - Path to list (file or directory)
    /// * `recursive` - Whether to list recursively
    ///
    /// # Returns
    ///
    /// Vector of file entries with metadata
    pub async fn file_list<P: AsRef<std::path::Path>>(
        &self,
        path: P,
        recursive: bool,
    ) -> DevItResult<Vec<file_ops::FileEntry>> {
        let file_ops = self.file_ops.read().await;
        file_ops.file_list(path, recursive).await
    }

    /// Search for pattern in files with context lines
    ///
    /// # Arguments
    ///
    /// * `pattern` - Regular expression pattern to search for
    /// * `path` - Path to search (file or directory)
    /// * `context_lines` - Number of context lines to include around matches
    ///
    /// # Returns
    ///
    /// Search results with matches and metadata
    pub async fn file_search<P: AsRef<std::path::Path>>(
        &self,
        pattern: &str,
        path: P,
        context_lines: Option<usize>,
    ) -> DevItResult<file_ops::SearchResults> {
        let file_ops = self.file_ops.read().await;
        file_ops.file_search(pattern, path, context_lines).await
    }

    /// Generate project structure tree view
    ///
    /// # Arguments
    ///
    /// * `path` - Path to analyze (directory)
    /// * `max_depth` - Maximum tree depth to traverse
    ///
    /// # Returns
    ///
    /// Project structure with tree view and metadata
    pub async fn project_structure<P: AsRef<std::path::Path>>(
        &self,
        path: P,
        max_depth: Option<u8>,
    ) -> DevItResult<file_ops::ProjectStructure> {
        let file_ops = self.file_ops.read().await;
        file_ops.project_structure(path, max_depth).await
    }

    /// Get the current working directory (auto-detected project root)
    pub async fn get_working_directory(&self) -> DevItResult<std::path::PathBuf> {
        let file_ops = self.file_ops.read().await;
        Ok(file_ops.get_root_path().to_path_buf())
    }

    // Extended methods with compression and advanced options

    /// Read file content with compression and filtering options
    pub async fn file_read_ext<P: AsRef<std::path::Path>>(
        &self,
        path: P,
        format: &formats::OutputFormat,
        fields: Option<&[String]>,
        line_numbers: Option<bool>,
        offset: Option<u32>,
        limit: Option<u32>,
    ) -> DevItResult<String> {
        // Get file content using existing method
        let file_content = self
            .file_read(
                path,
                line_numbers.unwrap_or(false),
                offset.map(|o| o as usize),
                limit.map(|l| l as usize),
            )
            .await?;

        // Filter fields if requested
        let filtered_content = if let Some(field_list) = fields {
            self.filter_file_content_fields(&file_content, field_list, line_numbers)?
        } else {
            file_content
        };

        // Apply compression format
        filtered_content.to_format(format)
    }

    /// List directory contents with compression and advanced options
    pub async fn file_list_ext<P: AsRef<std::path::Path>>(
        &self,
        path: P,
        format: &formats::OutputFormat,
        fields: Option<&[String]>,
        recursive: Option<bool>,
        _include_hidden: Option<bool>,
        _include_patterns: Option<&[String]>,
        _exclude_patterns: Option<&[String]>,
    ) -> DevItResult<String> {
        // Get file list using existing method
        let file_list = self.file_list(path, recursive.unwrap_or(false)).await?;

        // Filter fields if requested
        let filtered_list = if let Some(field_list) = fields {
            self.filter_file_list_fields(&file_list, field_list)?
        } else {
            file_list
        };

        // Apply compression format
        filtered_list.to_format(format)
    }

    /// Search files with compression and advanced options
    pub async fn file_search_ext<P: AsRef<std::path::Path>>(
        &self,
        pattern: &str,
        path: P,
        format: &formats::OutputFormat,
        fields: Option<&[String]>,
        context_lines: Option<u8>,
        _file_pattern: Option<&str>,
        _max_results: Option<usize>,
    ) -> DevItResult<String> {
        // Get search results using existing method
        let search_results = self
            .file_search(pattern, path, context_lines.map(|cl| cl as usize))
            .await?;

        // Filter fields if requested
        let filtered_results = if let Some(field_list) = fields {
            self.filter_search_results_fields(&search_results, field_list)?
        } else {
            search_results
        };

        // Apply compression format
        filtered_results.to_format(format)
    }

    /// Get project structure with compression and advanced options
    pub async fn project_structure_ext<P: AsRef<std::path::Path>>(
        &self,
        path: P,
        format: &formats::OutputFormat,
        fields: Option<&[String]>,
        max_depth: Option<u8>,
    ) -> DevItResult<String> {
        // Get project structure using existing method
        let project_structure = self.project_structure(path, max_depth).await?;

        // Filter fields if requested (for future use)
        let filtered_structure = if let Some(_field_list) = fields {
            // For now, project structure doesn't support field filtering
            // but we keep the parameter for consistency and future extension
            project_structure
        } else {
            project_structure
        };

        // Apply compression format
        filtered_structure.to_format(format)
    }

    // Helper methods for field filtering

    /// Filter file content fields based on requested field list
    fn filter_file_content_fields(
        &self,
        content: &file_ops::FileContent,
        fields: &[String],
        include_line_numbers: Option<bool>,
    ) -> DevItResult<file_ops::FileContent> {
        let mut filtered = file_ops::FileContent {
            path: content.path.clone(),
            content: String::new(),
            size: 0,
            lines: None,
            encoding: String::new(),
        };

        for field in fields {
            match field.as_str() {
                "path" => filtered.path = content.path.clone(),
                "content" => filtered.content = content.content.clone(),
                "size" => filtered.size = content.size,
                "lines" => {
                    if include_line_numbers.unwrap_or(false) {
                        filtered.lines = content.lines.clone();
                    }
                }
                "encoding" => filtered.encoding = content.encoding.clone(),
                _ => {
                    return Err(DevItError::InvalidFormat {
                        format: field.clone(),
                        supported: vec![
                            "path".to_string(),
                            "content".to_string(),
                            "size".to_string(),
                            "lines".to_string(),
                            "encoding".to_string(),
                        ],
                    });
                }
            }
        }

        // Set default values for required fields not specified
        if !fields.contains(&"path".to_string()) {
            filtered.path = content.path.clone();
        }
        if !fields.contains(&"content".to_string()) {
            filtered.content = content.content.clone();
        }
        if !fields.contains(&"size".to_string()) {
            filtered.size = content.size;
        }
        if !fields.contains(&"encoding".to_string()) {
            filtered.encoding = content.encoding.clone();
        }

        Ok(filtered)
    }

    /// Filter file list fields based on requested field list
    fn filter_file_list_fields(
        &self,
        list: &[file_ops::FileEntry],
        fields: &[String],
    ) -> DevItResult<Vec<file_ops::FileEntry>> {
        let mut filtered_list = Vec::new();

        for entry in list {
            let mut filtered_entry = file_ops::FileEntry {
                name: String::new(),
                path: std::path::PathBuf::new(),
                entry_type: file_ops::FileType::File,
                size: None,
                modified: None,
                permissions: file_ops::FilePermissions {
                    readable: false,
                    writable: false,
                    executable: false,
                },
            };

            for field in fields {
                match field.as_str() {
                    "name" => filtered_entry.name = entry.name.clone(),
                    "path" => filtered_entry.path = entry.path.clone(),
                    "type" | "entry_type" => filtered_entry.entry_type = entry.entry_type.clone(),
                    "size" => filtered_entry.size = entry.size,
                    "modified" => filtered_entry.modified = entry.modified,
                    "permissions" => filtered_entry.permissions = entry.permissions.clone(),
                    _ => {
                        return Err(DevItError::InvalidFormat {
                            format: field.clone(),
                            supported: vec![
                                "name".to_string(),
                                "path".to_string(),
                                "type".to_string(),
                                "size".to_string(),
                                "modified".to_string(),
                                "permissions".to_string(),
                            ],
                        });
                    }
                }
            }

            // Set default values for essential fields not specified
            if !fields.contains(&"name".to_string()) {
                filtered_entry.name = entry.name.clone();
            }
            if !fields.contains(&"path".to_string()) {
                filtered_entry.path = entry.path.clone();
            }
            if !fields.contains(&"type".to_string()) && !fields.contains(&"entry_type".to_string())
            {
                filtered_entry.entry_type = entry.entry_type.clone();
            }

            filtered_list.push(filtered_entry);
        }

        Ok(filtered_list)
    }

    /// Filter search results fields based on requested field list
    fn filter_search_results_fields(
        &self,
        results: &file_ops::SearchResults,
        fields: &[String],
    ) -> DevItResult<file_ops::SearchResults> {
        let mut filtered = file_ops::SearchResults {
            pattern: String::new(),
            path: std::path::PathBuf::new(),
            files_searched: 0,
            total_matches: 0,
            matches: Vec::new(),
            truncated: false,
        };

        for field in fields {
            match field.as_str() {
                "pattern" => filtered.pattern = results.pattern.clone(),
                "path" => filtered.path = results.path.clone(),
                "files_searched" => filtered.files_searched = results.files_searched,
                "total_matches" => filtered.total_matches = results.total_matches,
                "matches" => filtered.matches = results.matches.clone(),
                "truncated" => filtered.truncated = results.truncated,
                _ => {
                    return Err(DevItError::InvalidFormat {
                        format: field.clone(),
                        supported: vec![
                            "pattern".to_string(),
                            "path".to_string(),
                            "files_searched".to_string(),
                            "total_matches".to_string(),
                            "matches".to_string(),
                            "truncated".to_string(),
                        ],
                    });
                }
            }
        }

        // Set default values for essential fields not specified
        if !fields.contains(&"pattern".to_string()) {
            filtered.pattern = results.pattern.clone();
        }
        if !fields.contains(&"path".to_string()) {
            filtered.path = results.path.clone();
        }
        if !fields.contains(&"matches".to_string()) {
            filtered.matches = results.matches.clone();
        }

        Ok(filtered)
    }

    /// NOUVELLE méthode : Délégation de tâche (réutilise Agent existant)
    pub async fn orchestration_delegate(
        &self,
        goal: String,
        delegated_to: String,
        model: Option<String>,
        timeout: Option<Duration>,
        watch_patterns: Option<Vec<String>>,
        context: Option<serde_json::Value>,
        working_dir: Option<String>,
        response_format: Option<String>,
    ) -> DevItResult<String> {
        // 1. RÉUTILISER le journal existant pour audit
        let mut details = std::collections::HashMap::new();
        details.insert("goal".to_string(), goal.clone());
        details.insert("delegated_to".to_string(), delegated_to.clone());
        details.insert(
            "timeout_secs".to_string(),
            timeout.map(|t| t.as_secs().to_string()).unwrap_or_default(),
        );

        let requested_dir = working_dir.as_deref();
        let resolved_workdir = {
            let workspace = self.workspace.read().await;
            let resolved = if let Some(dir) = requested_dir {
                Some(workspace.resolve_relative_from_root(dir).map_err(|err| {
                    DevItError::Internal {
                        component: "workspace".to_string(),
                        message: err.to_string(),
                        cause: None,
                        correlation_id: Uuid::new_v4().to_string(),
                    }
                })?)
            } else {
                let current = workspace.current_relative();
                if current.components().next().is_none() || current == PathBuf::from(".") {
                    None
                } else {
                    Some(current)
                }
            };
            resolved
        };

        if let Some(dir) = resolved_workdir.as_ref() {
            details.insert(
                "working_dir".to_string(),
                dir.to_string_lossy().replace('\\', "/"),
            );
        }
        if let Some(format) = response_format.as_ref() {
            details.insert("response_format".to_string(), format.clone());
        }
        if let Some(model) = model.as_ref() {
            details.insert("model".to_string(), model.clone());
        }

        self.journal_append("orchestration_delegate", &details, None)
            .await?;

        // 2. Créer la tâche via orchestration manager
        let timeout = timeout
            .unwrap_or_else(|| Duration::from_secs(self.config.orchestration.default_timeout_secs));

        let orchestration = self.orchestration.write().await;
        let task_id = orchestration
            .create_task(
                goal,
                delegated_to,
                model,
                timeout,
                watch_patterns.unwrap_or(self.config.orchestration.default_watch_patterns.clone()),
                context,
                resolved_workdir,
                response_format,
            )
            .await?;

        Ok(task_id)
    }

    /// Returns true when the orchestration backend is the daemon backend.
    pub async fn orchestration_uses_daemon(&self) -> bool {
        let orchestration = self.orchestration.read().await;
        orchestration.is_using_daemon()
    }

    /// NOUVELLE méthode : Réception de notification
    pub async fn orchestration_notify(
        &self,
        task_id: String,
        status: String,
        summary: String,
        details: Option<serde_json::Value>,
        evidence: Option<serde_json::Value>,
    ) -> DevItResult<()> {
        // RÉUTILISER le journal pour l'audit
        let mut journal_details = std::collections::HashMap::new();
        journal_details.insert("task_id".to_string(), task_id.clone());
        journal_details.insert("status".to_string(), status.clone());
        journal_details.insert("summary".to_string(), summary.clone());

        self.journal_append("orchestration_notify", &journal_details, None)
            .await?;

        let orchestration = self.orchestration.write().await;
        orchestration
            .receive_notification(task_id, status, summary, details, evidence)
            .await
    }

    /// NOUVELLE méthode : Status de l'orchestration
    pub async fn orchestration_status(
        &self,
        format: &crate::core::formats::OutputFormat,
        filter: Option<String>,
    ) -> DevItResult<String> {
        let orchestration = self.orchestration.read().await;
        let status = orchestration.get_status(filter.as_deref()).await?;

        let status_format = match format {
            crate::core::formats::OutputFormat::Json => StatusFormat::Json,
            crate::core::formats::OutputFormat::Compact => StatusFormat::Compact,
            crate::core::formats::OutputFormat::Table => StatusFormat::Table,
            crate::core::formats::OutputFormat::MessagePack => {
                return Err(DevItError::InvalidFormat {
                    format: "messagepack".to_string(),
                    supported: vec![
                        "json".to_string(),
                        "compact".to_string(),
                        "table".to_string(),
                    ],
                });
            }
        };

        format_status(&status, status_format)
    }

    /// Change the secure workspace directory and update dependent subsystems.
    pub async fn workspace_change_dir(&self, path: &str) -> DevItResult<PathBuf> {
        let allow_internal_symlinks =
            self.config.policy.default_approval_level != ApprovalLevel::Untrusted;

        let absolute_path = {
            let mut workspace = self.workspace.write().await;
            workspace.change_dir(path).map_err(Self::workspace_error)?
        };

        {
            let mut patch_manager = self.patch_manager.write().await;
            patch_manager.set_working_directory(absolute_path.clone());
        }

        {
            let mut snapshot_manager = self.snapshot_manager.write().await;
            snapshot_manager.set_snapshot_dir(absolute_path.clone());
        }

        {
            let mut file_ops = self.file_ops.write().await;
            file_ops
                .set_root_path(absolute_path.clone())
                .map_err(|err| DevItError::Internal {
                    component: "workspace".to_string(),
                    message: err.to_string(),
                    cause: None,
                    correlation_id: Uuid::new_v4().to_string(),
                })?;
        }

        {
            let mut path_security = self.path_security.write().await;
            *path_security = PathSecurityContext::new(&absolute_path, allow_internal_symlinks)?;
        }

        Ok(absolute_path)
    }

    /// Current workspace directory (absolute, canonicalized).
    pub async fn workspace_current_dir(&self) -> DevItResult<PathBuf> {
        let workspace = self.workspace.read().await;
        Ok(workspace.current_dir())
    }

    /// Current workspace directory relative to sandbox root.
    pub async fn workspace_current_relative(&self) -> DevItResult<PathBuf> {
        let workspace = self.workspace.read().await;
        Ok(workspace.current_relative())
    }

    /// Resolve a path within the sandbox without changing state.
    pub async fn workspace_resolve_path(&self, path: &str) -> DevItResult<PathBuf> {
        let workspace = self.workspace.read().await;
        workspace
            .resolve_path(path)
            .map_err(|err| Self::workspace_error(err))
    }

    /// Resolve a sandbox-relative path into a normalized relative path.
    pub async fn workspace_resolve_relative(&self, path: &str) -> DevItResult<PathBuf> {
        let workspace = self.workspace.read().await;
        workspace
            .resolve_relative_from_root(path)
            .map_err(|err| Self::workspace_error(err))
    }

    fn workspace_error(err: anyhow::Error) -> DevItError {
        DevItError::Internal {
            component: "workspace".to_string(),
            message: err.to_string(),
            cause: None,
            correlation_id: Uuid::new_v4().to_string(),
        }
    }
}

/// Detailed analysis of patch contents before application.
///
/// Enables informed decision-making by showing what changes would be made
/// and identifying potential security or policy concerns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchPreview {
    /// Files that would be affected by applying the patch
    pub affected_files: Vec<PathBuf>,

    /// Estimated number of lines that would be changed
    pub estimated_line_changes: usize,

    /// Whether the patch would affect any protected paths
    pub affects_protected: bool,

    /// Whether binary files would be modified
    pub affects_binaries: bool,

    /// Security policy warnings about the proposed changes
    pub policy_warnings: Vec<String>,

    /// Recommended approval level for safely applying the patch
    pub recommended_approval: ApprovalLevel,

    /// File permission changes that would be applied
    pub permission_changes: Vec<PermissionChange>,
}

/// Comprehensive result information from patch application operations.
///
/// Provides detailed feedback about what changes were made, any issues
/// encountered, and recommendations for follow-up actions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchResult {
    /// Whether the patch application completed successfully
    pub success: bool,

    /// List of files that were modified during application
    pub modified_files: Vec<PathBuf>,

    /// Warning messages about potential issues or policy violations
    pub warnings: Vec<String>,

    /// Informational messages about the application process
    pub info_messages: Vec<String>,

    /// Snapshot ID capturing the state after patch application
    pub resulting_snapshot: Option<SnapshotId>,

    /// Time taken to complete the operation
    pub execution_time: Duration,

    /// Whether any files required elevated permissions
    pub required_elevation: bool,

    /// Commit SHA if a commit was created (optional)
    pub commit_sha: Option<String>,

    /// Rollback command for recovery (not executed here)
    pub rollback_cmd: Option<String>,

    /// Test results if post-tests were executed
    pub test_results: Option<TestResults>,

    /// Whether auto-revert was triggered due to test failure
    pub auto_reverted: bool,

    /// SHA of the revert commit if auto-revert was performed
    pub reverted_sha: Option<String>,
}

/// Lightweight test orchestration request types (legacy compatibility).
pub mod tester {
    use serde::{Deserialize, Serialize};
    use std::fmt;

    /// Supported test execution stacks.
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub enum Stack {
        /// Rust's `cargo test`.
        Cargo,
        /// Python's `pytest`.
        PyTest,
        /// Node's `npm test`.
        Npm,
    }

    impl Stack {
        /// Canonical lower-case identifier.
        pub fn as_str(&self) -> &'static str {
            match self {
                Stack::Cargo => "cargo",
                Stack::PyTest => "pytest",
                Stack::Npm => "npm",
            }
        }
    }

    impl fmt::Display for Stack {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(self.as_str())
        }
    }

    /// Minimal request payload used by the patch workflow to trigger tests.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct TestRunRequest {
        pub stack: Option<Stack>,
        pub command: Option<String>,
        pub timeout_s: Option<u64>,
        pub cpu_limit: Option<u32>,
        pub mem_limit_mb: Option<u32>,
    }
}

/// Description of a file permission change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionChange {
    /// Path to the file whose permissions would change
    pub file_path: PathBuf,

    /// Current permission mode (octal)
    pub current_mode: u32,

    /// New permission mode after change (octal)
    pub new_mode: u32,

    /// Human-readable description of the change
    pub description: String,
}

/// Status for an individual subsystem health check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthSectionStatus {
    /// Human-readable name of the subsystem
    pub name: String,
    /// Whether the subsystem passed the health check
    pub ok: bool,
    /// Optional diagnostic message when the check fails
    pub details: Option<String>,
}

/// Aggregated health report emitted by [`Core::health_check`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthReport {
    /// Overall health result
    pub overall_ok: bool,
    /// Timestamp when the health check ran
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Individual subsystem statuses
    pub sections: Vec<HealthSectionStatus>,
}

/// Configuration parameters for test execution.
///
/// Allows customization of test behavior, timeout handling, and execution
/// environment to match project requirements.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestConfig {
    /// Test framework to use (e.g., "cargo", "npm", "pytest")
    pub framework: Option<String>,

    /// Test patterns or filters to apply
    pub patterns: Vec<String>,

    /// Maximum time to allow for test execution
    pub timeout_secs: u64,

    /// Whether to run tests in parallel
    pub parallel: bool,

    /// Environment variables to set during test execution
    pub env_vars: HashMap<String, String>,
}

/// Results from test execution with detailed metrics.
///
/// Provides comprehensive information about test outcomes, performance,
/// and any issues encountered during execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResults {
    /// Whether all tests passed successfully
    pub success: bool,

    /// Total number of tests executed
    pub total_tests: u32,

    /// Number of tests that passed
    pub passed_tests: u32,

    /// Number of tests that failed
    pub failed_tests: u32,

    /// Number of tests that were skipped
    pub skipped_tests: u32,

    /// Time taken for test execution
    pub execution_time: Duration,

    /// Detailed failure information for failed tests
    pub failure_details: Vec<TestFailure>,

    /// Framework-specific output or logs
    pub output: String,

    /// Whether execution was terminated due to timeout
    pub timed_out: bool,
}

/// Information about a specific test failure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestFailure {
    /// Name or identifier of the failed test
    pub test_name: String,

    /// Error message or failure reason
    pub error_message: String,

    /// Stack trace or detailed diagnostic information
    pub details: Option<String>,

    /// File and line number where the failure occurred
    pub location: Option<String>,
}

// Default implementations for convenience
impl Default for TestConfig {
    fn default() -> Self {
        TestConfig {
            framework: None,
            patterns: vec!["test".to_string()],
            timeout_secs: 300,
            parallel: true,
            env_vars: HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use devit_common::ApprovalLevel;

    // Note: Patch apply tests are disabled due to Git format validation issues
    // The idempotency logic is tested via journal_append tests below

    #[tokio::test]
    async fn test_journal_append_idempotency_same_key_returns_same_response() {
        let config = CoreConfig::default();
        let core = CoreEngine::new(config).await.unwrap();

        let operation = "test_operation";
        let mut details = HashMap::new();
        details.insert("action".to_string(), "test".to_string());
        let idempotency_key = "journal-key-456";

        // First call
        let result1 = core
            .journal_append(operation, &details, Some(idempotency_key))
            .await
            .unwrap();

        // Second call with same idempotency key should return identical result
        let result2 = core
            .journal_append(operation, &details, Some(idempotency_key))
            .await
            .unwrap();

        // Results should be identical
        assert_eq!(result1.hmac, result2.hmac);
        assert_eq!(result1.offset, result2.offset);
        assert_eq!(result1.file, result2.file);
        assert_eq!(result1.request_id, result2.request_id);
    }

    #[tokio::test]
    async fn test_idempotency_cache_expires() {
        let config = CoreConfig::default();
        let core = CoreEngine::new(config).await.unwrap();

        let operation = "test_operation";
        let mut details = HashMap::new();
        details.insert("action".to_string(), "test".to_string());
        let idempotency_key = "expiry-test-key";

        // Manually insert an entry with very short TTL
        let cached_request_id = Uuid::new_v4();
        {
            let mut cache = core.idempotency_cache.write().await;
            use std::path::PathBuf;
            let cached_response = journal::JournalResponse {
                hmac: "cached".to_string(),
                offset: 0,
                file: PathBuf::from("journal.jsonl"),
                request_id: cached_request_id,
            };
            let serialized = serde_json::to_string(&cached_response).unwrap();
            cache.insert(
                idempotency_key.to_string(),
                serialized,
                Duration::from_millis(1), // Very short TTL
            );
        }

        // Wait for expiry
        tokio::time::sleep(Duration::from_millis(5)).await;

        // Call should not use expired cache entry
        let result = core
            .journal_append(operation, &details, Some(idempotency_key))
            .await
            .unwrap();

        // Should return fresh result, not cached one
        assert_ne!(result.request_id, cached_request_id);
    }

    #[tokio::test]
    async fn test_idempotency_without_key_generates_fresh_results() {
        let config = CoreConfig::default();
        let core = CoreEngine::new(config).await.unwrap();

        let operation = "test_operation";
        let mut details = HashMap::new();
        details.insert("action".to_string(), "test".to_string());

        // Two calls without idempotency key should be independent
        let result1 = core
            .journal_append(operation, &details, None)
            .await
            .unwrap();

        let result2 = core
            .journal_append(operation, &details, None)
            .await
            .unwrap();

        // Each call should generate its own request ID (should be different)
        assert_ne!(result1.request_id, result2.request_id);
    }

    #[tokio::test]
    async fn patch_preview_basic_statistics() {
        let temp = tempfile::tempdir().unwrap();
        let mut config = CoreConfig::default();
        config.workspace.sandbox_root = Some(temp.path().to_path_buf());
        let core = CoreEngine::new(config).await.unwrap();

        let patch = "\
diff --git a/example.txt b/example.txt
new file mode 100644
index 0000000..1111111
--- /dev/null
+++ b/example.txt
@@ -0,0 +1 @@
+hello world
";

        let preview = core.patch_preview(patch, None).await.unwrap();

        assert_eq!(preview.affected_files, vec![PathBuf::from("example.txt")]);
        assert_eq!(preview.estimated_line_changes, 1);
        assert!(!preview.affects_protected);
        assert_eq!(preview.recommended_approval, ApprovalLevel::Untrusted);
        assert!(preview.policy_warnings.is_empty());
    }

    #[tokio::test]
    async fn patch_preview_detects_protected_paths() {
        let temp = tempfile::tempdir().unwrap();
        let mut config = CoreConfig::default();
        config.workspace.sandbox_root = Some(temp.path().to_path_buf());
        let core = CoreEngine::new(config).await.unwrap();

        let patch = "\
diff --git a/.env b/.env
new file mode 100644
index 0000000..1111111
--- /dev/null
+++ b/.env
@@ -0,0 +1 @@
+SECRET=1
";

        let preview = core.patch_preview(patch, None).await.unwrap();

        assert!(preview.affects_protected);
        assert!(!preview.policy_warnings.is_empty());
        assert!(
            preview.policy_warnings.iter().any(|w| w.contains(".env")),
            "expected warning mentioning .env, got {:?}",
            preview.policy_warnings
        );
    }

    #[tokio::test]
    async fn patch_apply_reports_modified_files() {
        let temp = tempfile::tempdir().unwrap();
        let mut config = CoreConfig::default();
        config.workspace.sandbox_root = Some(temp.path().to_path_buf());
        let core = CoreEngine::new(config).await.unwrap();

        let workspace_dir = core.workspace_current_dir().await.unwrap();
        std::fs::write(workspace_dir.join("example.txt"), "hello\n").unwrap();

        let patch = "\
diff --git a/example.txt b/example.txt
--- a/example.txt
+++ b/example.txt
@@ -1 +1,2 @@
-hello
+hello
+world
";

        let result = core
            .patch_apply(patch, ApprovalLevel::Trusted, false, None)
            .await
            .unwrap();

        assert_eq!(result.modified_files, vec![PathBuf::from("example.txt")]);
        assert!(
            result
                .info_messages
                .iter()
                .any(|msg| msg.contains("Successfully applied patch")),
            "expected success message in info messages"
        );
    }

    #[tokio::test]
    async fn health_check_succeeds_on_fresh_workspace() {
        let temp = tempfile::tempdir().unwrap();
        let mut config = CoreConfig::default();
        config.workspace.sandbox_root = Some(temp.path().to_path_buf());
        let core = CoreEngine::new(config).await.unwrap();

        let healthy = core.health_check().await.unwrap();
        assert!(healthy);
    }
}
/// Result of executing a test command with sandboxing support.
#[derive(Debug, Clone)]
struct CommandExecutionResult {
    /// Standard output captured from the command
    pub stdout: String,
    /// Whether the command execution completed successfully
    pub success: bool,
}
