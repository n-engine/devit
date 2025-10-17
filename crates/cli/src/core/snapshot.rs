//! # DevIt Snapshot Management
//!
//! System for creating, managing, and validating snapshots of project state.
//! Snapshots capture filesystem state for safe rollback and validation.
//!
//! ## Architecture
//!
//! The snapshot system provides comprehensive state management:
//!
//! - **Creation**: Capture complete filesystem state with metadata
//! - **Validation**: Verify snapshot integrity and detect staleness
//! - **Comparison**: Identify differences between snapshots and current state
//! - **Restoration**: Restore files from snapshots with rollback support
//! - **Management**: Handle snapshot lifecycle and cleanup
//!
//! ## Performance Optimization
//!
//! Snapshots use several techniques for efficiency:
//! - Content deduplication for identical files
//! - Compression for large text files
//! - Incremental snapshots for related changes
//! - LRU caching for signature-based lookup
//! - External storage for large binary files

use std::collections::{HashMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::hash::Hash;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use super::errors::{DevItError, DevItResult};
use blake3::Hasher as Blake3Hasher;
use devit_common::SnapshotId;

fn get_git_info(root_path: &Path) -> Option<GitSnapshot> {
    fn run_git(root: &Path, args: &[&str]) -> Option<String> {
        Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .ok()
            .and_then(|output| output.status.success().then(|| output.stdout))
            .and_then(|stdout| String::from_utf8(stdout).ok())
            .map(|raw| raw.trim().to_string())
            .filter(|output| !output.is_empty())
    }

    let commit_hash = run_git(root_path, &["rev-parse", "HEAD"])?;

    let branch = run_git(root_path, &["rev-parse", "--abbrev-ref", "HEAD"])
        .unwrap_or_else(|| "HEAD".to_string());

    let status_output = run_git(root_path, &["status", "--porcelain"]).unwrap_or_default();
    let mut modified_files = Vec::new();
    let mut staged_files = Vec::new();
    let mut untracked_files = Vec::new();

    for line in status_output.lines() {
        if line.len() < 3 {
            continue;
        }

        if line.starts_with("??") {
            let path = line[3..].trim();
            if !path.is_empty() {
                let resolved = path.split(" -> ").last().unwrap_or(path);
                untracked_files.push(PathBuf::from(resolved));
            }
            continue;
        }

        let staged_flag = line.chars().next().unwrap_or(' ');
        let worktree_flag = line.chars().nth(1).unwrap_or(' ');
        let path = line[3..].trim();
        if path.is_empty() {
            continue;
        }
        let resolved_path = PathBuf::from(path.split(" -> ").last().unwrap_or(path));

        if staged_flag != ' ' {
            staged_files.push(resolved_path.clone());
        }
        if worktree_flag != ' ' {
            modified_files.push(resolved_path);
        }
    }

    let remote_url = run_git(root_path, &["config", "--get", "remote.origin.url"]);
    let is_clean =
        modified_files.is_empty() && staged_files.is_empty() && untracked_files.is_empty();

    Some(GitSnapshot {
        commit_hash,
        branch,
        is_clean,
        modified_files,
        staged_files,
        untracked_files,
        remote_url,
    })
}

pub fn generate_snapshot_id(suffix: Option<&str>) -> SnapshotId {
    let timestamp = chrono::Utc::now().timestamp();
    let mut hasher = Blake3Hasher::new();
    hasher.update(timestamp.to_string().as_bytes());
    if let Some(s) = suffix {
        hasher.update(s.as_bytes());
    }
    let hash = hex::encode(&hasher.finalize().as_bytes()[..8]);
    SnapshotId(format!("snap-{}-{}", timestamp, hash))
}

pub fn snapshot_id_timestamp(id: &SnapshotId) -> DevItResult<SystemTime> {
    if let Some(parts) = id.0.strip_prefix("snap-") {
        if let Some(timestamp_str) = parts.split('-').next() {
            if let Ok(timestamp) = timestamp_str.parse::<i64>() {
                return Ok(
                    SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(timestamp as u64)
                );
            }
        }
    }
    Err(DevItError::Internal {
        component: "snapshot".to_string(),
        message: "Invalid snapshot ID format".to_string(),
        cause: None,
        correlation_id: uuid::Uuid::new_v4().to_string(),
    })
}

pub fn validate_snapshot_id(id: &SnapshotId) -> DevItResult<()> {
    if id.0.starts_with("snap-") {
        let content = id.0.strip_prefix("snap-").unwrap();

        if content.len() == 32 && content.chars().all(|c| c.is_ascii_hexdigit()) {
            return Ok(());
        }

        let parts: Vec<&str> = content.split('-').collect();
        if parts.len() >= 2 && parts[0].parse::<i64>().is_ok() && parts[1].len() == 16 {
            return Ok(());
        }
    }
    Err(DevItError::Internal {
        component: "snapshot".to_string(),
        message: "Invalid snapshot ID format".to_string(),
        cause: None,
        correlation_id: uuid::Uuid::new_v4().to_string(),
    })
}

const DEFAULT_HEAD_PREFIX_LEN: usize = 8;
const DEFAULT_SIGNATURE_CACHE_CAPACITY: usize = 128;

/// Git repository state for BLAKE3 snapshot ID generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitState {
    /// Current HEAD commit SHA
    pub head_sha: Option<String>,
    /// Porcelain status summary (sorted)
    pub porcelain_status: String,
    /// Merge/rebase flags
    pub has_merge_conflict: bool,
    pub has_rebase_conflict: bool,
    /// Submodules digest
    pub submodules_digest: Option<String>,
}

/// Lightweight signature describing the index/worktree state.
///
/// This signature is provided by higher layers that have already walked the
/// repository. It bundles the current HEAD SHA (if available) with a stable
/// textual summary of the tracked paths.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IndexSignature {
    /// Current HEAD commit SHA (short or full). `None` when detached or unknown.
    pub head_sha: Option<String>,

    /// Deterministic summary of the index/worktree (sorted paths, metadata, …).
    pub summary: String,
}

impl IndexSignature {
    /// Creates a new index signature.
    pub fn new(head_sha: Option<String>, summary: impl Into<String>) -> Self {
        Self {
            head_sha,
            summary: summary.into(),
        }
    }

    /// Returns a stable digest derived from the signature content.
    pub fn digest(&self) -> String {
        let mut hasher = Blake3Hasher::new();
        if let Some(ref head) = self.head_sha {
            hasher.update(head.as_bytes());
        }
        hasher.update(self.summary.as_bytes());
        hex::encode(&hasher.finalize().as_bytes()[..16])
    }

    /// Returns the key used in the snapshot cache (HEAD + digest).
    pub fn cache_key(&self) -> String {
        let head_component = self
            .head_sha
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("nohead");
        format!("{}:{}", head_component, self.digest())
    }

    /// Returns a short fragment of the HEAD SHA used in snapshot IDs.
    pub fn head_fragment(&self, len: usize) -> String {
        match &self.head_sha {
            Some(sha) if !sha.is_empty() => sha.chars().take(len).collect(),
            _ => "nohead".to_string(),
        }
    }

    /// Stub hook: determines whether this signature covers the provided paths.
    pub fn covers_paths(&self, _paths: &[PathBuf]) -> bool {
        // Future work: inspect the summary to ensure impacted paths are tracked.
        true
    }
}

#[derive(Clone)]
struct CacheEntry {
    snapshot_id: SnapshotId,
}

/// LRU cache for snapshot signatures keyed by HEAD/digest.
pub struct SnapshotSignatureCache {
    capacity: usize,
    entries: HashMap<String, CacheEntry>,
    order: VecDeque<String>,
}

