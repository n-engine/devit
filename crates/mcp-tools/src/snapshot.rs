//! Création de snapshots filesystem légers pour l'état du projet.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use mcp_core::{McpResult, McpTool};
use serde_json::{json, Value};
use uuid::Uuid;
use walkdir::WalkDir;

use crate::errors::{io_error, validation_error};
use crate::file_read::FileSystemContext;

pub struct SnapshotContext {
    root_path: PathBuf,
    fs_context: Arc<FileSystemContext>,
}

impl SnapshotContext {
    pub fn new(root_path: PathBuf) -> McpResult<Self> {
        let fs_context = Arc::new(FileSystemContext::new(root_path.clone())?);
        Ok(Self {
            root_path: fs_context.root().to_path_buf(),
            fs_context,
        })
    }

    pub fn create_snapshot(&self, requested: &[String]) -> McpResult<SnapshotResult> {
        let mut canonical_paths = Vec::new();
        if requested.is_empty() {
            canonical_paths.push(self.fs_context.root().to_path_buf());
        } else {
            for raw in requested {
                let canonical = self.fs_context.resolve_path(raw)?;
                if canonical.starts_with(self.snapshot_store()) {
                    return Err(validation_error(
                        "Impossible de snapshot .devit/snapshots directement",
                    ));
                }
                canonical_paths.push(canonical);
            }
        }

        let timestamp = Utc::now().format("%Y%m%d-%H%M%S");
        let uuid_fragment = Uuid::new_v4().to_string();
        let snapshot_id = format!("snap-{}-{}", timestamp, &uuid_fragment[..8]);
        let snapshot_dir = self.snapshot_store().join(&snapshot_id);
        fs::create_dir_all(&snapshot_dir).map_err(|err| {
            io_error(
                "create snapshot directory",
                Some(&snapshot_dir),
                err.to_string(),
            )
        })?;

        let mut relative_paths = Vec::new();
        let mut warnings = Vec::new();

        for canonical in &canonical_paths {
            let relative = canonical
                .strip_prefix(&self.root_path)
                .unwrap_or(canonical)
                .to_path_buf();
            if relative.components().count() == 0 {
                self.copy_tree(&self.root_path, &snapshot_dir, &mut warnings)?;
                relative_paths.push(".".to_string());
                continue;
            } else {
                self.copy_entry(canonical, &snapshot_dir, &mut warnings)?;
                relative_paths.push(relative.to_string_lossy().to_string());
            }
        }

        Ok(SnapshotResult {
            id: snapshot_id,
            location: snapshot_dir,
            relative_paths,
            warnings,
        })
    }

    fn snapshot_store(&self) -> PathBuf {
        self.root_path.join(".devit/snapshots")
    }

    fn copy_entry(
        &self,
        source: &Path,
        snapshot_root: &Path,
        warnings: &mut Vec<SnapshotWarning>,
    ) -> McpResult<()> {
        if source.is_dir() {
            self.copy_tree(source, snapshot_root, warnings)
        } else {
            self.copy_file(source, snapshot_root, warnings)
        }
    }

