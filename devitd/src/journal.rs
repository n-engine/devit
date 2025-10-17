//! HMAC-chained journal for DevIt orchestration
//!
//! Provides append-only, verifiable audit trail for all orchestration events.
//! Each entry is HMAC-signed and chained to prevent tampering.

use anyhow::Result;
use base64::{engine::general_purpose, Engine as _};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

type HmacSha256 = Hmac<Sha256>;

/// Journal entry with HMAC chain
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct JEntry {
    pub seq: u64,
    pub ts: u64,
    pub ev: String,              // Event type: DELEGATE, LEASE, NOTIFY, ACK, POLICY
    pub msg_id: String,          // Message ID
    pub from: String,            // Source
    pub to: String,              // Destination
    pub meta: serde_json::Value, // Event metadata
    pub prev: String,            // Previous entry hash (base64)
    pub hash: String,            // This entry hash (base64)
}

/// HMAC-chained journal
#[allow(dead_code)]
pub struct Journal {
    file: Mutex<File>,
    key: Vec<u8>,
    seq: Mutex<u64>,
    last_hash: Mutex<String>,
}

impl Journal {
    /// Open or create journal file
    pub fn open(path: &str, key: &[u8]) -> Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;

        // TODO: Read last entry to get current sequence and hash
        // For now, start fresh
        Ok(Self {
            file: Mutex::new(file),
            key: key.to_vec(),
            seq: Mutex::new(0),
            last_hash: Mutex::new(String::new()),
        })
    }

    /// Append new entry to journal
    pub fn append(
        &self,
        event: &str,
        msg_id: &str,
        from: &str,
        to: &str,
        meta: serde_json::Value,
    ) -> Result<String> {
        let ts = now_ts();

        let mut seq = self.seq.lock().unwrap();
        *seq += 1;
        let current_seq = *seq;

        let prev_hash = self.last_hash.lock().unwrap().clone();

        // Create canonical string for HMAC
        let canonical = format!(
            "{}|{}|{}|{}|{}|{}",
            current_seq, ts, event, msg_id, from, to
        );

        // Calculate HMAC(key, prev_hash || canonical || meta)
        let mut mac = HmacSha256::new_from_slice(&self.key)?;
        mac.update(prev_hash.as_bytes());
        mac.update(canonical.as_bytes());
        mac.update(serde_json::to_string(&meta)?.as_bytes());
        let hash_bytes = mac.finalize().into_bytes();
        let current_hash = general_purpose::STANDARD.encode(hash_bytes);

        // Create journal entry
        let entry = JEntry {
            seq: current_seq,
            ts,
            ev: event.to_string(),
            msg_id: msg_id.to_string(),
            from: from.to_string(),
            to: to.to_string(),
            meta,
            prev: prev_hash,
            hash: current_hash.clone(),
        };

        // Write to file
        let line = serde_json::to_string(&entry)? + "\n";
        {
            let mut file = self.file.lock().unwrap();
            file.write_all(line.as_bytes())?;
            file.sync_all()?;
        }

        // Update last hash
        *self.last_hash.lock().unwrap() = current_hash.clone();

        Ok(current_hash)
    }

    /// Get current sequence number
    #[allow(dead_code)]
    pub fn current_seq(&self) -> u64 {
        *self.seq.lock().unwrap()
    }

    /// Get last hash
    #[allow(dead_code)]
    pub fn last_hash(&self) -> String {
        self.last_hash.lock().unwrap().clone()
    }
}

/// Journal verification utility
#[allow(dead_code)]
pub struct JournalVerifier {
    key: Vec<u8>,
}

impl JournalVerifier {
    #[allow(dead_code)]
    pub fn new(key: &[u8]) -> Self {
        Self { key: key.to_vec() }
    }

    /// Verify journal integrity
    #[allow(dead_code)]
    pub fn verify_file(&self, path: &str) -> Result<bool> {
        use std::io::{BufRead, BufReader};

        let file = File::open(path)?;
        let reader = BufReader::new(file);

        let mut prev_hash = String::new();
        let mut expected_seq = 1u64;

        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }

            let entry: JEntry = serde_json::from_str(&line)?;

            // Check sequence
            if entry.seq != expected_seq {
                return Ok(false);
            }

            // Check previous hash
            if entry.prev != prev_hash {
                return Ok(false);
            }

            // Verify HMAC
            let canonical = format!(
                "{}|{}|{}|{}|{}|{}",
                entry.seq, entry.ts, entry.ev, entry.msg_id, entry.from, entry.to
            );

            let mut mac = HmacSha256::new_from_slice(&self.key)?;
            mac.update(prev_hash.as_bytes());
            mac.update(canonical.as_bytes());
            mac.update(serde_json::to_string(&entry.meta)?.as_bytes());
            let expected_hash = general_purpose::STANDARD.encode(mac.finalize().into_bytes());

            if entry.hash != expected_hash {
                return Ok(false);
            }

            prev_hash = entry.hash;
            expected_seq += 1;
        }

        Ok(true)
    }
}

fn now_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_journal_basic() -> Result<()> {
        let temp_file = NamedTempFile::new()?;
        let key = b"test-key-123";
        let journal = Journal::open(temp_file.path().to_str().unwrap(), key)?;

        // Add some entries
        let hash1 = journal.append(
            "DELEGATE",
            "task-123",
            "client:smart",
            "worker:code",
            serde_json::json!({"action": "test"}),
        )?;

        let hash2 = journal.append(
            "NOTIFY",
            "task-123",
            "worker:code",
            "client:smart",
            serde_json::json!({"status": "completed"}),
        )?;

        assert_ne!(hash1, hash2);
        assert_eq!(journal.current_seq(), 2);

        // Verify journal
        let verifier = JournalVerifier::new(key);
        assert!(verifier.verify_file(temp_file.path().to_str().unwrap())?);

        Ok(())
    }

    #[test]
    fn test_journal_tamper_detection() -> Result<()> {
        let temp_file = NamedTempFile::new()?;
        let key = b"test-key-123";
        let journal = Journal::open(temp_file.path().to_str().unwrap(), key)?;

        journal.append(
            "DELEGATE",
            "task-123",
            "client:smart",
            "worker:code",
            serde_json::json!({"action": "test"}),
        )?;

        // Tamper with file (add invalid entry)
        {
            let mut file = OpenOptions::new().append(true).open(temp_file.path())?;
            file.write_all(b"{\"seq\":2,\"hash\":\"invalid\"}\n")?;
        }

        // Verification should fail
        let verifier = JournalVerifier::new(key);
        assert!(!verifier.verify_file(temp_file.path().to_str().unwrap())?);

        Ok(())
    }
}
