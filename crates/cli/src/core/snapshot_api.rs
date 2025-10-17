use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use tokio::sync::RwLock;

use crate::core::{
    errors::{DevItError, DevItResult},
    snapshot::{self, SnapshotInfo, SnapshotManager, SnapshotOptions, SnapshotValidation},
    SnapshotId,
};

/// Résumé structuré après capture d'un snapshot.
#[derive(Clone, Debug)]
pub struct SnapshotSummary {
    pub id: SnapshotId,
    pub description: String,
    pub created_at: DateTime<Utc>,
    pub file_count: usize,
    pub total_size: u64,
    pub snapshot_path: PathBuf,
    pub snapshot_file_bytes: u64,
    pub checksum_blake3: String,
    pub requested_paths: Option<Vec<PathBuf>>,
}

/// Service centralisant la capture et la lecture des snapshots.
pub struct SnapshotService {
    manager: RwLock<SnapshotManager>,
    snapshots_dir: PathBuf,
    root_dir: PathBuf,
}

impl SnapshotService {
    pub fn new(root_dir: PathBuf, max_snapshots: usize) -> DevItResult<Self> {
        let snapshots_dir = root_dir.join(".devit/snapshots");
        Ok(Self {
            manager: RwLock::new(SnapshotManager::new(snapshots_dir.clone(), max_snapshots)),
            snapshots_dir,
            root_dir,
        })
    }

    /// Capture l'état courant et renvoie un résumé complet.
    pub async fn capture(
        &self,
        description: String,
        requested_paths: Option<&[PathBuf]>,
        options: Option<&SnapshotOptions>,
    ) -> DevItResult<SnapshotSummary> {
        let snapshot_id = {
            let manager = self.manager.write().await;
            manager.create_snapshot(self.root_dir.clone(), description.clone(), options)?
        };

        let internal_id = snapshot::SnapshotId(snapshot_id.0.clone());

        let snapshot = {
            let manager = self.manager.read().await;
            manager.get_snapshot(&internal_id)?
        };

        let snapshot_path = Self::snapshot_file_path(&self.snapshots_dir, &snapshot_id);
        let bytes = tokio::fs::read(&snapshot_path)
            .await
            .map_err(|err| DevItError::io(Some(snapshot_path.clone()), "read snapshot", err))?;

        let canonical_path = tokio::fs::canonicalize(&snapshot_path)
            .await
            .unwrap_or(snapshot_path.clone());

        let created_at = DateTime::<Utc>::from(snapshot.created_at);
        let checksum_blake3 = blake3::hash(&bytes).to_hex().to_string();

        Ok(SnapshotSummary {
            id: snapshot_id,
            description,
            created_at,
            file_count: snapshot.files.len(),
            total_size: snapshot.total_size,
            snapshot_path: canonical_path,
            snapshot_file_bytes: bytes.len() as u64,
            checksum_blake3,
            requested_paths: requested_paths.map(|paths| paths.to_vec()),
        })
    }

    pub async fn restore(&self, snapshot_id: &SnapshotId) -> DevItResult<()> {
        let manager = self.manager.write().await;
        manager.restore_snapshot(snapshot_id)
    }

    /// Liste toutes les métadonnées de snapshots disponibles.
    pub async fn list(&self) -> DevItResult<Vec<SnapshotInfo>> {
        let manager = self.manager.read().await;
        manager.list_snapshots()
    }

    /// Supprime un snapshot donné.
    pub async fn delete(&self, snapshot_id: &SnapshotId) -> DevItResult<()> {
        let manager = self.manager.write().await;
        let internal_id = snapshot::SnapshotId(snapshot_id.0.clone());
        manager.delete_snapshot(&internal_id)
    }

    /// Valide l'ensemble des snapshots conservés sur disque.
    pub async fn validate_all(&self) -> DevItResult<Vec<SnapshotValidation>> {
        let manager = self.manager.read().await;
        manager.validate_all_snapshots()
    }

    pub fn snapshot_file_path(base: &Path, id: &SnapshotId) -> PathBuf {
        let mut file_name = id.0.clone();
        if !file_name.ends_with(".json") {
            file_name.push_str(".json");
        }
        base.join(file_name)
    }

    pub fn root_dir(&self) -> &Path {
        &self.root_dir
    }

    pub fn snapshots_dir(&self) -> &Path {
        &self.snapshots_dir
    }

    pub async fn manager(&self) -> tokio::sync::RwLockReadGuard<'_, SnapshotManager> {
        self.manager.read().await
    }

    pub async fn manager_mut(&self) -> tokio::sync::RwLockWriteGuard<'_, SnapshotManager> {
        self.manager.write().await
    }
}