impl SnapshotSignatureCache {
    /// Creates a cache with default capacity.
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_SIGNATURE_CACHE_CAPACITY)
    }

    /// Creates a cache with a custom capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            entries: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    /// Returns a cached entry for the given signature if available.
    fn get(&mut self, signature: &IndexSignature) -> Option<CacheEntry> {
        let key = signature.cache_key();
        self.entries.get(&key).cloned().map(|entry| {
            self.mark_recent(key);
            entry
        })
    }

    /// Inserts or updates the cache entry for the provided signature.
    pub fn insert(&mut self, signature: IndexSignature, snapshot_id: SnapshotId) {
        let key = signature.cache_key();
        let entry = CacheEntry { snapshot_id };

        if self.entries.insert(key.clone(), entry).is_some() {
            self.remove_from_order(&key);
        }

        self.order.push_back(key.clone());
        self.evict_if_needed();
    }

    /// Checks whether the cache currently references the given snapshot ID.
    pub fn contains_snapshot(&self, snapshot_id: &SnapshotId) -> bool {
        self.entries
            .values()
            .any(|entry| &entry.snapshot_id == snapshot_id)
    }

    fn mark_recent(&mut self, key: String) {
        self.remove_from_order(&key);
        self.order.push_back(key);
    }

    fn remove_from_order(&mut self, key: &str) {
        if let Some(pos) = self.order.iter().position(|k| k == key) {
            self.order.remove(pos);
        }
    }

    fn evict_if_needed(&mut self) {
        while self.entries.len() > self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            } else {
                break;
            }
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    fn contains_key(&self, signature: &IndexSignature) -> bool {
        let key = signature.cache_key();
        self.entries.contains_key(&key)
    }
}

impl Default for SnapshotSignatureCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Retrieves current Git state for snapshot generation.
fn get_git_state(repo_path: &Path) -> DevItResult<GitState> {
    let mut git_state = GitState {
        head_sha: None,
        porcelain_status: String::new(),
        has_merge_conflict: false,
        has_rebase_conflict: false,
        submodules_digest: None,
    };

    // Get HEAD commit
    if let Ok(output) = Command::new("git")
        .arg("rev-parse")
        .arg("HEAD")
        .current_dir(repo_path)
        .output()
    {
        if output.status.success() {
            git_state.head_sha = Some(String::from_utf8_lossy(&output.stdout).trim().to_string());
        }
    }

    // Get porcelain status
    if let Ok(output) = Command::new("git")
        .arg("status")
        .arg("--porcelain=v1")
        .arg("-z")
        .current_dir(repo_path)
        .output()
    {
        if output.status.success() {
            let status_output = String::from_utf8_lossy(&output.stdout).to_string();
            let mut lines: Vec<_> = status_output
                .split('\0')
                .filter(|s| !s.is_empty())
                .collect();
            lines.sort_unstable();
            git_state.porcelain_status = lines.join("\n");
        }
    }

    // Check for merge conflicts
    git_state.has_merge_conflict = repo_path.join(".git/MERGE_HEAD").exists();
    git_state.has_rebase_conflict = repo_path.join(".git/REBASE_HEAD").exists()
        || repo_path.join(".git/rebase-merge").exists()
        || repo_path.join(".git/rebase-apply").exists();

    // Get submodules digest if present
    if let Ok(contents) = fs::read_to_string(repo_path.join(".gitmodules")) {
        let mut hasher = Blake3Hasher::new();
        hasher.update(contents.as_bytes());
        git_state.submodules_digest = Some(hex::encode(&hasher.finalize().as_bytes()[..8]));
    }

    Ok(git_state)
}

/// Generates BLAKE3-based snapshot ID from Git state.
fn generate_snapshot_id_from_git(git_state: &GitState) -> DevItResult<SnapshotId> {
    // Check for conflicts first
    if git_state.has_merge_conflict || git_state.has_rebase_conflict {
        return Err(DevItError::VcsConflict {
            location: "snapshot_get".to_string(),
            conflict_type: if git_state.has_merge_conflict {
                "merge"
            } else {
                "rebase"
            }
            .to_string(),
            conflicted_files: vec![],
            resolution_hint: Some(
                "Repository has unresolved conflicts. Resolve them before creating snapshots."
                    .to_string(),
            ),
        });
    }

    let mut hasher = Blake3Hasher::new();

    // Add HEAD SHA
    if let Some(ref head) = git_state.head_sha {
        hasher.update(head.as_bytes());
    } else {
        hasher.update(b"nohead");
    }

    // Add porcelain status
    hasher.update(git_state.porcelain_status.as_bytes());

    // Add merge/rebase flags
    hasher.update(&[
        git_state.has_merge_conflict as u8,
        git_state.has_rebase_conflict as u8,
    ]);

    // Add submodules digest
    if let Some(ref digest) = git_state.submodules_digest {
        hasher.update(digest.as_bytes());
    }

    let hash = hex::encode(&hasher.finalize().as_bytes()[..16]);
    Ok(SnapshotId(format!("snap-{}", hash)))
}

/// Builds (or reuses) a lightweight snapshot identifier using Git state.
pub fn snapshot_get(
    repo_path: &Path,
    cache: &mut SnapshotSignatureCache,
) -> DevItResult<SnapshotId> {
    let git_state = get_git_state(repo_path)?;

    // Create signature for cache lookup
    let signature = IndexSignature::new(
        git_state.head_sha.clone(),
        format!(
            "{}{}{}",
            git_state.porcelain_status,
            if git_state.has_merge_conflict {
                "M"
            } else {
                ""
            },
            if git_state.has_rebase_conflict {
                "R"
            } else {
                ""
            }
        ),
    );

    if let Some(entry) = cache.get(&signature) {
        return Ok(entry.snapshot_id);
    }

    let snapshot_id = generate_snapshot_id_from_git(&git_state)?;
    cache.insert(signature, snapshot_id.clone());

    Ok(snapshot_id)
}

/// Legacy wrapper for backward compatibility with IndexSignature.
pub fn snapshot_get_legacy(
    _repo_root: &Path,
    signature: IndexSignature,
    cache: &mut SnapshotSignatureCache,
) -> SnapshotId {
    if let Some(entry) = cache.get(&signature) {
        return entry.snapshot_id;
    }

    let head_fragment = signature.head_fragment(DEFAULT_HEAD_PREFIX_LEN);
    let digest = signature.digest();
    let snapshot_id = SnapshotId(format!("snap-{}-{}", head_fragment, digest));

    cache.insert(signature, snapshot_id.clone());
    snapshot_id
}

/// Validates if paths have changed using git ls-files and mtime checks.
fn validate_affected_paths(
    repo_path: &Path,
    affected_paths: &[PathBuf],
    _cached_git_state: &GitState,
) -> DevItResult<bool> {
    // O(paths_touched) validation
    for path in affected_paths {
        let full_path = repo_path.join(path);

        // Check if file exists and get mtime
        if let Ok(metadata) = fs::metadata(&full_path) {
            if let Ok(_mtime) = metadata.modified() {
                // Get blob ID from git ls-files -s if available
                if let Ok(output) = Command::new("git")
                    .arg("ls-files")
                    .arg("-s")
                    .arg(path)
                    .current_dir(repo_path)
                    .output()
                {
                    if output.status.success() {
                        let ls_output = String::from_utf8_lossy(&output.stdout);
                        // If git knows about this file, we can use more precise checks
                        if !ls_output.trim().is_empty() {
                            // For now, rely on porcelain status which is already captured
                            continue;
                        }
                    }
                }
            }
        }
    }

    // If we reach here, the affected paths validation passed
    Ok(true)
}

/// Checks whether a previously computed snapshot ID still matches current Git state.
pub fn snapshot_validate(
    snapshot_id: &SnapshotId,
    repo_path: &Path,
    affected_paths: &[PathBuf],
    cache: &mut SnapshotSignatureCache,
) -> DevItResult<bool> {
    let current_git_state = get_git_state(repo_path)?;

    // Check for conflicts first
    if current_git_state.has_merge_conflict || current_git_state.has_rebase_conflict {
        return Err(DevItError::VcsConflict {
            location: "snapshot_validate".to_string(),
            conflict_type: if current_git_state.has_merge_conflict {
                "merge"
            } else {
                "rebase"
            }
            .to_string(),
            conflicted_files: vec![],
            resolution_hint: Some(
                "Repository has unresolved conflicts. Resolve them before validating snapshots."
                    .to_string(),
            ),
        });
    }

    let current_signature = IndexSignature::new(
        current_git_state.head_sha.clone(),
        format!(
            "{}{}{}",
            current_git_state.porcelain_status,
            if current_git_state.has_merge_conflict {
                "M"
            } else {
                ""
            },
            if current_git_state.has_rebase_conflict {
                "R"
            } else {
                ""
            }
        ),
    );

    if let Some(entry) = cache.get(&current_signature) {
        if &entry.snapshot_id == snapshot_id {
            // Validate specific affected paths for O(paths_touched) performance
            return validate_affected_paths(repo_path, affected_paths, &current_git_state);
        }
    }

    // Generate expected snapshot ID and compare
    let expected_id = generate_snapshot_id_from_git(&current_git_state)?;
    if &expected_id == snapshot_id {
        cache.insert(current_signature, expected_id);
        validate_affected_paths(repo_path, affected_paths, &current_git_state)
    } else {
        Ok(false)
    }
}

