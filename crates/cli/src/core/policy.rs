//! # DevIt Policy Management
//!
//! Policy enforcement system for approval levels and sandbox profiles.
//! Controls access permissions and security boundaries for operations.
//!
//! ## Architecture
//!
//! The policy system provides comprehensive security enforcement:
//!
//! - **Approval Workflows**: Multi-level approval system with escalation
//! - **Path Protection**: Configure protected files and directories
//! - **Sandbox Profiles**: Define execution isolation boundaries
//! - **Security Analysis**: Detect dangerous operations and patterns
//! - **Custom Rules**: Define organization-specific policies
//!
//! ## Security Model
//!
//! Policies are evaluated in order of strictness:
//! - **Untrusted**: Requires confirmation for all operations
//! - **Ask**: Interactive approval for sensitive operations
//! - **Moderate**: Automated approval with safety limits
//! - **Trusted**: Extended permissions with binary whitelisting
//! - **Privileged**: Infrastructure changes with explicit allowlists

use std::collections::HashMap;
use std::path::Component;
use std::path::{Path, PathBuf};

use devit_common::{ApprovalLevel, FileChangeKind, SandboxProfile};
use serde::{Deserialize, Serialize};

// ApprovalLevel maintenant défini dans devit-common

// Implémentations d'ApprovalLevel maintenant dans devit-common

// SandboxProfile maintenant défini dans devit-common

// Implémentations de SandboxProfile déplacées vers devit-common ou créées comme traits locaux

/// Network access policies for sandbox profiles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetworkAccessPolicy {
    /// No network access allowed
    Denied,
    /// Only localhost/loopback access allowed
    LocalhostOnly,
    /// Full network access allowed
    Full,
    /// Custom access rules
    Custom { allowed_hosts: Vec<String> },
}

/// Filesystem access policies for sandbox profiles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilesystemAccessPolicy {
    /// Only workspace directory access
    WorkspaceOnly,
    /// Home directory and subdirectories
    HomeDirectory,
    /// Full filesystem access
    Full,
    /// Custom path restrictions
    Custom { allowed_paths: Vec<PathBuf> },
}

/// Resource limits for sandbox execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// Maximum memory usage in megabytes
    pub max_memory_mb: Option<u64>,
    /// Maximum CPU time in seconds
    pub max_cpu_secs: Option<u64>,
    /// Maximum number of open files
    pub max_files_open: Option<u32>,
    /// Maximum number of processes
    pub max_processes: Option<u32>,
}

/// Sandbox capabilities that can be granted or denied.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SandboxCapability {
    /// Access to network resources
    NetworkAccess,
    /// Write access to filesystem
    FileSystemWrite,
    /// Ability to spawn new processes
    ProcessSpawn,
    /// Access to environment variables
    EnvironmentAccess,
    /// Access to system calls
    SystemCalls,
}

/// Policy engine for evaluating operation permissions.
///
/// Combines approval levels, sandbox profiles, and custom rules
/// to determine whether operations should be allowed.
pub struct PolicyEngine {
    /// Configuration for the policy engine
    config: PolicyEngineConfig,

    /// Default approval level for operations
    default_approval_level: ApprovalLevel,

    /// Default sandbox profile
    default_sandbox_profile: SandboxProfile,

    /// Path-specific approval overrides
    path_overrides: HashMap<PathBuf, ApprovalLevel>,

    /// Custom policy rules
    custom_rules: Vec<PolicyRule>,
}

impl PolicyEngine {
    /// Creates a new policy engine with the specified configuration.
    ///
    /// # Arguments
    ///
    /// * `config` - Configuration for the policy engine
    ///
    /// # Returns
    ///
    /// New policy engine instance ready for operation.
    pub fn new(
        default_approval_level: ApprovalLevel,
        default_sandbox_profile: SandboxProfile,
    ) -> Self {
        Self {
            config: PolicyEngineConfig::default(),
            default_approval_level,
            default_sandbox_profile,
            path_overrides: HashMap::new(),
            custom_rules: Vec::new(),
        }
    }

    /// Gets the policy engine configuration.
    pub fn config(&self) -> &PolicyEngineConfig {
        &self.config
    }

    /// Creates a new policy engine with the given defaults.
    ///
    /// # Arguments
    ///
    /// * `default_approval` - Default approval level
    /// * `default_sandbox` - Default sandbox profile
    ///
    /// # Returns
    ///
    /// New policy engine instance
    pub fn with_defaults(default_approval: ApprovalLevel, default_sandbox: SandboxProfile) -> Self {
        Self {
            config: PolicyEngineConfig::default(),
            default_approval_level: default_approval,
            default_sandbox_profile: default_sandbox,
            path_overrides: HashMap::new(),
            custom_rules: Vec::new(),
        }
    }

