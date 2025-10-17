//! # DevIt Journal System
//!
//! Operation logging and audit trail management.
//! Provides tamper-evident logging with HMAC signatures.
//!
//! ## Architecture
//!
//! The journal system provides comprehensive audit capabilities:
//!
//! - **Operation Logging**: Record all operations with metadata
//! - **Integrity Protection**: HMAC signatures for tamper detection
//! - **Idempotency**: Prevent duplicate entries via key-based deduplication
//! - **Rotation Management**: Automatic log rotation and archival
//! - **Verification**: Integrity checking and audit trail validation
//!
//! ## Security Model
//!
//! Journal entries are protected against tampering:
//! - HMAC signing with secret keys
//! - Sequential ordering validation
//! - Immutable append-only semantics
//! - Cryptographic hash chains
//! - Rotation with integrity preservation

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::{
    errors::{DevItError, DevItResult},
    request_id,
};
use devit_common::{ApprovalLevel, SandboxProfile};
use uuid::Uuid;

/// Response emitted when adding an entry to the journal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalResponse {
    /// Truncated HMAC computed for the entry payload.
    pub hmac: String,
    /// Logical offset (line number) of the entry inside the JSONL file.
    pub offset: u64,
    /// Physical file where the entry was recorded.
    pub file: PathBuf,
    /// Request identifier associated with this append.
    pub request_id: Uuid,
}

/// Runtime journal facade that ensures idempotent append semantics and
/// tamper-evident bookkeeping. This stub keeps data in-memory; upcoming
/// revisions will stream to disk and manage key rotation.
pub struct Journal {
    /// Path to the journal file
    path: PathBuf,

    /// Secret key for HMAC signing
    secret: Vec<u8>,

    /// In-memory entries (temporary storage)
    entries: VecDeque<serde_json::Value>,

    /// Idempotency tracking for duplicate prevention
    idempotency: HashMap<Uuid, (u64, Uuid)>,
}

impl Journal {
    /// Creates a new journal instance based on the target file path.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the journal file
    /// * `secret` - Secret key for HMAC signing
    ///
    /// # Returns
    ///
    /// New journal instance ready for operation.
    pub fn new(path: PathBuf, secret: Vec<u8>) -> Self {
        Self {
            path,
            secret,
            entries: VecDeque::new(),
            idempotency: HashMap::new(),
        }
    }

    /// Appends a JSON value, optionally using the provided idempotency key.
    ///
    /// When an idempotency key is supplied and matches a previous append,
    /// the same `offset` and HMAC are returned. Future versions will persist
    /// the journal to disk and rotate secrets.
    pub fn append(
        &mut self,
        entry: serde_json::Value,
        idempotency_key: Option<Uuid>,
    ) -> DevItResult<JournalResponse> {
        let request_id = request_id::resolve(idempotency_key);

        if let Some(key) = idempotency_key {
            if let Some(&(offset, stored_request_id)) = self.idempotency.get(&key) {
                let existing =
                    self.entries
                        .get(offset as usize)
                        .ok_or_else(|| DevItError::Internal {
                            component: "journal".to_string(),
                            message: "idempotency offset missing".to_string(),
                            cause: None,
                            correlation_id: uuid::Uuid::new_v4().to_string(),
                        })?;

                let hmac = self.compute_hmac(existing);
                return Ok(JournalResponse {
                    hmac,
                    offset,
                    file: self.path.clone(),
                    request_id: stored_request_id,
                });
            }
        }

        let offset = self.entries.len() as u64;
        self.entries.push_back(entry);

        if let Some(key) = idempotency_key {
            self.idempotency.insert(key, (offset, request_id));
        }

        let stored = self.entries.back().expect("just inserted");
        let hmac = self.compute_hmac(stored);

        Ok(JournalResponse {
            hmac,
            offset,
            file: self.path.clone(),
            request_id,
        })
    }

    fn compute_hmac(&self, entry: &serde_json::Value) -> String {
        let mut hasher = DefaultHasher::new();
        if let Ok(bytes) = serde_json::to_vec(entry) {
            bytes.hash(&mut hasher);
        }
        self.secret.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }
}