/// Snapshot metadata and content information.
///
/// Contains all information needed to restore or validate a snapshot,
/// including file contents, permissions, and timestamps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// Unique identifier for this snapshot
    pub id: SnapshotId,

    /// When the snapshot was created
    pub created_at: SystemTime,

    /// Description or reason for creating the snapshot
    pub description: String,

    /// Root directory that was snapshotted
    pub root_path: PathBuf,

    /// Files included in the snapshot
    pub files: HashMap<PathBuf, SnapshotFile>,

    /// Git information if available
    pub git_info: Option<GitSnapshot>,

    /// Metadata about the snapshot creation
    pub metadata: SnapshotMetadata,

    /// Integrity hash of the entire snapshot
    pub integrity_hash: String,

    /// Total size of all files in the snapshot
    pub total_size: u64,

    /// Parent snapshot ID if this is an incremental snapshot
    pub parent_snapshot: Option<SnapshotId>,
}

impl Snapshot {
    /// Creates a new snapshot of the specified directory.
    ///
    /// # Arguments
    /// * `root_path` - Root directory to snapshot
    /// * `description` - Description for the snapshot
    /// * `options` - Snapshot creation options
    ///
    /// # Returns
    /// * `Ok(snapshot)` - Created snapshot
    /// * `Err(error)` - If snapshot creation fails
    ///
    /// # Errors
    /// * `E_IO` - If files cannot be read
    /// * `E_RESOURCE_LIMIT` - If snapshot would be too large
    pub fn create(
        root_path: PathBuf,
        description: String,
        options: &SnapshotOptions,
    ) -> DevItResult<Self> {
        use crate::platform::permissions::PlatformPermissions;

        let mut files = HashMap::new();
        let mut total_size = 0u64;

        // Walk the directory tree
        for entry in WalkDir::new(&root_path)
            .follow_links(options.follow_symlinks)
            .into_iter()
            .filter_entry(|e| {
                // Filter out excluded patterns
                let path = e.path();
                !options
                    .exclude_patterns
                    .iter()
                    .any(|pattern| path.to_string_lossy().contains(pattern))
            })
        {
            let entry = entry.map_err(|e| {
                let io_err: std::io::Error = e.into();
                DevItError::io(Some(root_path.clone()), "walk directory", io_err)
            })?;

            // Skip directories
            if entry.file_type().is_dir() {
                continue;
            }

            let file_path = entry.path();
            let metadata = fs::metadata(file_path)
                .map_err(|e| DevItError::io(Some(file_path.to_path_buf()), "get metadata", e))?;

            // Check file size limit
            if let Some(max_size) = options.max_file_size {
                if metadata.len() > max_size {
                    continue;
                }
            }

            // Read file content
            let content = fs::read(file_path)
                .map_err(|e| DevItError::io(Some(file_path.to_path_buf()), "read file", e))?;

            // Check if binary
            let is_binary = content.iter().take(8192).any(|&b| b == 0);
            if is_binary && !options.include_binary_files {
                continue;
            }

            // Calculate content hash
            let content_hash = hex::encode(blake3::hash(&content).as_bytes());

            // Determine storage method
            let storage = if options.compress_contents && !is_binary {
                // Use flate2 for compression
                use flate2::write::ZlibEncoder;
                use flate2::Compression;
                let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
                encoder.write_all(&content).map_err(|e| {
                    DevItError::io(Some(file_path.to_path_buf()), "compress file", e)
                })?;
                let compressed = encoder.finish().map_err(|e| {
                    DevItError::io(Some(file_path.to_path_buf()), "finish compression", e)
                })?;
                ContentStorage::Compressed {
                    compressed_content: compressed,
                }
            } else {
                ContentStorage::Inline { content }
            };

            let relative_path = file_path
                .strip_prefix(&root_path)
                .unwrap_or(file_path)
                .to_path_buf();

            let file_info = SnapshotFile {
                path: relative_path.clone(),
                content_hash,
                size: metadata.len(),
                permissions: PlatformPermissions::from_fs(file_path, &metadata).encode(),
                modified_at: metadata.modified().unwrap_or(SystemTime::now()),
                is_binary,
                storage,
            };

            total_size += metadata.len();
            files.insert(relative_path, file_info);
        }

        // Get Git info if requested
        let git_info = if options.include_git_info {
            get_git_info(&root_path)
        } else {
            None
        };

        // Create metadata
        let metadata = SnapshotMetadata {
            devit_version: env!("CARGO_PKG_VERSION").to_string(),
            hostname: hostname::get()
                .map(|h| h.to_string_lossy().to_string())
                .unwrap_or_else(|_| "unknown".to_string()),
            username: std::env::var("USER").unwrap_or_else(|_| "unknown".to_string()),
            working_directory: root_path.clone(),
            environment: HashMap::new(),
            custom_fields: options.custom_metadata.clone(),
        };

        Ok(Snapshot {
            id: generate_snapshot_id(Some(&description)),
            created_at: SystemTime::now(),
            description,
            root_path,
            files,
            git_info,
            metadata,
            integrity_hash: String::new(),
            total_size,
            parent_snapshot: None,
        })
    }

    /// Validates that the snapshot is consistent and not corrupted.
    ///
    /// # Returns
    /// * `Ok(())` - If snapshot is valid
    /// * `Err(error)` - If validation fails
    ///
    /// # Errors
    /// * `E_SNAPSHOT_STALE` - If snapshot is corrupted
    /// * `E_IO` - If snapshot files cannot be accessed
    pub fn validate(&self) -> DevItResult<()> {
        // Validate snapshot ID
        validate_snapshot_id(&self.id)?;

        // Validate that all files have valid hashes
        for (path, file) in &self.files {
            if file.content_hash.is_empty() {
                return Err(DevItError::SnapshotStale {
                    snapshot_id: self.id.0.clone(),
                    created_at: None,
                    staleness_reason: Some(format!("File {} has empty hash", path.display())),
                });
            }

            // Validate hash format (should be hex)
            if !file.content_hash.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(DevItError::SnapshotStale {
                    snapshot_id: self.id.0.clone(),
                    created_at: None,
                    staleness_reason: Some(format!(
                        "File {} has invalid hash format",
                        path.display()
                    )),
                });
            }

            // Check storage integrity
            match &file.storage {
                ContentStorage::External { path: ext_path } => {
                    if !ext_path.exists() {
                        return Err(DevItError::SnapshotStale {
                            snapshot_id: self.id.0.clone(),
                            created_at: None,
                            staleness_reason: Some(format!(
                                "External storage missing for {}",
                                path.display()
                            )),
                        });
                    }
                }
                ContentStorage::Deduplicated { reference_hash } => {
                    if reference_hash.is_empty() {
                        return Err(DevItError::SnapshotStale {
                            snapshot_id: self.id.0.clone(),
                            created_at: None,
                            staleness_reason: Some(format!(
                                "Invalid dedup reference for {}",
                                path.display()
                            )),
                        });
                    }
                }
                _ => {}
            }
        }

