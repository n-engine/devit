// Process registry for devit_exec
// Version: v3.1 (final architecture)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io;
use std::path::PathBuf;

#[cfg(target_family = "unix")]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

/// Process status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProcessStatus {
    Running,
    Exited,
}

/// Process record in registry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessRecord {
    pub pid: u32,
    pub pgid: u32,
    pub start_ticks: u64,
    pub started_at: DateTime<Utc>,
    pub command: String,
    pub args: Vec<String>,
    pub status: ProcessStatus,
    pub exit_code: Option<i32>,
    pub terminated_by_signal: Option<i32>,
    pub auto_kill_at: Option<DateTime<Utc>>,
}

/// Process registry
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Registry {
    pub processes: HashMap<u32, ProcessRecord>,
}

impl Registry {
    pub fn new() -> Self {
        Self {
            processes: HashMap::new(),
        }
    }

    pub fn insert(&mut self, pid: u32, record: ProcessRecord) {
        self.processes.insert(pid, record);
    }

    pub fn get(&self, pid: u32) -> Option<&ProcessRecord> {
        self.processes.get(&pid)
    }

    pub fn get_mut(&mut self, pid: u32) -> Option<&mut ProcessRecord> {
        self.processes.get_mut(&pid)
    }

    pub fn remove(&mut self, pid: u32) -> Option<ProcessRecord> {
        self.processes.remove(&pid)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&u32, &ProcessRecord)> {
        self.processes.iter()
    }
}

/// Get registry directory path
pub fn get_registry_dir() -> io::Result<PathBuf> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "Neither HOME nor USERPROFILE environment variables are set",
            )
        })?;

    Ok(PathBuf::from(&home).join(".devit"))
}

/// Get registry file path
pub fn get_registry_path() -> io::Result<PathBuf> {
    Ok(get_registry_dir()?.join("process_registry.json"))
}

/// Get lock file path
pub fn get_lock_path() -> io::Result<PathBuf> {
    Ok(get_registry_dir()?.join("process_registry.lock"))
}

/// Load registry from disk
pub fn load_registry() -> io::Result<Registry> {
    let path = get_registry_path()?;

    if !path.exists() {
        return Ok(Registry::new());
    }

    let file = File::open(&path)?;
    let registry: Registry = serde_json::from_reader(file).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Failed to parse registry: {}", e),
        )
    })?;

    Ok(registry)
}

/// Save registry to disk with durability guarantees
/// - Create dir with 0700
/// - Lock with flock
/// - Write to temp file with 0600
/// - fsync file
/// - Atomic rename
/// - fsync directory
#[cfg(target_family = "unix")]
pub fn save_registry(registry: &Registry) -> io::Result<()> {
    let devit_dir = get_registry_dir()?;

    // Create directory with 0700
    std::fs::DirBuilder::new()
        .mode(0o700)
        .recursive(true)
        .create(&devit_dir)?;

    // Acquire lock
    let lockfile = get_lock_path()?;
    let lock = File::create(&lockfile)?;
    use fs2::FileExt;
    lock.lock_exclusive()?;

    // Write to temp file with 0600
    let temp_path = devit_dir.join("process_registry.json.tmp");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temp_path)?;

    serde_json::to_writer_pretty(&mut file, registry).map_err(|e| {
        io::Error::new(
            io::ErrorKind::Other,
            format!("Failed to serialize registry: {}", e),
        )
    })?;

    file.sync_all()?; // fsync file
    drop(file);

    // Atomic rename
    let final_path = get_registry_path()?;
    std::fs::rename(&temp_path, &final_path)?;

    // fsync directory
    let dir = File::open(&devit_dir)?;
    dir.sync_all()?;

    drop(lock); // Release lock
    Ok(())
}

/// Save registry (Windows fallback - no mode/flock)
#[cfg(not(target_family = "unix"))]
pub fn save_registry(registry: &Registry) -> io::Result<()> {
    let devit_dir = get_registry_dir()?;

    // Create directory
    std::fs::create_dir_all(&devit_dir)?;

    // Write to temp file
    let temp_path = devit_dir.join("process_registry.json.tmp");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&temp_path)?;

    serde_json::to_writer_pretty(&mut file, registry)
        .map_err(|e| io::Error::other(format!("Failed to serialize registry: {}", e)))?;

    file.sync_all()?;
    drop(file);

    // Atomic rename
    let final_path = get_registry_path()?;
    std::fs::rename(&temp_path, &final_path)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn test_registry_new() {
        let registry = Registry::new();
        assert!(registry.processes.is_empty());
    }

    #[test]
    fn test_registry_insert_get() {
        let mut registry = Registry::new();

        let record = ProcessRecord {
            pid: 12345,
            pgid: 12345,
            start_ticks: 123456789,
            started_at: Utc::now(),
            command: "test".into(),
            args: vec!["arg1".into()],
            status: ProcessStatus::Running,
            exit_code: None,
            terminated_by_signal: None,
            auto_kill_at: None,
        };

        registry.insert(12345, record.clone());

        let retrieved = registry.get(12345).unwrap();
        assert_eq!(retrieved.pid, 12345);
        assert_eq!(retrieved.command, "test");
    }

    #[test]
    fn test_registry_remove() {
        let mut registry = Registry::new();

        let record = ProcessRecord {
            pid: 12345,
            pgid: 12345,
            start_ticks: 123456789,
            started_at: Utc::now(),
            command: "test".into(),
            args: vec![],
            status: ProcessStatus::Running,
            exit_code: None,
            terminated_by_signal: None,
            auto_kill_at: None,
        };

        registry.insert(12345, record);
        assert!(registry.get(12345).is_some());

        registry.remove(12345);
        assert!(registry.get(12345).is_none());
    }

    #[test]
    fn test_save_load_registry() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        std::env::set_var("HOME", temp_dir.path());

        let mut registry = Registry::new();
        let record = ProcessRecord {
            pid: 12345,
            pgid: 12345,
            start_ticks: 123456789,
            started_at: Utc::now(),
            command: "test".into(),
            args: vec!["arg1".into()],
            status: ProcessStatus::Running,
            exit_code: None,
            terminated_by_signal: None,
            auto_kill_at: None,
        };

        registry.insert(12345, record);

        save_registry(&registry).expect("Failed to save");

        let loaded = load_registry().expect("Failed to load");
        assert_eq!(loaded.processes.len(), 1);
        assert_eq!(loaded.get(12345).unwrap().command, "test");
    }
}