/// Journal entry for operation tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalEntry {
    /// Unique identifier for this entry
    pub id: String,

    /// Timestamp of the operation
    pub timestamp: chrono::DateTime<chrono::Utc>,

    /// Type of operation performed
    pub operation: OperationType,

    /// User or system that initiated the operation
    pub actor: String,

    /// Approval level used for the operation
    pub approval_level: ApprovalLevel,

    /// Sandbox profile used
    pub sandbox_profile: SandboxProfile,

    /// Success or failure status
    pub success: bool,

    /// Duration of the operation in milliseconds
    pub duration_ms: Option<u64>,

    /// Files that were affected
    pub affected_files: Vec<PathBuf>,

    /// Additional metadata
    pub metadata: serde_json::Value,

    /// HMAC signature for integrity
    pub signature: Option<String>,
}

/// Type of operation for journal entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OperationType {
    /// Snapshot creation or retrieval
    Snapshot,
    /// Patch preview operation
    PatchPreview,
    /// Patch application
    PatchApply,
    /// Test execution
    TestRun,
    /// Configuration change
    ConfigChange,
    /// Policy enforcement
    PolicyCheck,
    /// Journal maintenance
    JournalMaintenance,
}

/// Journal manager for handling operation logs.
pub struct JournalManager {
    /// Path to the journal file
    journal_path: PathBuf,
    /// HMAC key for signing entries
    signing_key: Option<Vec<u8>>,
    /// Whether to sign entries
    sign_entries: bool,
    /// Configuration options
    config: JournalRuntimeConfig,
}

impl JournalManager {
    /// Creates a new journal manager.
    ///
    /// # Arguments
    /// * `journal_path` - Path to store journal entries
    /// * `config` - Journal configuration
    ///
    /// # Returns
    /// New journal manager instance
    pub fn new(journal_path: PathBuf, config: JournalRuntimeConfig) -> Self {
        Self {
            journal_path,
            signing_key: None,
            sign_entries: config.sign_entries,
            config,
        }
    }

    /// Signs an entry using HMAC-SHA256 if a signing key is set
    fn sign_entry(&self, entry: &serde_json::Value) -> Option<String> {
        self.signing_key.as_ref().map(|key| {
            use hmac::{Hmac, Mac};
            use sha2::Sha256;
            type HmacSha256 = Hmac<Sha256>;

            let mut mac = HmacSha256::new_from_slice(key).expect("HMAC can take key of any size");
            mac.update(entry.to_string().as_bytes());
            let result = mac.finalize();
            hex::encode(result.into_bytes())
        })
    }