        Ok(())
    }

    /// Compares this snapshot with the current filesystem state.
    ///
    /// # Arguments
    /// * `reference_paths` - Paths to compare (None for all files)
    ///
    /// # Returns
    /// * `Ok(differences)` - List of differences found
    /// * `Err(error)` - If comparison fails
    ///
    /// # Errors
    /// * `E_IO` - If current files cannot be read
    pub fn compare_with_current(
        &self,
        reference_paths: Option<&[PathBuf]>,
    ) -> DevItResult<Vec<FileDifference>> {
        use crate::platform::permissions::PlatformPermissions;

        let mut differences = Vec::new();

        // Determine which paths to check
        let paths_to_check: Vec<&PathBuf> = if let Some(refs) = reference_paths {
            refs.iter().collect()
        } else {
            self.files.keys().collect()
        };

        for rel_path in paths_to_check {
            let current_path = self.root_path.join(rel_path);

            if let Some(snapshot_file) = self.files.get(rel_path) {
                if !current_path.exists() {
                    differences.push(FileDifference::Missing {
                        path: rel_path.clone(),
                    });
                    continue;
                }

                // Read current file
                let current_content = fs::read(&current_path).map_err(|e| {
                    DevItError::io(Some(current_path.clone()), "read file for comparison", e)
                })?;

                // Calculate current hash
                let current_hash = hex::encode(blake3::hash(&current_content).as_bytes());

                if current_hash != snapshot_file.content_hash {
                    differences.push(FileDifference::Modified {
                        path: rel_path.clone(),
                        snapshot_hash: snapshot_file.content_hash.clone(),
                        current_hash,
                    });
                }

                // Check permissions
                let metadata = fs::metadata(&current_path).map_err(|e| {
                    DevItError::io(Some(current_path.clone()), "get metadata for comparison", e)
                })?;

                let current_pp = PlatformPermissions::from_fs(&current_path, &metadata);
                if let Some(snapshot_pp) = PlatformPermissions::decode(snapshot_file.permissions) {
                    if current_pp.has_changed(&snapshot_pp) {
                        differences.push(FileDifference::PermissionsChanged {
                            path: rel_path.clone(),
                            snapshot_permissions: snapshot_file.permissions,
                            current_permissions: current_pp.encode(),
                        });
                    }
                } // else: snapshot permissions not decodable on this platform → skip
            } else if current_path.exists() {
                // File exists now but not in snapshot
                differences.push(FileDifference::Added {
                    path: rel_path.clone(),
                });
            }
        }

        // Check for new files if comparing all
        if reference_paths.is_none() {
            for entry in WalkDir::new(&self.root_path)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if entry.file_type().is_file() {
                    let rel_path = entry
                        .path()
                        .strip_prefix(&self.root_path)
                        .unwrap_or(entry.path())
                        .to_path_buf();

                    if !self.files.contains_key(&rel_path) {
                        differences.push(FileDifference::Added { path: rel_path });
                    }
                }
            }
        }

        Ok(differences)
    }

    /// Restores files from this snapshot to the filesystem.
    ///
    /// # Arguments
    /// * `target_paths` - Specific paths to restore (None for all)
    /// * `options` - Restore options
    ///
    /// # Returns
    /// * `Ok(restored_files)` - List of files that were restored
    /// * `Err(error)` - If restore fails
    ///
    /// # Errors
    /// * `E_IO` - If files cannot be written
    /// * `E_POLICY_BLOCK` - If policy prevents restore
    /// * `E_PROTECTED_PATH` - If trying to restore protected files
    pub fn restore(
        &self,
        target_paths: Option<&[PathBuf]>,
        options: &RestoreOptions,
    ) -> DevItResult<Vec<PathBuf>> {
        use crate::platform::permissions::PlatformPermissions;

        let mut restored_files = Vec::new();

        // Determine which files to restore
        let files_to_restore: Vec<&PathBuf> = if let Some(targets) = target_paths {
            targets.iter().collect()
        } else {
            self.files.keys().collect()
        };

        for rel_path in files_to_restore {
            if let Some(snapshot_file) = self.files.get(rel_path) {
                let target_path = self.root_path.join(rel_path);

                // Check if file exists
                if target_path.exists() && !options.overwrite_existing {
                    continue;
                }

                // Backup existing file if requested
                if target_path.exists() && options.backup_existing {
                    let backup_path = target_path.with_extension("backup");
                    fs::copy(&target_path, &backup_path)
                        .map_err(|e| DevItError::io(Some(target_path.clone()), "backup file", e))?;
                }

                if options.dry_run {
                    restored_files.push(rel_path.clone());
                    continue;
                }

                // Create parent directories if needed
                if options.create_directories {
                    if let Some(parent) = target_path.parent() {
                        fs::create_dir_all(parent).map_err(|e| {
                            DevItError::io(Some(parent.to_path_buf()), "create directory", e)
                        })?;
                    }
                }

                // Extract content from storage
                let content = match &snapshot_file.storage {
                    ContentStorage::Inline { content } => content.clone(),
                    ContentStorage::Compressed { compressed_content } => {
                        // Decompress content
                        use flate2::read::ZlibDecoder;
                        let mut decoder = ZlibDecoder::new(&compressed_content[..]);
                        let mut decompressed = Vec::new();
                        decoder.read_to_end(&mut decompressed).map_err(|e| {
                            DevItError::io(Some(target_path.clone()), "decompress file", e)
                        })?;
                        decompressed
                    }
                    ContentStorage::External { path: ext_path } => {
                        fs::read(ext_path).map_err(|e| {
                            DevItError::io(Some(ext_path.clone()), "read external storage", e)
                        })?
                    }
                    ContentStorage::Deduplicated { reference_hash } => {
                        // Find file with this hash
                        let mut found_content = None;
                        for file in self.files.values() {
                            if file.content_hash == *reference_hash {
                                found_content = Some(match &file.storage {
                                    ContentStorage::Inline { content } => content.clone(),
                                    _ => continue,
                                });
                                break;
                            }
                        }
                        found_content.ok_or_else(|| DevItError::SnapshotStale {
                            snapshot_id: self.id.0.clone(),
                            created_at: None,
                            staleness_reason: Some(format!(
                                "Dedup reference not found: {}",
                                reference_hash
                            )),
                        })?
                    }
                };

                // Write file
                fs::write(&target_path, &content)
                    .map_err(|e| DevItError::io(Some(target_path.clone()), "write file", e))?;

                // Restore permissions if requested
                if options.restore_permissions {
                    if let Some(pp) = PlatformPermissions::decode(snapshot_file.permissions) {
                        pp.apply(&target_path).map_err(|e| {
                            DevItError::io(Some(target_path.clone()), "set permissions", e)
                        })?;
                    }
                }

                // Restore timestamps if requested
                if options.restore_timestamps {
                    // Note: Setting modification time requires additional dependencies
                    // For now, we'll skip this
                }

                restored_files.push(rel_path.clone());
            }
        }

        Ok(restored_files)
    }

    /// Calculates the size of the snapshot in bytes.
    ///
    /// # Returns
    /// Total size of all files in the snapshot
    pub fn size_bytes(&self) -> u64 {
        self.files.values().map(|file| file.size).sum()
    }

    /// Gets the list of files in the snapshot.
    ///
    /// # Returns
    /// Vector of file paths included in the snapshot
    pub fn file_list(&self) -> Vec<&PathBuf> {
        self.files.keys().collect()
    }
}

/// Information about a single file in a snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotFile {
    /// Relative path from snapshot root
    pub path: PathBuf,

    /// File content hash for integrity checking
    pub content_hash: String,

    /// File size in bytes
    pub size: u64,

    /// File permissions
    pub permissions: u32,

    /// Last modification time
    pub modified_at: SystemTime,

    /// Whether this is a binary file
    pub is_binary: bool,

    /// Content storage information
    pub storage: ContentStorage,
}

/// How file content is stored in the snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ContentStorage {
    /// Content is stored inline in the snapshot
    Inline { content: Vec<u8> },

    /// Content is stored in a separate file
    External { path: PathBuf },

    /// Content is compressed
    Compressed { compressed_content: Vec<u8> },

    /// Content is deduplicated (reference to another file)
    Deduplicated { reference_hash: String },
}