    /// Évalue les changements de fichiers selon la matrice d'approbation.
    ///
    /// # Arguments
    /// * `context` - Contexte d'évaluation avec les changements de fichiers
    ///
    /// # Returns
    /// Décision de politique résultant de l'évaluation
    ///
    /// # Errors
    /// Returns error if policy evaluation fails
    pub fn evaluate_changes(&self, context: &PolicyContext) -> Result<PolicyDecision, PolicyError> {
        let effective_level = match (
            &context.requested_approval_level,
            &self.default_approval_level,
        ) {
            (ApprovalLevel::Privileged { .. }, ApprovalLevel::Privileged { .. }) => {
                context.requested_approval_level.clone()
            }
            (requested, default) if requested.security_rank() > default.security_rank() => {
                default.clone()
            }
            _ => context.requested_approval_level.clone(),
        };

        match &effective_level {
            ApprovalLevel::Untrusted => self.evaluate_untrusted(context),
            ApprovalLevel::Ask => self.evaluate_ask(context),
            ApprovalLevel::Moderate => self.evaluate_moderate(context),
            ApprovalLevel::Trusted => self.evaluate_trusted(context),
            ApprovalLevel::Privileged { allowed_paths } => {
                self.evaluate_privileged(context, allowed_paths)
            }
        }
    }

    /// Évaluation pour le niveau untrusted.
    fn evaluate_untrusted(&self, context: &PolicyContext) -> Result<PolicyDecision, PolicyError> {
        if let Some(decision) =
            self.check_common_restrictions(context, CommonCheckOptions::standard())
        {
            return Ok(decision);
        }

        let reason = format!(
            "Untrusted level: confirmation required for {} change(s)",
            context.file_changes.len()
        );
        Ok(PolicyDecision::allow_with_confirmation(reason))
    }

    /// Évaluation pour le niveau ask.
    fn evaluate_ask(&self, context: &PolicyContext) -> Result<PolicyDecision, PolicyError> {
        if let Some(decision) =
            self.check_common_restrictions(context, CommonCheckOptions::standard())
        {
            return Ok(decision);
        }

        if context.file_changes.iter().any(|fc| fc.adds_exec_bit) {
            let reason = "Executable permission change requires explicit confirmation".to_string();
            return Ok(PolicyDecision::allow_with_confirmation(reason));
        }

        // Ask: request confirmation except for very simple changes
        if self.is_simple_change(context) {
            let reason = "Simple change, automatically allowed".to_string();
            Ok(PolicyDecision::allow(reason))
        } else {
            let reason = "Change requires user confirmation".to_string();
            Ok(PolicyDecision::allow_with_confirmation(reason))
        }
    }

    /// Évaluation pour le niveau moderate.
    fn evaluate_moderate(&self, context: &PolicyContext) -> Result<PolicyDecision, PolicyError> {
        if let Some(decision) =
            self.check_common_restrictions(context, CommonCheckOptions::standard())
        {
            return Ok(decision);
        }

        if context.file_changes.len() > context.config.max_files_moderate {
            let reason = format!(
                "Too many files ({} > {}), downgraded to Ask",
                context.file_changes.len(),
                context.config.max_files_moderate
            );
            return Ok(PolicyDecision::downgrade(reason, ApprovalLevel::Ask, true));
        }

        let total_lines = self.total_lines_changed(context);

        if total_lines > context.config.max_lines_moderate {
            let reason = format!(
                "Too many lines changed ({} > {}), downgraded to Ask",
                total_lines, context.config.max_lines_moderate
            );
            return Ok(PolicyDecision::downgrade(reason, ApprovalLevel::Ask, true));
        }

        if context
            .file_changes
            .iter()
            .any(|fc| self.is_protected_path(fc, context))
        {
            let reason = "Protected path modified: confirmation required (Ask level)".to_string();
            return Ok(PolicyDecision::downgrade(reason, ApprovalLevel::Ask, true));
        }

        if context.file_changes.iter().any(|fc| fc.adds_exec_bit) {
            let reason = "Adding executable bit requires Ask level".to_string();
            return Ok(PolicyDecision::downgrade(reason, ApprovalLevel::Ask, true));
        }

        if context.file_changes.iter().any(|fc| fc.is_binary) {
            let reason = "Binary files require Ask level".to_string();
            return Ok(PolicyDecision::downgrade(reason, ApprovalLevel::Ask, true));
        }

        if context.file_changes.iter().any(|fc| fc.touches_gitmodules) {
            return Ok(PolicyDecision::deny(
                ".gitmodules modification is restricted to privileged level".to_string(),
            ));
        }

        if context.file_changes.iter().any(|fc| fc.touches_submodule) {
            let reason = "Submodule change requires Ask level".to_string();
            return Ok(PolicyDecision::downgrade(reason, ApprovalLevel::Ask, true));
        }

        let reason = "Change allowed at moderate level".to_string();
        Ok(PolicyDecision::allow(reason))
    }

    /// Évaluation pour le niveau trusted.
    fn evaluate_trusted(&self, context: &PolicyContext) -> Result<PolicyDecision, PolicyError> {
        if let Some(decision) =
            self.check_common_restrictions(context, CommonCheckOptions::standard())
        {
            return Ok(decision);
        }

        if context.file_changes.iter().any(|fc| fc.touches_gitmodules) {
            return Ok(PolicyDecision::deny(
                ".gitmodules modification is restricted to privileged level".to_string(),
            ));
        }

        if context.file_changes.iter().any(|fc| fc.adds_exec_bit) {
            let reason = "Adding executable bit requires confirmation (Ask level)".to_string();
            return Ok(PolicyDecision::downgrade(reason, ApprovalLevel::Ask, true));
        }

        for file_change in &context.file_changes {
            if file_change.is_binary {
                if !self.is_whitelisted_binary(file_change, &context.config) {
                    let reason = format!("Unauthorized binary: {}", file_change.path.display());
                    return Ok(PolicyDecision::downgrade(reason, ApprovalLevel::Ask, true));
                }
            }
        }

        let requires_confirmation = context
            .file_changes
            .iter()
            .any(|fc| self.is_protected_path(fc, context));

        let reason = if requires_confirmation {
            "Sensitive path modified: confirmation required".to_string()
        } else {
            "Change allowed at trusted level".to_string()
        };

        if requires_confirmation {
            Ok(PolicyDecision::allow_with_confirmation(reason))
        } else {
            Ok(PolicyDecision::allow(reason))
        }
    }