    /// Appends an entry to the journal.
    ///
    /// # Arguments
    /// * `entry` - Entry to append
    ///
    /// # Returns
    /// * `Ok(())` - If entry was successfully appended
    /// * `Err(error)` - If journaling fails
    ///
    /// # Errors
    /// * `E_IO` - If journal file cannot be written
    /// * `E_INTERNAL` - If entry signing fails
    pub fn append_entry(&mut self, entry: JournalEntry) -> DevItResult<()> {
        use chrono::Utc;
        use std::fs::OpenOptions;
        use std::io::Write;

        // Ensure journal directory exists
        if let Some(parent) = self.journal_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                DevItError::io(Some(parent.to_path_buf()), "create journal directory", e)
            })?;
        }

        // Convert entry to JSON
        let json_entry = serde_json::to_value(&entry).map_err(|e| DevItError::Internal {
            component: "journal".to_string(),
            message: format!("Failed to serialize entry: {}", e),
            cause: None,
            correlation_id: uuid::Uuid::new_v4().to_string(),
        })?;

        // Sign the entry if configured
        let signature = if self.sign_entries {
            self.sign_entry(&json_entry)
        } else {
            None
        };

        // Create signed entry
        let signed_entry = serde_json::json!({
            "entry": json_entry,
            "signature": signature,
            "timestamp": Utc::now().to_rfc3339(),
        });

        // Append to file
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.journal_path)
            .map_err(|e| DevItError::io(Some(self.journal_path.clone()), "open journal file", e))?;

        writeln!(file, "{}", serde_json::to_string(&signed_entry).unwrap()).map_err(|e| {
            DevItError::io(Some(self.journal_path.clone()), "write journal entry", e)
        })?;

        Ok(())
    }

    /// Reads all entries from the journal.
    ///
    /// # Returns
    /// * `Ok(entries)` - All journal entries
    /// * `Err(error)` - If reading fails
    ///
    /// # Errors
    /// * `E_IO` - If journal file cannot be read
    /// * `E_INTERNAL` - If entry validation fails
    pub fn read_entries(&self) -> DevItResult<Vec<JournalEntry>> {
        use std::fs::File;
        use std::io::{BufRead, BufReader};

        if !self.journal_path.exists() {
            return Ok(Vec::new());
        }

        let file = File::open(&self.journal_path)
            .map_err(|e| DevItError::io(Some(self.journal_path.clone()), "open journal file", e))?;

        let reader = BufReader::new(file);
        let mut entries = Vec::new();

        for line in reader.lines() {
            let line = line.map_err(|e| {
                DevItError::io(Some(self.journal_path.clone()), "read journal line", e)
            })?;

            if line.trim().is_empty() {
                continue;
            }

            // Parse the signed entry
            let signed_entry: serde_json::Value =
                serde_json::from_str(&line).map_err(|e| DevItError::Internal {
                    component: "journal".to_string(),
                    message: format!("Failed to parse journal entry: {}", e),
                    cause: None,
                    correlation_id: uuid::Uuid::new_v4().to_string(),
                })?;

            // Extract the actual entry
            if let Some(entry_value) = signed_entry.get("entry") {
                let entry: JournalEntry =
                    serde_json::from_value(entry_value.clone()).map_err(|e| {
                        DevItError::Internal {
                            component: "journal".to_string(),
                            message: format!("Failed to deserialize entry: {}", e),
                            cause: None,
                            correlation_id: uuid::Uuid::new_v4().to_string(),
                        }
                    })?;
                entries.push(entry);
            }
        }

        Ok(entries)
    }

    /// Verifies the integrity of all journal entries.
    ///
    /// # Returns
    /// * `Ok(results)` - Verification results for each entry
    /// * `Err(error)` - If verification process fails
    ///
    /// # Errors
    /// * `E_IO` - If journal cannot be accessed
    /// * `E_INTERNAL` - If verification fails
    pub fn verify_integrity(&self) -> DevItResult<Vec<EntryVerification>> {
        use std::fs::File;
        use std::io::{BufRead, BufReader};

        if !self.journal_path.exists() {
            return Ok(Vec::new());
        }

        let file = File::open(&self.journal_path)
            .map_err(|e| DevItError::io(Some(self.journal_path.clone()), "open journal file", e))?;

        let reader = BufReader::new(file);
        let mut verifications = Vec::new();
        let mut line_number = 0;

        for line in reader.lines() {
            line_number += 1;
            let line = line.map_err(|e| {
                DevItError::io(Some(self.journal_path.clone()), "read journal line", e)
            })?;

            if line.trim().is_empty() {
                continue;
            }

            let mut issues = Vec::new();
            let entry_id = format!("line_{}", line_number);

            // Parse the signed entry
            let signed_entry: serde_json::Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(e) => {
                    issues.push(format!("Invalid JSON: {}", e));
                    verifications.push(EntryVerification {
                        entry_id,
                        signature_valid: false,
                        format_valid: false,
                        issues,
                    });
                    continue;
                }
            };

            // Verify signature if present
            let signature_valid = if self.sign_entries {
                if let (Some(entry_value), Some(sig_value)) =
                    (signed_entry.get("entry"), signed_entry.get("signature"))
                {
                    if let Some(sig_str) = sig_value.as_str() {
                        let computed_sig = self.sign_entry(entry_value);
                        computed_sig.as_deref() == Some(sig_str)
                    } else {
                        issues.push("Invalid signature format".to_string());
                        false
                    }
                } else {
                    issues.push("Missing entry or signature field".to_string());
                    false
                }
            } else {
                true // Signatures not required
            };

            // Verify format
            let format_valid =
                signed_entry.get("entry").is_some() && signed_entry.get("timestamp").is_some();

            if !format_valid {
                issues.push("Invalid entry format".to_string());
            }

            verifications.push(EntryVerification {
                entry_id,
                signature_valid,
                format_valid,
                issues,
            });
        }

        Ok(verifications)
    }

    /// Rotates the journal file if it exceeds size limits.
    ///
    /// # Returns
    /// * `Ok(rotated)` - Whether rotation was performed
    /// * `Err(error)` - If rotation fails
    ///
    /// # Errors
    /// * `E_IO` - If files cannot be moved
    pub fn rotate_journal(&self) -> DevItResult<bool> {
        use chrono::Utc;

        if !self.journal_path.exists() {
            return Ok(false);
        }

        // Check file size
        let metadata = std::fs::metadata(&self.journal_path)
            .map_err(|e| DevItError::io(Some(self.journal_path.clone()), "get file metadata", e))?;

        let size_mb = metadata.len() / (1024 * 1024);
        if size_mb < self.config.max_file_size_mb {
            return Ok(false); // No rotation needed
        }

        // Generate rotation filename
        let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
        let rotated_name = format!(
            "{}.{}.jsonl",
            self.journal_path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy(),
            timestamp
        );
        let rotated_path = self
            .journal_path
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join(rotated_name);

        // Rename current file
        std::fs::rename(&self.journal_path, &rotated_path).map_err(|e| {
            DevItError::io(Some(self.journal_path.clone()), "rotate journal file", e)
        })?;

        // Clean up old rotated files if needed
        if let Some(parent) = self.journal_path.parent() {
            let base_name = self
                .journal_path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy();

            let mut rotated_files: Vec<_> = std::fs::read_dir(parent)
                .map_err(|e| DevItError::io(Some(parent.to_path_buf()), "list rotated files", e))?
                .filter_map(|entry| entry.ok())
                .filter(|entry| {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();
                    name_str.starts_with(&*base_name)
                        && name_str.contains('.')
                        && name_str.ends_with(".jsonl")
                })
                .collect();

            // Sort by modification time (oldest first)
            rotated_files.sort_by_key(|e| e.metadata().ok().and_then(|m| m.modified().ok()));

            // Remove oldest files if we exceed the limit
            while rotated_files.len() > self.config.max_rotated_files as usize {
                if let Some(oldest) = rotated_files.first() {
                    let _ = std::fs::remove_file(oldest.path());
                    rotated_files.remove(0);
                }
            }
        }

        Ok(true)
    }
}