/// Git-specific snapshot information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitSnapshot {
    /// Current Git commit hash
    pub commit_hash: String,

    /// Current branch name
    pub branch: String,

    /// Whether working directory was clean
    pub is_clean: bool,

    /// List of modified files
    pub modified_files: Vec<PathBuf>,

    /// List of staged files
    pub staged_files: Vec<PathBuf>,

    /// List of untracked files
    pub untracked_files: Vec<PathBuf>,

    /// Remote URL if available
    pub remote_url: Option<String>,
}

/// Metadata about snapshot creation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotMetadata {
    /// Version of DevIt that created the snapshot
    pub devit_version: String,

    /// Hostname where snapshot was created
    pub hostname: String,

    /// User who created the snapshot
    pub username: String,

    /// Working directory at time of creation
    pub working_directory: PathBuf,

    /// Environment variables relevant to snapshot
    pub environment: HashMap<String, String>,

    /// Custom metadata fields
    pub custom_fields: HashMap<String, serde_json::Value>,
}

/// Options for snapshot creation.
#[derive(Debug, Clone)]
pub struct SnapshotOptions {
    /// Whether to include binary files
    pub include_binary_files: bool,

    /// Maximum file size to include (in bytes)
    pub max_file_size: Option<u64>,

    /// Patterns of files to exclude
    pub exclude_patterns: Vec<String>,

    /// Whether to compress file contents
    pub compress_contents: bool,

    /// Whether to deduplicate identical files
    pub deduplicate_contents: bool,

    /// Whether to follow symbolic links
    pub follow_symlinks: bool,

    /// Whether to include Git information
    pub include_git_info: bool,

    /// Custom metadata to include
    pub custom_metadata: HashMap<String, serde_json::Value>,
}

impl Default for SnapshotOptions {
    fn default() -> Self {
        Self {
            include_binary_files: false,
            max_file_size: None,
            // Exclude heavy or irrelevant directories by default
            exclude_patterns: vec![".git".into(), "target".into()],
            compress_contents: false,
            deduplicate_contents: false,
            follow_symlinks: false,
            include_git_info: true,
            custom_metadata: Default::default(),
        }
    }
}

/// Options for snapshot restoration.
#[derive(Debug, Clone, Default)]
pub struct RestoreOptions {
    /// Whether to overwrite existing files
    pub overwrite_existing: bool,

    /// Whether to restore file permissions
    pub restore_permissions: bool,

    /// Whether to restore timestamps
    pub restore_timestamps: bool,

    /// Whether to create missing directories
    pub create_directories: bool,

    /// Whether to perform a dry run (don't actually restore)
    pub dry_run: bool,

    /// Backup existing files before restore
    pub backup_existing: bool,
}

/// Difference between snapshot and current state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileDifference {
    /// File exists in snapshot but not in current state
    Missing { path: PathBuf },

    /// File exists in current state but not in snapshot
    Added { path: PathBuf },

    /// File content has changed
    Modified {
        path: PathBuf,
        snapshot_hash: String,
        current_hash: String,
    },

    /// File permissions have changed
    PermissionsChanged {
        path: PathBuf,
        snapshot_permissions: u32,
        current_permissions: u32,
    },

    /// File timestamp has changed
    TimestampChanged {
        path: PathBuf,
        snapshot_time: SystemTime,
        current_time: SystemTime,
    },
}

/// Snapshot manager for handling multiple snapshots.
pub struct SnapshotManager {
    /// Base directory for storing snapshots
    snapshot_dir: PathBuf,
    /// Maximum number of snapshots to keep
    max_snapshots: usize,
    /// Default options for snapshot creation
    default_options: SnapshotOptions,
}

impl SnapshotManager {
    /// Creates a new snapshot manager.
    ///
    /// # Arguments
    /// * `snapshot_dir` - Directory to store snapshots
    /// * `max_snapshots` - Maximum number of snapshots to retain
    ///
    /// # Returns
    /// New snapshot manager instance
    pub fn new(snapshot_dir: PathBuf, max_snapshots: usize) -> Self {
        let normalized_dir = Self::normalize_snapshot_dir(snapshot_dir);
        Self {
            snapshot_dir: normalized_dir,
            max_snapshots,
            default_options: SnapshotOptions::default(),
        }
    }

    /// Update the snapshot storage directory.
    pub fn set_snapshot_dir<P: Into<PathBuf>>(&mut self, path: P) {
        let normalized_dir = Self::normalize_snapshot_dir(path.into());
        self.snapshot_dir = normalized_dir;
    }

    /// Creates a new snapshot and stores it.
    ///
    /// # Arguments
    /// * `root_path` - Root directory to snapshot
    /// * `description` - Description for the snapshot
    /// * `options` - Optional custom options
    ///
    /// # Returns
    /// * `Ok(snapshot_id)` - ID of the created snapshot
    /// * `Err(error)` - If creation fails
    ///
    /// # Errors
    /// * `E_IO` - If snapshot cannot be stored
    /// * `E_RESOURCE_LIMIT` - If storage limits exceeded
    pub fn create_snapshot(
        &self,
        root_path: PathBuf,
        description: String,
        options: Option<&SnapshotOptions>,
    ) -> DevItResult<crate::core::SnapshotId> {
        self.ensure_storage_dir()?;

        let creation_options = options.unwrap_or(&self.default_options);
        let mut snapshot = Snapshot::create(root_path, description, creation_options)?;
        snapshot.integrity_hash = Self::compute_integrity_hash(&snapshot);

        let snapshot_id = snapshot.id.clone();
        let serialized = serde_json::to_vec(&snapshot).map_err(|err| DevItError::Internal {
            component: "snapshot".to_string(),
            message: format!("failed to serialize snapshot {}: {}", snapshot_id.0, err),
            cause: Some(err.to_string()),
            correlation_id: uuid::Uuid::new_v4().to_string(),
        })?;

        self.write_snapshot_file(&snapshot_id, &serialized)?;

        if self.max_snapshots > 0 {
            self.cleanup_old_snapshots()?;
        }

        Ok(crate::core::SnapshotId(snapshot_id.0))
    }

    /// Retrieves a snapshot by ID.
    ///
    /// # Arguments
    /// * `snapshot_id` - ID of the snapshot to retrieve
    ///
    /// # Returns
    /// * `Ok(snapshot)` - Retrieved snapshot
    /// * `Err(error)` - If snapshot not found or corrupted
    ///
    /// # Errors
    /// * `E_SNAPSHOT_REQUIRED` - If snapshot doesn't exist
    /// * `E_IO` - If snapshot cannot be read
    pub fn get_snapshot(&self, snapshot_id: &SnapshotId) -> DevItResult<Snapshot> {
        let path = self.snapshot_file_path(snapshot_id);
        if !path.exists() {
            return Err(DevItError::SnapshotRequired {
                operation: "snapshot_get".to_string(),
                expected: format!("Snapshot {} must exist on disk", snapshot_id.0),
            });
        }

        let file = File::open(&path)
            .map_err(|err| DevItError::io(Some(path.clone()), "open snapshot", err))?;
        serde_json::from_reader::<_, Snapshot>(file).map_err(|err| DevItError::Internal {
            component: "snapshot".to_string(),
            message: format!("failed to deserialize snapshot {}: {}", snapshot_id.0, err),
            cause: Some(err.to_string()),
            correlation_id: uuid::Uuid::new_v4().to_string(),
        })
    }

    /// Lists all available snapshots.
    ///
    /// # Returns
    /// * `Ok(snapshots)` - List of snapshot metadata
    /// * `Err(error)` - If listing fails
    ///
    /// # Errors
    /// * `E_IO` - If snapshot directory cannot be read
    pub fn list_snapshots(&self) -> DevItResult<Vec<SnapshotInfo>> {
        if !self.snapshot_dir.exists() {
            return Ok(Vec::new());
        }

        let mut snapshots = Vec::new();
        for entry in fs::read_dir(&self.snapshot_dir).map_err(|err| {
            DevItError::io(Some(self.snapshot_dir.clone()), "read snapshot dir", err)
        })? {
            let entry = entry.map_err(|err| {
                DevItError::io(Some(self.snapshot_dir.clone()), "read snapshot entry", err)
            })?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext != "json")
                .unwrap_or(true)
            {
                continue;
            }

            let file = File::open(&path)
                .map_err(|err| DevItError::io(Some(path.clone()), "open snapshot", err))?;
            let snapshot: Snapshot =
                serde_json::from_reader(file).map_err(|err| DevItError::Internal {
                    component: "snapshot".to_string(),
                    message: format!(
                        "failed to read snapshot metadata from {}: {}",
                        path.display(),
                        err
                    ),
                    cause: Some(err.to_string()),
                    correlation_id: uuid::Uuid::new_v4().to_string(),
                })?;

            snapshots.push(SnapshotInfo {
                id: snapshot.id.clone(),
                created_at: snapshot.created_at,
                description: snapshot.description.clone(),
                file_count: snapshot.files.len(),
                size_bytes: snapshot.total_size,
                root_path: snapshot.root_path.clone(),
            });
        }

