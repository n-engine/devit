use chrono::Utc;
use serde_json::Value;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

fn journal_path() -> PathBuf {
    // Keep it simple for S1: project-local .devit/journal.jsonl
    let p = PathBuf::from(".devit/journal.jsonl");
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    p
}

pub fn append(op: &str, meta: &Value) {
    // Non-blocking best effort: ignore all errors
    let line = serde_json::json!({
        "timestamp": Utc::now().to_rfc3339(),
        "op": op,
        "meta": meta,
    });
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(journal_path()) {
        let _ = writeln!(file, "{}", line.to_string());
    }
}