    fn copy_tree(
        &self,
        directory: &Path,
        snapshot_root: &Path,
        warnings: &mut Vec<SnapshotWarning>,
    ) -> McpResult<()> {
        let relative = directory.strip_prefix(&self.root_path).unwrap_or(directory);
        let destination_root = snapshot_root.join(relative);
        fs::create_dir_all(&destination_root).map_err(|err| {
            io_error(
                "create snapshot directory",
                Some(&destination_root),
                err.to_string(),
            )
        })?;

        for entry in WalkDir::new(directory) {
            let entry = entry.map_err(|err| {
                io_error("read directory entry", Some(directory), err.to_string())
            })?;
            let entry_path = entry.path();
            let rel = entry_path
                .strip_prefix(&self.root_path)
                .unwrap_or(entry_path);
            if rel.starts_with(Path::new(".devit/snapshots")) {
                continue;
            }
            let dest = snapshot_root.join(rel);

            if entry.file_type().is_dir() {
                fs::create_dir_all(&dest).map_err(|err| {
                    io_error("create snapshot directory", Some(&dest), err.to_string())
                })?;
            } else if entry.file_type().is_symlink() {
                warnings.push(self.warning(rel, "Lien symbolique ignoré"));
            } else if entry.file_type().is_file() {
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent).map_err(|err| {
                        io_error("create snapshot directory", Some(parent), err.to_string())
                    })?;
                }
                match fs::copy(entry_path, &dest) {
                    Ok(_) => {}
                    Err(err) => {
                        if matches!(
                            err.kind(),
                            ErrorKind::NotFound | ErrorKind::PermissionDenied
                        ) {
                            warnings
                                .push(self.warning(rel, format!("Copie ignorée ({})", err.kind())));
                            continue;
                        }
                        return Err(io_error(
                            "snapshot copy file",
                            Some(entry_path),
                            err.to_string(),
                        ));
                    }
                }
            }
        }

        Ok(())
    }

    fn copy_file(
        &self,
        file: &Path,
        snapshot_root: &Path,
        warnings: &mut Vec<SnapshotWarning>,
    ) -> McpResult<()> {
        let relative = file.strip_prefix(&self.root_path).unwrap_or(file);
        if relative.starts_with(Path::new(".devit/snapshots")) {
            return Ok(());
        }
        let destination = snapshot_root.join(relative);
        if !file.exists() {
            warnings.push(self.warning(relative, "File not found"));
            return Ok(());
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                io_error("create snapshot directory", Some(parent), err.to_string())
            })?;
        }
        match fs::copy(file, &destination) {
            Ok(_) => {}
            Err(err) => {
                if matches!(
                    err.kind(),
                    ErrorKind::NotFound | ErrorKind::PermissionDenied
                ) {
                    warnings
                        .push(self.warning(relative, format!("Copie ignorée ({})", err.kind())));
                    return Ok(());
                }
                return Err(io_error("snapshot copy file", Some(file), err.to_string()));
            }
        }
        Ok(())
    }

    fn warning(&self, path: &Path, reason: impl Into<String>) -> SnapshotWarning {
        let relative = path
            .strip_prefix(&self.root_path)
            .unwrap_or(path)
            .to_path_buf();
        SnapshotWarning {
            path: relative,
            reason: reason.into(),
        }
    }
}

pub struct SnapshotTool {
    context: Arc<SnapshotContext>,
}

impl SnapshotTool {
    pub fn new(context: Arc<SnapshotContext>) -> Self {
        Self { context }
    }
}

#[async_trait]
impl McpTool for SnapshotTool {
    fn name(&self) -> &str {
        "devit_snapshot"
    }

    fn description(&self) -> &str {
        "Create filesystem snapshots under .devit/snapshots"
    }

    async fn execute(&self, params: Value) -> McpResult<Value> {
        let paths: Vec<String> = params
            .get("paths")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let result = self.context.create_snapshot(&paths)?;
        Ok(build_response(&result))
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "paths": {"type": "array", "items": {"type": "string"}},
            }
        })
    }
}

pub struct SnapshotResult {
    pub id: String,
    pub location: PathBuf,
    pub relative_paths: Vec<String>,
    pub warnings: Vec<SnapshotWarning>,
}

pub struct SnapshotWarning {
    pub path: PathBuf,
    pub reason: String,
}

fn build_response(result: &SnapshotResult) -> Value {
    let mut message = format!(
        "📸 Snapshot {} créé dans {}",
        result.id,
        result.location.to_string_lossy()
    );

    if !result.warnings.is_empty() {
        message.push_str(&format!(
            "\n⚠️ {} élément(s) n'ont pas été copiés",
            result.warnings.len()
        ));
    }

    json!({
        "content": [
            {
                "type": "text",
                "text": message
            }
        ],
        "snapshot": {
            "id": result.id,
            "paths": result.relative_paths,
            "location": result.location.to_string_lossy(),
            "warnings": result.warnings.iter().map(|warning| json!({
                "path": warning.path.to_string_lossy(),
                "reason": warning.reason,
            })).collect::<Vec<_>>()
        }
    })
}
