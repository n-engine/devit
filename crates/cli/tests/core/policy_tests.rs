use devit_cli::core::{
    PolicyEngine, PolicyContext, PolicyEngineConfig, FileChange
};
use devit_common::{SnapshotId, ApprovalLevel, SandboxProfile, FileChangeKind};
use std::path::PathBuf;

/// Crée un contexte de test avec des valeurs par défaut.
fn create_test_context(
    file_changes: Vec<FileChange>,
    approval_level: ApprovalLevel
) -> PolicyContext
{
    PolicyContext {
        file_changes,
        requested_approval_level: approval_level,
        protected_paths: vec![
            PathBuf::from("Cargo.toml"),
            PathBuf::from(".git"),
            PathBuf::from("src/secrets"),
        ],
        config: PolicyEngineConfig::default(),
    }
}

/// Crée un changement de fichier simple pour les tests.
fn create_simple_file_change(path: &str) -> FileChange
{
    FileChange {
        path: PathBuf::from(path),
        kind: FileChangeKind::Modify,
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
fn create_test_engine() -> PolicyEngine
{
    PolicyEngine::new(ApprovalLevel::Ask, SandboxProfile::Strict)
}

#[test]
fn test_untrusted_always_requires_confirmation()
{
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
fn test_ask_simple_change_allowed()
{
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
fn test_ask_complex_change_requires_confirmation()
{
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
fn test_moderate_too_many_files_downgrades()
{
    let engine = create_test_engine();
    let mut changes = Vec::new();
    // Créer plus de fichiers que la limite moderate
    for i in 0..15
    {
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
fn test_moderate_too_many_lines_downgrades()
{
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
fn test_moderate_protected_path_requires_confirmation()
{
    let engine = create_test_engine();
    let mut change = create_simple_file_change("Cargo.toml");
    change.touches_protected = true;
    let changes = vec![change];
    let context = create_test_context(changes, ApprovalLevel::Moderate);

    let decision = engine.evaluate_changes(&context).unwrap();

    assert!(decision.allow);
    assert!(decision.requires_confirmation);
    assert!(decision.reason.contains("protégés"));
    assert_eq!(decision.downgraded_to, None);
}

#[test]
fn test_moderate_normal_change_allowed()
{
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
fn test_trusted_whitelisted_binary_allowed()
{
    let engine = create_test_engine();
    let mut change = create_simple_file_change("assets/logo.png");
    change.is_binary = true;
    change.file_size_bytes = Some(512 * 1024); // 512 KB < 1 MB
    let changes = vec![change];
    let context = create_test_context(changes, ApprovalLevel::Trusted);

    let decision = engine.evaluate_changes(&context).unwrap();

    assert!(decision.allow);
    assert!(!decision.requires_confirmation);
    assert!(decision.reason.contains("trusted"));
}

#[test]
fn test_trusted_non_whitelisted_binary_downgrades()
{
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
    assert!(decision.reason.contains("Binaire non autorisé"));
}

#[test]
fn test_trusted_oversized_binary_downgrades()
{
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
fn test_trusted_submodule_requires_confirmation()
{
    let engine = create_test_engine();
    let mut change = create_simple_file_change(".gitmodules");
    change.touches_gitmodules = true;
    let changes = vec![change];
    let context = create_test_context(changes, ApprovalLevel::Trusted);

    let decision = engine.evaluate_changes(&context).unwrap();

    assert!(decision.allow);
    assert!(decision.requires_confirmation);
    assert!(decision.reason.contains("sous-module"));
    assert_eq!(decision.downgraded_to, None);
}

#[test]
fn test_privileged_allowed_path_succeeds()
{
    let engine = create_test_engine();
    let changes = vec![create_simple_file_change("docs/README.md")];
    let approval_level = ApprovalLevel::Privileged {
        allowed_paths: vec![PathBuf::from("docs"), PathBuf::from("examples")]
    };
    let context = create_test_context(changes, approval_level);

    let decision = engine.evaluate_changes(&context).unwrap();

    assert!(decision.allow);
    assert!(!decision.requires_confirmation);
}

#[test]
fn test_privileged_forbidden_path_denied()
{
    let engine = create_test_engine();
    let changes = vec![create_simple_file_change("src/main.rs")];
    let approval_level = ApprovalLevel::Privileged {
        allowed_paths: vec![PathBuf::from("docs")]
    };
    let context = create_test_context(changes, approval_level);

    let decision = engine.evaluate_changes(&context).unwrap();

    assert!(!decision.allow);
    assert!(!decision.requires_confirmation);
    assert!(decision.reason.contains("non autorisé en mode privileged"));
}

#[test]
fn test_dangerous_symlink_denied()
{
    let engine = create_test_engine();
    let mut change = create_simple_file_change("malicious_link");
    change.is_symlink = true;
    change.symlink_target_abs = Some(PathBuf::from("/etc/passwd"));
    let changes = vec![change];
    let context = create_test_context(changes, ApprovalLevel::Ask);

    let decision = engine.evaluate_changes(&context).unwrap();

    assert!(!decision.allow);
    assert!(decision.reason.contains("dangereux"));
}

#[test]
fn test_safe_symlink_allowed()
{
    let engine = create_test_engine();
    let mut change = create_simple_file_change("safe_link");
    change.is_symlink = true;
    change.symlink_target_abs = Some(PathBuf::from("/home/user/project/file.txt"));
    let changes = vec![change];
    let context = create_test_context(changes, ApprovalLevel::Ask);

    let decision = engine.evaluate_changes(&context).unwrap();

    assert!(decision.allow);
    // Devrait être simple donc pas de confirmation
    assert!(!decision.requires_confirmation);
}

#[test]
fn test_exec_bit_on_sensitive_file_requires_confirmation()
{
    let engine = create_test_engine();
    let mut change = create_simple_file_change("scripts/install.sh");
    change.adds_exec_bit = true;
    let changes = vec![change];
    let context = create_test_context(changes, ApprovalLevel::Ask);

    let decision = engine.evaluate_changes(&context).unwrap();

    assert!(decision.allow);
    assert!(decision.requires_confirmation);
    assert!(decision.reason.contains("exécutable"));
    assert!(decision.reason.contains("sensible"));
}

#[test]
fn test_exec_bit_on_normal_file_follows_normal_rules()
{
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
fn test_binary_addition_trusted_level()
{
    let engine = create_test_engine();
    let mut change = create_simple_file_change("assets/favicon.ico");
    change.kind = FileChangeKind::Create;
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
fn test_file_deletion_moderate_level()
{
    let engine = create_test_engine();
    let mut change = create_simple_file_change("old_file.rs");
    change.kind = FileChangeKind::Delete;
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
fn test_submodule_change_in_moderate()
{
    let engine = create_test_engine();
    let mut change = create_simple_file_change("vendor/lib");
    change.touches_submodule = true;
    let changes = vec![change];
    let context = create_test_context(changes, ApprovalLevel::Moderate);

    let decision = engine.evaluate_changes(&context).unwrap();

    assert!(decision.allow);
    // En moderate, les sous-modules ne demandent confirmation qu'en trusted+
    assert!(!decision.requires_confirmation);
}

#[test]
fn test_mixed_changes_moderate()
{
    let engine = create_test_engine();
    let changes = vec![
        create_simple_file_change("src/main.rs"),
        create_simple_file_change("tests/test.rs"),
        {
            let mut change = create_simple_file_change("README.md");
            change.lines_added = 50;
            change
        }
    ];
    let context = create_test_context(changes, ApprovalLevel::Moderate);

    let decision = engine.evaluate_changes(&context).unwrap();

    assert!(decision.allow);
    assert!(!decision.requires_confirmation);
    assert!(decision.reason.contains("moderate"));
}