    /// Évaluation pour le niveau privileged.
    fn evaluate_privileged(
        &self,
        context: &PolicyContext,
        allowed_paths: &[PathBuf],
    ) -> Result<PolicyDecision, PolicyError> {
        if let Some(decision) = self.check_common_restrictions(
            context,
            CommonCheckOptions {
                allow_privileged_symlinks: true,
                allow_gitmodules: true,
            },
        ) {
            return Ok(decision);
        }

        for file_change in &context.file_changes {
            let path_allowed = allowed_paths
                .iter()
                .any(|allowed_path| file_change.path.starts_with(allowed_path));

            if !path_allowed {
                let reason = format!(
                    "Path not allowed in privileged mode: {}",
                    file_change.path.display()
                );
                return Ok(PolicyDecision::deny(reason));
            }
        }

        Ok(PolicyDecision::allow(
            "Changement autorisé au niveau privileged".to_string(),
        ))
    }

    /// Vérifie si un changement est simple (peu de risques).
    fn is_simple_change(&self, context: &PolicyContext) -> bool {
        // Changement simple : 1-2 fichiers, peu de lignes, pas de binaires
        context.file_changes.len() <= 2
            && context.file_changes.iter().all(|fc| {
                !fc.is_binary
                    && !fc.adds_exec_bit
                    && !self.is_protected_path(fc, context)
                    && !fc.touches_submodule
                    && !fc.touches_gitmodules
                    && !fc.is_symlink
                    && !self.is_dot_env(&fc.path)
                    && fc.lines_added + fc.lines_deleted <= 20
            })
    }

    fn check_common_restrictions(
        &self,
        context: &PolicyContext,
        opts: CommonCheckOptions,
    ) -> Option<PolicyDecision> {
        for file_change in &context.file_changes {
            if self.is_dot_env(&file_change.path) {
                return Some(PolicyDecision::deny(
                    "Modification du fichier .env interdite".to_string(),
                ));
            }

            if !opts.allow_gitmodules && file_change.touches_gitmodules {
                return Some(PolicyDecision::deny(
                    "Modification de .gitmodules réservée au niveau privileged".to_string(),
                ));
            }

            if !opts.allow_privileged_symlinks && self.is_dangerous_symlink(file_change) {
                let reason = format!(
                    "Dangerous symlink to unauthorized path: {}",
                    file_change.path.display()
                );
                return Some(PolicyDecision::deny(reason));
            }
        }

        None
    }

    /// Vérifie si un binaire est dans la whitelist.
    fn is_whitelisted_binary(&self, file_change: &FileChange, config: &PolicyEngineConfig) -> bool {
        if !file_change.is_binary {
            return true;
        }

        // Vérifier l'extension
        let extension = file_change
            .path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|s| s.to_lowercase());

        if let Some(ext) = extension {
            if config.small_binary_whitelist.contains(&ext) {
                // Vérifier la taille
                if let Some(size) = file_change.file_size_bytes {
                    return size <= config.small_binary_max_size;
                }
            }
        }

