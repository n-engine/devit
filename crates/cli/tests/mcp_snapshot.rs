use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use tempfile::tempdir;

mod fixtures;

use fixtures::mcp_server_path;

#[test]
fn devit_mcpd_m1_creates_snapshot_file() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let working_dir = temp.path();
    let file_path = working_dir.join("note.txt");
    std::fs::write(&file_path, "hello snapshot\n")?;

    let mut child = std::process::Command::new(mcp_server_path())
        .arg("--working-dir")
        .arg(working_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;

    let mut stdin = child.stdin.take().expect("stdin available");
    let stdout = child.stdout.take().expect("stdout available");
    let mut reader = BufReader::new(stdout);

    let init_request = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    let snapshot_request = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"devit_snapshot","arguments":{"paths":["note.txt"]}}}"#;

    writeln!(stdin, "{}", init_request)?;
    writeln!(stdin, "{}", snapshot_request)?;
    stdin.flush()?;

    let mut responses = Vec::new();
    let mut line = String::new();
    while responses.len() < 2 {
        line.clear();
        let bytes_read = reader.read_line(&mut line)?;
        if bytes_read == 0 {
            break;
        }
        if !line.trim().is_empty() {
            responses.push(line.trim().to_string());
        }
    }

    assert_eq!(
        responses.len(),
        2,
        "expected initialize and snapshot responses"
    );

    let response: Value = serde_json::from_str(&responses[1])?;
    let metadata = response["result"]["metadata"]
        .as_object()
        .expect("metadata object");

    let snapshot_path = metadata
        .get("snapshot_path")
        .and_then(|v| v.as_str())
        .expect("snapshot_path field");
    assert!(Path::new(snapshot_path).exists(), "snapshot file missing");

    let checksum = metadata
        .get("checksum_blake3")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(!checksum.is_empty(), "checksum should not be empty");

    // Allow the manager to flush file contents before asserting
    std::thread::sleep(Duration::from_millis(100));

    let snapshot_bytes = std::fs::read(snapshot_path)?;
    let snapshot_json: Value = serde_json::from_slice(&snapshot_bytes)?;

    let files = snapshot_json["files"]
        .as_object()
        .expect("files map in snapshot");
    assert!(
        files.contains_key("note.txt"),
        "expected note.txt in snapshot"
    );

    let total_size = snapshot_json["total_size"].as_u64().unwrap_or(0);
    assert!(total_size > 0, "snapshot should record file sizes");

    let _ = child.kill();
    let _ = child.wait();

    Ok(())
}
