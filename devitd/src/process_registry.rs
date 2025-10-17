use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};

#[cfg(target_family = "unix")]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

use crate::process_utils::{canonicalize_with_nofollow, process_exists, read_proc_stat};

const REGISTRY_FILE: &str = "process_registry.json";
const REGISTRY_LOCK: &str = "process_registry.lock";

/// Process lifetime status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProcessStatus {
    Running,
    Exited,
}

/// Stored metadata for a background process.
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

/// Registry backing storage.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Registry {
    pub processes: HashMap<u32, ProcessRecord>,
}

#[allow(dead_code)]
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

/// Resolve the DevIt runtime directory (`~/.devit`) with canonical checks.
pub fn registry_dir() -> io::Result<PathBuf> {
    let home = std::env::var("HOME").map_err(|_| {
        io::Error::new(io::ErrorKind::NotFound, "HOME environment variable missing")
    })?;
    let dir = PathBuf::from(home).join(".devit");
    canonicalize_parent(&dir)
}

fn canonicalize_parent(path: &PathBuf) -> io::Result<PathBuf> {
    if let Some(parent) = path.parent() {
        let _ = canonicalize_with_nofollow(parent)?;
    }
    Ok(path.clone())
}

fn registry_path() -> io::Result<PathBuf> {
    Ok(registry_dir()?.join(REGISTRY_FILE))
}

fn lock_path() -> io::Result<PathBuf> {
    Ok(registry_dir()?.join(REGISTRY_LOCK))
}

/// Load the registry from disk.
pub fn load_registry() -> io::Result<Registry> {
    let path = registry_path()?;
    if !path.exists() {
        return Ok(Registry::new());
    }

    let file = File::open(&path)?;
    let registry: Registry = serde_json::from_reader(file).map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Failed to parse registry {}: {}", path.display(), err),
        )
    })?;

    Ok(registry)
}

/// Persist registry with durable semantics (flock + fsync + atomic rename).
#[cfg(target_family = "unix")]
pub fn save_registry(registry: &Registry) -> io::Result<()> {
    let dir = registry_dir()?;

    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)?;

    let lock_file = lock_path()?;
    let lock = File::create(&lock_file)?;
    lock.lock_exclusive()?;

    let temp_path = dir.join(format!("{}.tmp", REGISTRY_FILE));
    let mut temp = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temp_path)?;

    serde_json::to_writer_pretty(&mut temp, registry).map_err(|err| {
        io::Error::new(
            io::ErrorKind::Other,
            format!("Failed to serialise registry: {}", err),
        )
    })?;

    temp.sync_all()?;
    drop(temp);

    let final_path = registry_path()?;
    std::fs::rename(&temp_path, &final_path)?;

    let dir_handle = File::open(&dir)?;
    dir_handle.sync_all()?;

    drop(lock);
    Ok(())
}

#[cfg(not(target_family = "unix"))]
pub fn save_registry(registry: &Registry) -> io::Result<()> {
    let dir = registry_dir()?;
    std::fs::create_dir_all(&dir)?;

    let temp_path = dir.join(format!("{}.tmp", REGISTRY_FILE));
    let mut temp = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&temp_path)?;

    serde_json::to_writer_pretty(&mut temp, registry).map_err(|err| {
        io::Error::new(
            io::ErrorKind::Other,
            format!("Failed to serialise registry: {}", err),
        )
    })?;

    temp.sync_all()?;
    drop(temp);

    let final_path = registry_path()?;
    std::fs::rename(&temp_path, &final_path)?;
    Ok(())
}

/// Validate that a process entry still matches the real process.
pub fn validate_process(record: &ProcessRecord) -> bool {
    if !process_exists(record.pid) {
        return false;
    }

    match read_proc_stat(record.pid) {
        Ok(stat) => stat.starttime == record.start_ticks,
        Err(_) => false,
    }
}