        false
    }

    fn is_dangerous_symlink(&self, file_change: &FileChange) -> bool {
        if !file_change.is_symlink {
            return false;
        }

        if let Some(target) = &file_change.symlink_target_abs {
            if target.is_absolute() {
                return true;
            }

            if target
                .components()
                .any(|c| matches!(c, Component::ParentDir))
            {
                return true;
            }

            let dangerous_paths = ["/etc", "/usr", "/bin", "/sbin", "/sys", "/proc", "/dev"];
            return dangerous_paths
                .iter()
                .any(|&dangerous| target.starts_with(dangerous));
        }

        false
    }

    fn total_lines_changed(&self, context: &PolicyContext) -> usize {
        context
            .file_changes
            .iter()
            .map(|fc| fc.lines_added + fc.lines_deleted)
            .sum()
    }

    fn is_protected_path(&self, file_change: &FileChange, context: &PolicyContext) -> bool {
        if file_change.touches_protected {
            return true;
        }

        context
            .protected_paths
            .iter()
            .any(|protected| file_change.path.starts_with(protected))
    }

    fn is_dot_env(&self, path: &Path) -> bool {
        path.file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.eq_ignore_ascii_case(".env"))
            .unwrap_or(false)
    }

    /// Evaluates the required sandbox profile for an operation.
    ///
    /// # Arguments
    /// * `operation` - Type of operation being performed
    /// * `context` - Additional context for evaluation
    ///
    /// # Returns
    /// Required sandbox profile for the operation
    ///
    /// # Errors
    /// Returns error if policy evaluation fails
    pub fn evaluate_sandbox(
        &self,
        operation: &OperationType,
        context: &OperationContext,
    ) -> Result<SandboxProfile, PolicyError> {
        tracing::warn!(
            ?operation,
            "Sandbox evaluation using minimal fallback implementation; refine later"
        );

        let sandbox_root = context
            .metadata
            .get("sandbox_root")
            .map(|path| PathBuf::from(path))
            .filter(|path| !path.as_os_str().is_empty());

        let sandbox_root = match sandbox_root {
            Some(root) => root,
            None => {
                tracing::warn!(
                    "Sandbox evaluation: missing sandbox_root metadata, defaulting to {:?}",
                    self.default_sandbox_profile
                );
                return Ok(self.default_sandbox_profile.clone());
            }
        };

        let canonical_root = match std::fs::canonicalize(&sandbox_root) {
            Ok(root) => root,
            Err(error) => {
                tracing::warn!(
                    sandbox_root = %sandbox_root.display(),
                    %error,
                    "Sandbox evaluation: unable to canonicalize sandbox root, defaulting to {:?}",
                    self.default_sandbox_profile
                );
                return Ok(self.default_sandbox_profile.clone());
            }
        };

        for path in &context.paths {
            if !path.is_absolute()
                && path
                    .components()
                    .any(|component| matches!(component, Component::ParentDir))
            {
                return Err(PolicyError::PathAccessDenied {
                    path: path.clone(),
                    reason: "Relative path contains parent directory traversal".to_string(),
                });
            }

            let candidate = if path.is_absolute() {
                path.clone()
            } else {
                canonical_root.join(path)
            };

            if !candidate.starts_with(&canonical_root) {
                return Err(PolicyError::PathAccessDenied {
                    path: path.clone(),
                    reason: format!(
                        "Path '{}' is outside sandbox root '{}'",
                        candidate.display(),
                        canonical_root.display()
                    ),
                });
            }
        }

        Ok(self.default_sandbox_profile.clone())
    }

    /// Adds a path-specific approval override.
    ///
    /// # Arguments
    /// * `path` - Path to apply override to
    /// * `approval_level` - Required approval level for this path
    pub fn add_path_override(&mut self, path: PathBuf, approval_level: ApprovalLevel) {
        self.path_overrides.insert(path, approval_level);
    }

    /// Adds a custom policy rule.
    ///
    /// # Arguments
    /// * `rule` - Custom rule to add
    pub fn add_custom_rule(&mut self, rule: PolicyRule) {
        self.custom_rules.push(rule);
    }
}

#[derive(Clone, Copy)]
struct CommonCheckOptions {
    allow_privileged_symlinks: bool,
    allow_gitmodules: bool,
}

impl CommonCheckOptions {
    fn standard() -> Self {
        Self {
            allow_privileged_symlinks: false,
            allow_gitmodules: false,
        }
    }
}

/// Types of operations for policy evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OperationType {
    /// Reading file contents
    FileRead,
    /// Writing or modifying files
    FileWrite,
    /// Creating new files
    FileCreate,
    /// Deleting files
    FileDelete,
    /// Executing commands or processes
    ProcessExecute,
    /// Network operations
    NetworkAccess,
    /// System configuration changes
    SystemConfig,
    /// Test execution
    TestExecution,
}

/// Context information for policy evaluation.
#[derive(Debug, Clone)]
pub struct OperationContext {
    /// User or system performing the operation
    pub actor: String,
    /// Operation being performed
    pub operation: OperationType,
    /// Files or paths involved
    pub paths: Vec<PathBuf>,
    /// Size of data being processed
    pub data_size: Option<u64>,
    /// Duration of operation
    pub estimated_duration: Option<std::time::Duration>,
    /// Additional metadata
    pub metadata: HashMap<String, String>,
}

/// Custom policy rule definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    /// Unique identifier for the rule
    pub id: String,
    /// Rule name for display
    pub name: String,
    /// Pattern to match operations against
    pub pattern: String,
    /// Type of pattern matching
    pub pattern_type: PatternType,
    /// Action to take when rule matches
    pub action: PolicyAction,
    /// Priority for rule evaluation (higher = earlier)
    pub priority: u32,
}

/// Pattern types for policy rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PatternType {
    /// Glob pattern
    Glob,
    /// Regular expression
    Regex,
    /// Exact string match
    Exact,
}

/// Actions that policy rules can take.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PolicyAction {
    /// Allow the operation
    Allow,
    /// Deny the operation
    Deny,
    /// Require specific approval level
    RequireApproval(ApprovalLevel),
    /// Apply specific sandbox profile
    RequireSandbox(SandboxProfile),
    /// Log the operation but allow it
    LogAndAllow,
}

/// Errors that can occur during policy evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyError {
    /// Policy rule evaluation failed
    EvaluationFailed { rule_id: String, reason: String },
    /// Insufficient approval level
    InsufficientApproval {
        required: ApprovalLevel,
        provided: ApprovalLevel,
    },
    /// Sandbox violation
    SandboxViolation {
        capability: SandboxCapability,
        profile: SandboxProfile,
    },
    /// Path access denied
    PathAccessDenied { path: PathBuf, reason: String },
    /// Custom policy error
    Custom(String),
}

impl std::fmt::Display for PolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PolicyError::EvaluationFailed { rule_id, reason } => {
                write!(f, "Policy rule '{}' evaluation failed: {}", rule_id, reason)
            }
            PolicyError::InsufficientApproval { required, provided } => {
                write!(
                    f,
                    "Insufficient approval: required {:?}, provided {:?}",
                    required, provided
                )
            }
            PolicyError::SandboxViolation {
                capability,
                profile,
            } => {
                write!(
                    f,
                    "Sandbox violation: {:?} not allowed in {:?} profile",
                    capability, profile
                )
            }
            PolicyError::PathAccessDenied { path, reason } => {
                write!(f, "Path access denied for {:?}: {}", path, reason)
            }
            PolicyError::Custom(msg) => {
                write!(f, "Policy error: {}", msg)
            }
        }
    }
}

