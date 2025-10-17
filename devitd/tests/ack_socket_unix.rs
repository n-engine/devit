#![cfg(unix)]

use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use devitd_client::DevitClient;
use tokio::process::Command;
use tokio::time::sleep;

fn find_devitd_binary() -> PathBuf {
    // Mirror the approach used in windows_pipes.rs but for Unix
    let exe = std::env::current_exe().expect("current_exe");
    // target/debug/deps/<test-bin>
    let target_dir = exe
        .parent()
        .and_then(|p| p.parent())
        .expect("target debug dir");
    let candidate = target_dir.join("devitd");
    if candidate.is_file() {
        return candidate;
    }
    // Fallback to workspace target
    target_dir
        .parent()
        .map(|p| p.join("debug").join("devitd"))
        .unwrap_or(candidate)
}

fn unique(name: &str) -> String {
    format!(
        "{}-{}-{}",
        name,
        std::process::id(),
        chrono::Utc::now().timestamp_millis()
    )
}

#[tokio::test]
async fn ack_socket_fallbacks_to_marker_and_preserves_status() {
    let secret = "test-secret";
    let sock = format!("/tmp/{}.sock", unique("devitd-acktest"));

    // Prepare a simple hook script that records DEVIT_ACK_* to a known dir
    let out_dir = PathBuf::from(format!("/tmp/{}", unique("devit-ack-out")));
    fs::create_dir_all(&out_dir).expect("create out dir");
    let hook_path = out_dir.join("notify_hook.sh");
    let script = format!(
        "#!/usr/bin/env bash\nset -euo pipefail\nmkdir -p '{out}'\nprintenv DEVIT_ACK_MARKER > '{out}/ack_marker_path' || true\nprintenv DEVIT_ACK_SOCKET > '{out}/ack_socket_path' || true\n# do not block; just exit 0\n",
        out = out_dir.display()
    );
    fs::write(&hook_path, script).expect("write hook script");
    let _ = std::fs::set_permissions(&hook_path, fs::Permissions::from_mode(0o755));

    // Spawn daemon
    let devitd_path = find_devitd_binary();
    let mut child = Command::new(&devitd_path)
        .arg("--socket")
        .arg(&sock)
        .arg("--secret")
        .arg(secret)
        .arg("--debug")
        .env("DEVIT_NOTIFY_HOOK", &hook_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn devitd");

    // Wait until daemon ready
    let mut connected = false;
    for _ in 0..50 {
        // up to ~5s
        match DevitClient::connect(&sock, "client-a", secret).await {
            Ok(_) => {
                connected = true;
                break;
            }
            Err(_) => sleep(Duration::from_millis(100)).await,
        }
    }
    assert!(connected, "daemon should accept connections");

    // Create clients
    let client = DevitClient::connect(&sock, "client-a", secret)
        .await
        .expect("connect client-a");
    let worker = DevitClient::connect(&sock, "worker-a", secret)
        .await
        .expect("connect worker-a");

    // Delegate a task to worker-a
    let task = serde_json::json!({"goal":"ack socket test","timeout":30});
    let task_id = client
        .delegate("worker-a", task, "client-a")
        .await
        .expect("delegate task");

    // Worker completes the task (this will trigger the notify hook and record ACK marker/socket)
    worker
        .notify(
            "orchestrator",
            &task_id,
            "completed",
            serde_json::json!({"summary":"ok"}),
            None,
        )
        .await
        .expect("notify completed");

    // Read recorded marker path from hook output
    let marker_path_file = out_dir.join("ack_marker_path");
    let mut marker_path = String::new();
    for _ in 0..50 {
        // wait up to ~5s for hook to run
        if marker_path_file.is_file() {
            let mut f = fs::File::open(&marker_path_file).expect("open marker path file");
            f.read_to_string(&mut marker_path)
                .expect("read marker path");
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }
    let marker_path = marker_path.trim().to_string();
    assert!(
        !marker_path.is_empty(),
        "hook should record DEVIT_ACK_MARKER path"
    );

    // Send ACK (no socket consumer connected -> daemon should fall back to touching marker file)
    client
        .notify("orchestrator", &task_id, "ack", serde_json::json!({}), None)
        .await
        .expect("send ack");

    // Marker file should now exist and contain 'ack'
    for _ in 0..50 {
        if std::path::Path::new(&marker_path).is_file() {
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }
    let content = fs::read(&marker_path).expect("read marker");
    assert_eq!(content, b"ack", "daemon should write 'ack' to marker");

    // Ensure task remains completed (ACK must not mutate status)
    let snapshot = client.status_snapshot().await.expect("status");
    let payload = snapshot.expect("status response").payload;
    let total_completed = payload
        .get("summary")
        .and_then(|s| s.get("total_completed"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    assert!(
        total_completed >= 1,
        "completed tasks should remain completed after ACK"
    );

    // Cleanup
    let _ = child.kill().await;
}