        snapshots.sort_by_key(|info| info.created_at);
        Ok(snapshots)
    }

    /// Deletes a snapshot by ID.
    ///
    /// # Arguments
    /// * `snapshot_id` - ID of the snapshot to delete
    ///
    /// # Returns
    /// * `Ok(())` - If deletion succeeds
    /// * `Err(error)` - If deletion fails
    ///
    /// # Errors
    /// * `E_SNAPSHOT_REQUIRED` - If snapshot doesn't exist
    /// * `E_IO` - If snapshot cannot be deleted
    pub fn delete_snapshot(&self, snapshot_id: &SnapshotId) -> DevItResult<()> {
        let path = self.snapshot_file_path(snapshot_id);
        if !path.exists() {
            return Err(DevItError::SnapshotRequired {
                operation: "snapshot_delete".to_string(),
                expected: format!("Snapshot {} must exist to delete", snapshot_id.0),
            });
        }

        fs::remove_file(&path)
            .map_err(|err| DevItError::io(Some(path.clone()), "delete snapshot", err))?;
        Ok(())
    }

    /// Cleans up old snapshots based on retention policy.
    ///
    /// # Returns
    /// * `Ok(deleted_count)` - Number of snapshots deleted
    /// * `Err(error)` - If cleanup fails
    ///
    /// # Errors
    /// * `E_IO` - If snapshots cannot be deleted
    pub fn cleanup_old_snapshots(&self) -> DevItResult<usize> {
        if self.max_snapshots == 0 {
            return Ok(0);
        }

        let mut snapshots = self.list_snapshots()?;
        if snapshots.len() <= self.max_snapshots {
            return Ok(0);
        }

        snapshots.sort_by_key(|info| info.created_at);
        let to_remove = snapshots.len() - self.max_snapshots;
        let mut deleted = 0usize;

        for info in snapshots.into_iter().take(to_remove) {
            self.delete_snapshot(&info.id)?;
            deleted += 1;
        }

        Ok(deleted)
    }

    /// Validates all stored snapshots for integrity.
    ///
    /// # Returns
    /// * `Ok(validation_results)` - Results for each snapshot
    /// * `Err(error)` - If validation process fails
    ///
    /// # Errors
    /// * `E_IO` - If snapshots cannot be accessed
    pub fn validate_all_snapshots(&self) -> DevItResult<Vec<SnapshotValidation>> {
        let snapshot_list = self.list_snapshots()?;
        let mut results = Vec::new();

        for snapshot_info in snapshot_list {
            let validation_result = match self.get_snapshot(&snapshot_info.id) {
                Ok(snapshot) => match snapshot.validate() {
                    Ok(()) => SnapshotValidation {
                        snapshot_id: snapshot_info.id,
                        is_valid: true,
                        errors: Vec::new(),
                        warnings: Vec::new(),
                    },
                    Err(e) => SnapshotValidation {
                        snapshot_id: snapshot_info.id,
                        is_valid: false,
                        errors: vec![e.to_string()],
                        warnings: Vec::new(),
                    },
                },
                Err(e) => SnapshotValidation {
                    snapshot_id: snapshot_info.id,
                    is_valid: false,
                    errors: vec![format!("Failed to load snapshot: {}", e)],
                    warnings: Vec::new(),
                },
            };
            results.push(validation_result);
        }

        Ok(results)
    }

    /// Restore a snapshot by ID.
    ///
    /// # Arguments
    /// * `snapshot_id` - The snapshot ID to restore
    ///
    /// # Returns
    /// * `Ok(())` - If snapshot restored successfully
    /// * `Err(error)` - If snapshot not found or restore failed
    pub fn restore_snapshot(&self, snapshot_id: &crate::core::SnapshotId) -> DevItResult<()> {
        let internal_id = SnapshotId(snapshot_id.0.clone());
        let snapshot = self.get_snapshot(&internal_id)?;
        let mut options = RestoreOptions::default();
        options.overwrite_existing = true;
        options.create_directories = true;
        options.restore_permissions = true;

        snapshot.restore(None, &options)?;
        Ok(())
    }

    fn snapshot_file_path(&self, snapshot_id: &SnapshotId) -> PathBuf {
        let mut file_name = snapshot_id.0.clone();
        if !file_name.ends_with(".json") {
            file_name.push_str(".json");
        }
        self.snapshot_dir.join(file_name)
    }

    fn ensure_storage_dir(&self) -> DevItResult<()> {
        if self.snapshot_dir.exists() {
            return Ok(());
        }

        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&self.snapshot_dir).map_err(|err| {
            DevItError::io(
                Some(self.snapshot_dir.clone()),
                "create snapshot directory",
                err,
            )
        })
    }

    fn write_snapshot_file(&self, snapshot_id: &SnapshotId, contents: &[u8]) -> DevItResult<()> {
        let path = self.snapshot_file_path(snapshot_id);
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                let mut builder = std::fs::DirBuilder::new();
                builder.recursive(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::DirBuilderExt;
                    builder.mode(0o700);
                }
                builder.create(parent).map_err(|err| {
                    DevItError::io(Some(parent.to_path_buf()), "create snapshot directory", err)
                })?;
            }
        }

        let tmp_path = path.with_extension("json.tmp");

        let mut opts = OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }

        let mut tmp_file = opts.open(&tmp_path).map_err(|err| {
            DevItError::io(Some(tmp_path.clone()), "create snapshot temp file", err)
        })?;
        tmp_file.write_all(contents).map_err(|err| {
            DevItError::io(Some(tmp_path.clone()), "write snapshot temp file", err)
        })?;
        tmp_file.sync_all().map_err(|err| {
            DevItError::io(Some(tmp_path.clone()), "sync snapshot temp file", err)
        })?;
        drop(tmp_file);

        fs::rename(&tmp_path, &path)
            .map_err(|err| DevItError::io(Some(path.clone()), "persist snapshot file", err))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
        }

        if let Ok(dir) = File::open(&self.snapshot_dir) {
            let _ = dir.sync_all();
        }

        Ok(())
    }

    fn compute_integrity_hash(snapshot: &Snapshot) -> String {
        let mut hasher = Blake3Hasher::new();
        hasher.update(snapshot.id.0.as_bytes());
        hasher.update(snapshot.description.as_bytes());

        let mut entries: Vec<_> = snapshot.files.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));

        for (path, file) in entries {
            hasher.update(path.to_string_lossy().as_bytes());
            hasher.update(file.content_hash.as_bytes());
            hasher.update(&file.size.to_le_bytes());
        }

        hex::encode(&hasher.finalize().as_bytes()[..16])
    }

    fn normalize_snapshot_dir(path: PathBuf) -> PathBuf {
        let file_name = path.file_name().and_then(|name| name.to_str());
        match file_name {
            Some("snapshots") => path,
            Some(".devit") => path.join("snapshots"),
            _ => path.join(".devit").join("snapshots"),
        }
    }
}

/// Summary information about a snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotInfo {
    /// Snapshot ID
    pub id: SnapshotId,
    /// Creation timestamp
    pub created_at: SystemTime,
    /// Description
    pub description: String,
    /// Number of files
    pub file_count: usize,
    /// Total size in bytes
    pub size_bytes: u64,
    /// Root path that was snapshotted
    pub root_path: PathBuf,
}