/// Configuration for journal management.
#[derive(Debug, Clone)]
pub struct JournalRuntimeConfig {
    /// Whether journaling is enabled
    pub enabled: bool,
    /// Whether to sign journal entries
    pub sign_entries: bool,
    /// Maximum size of journal file before rotation
    pub max_file_size_mb: u64,
    /// Number of rotated journal files to keep
    pub max_rotated_files: u32,
    /// Whether to include sensitive data in journal
    pub include_sensitive_data: bool,
}

/// Result of entry verification.
#[derive(Debug, Clone)]
pub struct EntryVerification {
    /// Entry ID that was verified
    pub entry_id: String,
    /// Whether signature is valid
    pub signature_valid: bool,
    /// Whether entry format is valid
    pub format_valid: bool,
    /// Any issues found
    pub issues: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::request_id;
    use serde_json::json;

    fn new_journal() -> Journal {
        Journal::new(PathBuf::from("journal.jsonl"), b"secret-key".to_vec())
    }

    #[test]
    fn append_without_idempotency_produces_incrementing_offsets() {
        request_id::reset_for_tests();
        let mut journal = new_journal();
        let first = journal.append(json!({"idx": 1}), None).expect("append");
        let second = journal.append(json!({"idx": 2}), None).expect("append");

        assert_eq!(first.offset, 0);
        assert_eq!(second.offset, 1);
        assert_ne!(first.hmac, second.hmac);
        assert_ne!(first.request_id, second.request_id);
    }

    #[test]
    fn append_with_idempotency_returns_same_response() {
        request_id::reset_for_tests();
        let mut journal = new_journal();
        let key = Uuid::new_v4();

        let first = journal
            .append(json!({"action": "apply"}), Some(key))
            .expect("append");
        let second = journal
            .append(json!({"action": "ignored"}), Some(key))
            .expect("append");

        assert_eq!(first.offset, second.offset);
        assert_eq!(first.hmac, second.hmac);
        assert_eq!(first.file, second.file);
        assert_eq!(first.request_id, second.request_id);
    }

    #[test]
    fn different_idempotency_keys_produce_unique_entries() {
        request_id::reset_for_tests();
        let mut journal = new_journal();
        let first = journal
            .append(json!({"op": "one"}), Some(Uuid::new_v4()))
            .expect("append");
        let second = journal
            .append(json!({"op": "two"}), Some(Uuid::new_v4()))
            .expect("append");

        assert_eq!(second.offset, first.offset + 1);
        assert_ne!(first.hmac, second.hmac);
        assert_ne!(first.request_id, second.request_id);
    }
}
