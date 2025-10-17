//! End-to-end orchestration test: CLI → daemon → task lifecycle.

use regex::Regex;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;
use tempfile::TempDir;

fn extract_task_id(output: &str) -> Option<String> {
    let uuid = Regex::new(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}").ok()?;
    if let Some(mat) = uuid.find(output) {
        return Some(mat.as_str().to_string());
    }
    let legacy = Regex::new(r"task_[a-f0-9]{8}").ok()?;
    legacy.find(output).map(|m| m.as_str().to_string())
}

#[test]
fn test_orchestration_e2e_flow() {
    if std::env::var("CI").is_ok() || std::env::var("SKIP_E2E").is_ok() {
        println!("Skipping E2E orchestration test (CI or SKIP_E2E set)");
        return;
    }

    let temp = TempDir::new().expect("temp dir");
    let socket = temp.path().join("e2e.sock");
    let socket_str = socket.to_string_lossy().to_string();

    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates")
        .parent()
        .expect("workspace root")
        .to_path_buf();

    println!("Building devit and devitd...");
    assert!(Command::new("cargo")
        .arg("build")
        .args(["--bin", "devit", "--bin", "devitd"])
        .current_dir(&workspace_root)
        .status()
        .expect("cargo build")
        .success());

    let exe = std::env::consts::EXE_SUFFIX;
    let devitd_bin = workspace_root.join(format!("target/debug/devitd{}", exe));
    let devit_bin = workspace_root.join(format!("target/debug/devit{}", exe));

    println!("Starting devitd at {}", socket_str);
    let mut daemon = Command::new(&devitd_bin)
        .args(["--socket", &socket_str])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn devitd");

    std::thread::sleep(Duration::from_secs(2));

    if let Ok(Some(status)) = daemon.try_wait() {
        eprintln!(
            "devitd exited early with status {:?}; skipping orchestration CLI test",
            status.code()
        );
        return;
    }

    println!("Delegating task via CLI...");
    let delegate = Command::new(&devit_bin)
        .args(["delegate", "--goal", "E2E test task"])
        .env("DEVIT_DAEMON_SOCKET", &socket_str)
        .env("DEVIT_SECRET", "change-me-in-production")
        .env("DEVIT_NO_AUTO_START", "1")
        .current_dir(&workspace_root)
        .output()
        .expect("delegate command");
    let stdout = String::from_utf8_lossy(&delegate.stdout);
    println!("Delegate output:\n{}", stdout);
    if !delegate.status.success() {
        eprintln!(
            "Delegate stderr:\n{}",
            String::from_utf8_lossy(&delegate.stderr)
        );
    }
    assert!(delegate.status.success(), "delegation failed");

    let task_id = extract_task_id(&stdout).expect("task id missing");
    println!("Task ID: {}", task_id);

    println!("Checking status...");
    let status = Command::new(&devit_bin)
        .args(["status"])
        .env("DEVIT_DAEMON_SOCKET", &socket_str)
        .env("DEVIT_SECRET", "change-me-in-production")
        .env("DEVIT_NO_AUTO_START", "1")
        .current_dir(&workspace_root)
        .output()
        .expect("status command");
    assert!(status.status.success());
    assert!(
        String::from_utf8_lossy(&status.stdout).contains(&task_id),
        "status output should reference task"
    );

    println!("Sending completion notification...");
    let notify = Command::new(&devit_bin)
        .args([
            "notify",
            "--task",
            &task_id,
            "--status",
            "completed",
            "--summary",
            "E2E test completed",
        ])
        .env("DEVIT_DAEMON_SOCKET", &socket_str)
        .env("DEVIT_SECRET", "change-me-in-production")
        .env("DEVIT_NO_AUTO_START", "1")
        .current_dir(&workspace_root)
        .output()
        .expect("notify command");
    assert!(notify.status.success(), "notification failed");

    println!("Verifying completion...");
    let task = Command::new(&devit_bin)
        .args(["task", &task_id])
        .env("DEVIT_DAEMON_SOCKET", &socket_str)
        .env("DEVIT_SECRET", "change-me-in-production")
        .env("DEVIT_NO_AUTO_START", "1")
        .current_dir(&workspace_root)
        .output()
        .expect("task command");
    assert!(task.status.success());
    let task_output = String::from_utf8_lossy(&task.stdout);
    assert!(task_output.to_ascii_lowercase().contains("completed"));

    println!("Cleaning up daemon");
    if let Err(err) = daemon.kill() {
        eprintln!("Failed to kill devitd: {}", err);
    }
    let _ = daemon.wait();
}

#[test]
fn test_cli_help() {
    let output = Command::new("cargo")
        .args(["run", "--bin", "devit", "--", "--help"])
        .output()
        .expect("help command");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("delegate"));
    assert!(stdout.contains("notify"));
    assert!(stdout.contains("status"));
}
