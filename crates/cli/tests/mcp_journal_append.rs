use std::io::{BufRead, BufReader, Write};
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use tempfile::tempdir;

mod fixtures;

use fixtures::mcp_server_path;

#[test]
fn devit_mcpd_m1_appends_journal_in_working_dir() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let working_dir = temp.path();

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

    writeln!(
        stdin,
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{{}}}}"
    )?;
    writeln!(
        stdin,
        "{{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{{\"name\":\"devit_journal_append\",\"arguments\":{{\"operation\":\"test_op\",\"details\":{{\"k\":\"v\"}}}}}}}}"
    )?;
    stdin.flush()?;

    let mut responses = 0;
    let mut line = String::new();
    let mut journal_response: Option<Value> = None;
    while responses < 2 {
        line.clear();
        let bytes_read = reader.read_line(&mut line)?;
        if bytes_read == 0 {
            break;
        }
        if !line.trim().is_empty() {
            responses += 1;
            if let Ok(value) = serde_json::from_str::<Value>(line.trim()) {
                if value["id"] == 2 {
                    // response to tools/call
                    journal_response = Some(value);
                }
            }
        }
    }

    std::thread::sleep(Duration::from_millis(200));

    let response = journal_response.expect("journal response");
    assert!(response["error"].is_null(), "{:?}", response);
    let metadata = response["result"]["metadata"]
        .as_object()
        .expect("metadata object");
    let journal_path = metadata["journal_path"].as_str().expect("journal path");
    let journal_file = std::path::Path::new(journal_path);
    assert!(journal_file.exists());
    let content = std::fs::read_to_string(journal_file)?;
    assert!(content.contains("test_op"));

    let _ = child.kill();
    let _ = child.wait();

    Ok(())
}