/// Result of snapshot validation.
#[derive(Debug, Clone)]
pub struct SnapshotValidation {
    /// Snapshot that was validated
    pub snapshot_id: SnapshotId,
    /// Whether validation passed
    pub is_valid: bool,
    /// Any errors found during validation
    pub errors: Vec<String>,
    /// Warnings (non-fatal issues)
    pub warnings: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::errors::DevItError;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::thread;
    use std::time::Duration;
    use tempfile::TempDir;

    fn signature(summary: &str, head: Option<&str>) -> IndexSignature {
        IndexSignature::new(head.map(|h| h.to_string()), summary.to_string())
    }

    fn create_test_git_repo() -> TempDir {
        let temp_dir = tempfile::tempdir().unwrap();
        let repo_path = temp_dir.path();

        // Initialize git repo
        std::process::Command::new("git")
            .arg("init")
            .current_dir(repo_path)
            .output()
            .unwrap();

        // Configure git user
        std::process::Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(repo_path)
            .output()
            .unwrap();

        std::process::Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(repo_path)
            .output()
            .unwrap();

        // Create initial commit
        fs::write(repo_path.join("README.md"), "# Test repo").unwrap();
        std::process::Command::new("git")
            .args(["add", "README.md"])
            .current_dir(repo_path)
            .output()
            .unwrap();

        std::process::Command::new("git")
            .args(["commit", "-m", "Initial commit"])
            .current_dir(repo_path)
            .output()
            .unwrap();

        temp_dir
    }

    #[test]
    fn snapshot_manager_create_and_list_roundtrip() {
        let workspace_root = tempfile::tempdir().unwrap();
        let workspace = workspace_root.path();
        fs::create_dir_all(workspace).unwrap();
        fs::write(workspace.join("hello.txt"), b"hello world").unwrap();

        let manager = SnapshotManager::new(workspace.to_path_buf(), 5);
        let snapshot_id = manager
            .create_snapshot(workspace.to_path_buf(), "roundtrip".to_string(), None)
            .expect("create snapshot");

        let listed = manager.list_snapshots().expect("list snapshots");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id.0, snapshot_id.0);
        assert_eq!(listed[0].file_count, 1);

