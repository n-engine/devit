use std::io::{BufRead, BufReader, Write};
use std::process::Stdio;
use std::time::Duration;

use tempfile::tempdir;

mod fixtures;

use fixtures::mcp_server_path;

#[test]
fn devit_mcpd_m1_applies_patch_in_working_dir() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let working_dir = temp.path();
    let file_path = working_dir.join("hello.txt");
    std::fs::write(&file_path, "old\n")?;

    let diff = r#"diff --git a/hello.txt b/hello.txt
index e69de29..4b825dc 100644
--- a/hello.txt
+++ b/hello.txt
@@ -1 +1 @@
-old
+new
"#;

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
        "{{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{{\"name\":\"devit_patch_apply\",\"arguments\":{{\"diff\":{diff:?}}}}}}}"
    )?;
    stdin.flush()?;

    let mut responses = 0;
    let mut line = String::new();
    while responses < 2 {
        line.clear();
        let bytes_read = reader.read_line(&mut line)?;
        if bytes_read == 0 {
            break;
        }
        if !line.trim().is_empty() {
            responses += 1;
        }
    }

    std::thread::sleep(Duration::from_millis(200));

    let content = std::fs::read_to_string(&file_path)?;
    assert_eq!(content.trim_end(), "new");

    let _ = child.kill();
    let _ = child.wait();

    Ok(())
}
