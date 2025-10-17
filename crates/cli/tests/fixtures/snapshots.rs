//! # Snapshot fixtures
//!
//! Mock snapshots for testing staleness detection and validation

use devit_cli::core::SnapshotId;
use std::path::PathBuf;

/// Creates a mock stale snapshot that should be detected as outdated
pub fn create_stale_snapshot() -> SnapshotId {
    SnapshotId("snapshot_stale_12345678".to_string())
}

/// Creates a mock fresh snapshot that should pass validation
pub fn create_fresh_snapshot() -> SnapshotId {
    SnapshotId("snapshot_fresh_87654321".to_string())
}

/// Creates a mock snapshot for a specific test scenario
pub fn create_scenario_snapshot(scenario: &str) -> SnapshotId {
    SnapshotId(format!("snapshot_{}_{}", scenario, "test12345"))
}

/// Mock snapshot metadata for testing
#[derive(Debug, Clone)]
pub struct MockSnapshotMetadata {
    pub id: SnapshotId,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub reference_paths: Vec<PathBuf>,
    pub is_stale: bool,
    pub content_hash: String,
}

impl MockSnapshotMetadata {
    pub fn new_fresh(id: SnapshotId) -> Self {
        Self {
            id,
            created_at: chrono::Utc::now(),
            reference_paths: vec![
                PathBuf::from("src/main.rs"),
                PathBuf::from("Cargo.toml"),
                PathBuf::from("src/lib.rs"),
            ],
            is_stale: false,
            content_hash: "abcdef123456".to_string(),
        }
    }

    pub fn new_stale(id: SnapshotId) -> Self {
        Self {
            id,
            created_at: chrono::Utc::now() - chrono::Duration::hours(24),
            reference_paths: vec![PathBuf::from("src/main.rs"), PathBuf::from("Cargo.toml")],
            is_stale: true,
            content_hash: "outdated_hash".to_string(),
        }
    }
}

/// Creates snapshot metadata for different test scenarios
pub fn create_test_snapshots() -> Vec<MockSnapshotMetadata> {
    vec![
        MockSnapshotMetadata::new_fresh(create_fresh_snapshot()),
        MockSnapshotMetadata::new_stale(create_stale_snapshot()),
        MockSnapshotMetadata::new_fresh(create_scenario_snapshot("ok_add_fn")),
        MockSnapshotMetadata::new_stale(create_scenario_snapshot("stale_during_apply")),
    ]
}

/// Mock VCS state for testing snapshot validation
#[derive(Debug, Clone)]
pub struct MockVcsState {
    pub head_commit: String,
    pub working_tree_clean: bool,
    pub staged_files: Vec<PathBuf>,
    pub modified_files: Vec<PathBuf>,
    pub untracked_files: Vec<PathBuf>,
}

impl Default for MockVcsState {
    fn default() -> Self {
        Self {
            head_commit: "abcdef123456789".to_string(),
            working_tree_clean: true,
            staged_files: vec![],
            modified_files: vec![],
            untracked_files: vec![],
        }
    }
}

impl MockVcsState {
    pub fn with_dirty_working_tree() -> Self {
        Self {
            working_tree_clean: false,
            modified_files: vec![PathBuf::from("src/main.rs")],
            ..Default::default()
        }
    }

    pub fn with_staged_changes() -> Self {
        Self {
            staged_files: vec![PathBuf::from("src/lib.rs"), PathBuf::from("tests/mod.rs")],
            ..Default::default()
        }
    }
}

/// Creates VCS state for different test scenarios
pub fn create_test_vcs_states() -> std::collections::HashMap<String, MockVcsState> {
    let mut states = std::collections::HashMap::new();

    states.insert("clean".to_string(), MockVcsState::default());
    states.insert("dirty".to_string(), MockVcsState::with_dirty_working_tree());
    states.insert("staged".to_string(), MockVcsState::with_staged_changes());

    states
}
