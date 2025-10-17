#![cfg(all(unix, feature = "daemon"))]

use std::process::{Command, Stdio};
use std::time::Duration;

use devit_common::orchestration::OrchestrationStatus;
use devit_orchestration::daemon::DaemonBackend;
use devitd_client::DevitClient;
use tempfile::tempdir;
use tokio::time::sleep;

const SECRET: &str = "ack-stability-test-secret";

#[tokio::test(flavor = "multi_thread")]
async fn ack_does_not_change_completed_status() -> anyhow::Result<()> {
    // Locate daemon binary
    let binary = DaemonBackend::find_devitd_binary()?;

    // Prepare socket in a temp dir (unix domain socket)
    let dir = tempdir()?;
    let socket_path = dir.path().join("devitd.sock");
    let socket = socket_path.to_string_lossy().to_string();

    // Spawn daemon
    let mut child = Command::new(&binary)
        .arg("--socket")
        .arg(&socket)
        .arg("--secret")
        .arg(SECRET)
        .env(
            "RUST_LOG",
            std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()),
        )
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()?;

    // Give the daemon a moment to boot
    sleep(Duration::from_millis(300)).await;

    // Connect orchestrator and worker
    let orch_ident = "orch:ack-test";
    let worker_ident = "worker:ack-test";
    let orchestrator = DevitClient::connect(&socket, orch_ident, SECRET).await?;
    let worker = DevitClient::connect(&socket, worker_ident, SECRET).await?;

    // Delegate a minimal task to the worker
    let task_payload = serde_json::json!({
        "goal": "ack-stability",
        "timeout": 30
    });
    let task_id = orchestrator
        .delegate(worker_ident, task_payload, orchestrator.ident())
        .await?;

    // Worker sends a completion
    let artifacts = serde_json::json!({ "summary": "done" });
    worker
        .notify("orchestrator", &task_id, "completed", artifacts, None)
        .await?;

    // Request a snapshot and assert completed
    let status_msg = orchestrator.status_snapshot().await?;
    let snapshot = match status_msg {
        Some(msg) if msg.msg_type == "STATUS_RESPONSE" => {
            serde_json::from_value::<OrchestrationStatus>(msg.payload)?
        }
        _ => anyhow::bail!("missing status snapshot"),
    };
    let was_completed = snapshot.completed_tasks.iter().any(|t| {
        t.id == task_id
            && matches!(
                t.status,
                devit_common::orchestration::types::TaskStatus::Completed
            )
    });
    assert!(was_completed, "task should be completed before ACK");

    // Send ACK back (orchestrator side) and verify status doesn't change
    let ack_artifacts = serde_json::json!({});
    orchestrator
        .notify("orchestrator", &task_id, "ack", ack_artifacts, None)
        .await?;

    // Give daemon a tick
    sleep(Duration::from_millis(100)).await;

    // Snapshot again
    let status_msg2 = orchestrator.status_snapshot().await?;
    let snapshot2 = match status_msg2 {
        Some(msg) if msg.msg_type == "STATUS_RESPONSE" => {
            serde_json::from_value::<OrchestrationStatus>(msg.payload)?
        }
        _ => anyhow::bail!("missing status snapshot after ACK"),
    };
    let still_completed = snapshot2.completed_tasks.iter().any(|t| {
        t.id == task_id
            && matches!(
                t.status,
                devit_common::orchestration::types::TaskStatus::Completed
            )
    });
    assert!(still_completed, "ACK must not change completed status");

    // Cleanup
    let _ = child.kill();
    Ok(())
}