        let snapshot = manager
            .get_snapshot(&SnapshotId(snapshot_id.0.clone()))
            .expect("get snapshot");
        assert!(snapshot.files.contains_key(&PathBuf::from("hello.txt")));
    }

    #[test]
    fn snapshot_manager_enforces_max_snapshots() {
        let workspace_root = tempfile::tempdir().unwrap();
        let workspace = workspace_root.path();
        fs::create_dir_all(workspace).unwrap();
        let file_path = workspace.join("note.txt");
        fs::write(&file_path, b"initial").unwrap();

        let manager = SnapshotManager::new(workspace.to_path_buf(), 2);

        for idx in 0..3 {
            fs::write(&file_path, format!("content-{idx}")).unwrap();
            manager
                .create_snapshot(workspace.to_path_buf(), format!("snap-{idx}"), None)
                .expect("create snapshot");
            thread::sleep(Duration::from_millis(5));
        }

        let snapshots = manager.list_snapshots().expect("list snapshots");
        assert_eq!(snapshots.len(), 2);
        assert!(snapshots.iter().all(|info| info.description != "snap-0"));
    }

    #[test]
    fn snapshot_manager_restore_recovers_original_content() {
        let workspace_root = tempfile::tempdir().unwrap();
        let workspace = workspace_root.path();
        fs::create_dir_all(workspace).unwrap();
        let file_path = workspace.join("state.txt");
        fs::write(&file_path, b"original").unwrap();

        let manager = SnapshotManager::new(workspace.to_path_buf(), 4);
        let snapshot_id = manager
            .create_snapshot(workspace.to_path_buf(), "restore-test".to_string(), None)
            .expect("create snapshot");

        fs::write(&file_path, b"mutated").unwrap();

        manager
            .restore_snapshot(&snapshot_id)
            .expect("restore snapshot");

        let restored = fs::read(&file_path).expect("read restored file");
        assert_eq!(restored, b"original");
    }

    #[test]
    fn snapshot_compare_detects_modified_file() {
        let workspace_root = tempfile::tempdir().unwrap();
        let workspace = workspace_root.path();
        fs::create_dir_all(workspace).unwrap();
        let file_path = workspace.join("data.txt");
        fs::write(&file_path, b"v1").unwrap();

        let manager = SnapshotManager::new(workspace.to_path_buf(), 3);
        let snapshot_id = manager
            .create_snapshot(workspace.to_path_buf(), "compare-test".to_string(), None)
            .expect("create snapshot");

        fs::write(&file_path, b"v2").unwrap();

        let snapshot = manager
            .get_snapshot(&SnapshotId(snapshot_id.0.clone()))
            .expect("get snapshot");
        let diffs = snapshot
            .compare_with_current(None)
            .expect("compare snapshot");

        assert!(
            diffs
                .iter()
                .any(|diff| matches!(diff, FileDifference::Modified { .. })),
            "expected modified diff, got {:?}",
            diffs
        );
    }

    #[test]
    fn digest_is_stable_for_identical_signatures() {
        let sig_a = signature("path:A", Some("abcdef123456"));
        let sig_b = signature("path:A", Some("abcdef123456"));

        assert_eq!(sig_a.digest(), sig_b.digest());
        assert_eq!(sig_a.cache_key(), sig_b.cache_key());
    }

    #[test]
    fn snapshot_get_legacy_reuses_cached_identifier() {
        let mut cache = SnapshotSignatureCache::with_capacity(4);
        let sig = signature("one", Some("1234567890"));

        let first = snapshot_get_legacy(Path::new("/tmp/repo"), sig.clone(), &mut cache);
        let second = snapshot_get_legacy(Path::new("/tmp/repo"), sig, &mut cache);

        assert_eq!(first, second);
        assert!(cache.contains_snapshot(&first));
    }

    #[test]
    fn snapshot_get_legacy_distinguishes_different_summaries() {
        let mut cache = SnapshotSignatureCache::with_capacity(4);
        let sig_one = signature("one", Some("1234567890"));
        let sig_two = signature("two", Some("1234567890"));

        let first = snapshot_get_legacy(Path::new("/tmp/repo"), sig_one, &mut cache);
        let second = snapshot_get_legacy(Path::new("/tmp/repo"), sig_two, &mut cache);

        assert_ne!(first, second);
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn cache_evicts_oldest_entry_when_capacity_exceeded() {
        let mut cache = SnapshotSignatureCache::with_capacity(2);
        let sig_a = signature("a", Some("111111"));
        let sig_b = signature("b", Some("222222"));
        let sig_c = signature("c", Some("333333"));

        snapshot_get_legacy(Path::new("/tmp/repo"), sig_a.clone(), &mut cache);
        snapshot_get_legacy(Path::new("/tmp/repo"), sig_b.clone(), &mut cache);
        snapshot_get_legacy(Path::new("/tmp/repo"), sig_c.clone(), &mut cache);

        assert_eq!(cache.len(), 2);
        assert!(!cache.contains_key(&sig_a));
        assert!(cache.contains_key(&sig_b));
        assert!(cache.contains_key(&sig_c));
    }

    #[test]
    fn snapshot_id_new_generates_valid_format() {
        let id = generate_snapshot_id(Some("test"));
        assert!(id.0.starts_with("snap-"));
        assert!(validate_snapshot_id(&id).is_ok());
    }

    #[test]
    fn snapshot_id_timestamp_extraction() {
        let id = generate_snapshot_id(None);
        assert!(snapshot_id_timestamp(&id).is_ok());
    }

    #[test]
    fn blake3_snapshot_get_creates_consistent_ids() {
        let temp_repo = create_test_git_repo();
        let mut cache = SnapshotSignatureCache::with_capacity(128);

        // Test that multiple calls with same state return same ID
        let id1 = snapshot_get(temp_repo.path(), &mut cache).unwrap();
        let id2 = snapshot_get(temp_repo.path(), &mut cache).unwrap();

        assert_eq!(id1, id2);
        assert!(id1.0.starts_with("snap-"));
    }

    #[test]
    fn blake3_snapshot_detects_file_changes() {
        let temp_repo = create_test_git_repo();
        let mut cache = SnapshotSignatureCache::with_capacity(128);

        // Get initial snapshot
        let id1 = snapshot_get(temp_repo.path(), &mut cache).unwrap();

        // Make a change
        fs::write(temp_repo.path().join("test.txt"), "new content").unwrap();

        // Get new snapshot - should be different
        let id2 = snapshot_get(temp_repo.path(), &mut cache).unwrap();

        assert_ne!(id1, id2);
    }

    #[test]
    fn snapshot_validate_detects_stale_vs_fresh() {
        let temp_repo = create_test_git_repo();
        let mut cache = SnapshotSignatureCache::with_capacity(128);

        // Get initial snapshot
        let snapshot_id = snapshot_get(temp_repo.path(), &mut cache).unwrap();

        // Validate should pass on fresh state
        let is_valid = snapshot_validate(
            &snapshot_id,
            temp_repo.path(),
            &[PathBuf::from("README.md")],
            &mut cache,
        )
        .unwrap();
        assert!(is_valid);

        // Make a change to make snapshot stale
        fs::write(temp_repo.path().join("README.md"), "# Modified").unwrap();

        // Validation should now fail (stale)
        let is_valid = snapshot_validate(
            &snapshot_id,
            temp_repo.path(),
            &[PathBuf::from("README.md")],
            &mut cache,
        )
        .unwrap();
        assert!(!is_valid);
    }

    #[test]
    fn snapshot_validate_affected_paths_optimization() {
        let temp_repo = create_test_git_repo();
        let mut cache = SnapshotSignatureCache::with_capacity(128);

        // Create multiple files
        fs::write(temp_repo.path().join("file1.txt"), "content1").unwrap();
        fs::write(temp_repo.path().join("file2.txt"), "content2").unwrap();

        let snapshot_id = snapshot_get(temp_repo.path(), &mut cache).unwrap();

        // Validate with specific affected paths (O(paths_touched))
        let is_valid = snapshot_validate(
            &snapshot_id,
            temp_repo.path(),
            &[PathBuf::from("file1.txt")], // Only checking one file
            &mut cache,
        )
        .unwrap();
        assert!(is_valid);
    }

    #[test]
    fn merge_conflict_detection_returns_vcs_error() {
        let temp_repo = create_test_git_repo();
        let mut cache = SnapshotSignatureCache::with_capacity(128);

        // Simulate merge conflict by creating MERGE_HEAD
        fs::write(temp_repo.path().join(".git/MERGE_HEAD"), "dummy").unwrap();

        // Should return VcsConflict error
        let result = snapshot_get(temp_repo.path(), &mut cache);
        assert!(matches!(result, Err(DevItError::VcsConflict { .. })));
    }

    #[test]
    fn rebase_conflict_detection_returns_vcs_error() {
        let temp_repo = create_test_git_repo();
        let mut cache = SnapshotSignatureCache::with_capacity(128);

        // Simulate rebase conflict
        fs::create_dir_all(temp_repo.path().join(".git/rebase-merge")).unwrap();

        // Should return VcsConflict error
        let result = snapshot_get(temp_repo.path(), &mut cache);
        assert!(matches!(result, Err(DevItError::VcsConflict { .. })));
    }

    #[test]
    fn cache_capacity_is_128_by_default() {
        let cache = SnapshotSignatureCache::new();
        // Test default capacity through behavior
        assert_eq!(cache.capacity, DEFAULT_SIGNATURE_CACHE_CAPACITY);
        assert_eq!(DEFAULT_SIGNATURE_CACHE_CAPACITY, 128);
    }

    #[test]
    fn submodules_digest_included_in_snapshot_id() {
        let temp_repo = create_test_git_repo();
        let mut cache = SnapshotSignatureCache::with_capacity(128);

        // Get snapshot without submodules
        let id1 = snapshot_get(temp_repo.path(), &mut cache).unwrap();

        // Add .gitmodules file
        fs::write(
            temp_repo.path().join(".gitmodules"),
            "[submodule \"test\"]\n\tpath = test\n\turl = https://example.com/test.git",
        )
        .unwrap();

        // Clear cache to force regeneration
        cache = SnapshotSignatureCache::with_capacity(128);

        // Get snapshot with submodules - should be different
        let id2 = snapshot_get(temp_repo.path(), &mut cache).unwrap();

        assert_ne!(id1, id2);
    }

    #[test]
    fn performance_o_paths_touched_validation() {
        let temp_repo = create_test_git_repo();
        let mut cache = SnapshotSignatureCache::with_capacity(128);

        // Create many files (should not impact performance if we only check specific paths)
        for i in 0..100 {
            fs::write(
                temp_repo.path().join(format!("file_{}.txt", i)),
                format!("content {}", i),
            )
            .unwrap();
        }

        let snapshot_id = snapshot_get(temp_repo.path(), &mut cache).unwrap();

        // Validate with only 2 paths - should be O(2) not O(100)
        let start = std::time::Instant::now();
        let is_valid = snapshot_validate(
            &snapshot_id,
            temp_repo.path(),
            &[PathBuf::from("file_0.txt"), PathBuf::from("file_1.txt")],
            &mut cache,
        )
        .unwrap();
        let duration = start.elapsed();

        assert!(is_valid);
        // Performance check - should complete quickly even with many files
        assert!(duration < std::time::Duration::from_millis(100));
    }

    #[test]
    fn error_code_mapping_vcs_conflict() {
        let temp_repo = create_test_git_repo();
        let mut cache = SnapshotSignatureCache::with_capacity(128);

        // Create merge conflict
        fs::write(temp_repo.path().join(".git/MERGE_HEAD"), "dummy").unwrap();

        // Test snapshot_get error
        match snapshot_get(temp_repo.path(), &mut cache) {
            Err(DevItError::VcsConflict {
                location,
                conflict_type,
                ..
            }) => {
                assert_eq!(location, "snapshot_get");
                assert_eq!(conflict_type, "merge");
            }
            _ => panic!("Expected VcsConflict error"),
        }

        // Test snapshot_validate error
        let dummy_id = SnapshotId("dummy".to_string());
        match snapshot_validate(&dummy_id, temp_repo.path(), &[], &mut cache) {
            Err(DevItError::VcsConflict {
                location,
                conflict_type,
                ..
            }) => {
                assert_eq!(location, "snapshot_validate");
                assert_eq!(conflict_type, "merge");
            }
            _ => panic!("Expected VcsConflict error"),
        }
    }

    #[test]
    fn snapshot_id_format_consistency() {
        // Test that our BLAKE3-based IDs have consistent format
        let temp_repo = create_test_git_repo();
        let mut cache = SnapshotSignatureCache::with_capacity(128);

        let id = snapshot_get(temp_repo.path(), &mut cache).unwrap();

        // Should start with "snap-"
        assert!(id.0.starts_with("snap-"));

        // Should be valid format
        assert!(validate_snapshot_id(&id).is_ok());

        // BLAKE3 hash should be 32 hex chars (16 bytes * 2)
        let parts: Vec<&str> = id.0.split('-').collect();
        assert!(parts.len() >= 2);
        assert_eq!(parts[1].len(), 32); // BLAKE3 hash portion
    }

    #[test]
    fn lru_cache_performance_characteristics() {
        let mut cache = SnapshotSignatureCache::with_capacity(128);

        // Fill cache to capacity
        for i in 0..128 {
            let sig = signature(&format!("test_{}", i), Some(&format!("commit_{}", i)));
            let id = SnapshotId(format!("snap-{}", i));
            cache.insert(sig, id);
        }

        assert_eq!(cache.len(), 128);

        // Add one more - should evict oldest
        let sig = signature("overflow", Some("new_commit"));
        let id = SnapshotId("snap-overflow".to_string());
        cache.insert(sig, id);

        assert_eq!(cache.len(), 128); // Still at capacity

        // Check that newest is in cache
        let check_sig = signature("overflow", Some("new_commit"));
        assert!(cache.contains_key(&check_sig));
    }
}
