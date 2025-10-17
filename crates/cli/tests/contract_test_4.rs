use devit_cli::core::ApprovalLevel;

/// Contract: Different approval levels have different file/line limits
#[test]
fn contract_approval_levels_respect_size_limits() {
    use devit_cli::core::policy::{FileChange, PolicyContext, PolicyEngine, PolicyEngineConfig};
    use devit_common::SandboxProfile;
    use std::path::PathBuf;

    let mut config = PolicyEngineConfig::default();
    config.max_lines_moderate = 100;
    config.max_files_moderate = 10;

    let engine_moderate = PolicyEngine::new(ApprovalLevel::Moderate, SandboxProfile::Strict);

    let large_change = FileChange {
        path: PathBuf::from("src/large_file.rs"),
        kind: devit_cli::core::FileChangeKind::Mod,
        is_binary: false,
        adds_exec_bit: false,
        lines_added: 150,
        lines_deleted: 0,
        is_symlink: false,
        symlink_target_abs: None,
        touches_protected: false,
        touches_submodule: false,
        touches_gitmodules: false,
        file_size_bytes: None,
    };

    let moderate_context = PolicyContext {
        file_changes: vec![large_change.clone()],
        requested_approval_level: ApprovalLevel::Moderate,
        protected_paths: vec![],
        config: config.clone(),
    };

    let total_lines: usize = moderate_context
        .file_changes
        .iter()
        .map(|fc| fc.lines_added + fc.lines_deleted)
        .sum();
    assert_eq!(total_lines, 150);

    let moderate_decision = engine_moderate
        .evaluate_changes(&moderate_context)
        .expect("policy evaluation should succeed");

    assert!(
        moderate_decision.requires_confirmation,
        "Moderate level must require confirmation for large patches"
    );
    assert_eq!(
        moderate_decision.downgraded_to,
        Some(ApprovalLevel::Ask),
        "Moderate level should downgrade to Ask when limits are exceeded"
    );

    let engine_trusted = PolicyEngine::new(ApprovalLevel::Trusted, SandboxProfile::Strict);
    let trusted_context = PolicyContext {
        file_changes: vec![large_change],
        requested_approval_level: ApprovalLevel::Trusted,
        protected_paths: vec![],
        config,
    };

    let trusted_decision = engine_trusted
        .evaluate_changes(&trusted_context)
        .expect("policy evaluation should succeed");

    assert!(
        trusted_decision.allow,
        "Trusted level should allow larger patches within its limits"
    );
    assert!(
        !trusted_decision.requires_confirmation,
        "Trusted level should not require confirmation for allowed patches"
    );
}