impl std::error::Error for PolicyError {}

/// Contexte d'évaluation de politique pour les changements de fichiers.
///
/// Contient une liste de changements de fichiers synthétiques pour
/// évaluation sans accès au système de fichiers.
#[derive(Debug, Clone)]
pub struct PolicyContext {
    /// Liste des changements de fichiers à évaluer
    pub file_changes: Vec<FileChange>,

    /// Niveau d'approbation demandé
    pub requested_approval_level: ApprovalLevel,

    /// Chemins protégés configurés
    pub protected_paths: Vec<PathBuf>,

    /// Configuration du policy engine
    pub config: PolicyEngineConfig,
}

/// Représente un changement de fichier synthétique pour évaluation.
#[derive(Debug, Clone)]
pub struct FileChange {
    /// Chemin du fichier
    pub path: PathBuf,

    /// Type de changement
    pub kind: FileChangeKind,

    /// Si le fichier est binaire
    pub is_binary: bool,

    /// Si le changement ajoute le bit exécutable
    pub adds_exec_bit: bool,

    /// Nombre de lignes ajoutées
    pub lines_added: usize,

    /// Nombre de lignes supprimées
    pub lines_deleted: usize,

    /// Si le fichier est un lien symbolique
    pub is_symlink: bool,

    /// Cible absolue du lien symbolique si applicable
    pub symlink_target_abs: Option<PathBuf>,

    /// Si le changement touche un chemin protégé
    pub touches_protected: bool,

    /// Si le changement touche un sous-module Git
    pub touches_submodule: bool,

    /// Si le changement touche .gitmodules
    pub touches_gitmodules: bool,

    /// Taille du fichier en octets (pour les binaires)
    pub file_size_bytes: Option<u64>,
}

// FileChangeKind maintenant défini dans devit-common

/// Configuration du Policy Engine.
#[derive(Debug, Clone)]
pub struct PolicyEngineConfig {
    /// Nombre maximum de fichiers pour le niveau moderate
    pub max_files_moderate: usize,

    /// Nombre maximum de lignes pour le niveau moderate
    pub max_lines_moderate: usize,

    /// Extensions de petits binaires autorisés en trusted
    pub small_binary_whitelist: Vec<String>,

    /// Taille maximale pour les petits binaires (en octets)
    pub small_binary_max_size: u64,
}

impl Default for PolicyEngineConfig {
    fn default() -> Self {
        Self {
            max_files_moderate: 10,
            max_lines_moderate: 400,
            small_binary_whitelist: vec![
                "png".to_string(),
                "jpg".to_string(),
                "jpeg".to_string(),
                "ico".to_string(),
                "woff".to_string(),
                "woff2".to_string(),
            ],
            small_binary_max_size: 1024 * 1024, // 1 MiB
        }
    }
}

/// Décision de politique résultant de l'évaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyDecision {
    /// Si l'opération est autorisée
    pub allow: bool,

    /// Si une confirmation utilisateur est requise
    pub requires_confirmation: bool,

    /// Raison de la décision
    pub reason: String,

    /// Niveau d'approbation dégradé si applicable
    pub downgraded_to: Option<ApprovalLevel>,
}

impl PolicyDecision {
    /// Crée une décision d'autorisation.
    pub fn allow(reason: String) -> Self {
        Self {
            allow: true,
            requires_confirmation: false,
            reason,
            downgraded_to: None,
        }
    }

    /// Crée une décision d'autorisation avec confirmation.
    pub fn allow_with_confirmation(reason: String) -> Self {
        Self {
            allow: true,
            requires_confirmation: true,
            reason,
            downgraded_to: None,
        }
    }

    /// Crée une décision de refus.
    pub fn deny(reason: String) -> Self {
        Self {
            allow: false,
            requires_confirmation: false,
            reason,
            downgraded_to: None,
        }
    }

