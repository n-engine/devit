use std::fs::File;
use std::io::Write;
use std::panic::{self, AssertUnwindSafe};
use std::sync::mpsc;
use std::time::Duration;

fn with_timeout<F, R>(duration: Duration, f: F) -> R
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = panic::catch_unwind(AssertUnwindSafe(f));
        let _ = tx.send(result);
    });

    match rx.recv_timeout(duration) {
        Ok(Ok(output)) => output,
        Ok(Err(err)) => panic::resume_unwind(err),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            panic!("test timed out after {:?}", duration)
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            panic!("test worker disconnected without signalling completion")
        }
    }
}

#[test]
fn missing_journal_exits_1() {
    with_timeout(Duration::from_secs(5), || {
        let mut cmd = assert_cmd::Command::cargo_bin("devit-tui").unwrap();
        cmd.env("DEVIT_TUI_HEADLESS", "1");
        cmd.timeout(Duration::from_secs(5));
        let assert = cmd
            .arg("--journal-path")
            .arg("/no/such/file.jsonl")
            .assert();
        let output = assert.get_output();
        assert!(output.status.code().unwrap_or(0) != 0);
        let err = String::from_utf8_lossy(&output.stderr);
        assert!(err.contains("journal_not_found"));
    });
}

#[test]
fn start_with_journal_and_quit() {
    with_timeout(Duration::from_secs(5), || {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("journal.jsonl");
        let mut f = File::create(&p).unwrap();
        for i in 0..10 {
            writeln!(f, "{{\"type\":\"test\",\"n\":{}}}", i).unwrap();
        }
        let mut cmd = assert_cmd::Command::cargo_bin("devit-tui").unwrap();
        cmd.env("DEVIT_TUI_HEADLESS", "1");
        cmd.timeout(Duration::from_secs(5));
        cmd.arg("--journal-path").arg(&p);
        cmd.assert().success();
    });
}

#[test]
fn open_diff_headless_from_file() {
    with_timeout(Duration::from_secs(5), || {
        let dir = tempfile::tempdir().unwrap();
        let diff_path = dir.path().join("sample.diff");
        let mut f = File::create(&diff_path).unwrap();
        writeln!(
            f,
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,2 @@\n-old\n+new"
        )
        .unwrap();

        let mut cmd = assert_cmd::Command::cargo_bin("devit-tui").unwrap();
        cmd.env("DEVIT_TUI_HEADLESS", "1");
        cmd.timeout(Duration::from_secs(5));
        cmd.arg("--open-diff").arg(&diff_path);
        cmd.assert().success();
    });
}

#[test]
fn open_diff_headless_from_stdin() {
    with_timeout(Duration::from_secs(5), || {
        let diff = "diff --git a/foo b/foo\n--- a/foo\n+++ b/foo\n@@ -1 +1 @@\n-old\n+new\n";
        let mut cmd = assert_cmd::Command::cargo_bin("devit-tui").unwrap();
        cmd.env("DEVIT_TUI_HEADLESS", "1");
        cmd.timeout(Duration::from_secs(5));
        cmd.arg("--open-diff").arg("-");
        cmd.write_stdin(diff);
        cmd.assert().success();
    });
}

#[test]
fn open_diff_missing_file_reports_error() {
    with_timeout(Duration::from_secs(5), || {
        let mut cmd = assert_cmd::Command::cargo_bin("devit-tui").unwrap();
        cmd.env("DEVIT_TUI_HEADLESS", "1");
        cmd.timeout(Duration::from_secs(5));
        cmd.arg("--open-diff").arg("/no/such/diff.patch");
        let assert = cmd.assert().failure();
        let output = assert.get_output();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("diff_load_failed"));
        assert!(stderr.contains("not_found"));
        assert_eq!(output.status.code(), Some(2));
    });
}
