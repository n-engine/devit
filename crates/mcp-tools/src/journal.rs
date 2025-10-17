use std::collections::{HashMap, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{SecondsFormat, Utc};
use mcp_core::{McpResult, McpTool};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::errors::{internal_error, io_error, validation_error};
use crate::file_read::FileSystemContext;

const DEFAULT_JOURNAL_RELATIVE_PATH: &str = ".devit/journal.jsonl";
const DEFAULT_JOURNAL_SECRET: &[u8] = b"devit-journal-secret";

pub struct JournalContext {
    journal_path: PathBuf,
    secret: Vec<u8>,
    state: Mutex<JournalState>,
}

struct JournalState {
    entries: VecDeque<Value>,
    offset_base: u64,
}

impl JournalContext {
    pub fn new(file_context: Arc<FileSystemContext>) -> McpResult<Self> {
        let journal_path = file_context.root().join(DEFAULT_JOURNAL_RELATIVE_PATH);

        let secret = std::env::var("DEVIT_JOURNAL_SECRET")
            .map(|value| value.into_bytes())
            .unwrap_or_else(|_| DEFAULT_JOURNAL_SECRET.to_vec());

        let existing = count_existing_entries(&journal_path)?;

        Ok(Self {
            journal_path,
            secret,
            state: Mutex::new(JournalState {
                entries: VecDeque::new(),
                offset_base: existing,
            }),
        })
    }

    pub fn append(
        &self,
        operation: &str,
        details: &HashMap<String, String>,
    ) -> McpResult<JournalAppendResult> {
        let timestamp = current_timestamp();
        let request_id = Uuid::new_v4();
        let entry = json!({
            "operation": operation,
            "timestamp": timestamp,
            "request_id": request_id.to_string(),
            "details": details,
        });

        let mut state = self
            .state
            .lock()
            .map_err(|_| internal_error("Journal context poisoned"))?;

        let offset = state.offset_base + state.entries.len() as u64;
        state.entries.push_back(entry.clone());

        let hmac = compute_hmac(&self.secret, &entry);
        drop(state);

        self.persist_entry(&entry, &hmac, offset, request_id)?;

        Ok(JournalAppendResult {
            hmac,
            offset,
            file: self.journal_path.clone(),
            request_id,
            timestamp,
        })
    }

    fn persist_entry(
        &self,
        entry: &Value,
        hmac: &str,
        offset: u64,
        request_id: Uuid,
    ) -> McpResult<()> {
        if let Some(parent) = self.journal_path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| {
                io_error("create journal directory", Some(parent), err.to_string())
            })?;
        }

        let signed_entry = json!({
            "entry": entry,
            "hmac": hmac,
            "offset": offset,
            "request_id": request_id.to_string(),
            "recorded_at": current_timestamp(),
        });

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.journal_path)
            .map_err(|err| {
                io_error(
                    "open journal file",
                    Some(&self.journal_path),
                    err.to_string(),
                )
            })?;

        writeln!(file, "{}", signed_entry).map_err(|err| {
            io_error(
                "write journal entry",
                Some(&self.journal_path),
                err.to_string(),
            )
        })?;

        Ok(())
    }
}

pub struct JournalAppendResult {
    pub hmac: String,
    pub offset: u64,
    pub file: PathBuf,
    pub request_id: Uuid,
    pub timestamp: String,
}

fn compute_hmac(secret: &[u8], entry: &Value) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    if let Ok(bytes) = serde_json::to_vec(entry) {
        bytes.hash(&mut hasher);
    }
    secret.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn count_existing_entries(path: &Path) -> McpResult<u64> {
    if !path.exists() {
        return Ok(0);
    }

    let file = File::open(path)
        .map_err(|err| io_error("open journal file", Some(path), err.to_string()))?;
    let reader = BufReader::new(file);

    let mut count = 0u64;
    for line in reader.lines() {
        match line {
            Ok(text) => {
                if !text.trim().is_empty() {
                    count += 1;
                }
            }
            Err(err) => {
                return Err(io_error("read journal file", Some(path), err.to_string()));
            }
        }
    }

    Ok(count)
}

fn current_timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub struct JournalAppendTool {
    context: Arc<JournalContext>,
}

impl JournalAppendTool {
    pub fn new(context: Arc<JournalContext>) -> Self {
        Self { context }
    }
}

#[async_trait]
impl McpTool for JournalAppendTool {
    fn name(&self) -> &str {
        "devit_journal_append"
    }

    fn description(&self) -> &str {
        "Add entries to DevIt audit journal"
    }

    async fn execute(&self, params: Value) -> McpResult<Value> {
        let operation = params
            .get("operation")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| validation_error("Le paramètre 'operation' est requis"))?;

        let details_value = params
            .get("details")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                validation_error("Le paramètre 'details' est requis et doit être un objet")
            })?;

        let details = details_value
            .iter()
            .map(|(key, value)| (key.clone(), value_to_string(value)))
            .collect::<HashMap<_, _>>();

        let result = self.context.append(operation, &details)?;

        let message = format!(
            "📝 Journal entry added successfully!\n\nOperation: {}\nTimestamp: {}\nDetails: {} entries",
            operation,
            result.timestamp,
            details.len()
        );

        Ok(json!({
            "content": [
                {
                    "type": "text",
                    "text": message
                }
            ]
        }))
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {"type": "string"},
                "details": {"type": "object"}
            },
            "required": ["operation", "details"]
        })
    }
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::tempdir;

    #[tokio::test]
    async fn journal_append_writes_entry() {
        let dir = tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let file_context = Arc::new(FileSystemContext::new(root).unwrap());
        let context = Arc::new(JournalContext::new(Arc::clone(&file_context)).unwrap());
        let tool = JournalAppendTool::new(context);

        let params = json!({
            "operation": "unit_test",
            "details": {"status": "ok"}
        });

        let response = tool.execute(params).await.unwrap();
        assert_eq!(response["content"][0]["type"], "text");
    }
}