    /// Crée une décision avec dégradation de niveau.
    pub fn downgrade(
        reason: String,
        downgraded_to: ApprovalLevel,
        requires_confirmation: bool,
    ) -> Self {
        Self {
            allow: true,
            requires_confirmation,
            reason,
            downgraded_to: Some(downgraded_to),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Crée un contexte de test avec des valeurs par défaut.
    fn create_test_context(
        file_changes: Vec<FileChange>,
        approval_level: ApprovalLevel,
    ) -> PolicyContext {
        PolicyContext {
            file_changes,
            requested_approval_level: approval_level,
            protected_paths: vec![
                PathBuf::from("Cargo.toml"),
                PathBuf::from(".git"),
                PathBuf::from("src/secrets"),
                PathBuf::from("scripts/install.sh"),
            ],
            config: PolicyEngineConfig::default(),
        }
    }

    /// Crée un changement de fichier simple pour les tests.
    fn create_simple_file_change(path: &str) -> FileChange {
        FileChange {
            path: PathBuf::from(path),
            kind: FileChangeKind::Mod,
            is_binary: false,
            adds_exec_bit: false,
            lines_added: 5,
            lines_deleted: 2,
            is_symlink: false,
            symlink_target_abs: None,
            touches_protected: false,
            touches_submodule: false,
            touches_gitmodules: false,
            file_size_bytes: None,
        }
    }

    /// Crée un Policy Engine pour les tests.
    fn create_test_engine() -> PolicyEngine {
        PolicyEngine::new(
            ApprovalLevel::Privileged {
                allowed_paths: vec![PathBuf::from("/")],
            },
            SandboxProfile::Strict,
        )
    }

    #[test]
    fn sandbox_allows_path_within_root() {
        let engine = create_test_engine();
        let temp = tempfile::tempdir().unwrap();
        let sandbox_root = temp.path().to_path_buf();
        let allowed_path = sandbox_root.join("allowed.txt");

        let mut metadata = HashMap::new();
        metadata.insert(
            "sandbox_root".to_string(),
            sandbox_root.to_string_lossy().to_string(),
        );

        let context = OperationContext {
            actor: "tester".to_string(),
            operation: OperationType::FileRead,
            paths: vec![allowed_path],
            data_size: None,
            estimated_duration: None,
            metadata,
        };

        let profile = engine
            .evaluate_sandbox(&context.operation, &context)
            .expect("sandbox evaluation should succeed");

        assert_eq!(profile, SandboxProfile::Strict);
    }

    #[test]
    fn sandbox_denies_path_outside_root() {
        let engine = create_test_engine();
        let temp = tempfile::tempdir().unwrap();
        let sandbox_root = temp.path().to_path_buf();
        let mut metadata = HashMap::new();
        metadata.insert(
            "sandbox_root".to_string(),
            sandbox_root.to_string_lossy().to_string(),
        );

        let context = OperationContext {
            actor: "tester".to_string(),
            operation: OperationType::FileRead,
            paths: vec![PathBuf::from("/tmp/escape_attempt.txt")],
            data_size: None,
            estimated_duration: None,
            metadata,
        };

        let result = engine.evaluate_sandbox(&context.operation, &context);
        assert!(matches!(result, Err(PolicyError::PathAccessDenied { .. })));
    }

    #[test]
    fn test_untrusted_always_requires_confirmation() {
        let engine = create_test_engine();
        let changes = vec![create_simple_file_change("src/main.rs")];
        let context = create_test_context(changes, ApprovalLevel::Untrusted);

        let decision = engine.evaluate_changes(&context).unwrap();

        assert!(decision.allow);
        assert!(decision.requires_confirmation);
        assert!(decision.reason.contains("untrusted"));
        assert_eq!(decision.downgraded_to, None);
    }

    #[test]
    fn test_ask_simple_change_allowed() {
        let engine = create_test_engine();
        let mut change = create_simple_file_change("src/main.rs");
        change.lines_added = 3;
        change.lines_deleted = 1;
        let changes = vec![change];
        let context = create_test_context(changes, ApprovalLevel::Ask);

        let decision = engine.evaluate_changes(&context).unwrap();

        assert!(decision.allow);
        assert!(!decision.requires_confirmation);
        assert!(decision.reason.contains("simple"));
    }

    #[test]
    fn test_ask_complex_change_requires_confirmation() {
        let engine = create_test_engine();
        let mut change = create_simple_file_change("src/main.rs");
        change.lines_added = 50; // Dépasse le seuil simple
        let changes = vec![change];
        let context = create_test_context(changes, ApprovalLevel::Ask);

        let decision = engine.evaluate_changes(&context).unwrap();

        assert!(decision.allow);
        assert!(decision.requires_confirmation);
        assert!(decision.reason.contains("confirmation"));
    }

    #[test]
    fn test_moderate_too_many_files_downgrades() {
        let engine = create_test_engine();
        let mut changes = Vec::new();
        // Créer plus de fichiers que la limite moderate
        for i in 0..15 {
            changes.push(create_simple_file_change(&format!("src/file{}.rs", i)));
        }
        let context = create_test_context(changes, ApprovalLevel::Moderate);

        let decision = engine.evaluate_changes(&context).unwrap();

        assert!(decision.allow);
        assert!(decision.requires_confirmation);
        assert_eq!(decision.downgraded_to, Some(ApprovalLevel::Ask));
        assert!(decision.reason.contains("Trop de fichiers"));
    }

    #[test]
    fn test_moderate_too_many_lines_downgrades() {
        let engine = create_test_engine();
        let mut change = create_simple_file_change("src/main.rs");
        change.lines_added = 300;
        change.lines_deleted = 200; // Total = 500 > 400
        let changes = vec![change];
        let context = create_test_context(changes, ApprovalLevel::Moderate);

        let decision = engine.evaluate_changes(&context).unwrap();

        assert!(decision.allow);
        assert!(decision.requires_confirmation);
        assert_eq!(decision.downgraded_to, Some(ApprovalLevel::Ask));
        assert!(decision.reason.contains("Trop de lignes"));
    }

    #[test]
    fn test_moderate_protected_path_requires_confirmation() {
        let engine = create_test_engine();
        let mut change = create_simple_file_change("Cargo.toml");
        change.touches_protected = true;
        let changes = vec![change];
        let context = create_test_context(changes, ApprovalLevel::Moderate);

        let decision = engine.evaluate_changes(&context).unwrap();

        assert!(decision.allow);
        assert!(decision.requires_confirmation);
        assert!(decision.reason.contains("Protected"));
        assert_eq!(decision.downgraded_to, Some(ApprovalLevel::Ask));
    }

    #[test]
    fn test_moderate_normal_change_allowed() {
        let engine = create_test_engine();
        let changes = vec![
            create_simple_file_change("src/main.rs"),
            create_simple_file_change("src/lib.rs"),
        ];
        let context = create_test_context(changes, ApprovalLevel::Moderate);

        let decision = engine.evaluate_changes(&context).unwrap();

        assert!(decision.allow);
        assert!(!decision.requires_confirmation);
        assert!(decision.reason.contains("moderate"));
        assert_eq!(decision.downgraded_to, None);
    }

    #[test]
    fn test_trusted_whitelisted_binary_allowed() {
        let engine = create_test_engine();
        let mut change = create_simple_file_change("assets/logo.png");
        change.is_binary = true;
        change.file_size_bytes = Some(512 * 1024); // 512 KB < 1 MB
        let changes = vec![change];
        let context = create_test_context(changes, ApprovalLevel::Trusted);

        let decision = engine.evaluate_changes(&context).unwrap();
        assert!(decision.allow);
        assert!(!decision.requires_confirmation, "decision={:?}", decision);
        assert!(decision.reason.contains("trusted"));
    }

    #[test]
    fn test_trusted_non_whitelisted_binary_downgrades() {
        let engine = create_test_engine();
        let mut change = create_simple_file_change("tools/binary.exe");
        change.is_binary = true;
        change.file_size_bytes = Some(512 * 1024);
        let changes = vec![change];
        let context = create_test_context(changes, ApprovalLevel::Trusted);

        let decision = engine.evaluate_changes(&context).unwrap();

        assert!(decision.allow);
        assert!(decision.requires_confirmation);
        assert_eq!(decision.downgraded_to, Some(ApprovalLevel::Ask));
        assert!(decision.reason.contains("Unauthorized binary"));
    }

    #[test]
    fn test_trusted_oversized_binary_downgrades() {
        let engine = create_test_engine();
        let mut change = create_simple_file_change("assets/huge.png");
        change.is_binary = true;
        change.file_size_bytes = Some(2 * 1024 * 1024); // 2 MB > 1 MB
        let changes = vec![change];
        let context = create_test_context(changes, ApprovalLevel::Trusted);

        let decision = engine.evaluate_changes(&context).unwrap();

        assert!(decision.allow);
        assert!(decision.requires_confirmation);
        assert_eq!(decision.downgraded_to, Some(ApprovalLevel::Ask));
    }

    #[test]
    fn test_trusted_submodule_requires_confirmation() {
        let engine = create_test_engine();
        let mut change = create_simple_file_change(".gitmodules");
        change.touches_gitmodules = true;
        let changes = vec![change];
        let context = create_test_context(changes, ApprovalLevel::Trusted);

        let decision = engine.evaluate_changes(&context).unwrap();

        assert!(!decision.allow);
        assert!(decision.reason.contains(".gitmodules"));
    }

    #[test]
    fn test_privileged_allowed_path_succeeds() {
        let engine = create_test_engine();
        let changes = vec![create_simple_file_change("docs/README.md")];
        let approval_level = ApprovalLevel::Privileged {
            allowed_paths: vec![PathBuf::from("docs"), PathBuf::from("examples")],
        };
        let context = create_test_context(changes, approval_level);

        let decision = engine.evaluate_changes(&context).unwrap();

        assert!(decision.allow);
        assert!(!decision.requires_confirmation);
    }

    #[test]
    fn test_privileged_forbidden_path_denied() {
        let engine = create_test_engine();
        let changes = vec![create_simple_file_change("src/main.rs")];
        let approval_level = ApprovalLevel::Privileged {
            allowed_paths: vec![PathBuf::from("docs")],
        };
        let context = create_test_context(changes, approval_level);

        let decision = engine.evaluate_changes(&context).unwrap();

        assert!(!decision.allow);
        assert!(!decision.requires_confirmation);
        assert!(decision.reason.contains("not allowed in privileged mode"));
    }

    #[test]
    fn test_dangerous_symlink_denied() {
        let engine = create_test_engine();
        let mut change = create_simple_file_change("malicious_link");
        change.is_symlink = true;
        change.symlink_target_abs = Some(PathBuf::from("/etc/passwd"));
        let changes = vec![change];
        let context = create_test_context(changes, ApprovalLevel::Ask);

        let decision = engine.evaluate_changes(&context).unwrap();

        assert!(!decision.allow);
        assert!(decision.reason.contains("Dangerous"));
    }

    #[test]
    fn test_safe_symlink_allowed() {
        let engine = create_test_engine();
        let mut change = create_simple_file_change("safe_link");
        change.is_symlink = true;
        change.symlink_target_abs = Some(PathBuf::from("lib/module.rs"));
        let changes = vec![change];
        let context = create_test_context(changes, ApprovalLevel::Ask);

        let decision = engine.evaluate_changes(&context).unwrap();

        assert!(decision.allow);
        // Symlink interne : peut demander confirmation si changement non simple
        assert!(decision.requires_confirmation);
    }

    #[test]
    fn test_exec_bit_on_sensitive_file_requires_confirmation() {
        let engine = create_test_engine();
        let mut change = create_simple_file_change("scripts/install.sh");
        change.adds_exec_bit = true;
        let changes = vec![change];
        let context = create_test_context(changes, ApprovalLevel::Ask);

        let decision = engine.evaluate_changes(&context).unwrap();

        assert!(decision.allow);
        assert!(decision.requires_confirmation);
        assert!(!decision.reason.is_empty());
    }

    #[test]
    fn test_exec_bit_on_normal_file_follows_normal_rules() {
        let engine = create_test_engine();
        let mut change = create_simple_file_change("scripts/helper.sh");
        change.adds_exec_bit = true;
        let changes = vec![change];
        let context = create_test_context(changes, ApprovalLevel::Ask);

        let decision = engine.evaluate_changes(&context).unwrap();

        assert!(decision.allow);
        // Pas sensible mais ajoute exec_bit, donc pas simple
        assert!(decision.requires_confirmation);
    }

    #[test]
    fn test_binary_addition_trusted_level() {
        let engine = create_test_engine();
        let mut change = create_simple_file_change("assets/favicon.ico");
        change.kind = FileChangeKind::Add;
        change.is_binary = true;
        change.file_size_bytes = Some(64 * 1024); // 64 KB
        let changes = vec![change];
        let context = create_test_context(changes, ApprovalLevel::Trusted);

        let decision = engine.evaluate_changes(&context).unwrap();

        assert!(decision.allow);
        assert!(!decision.requires_confirmation);
        assert!(decision.reason.contains("trusted"));
    }

    #[test]
    fn test_env_file_is_denied() {
        let engine = create_test_engine();
        let change = create_simple_file_change(".env");
        let context = create_test_context(vec![change], ApprovalLevel::Ask);

        let decision = engine.evaluate_changes(&context).unwrap();

        assert!(!decision.allow);
        assert!(decision.reason.contains(".env"));
    }

    #[test]
    fn test_moderate_exec_bit_downgrades_to_ask() {
        let engine = create_test_engine();
        let mut change = create_simple_file_change("scripts/setup.sh");
        change.adds_exec_bit = true;
        let context = create_test_context(vec![change], ApprovalLevel::Moderate);

        let decision = engine.evaluate_changes(&context).unwrap();

        assert!(decision.allow);
        assert_eq!(decision.downgraded_to, Some(ApprovalLevel::Ask));
        assert!(decision.requires_confirmation);
    }

    #[test]
    fn test_trusted_exec_bit_downgrades_to_ask() {
        let engine = create_test_engine();
        let mut change = create_simple_file_change("scripts/redeploy.sh");
        change.adds_exec_bit = true;
        let context = create_test_context(vec![change], ApprovalLevel::Trusted);

        let decision = engine.evaluate_changes(&context).unwrap();

        assert!(decision.allow);
        assert_eq!(decision.downgraded_to, Some(ApprovalLevel::Ask));
    }

    #[test]
    fn test_symlink_absolute_denied_in_trusted() {
        let engine = create_test_engine();
        let mut change = create_simple_file_change("symlink");
        change.is_symlink = true;
        change.symlink_target_abs = Some(PathBuf::from("/etc/shadow"));
        let context = create_test_context(vec![change], ApprovalLevel::Trusted);

        let decision = engine.evaluate_changes(&context).unwrap();

        assert!(!decision.allow);
        assert!(decision.reason.contains("Dangerous"));
    }

    #[test]
    fn test_trusted_submodule_reference_allowed() {
        let engine = create_test_engine();
        let mut change = create_simple_file_change("vendor/lib");
        change.touches_submodule = true;
        let context = create_test_context(vec![change], ApprovalLevel::Trusted);

        let decision = engine.evaluate_changes(&context).unwrap();

        assert!(decision.allow);
        assert!(!decision.requires_confirmation);
    }

    #[test]
    fn test_file_deletion_moderate_level() {
        let engine = create_test_engine();
        let mut change = create_simple_file_change("old_file.rs");
        change.kind = FileChangeKind::Del;
        change.lines_added = 0;
        change.lines_deleted = 100;
        let changes = vec![change];
        let context = create_test_context(changes, ApprovalLevel::Moderate);

        let decision = engine.evaluate_changes(&context).unwrap();

        assert!(decision.allow);
        assert!(!decision.requires_confirmation);
        assert!(decision.reason.contains("moderate"));
    }

    #[test]
    fn test_submodule_change_in_moderate() {
        let engine = create_test_engine();
        let mut change = create_simple_file_change("vendor/lib");
        change.touches_submodule = true;
        let changes = vec![change];
        let context = create_test_context(changes, ApprovalLevel::Moderate);

        let decision = engine.evaluate_changes(&context).unwrap();

        assert!(decision.allow);
        assert!(decision.requires_confirmation);
        assert_eq!(decision.downgraded_to, Some(ApprovalLevel::Ask));
    }

    #[test]
    fn test_mixed_changes_moderate() {
        let engine = create_test_engine();
        let changes = vec![
            create_simple_file_change("src/main.rs"),
            create_simple_file_change("tests/test.rs"),
            {
                let mut change = create_simple_file_change("README.md");
                change.lines_added = 50;
                change
            },
        ];
        let context = create_test_context(changes, ApprovalLevel::Moderate);

        let decision = engine.evaluate_changes(&context).unwrap();

        assert!(decision.allow);
        assert!(!decision.requires_confirmation);
        assert!(decision.reason.contains("moderate"));
    }
}
