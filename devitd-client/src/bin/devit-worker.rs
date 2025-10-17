//! DevIt Worker Client
//!
//! Example "worker" AI client that receives and executes tasks.
//! Represents Claude Code or other execution AI.

use anyhow::Result;
use devitd_client::{DevitClient, DEFAULT_SOCK};
use serde_json::json;
use std::env;
use tokio::time::{sleep, Duration};
use tracing::{debug, error, info};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt().with_env_filter("debug").init();

    let ident = env::var("DEVIT_IDENT").unwrap_or_else(|_| "worker:code".to_string());

    let secret = env::var("DEVIT_SECRET").unwrap_or_else(|_| "change-me".to_string());

    let sock_path = env::var("DEVIT_SOCK").unwrap_or_else(|_| DEFAULT_SOCK.to_string());

    info!("Starting DevIt worker: {}", ident);

    // Connect to daemon
    let client = DevitClient::connect(&sock_path, &ident, &secret).await?;

    info!("Worker ready, waiting for tasks...");

    // Run heartbeat loop that handles incoming tasks
    client
        .run_heartbeat_loop(|msg| {
            if DevitClient::is_delegate(&msg) {
                info!("Received task delegation: {}", msg.msg_id);

                // Extract task details
                let task_id = msg.msg_id.clone();
                let return_to = DevitClient::extract_return_to(&msg)
                    .unwrap_or_else(|| "client:smart".to_string());

                if let Some(task) = DevitClient::extract_task(&msg) {
                    debug!("Task details: {}", task);

                    // Spawn task execution
                    let sock_path_clone = sock_path.clone();
                    let ident_clone = ident.clone();
                    let secret_clone = secret.clone();
                    let task_clone = task.clone();
                    tokio::spawn(async move {
                        let client = match DevitClient::connect(
                            &sock_path_clone,
                            &ident_clone,
                            &secret_clone,
                        )
                        .await
                        {
                            Ok(c) => c,
                            Err(e) => {
                                error!("Failed to connect for task execution: {}", e);
                                return;
                            }
                        };

                        // Execute the task
                        match execute_task(&task_clone).await {
                            Ok(artifacts) => {
                                info!("Task {} completed successfully", task_id);

                                // Send completion notification
                                if let Err(e) = client
                                    .notify(
                                        "orchestrator",
                                        &task_id,
                                        "completed",
                                        artifacts,
                                        Some(&return_to),
                                    )
                                    .await
                                {
                                    error!("Failed to send completion notification: {}", e);
                                }
                            }
                            Err(e) => {
                                error!("Task {} failed: {}", task_id, e);

                                // Send failure notification
                                let error_artifacts = json!({
                                    "error": e.to_string(),
                                    "timestamp": chrono::Utc::now().to_rfc3339()
                                });

                                if let Err(e) = client
                                    .notify(
                                        "orchestrator",
                                        &task_id,
                                        "failed",
                                        error_artifacts,
                                        Some(&return_to),
                                    )
                                    .await
                                {
                                    error!("Failed to send failure notification: {}", e);
                                }
                            }
                        }
                    });
                } else {
                    error!("No task found in delegation message");
                }
            } else if DevitClient::is_notify(&msg) {
                debug!("Received notification: {}", msg.msg_type);
            } else {
                debug!("Received other message: {}", msg.msg_type);
            }

            Ok(())
        })
        .await
}

/// Execute a task based on its specification
async fn execute_task(task: &serde_json::Value) -> Result<serde_json::Value> {
    let action = task
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    info!("Executing task action: {}", action);

    match action {
        "create_file" => execute_create_file(task).await,
        "patch_apply" => execute_patch_apply(task).await,
        "run_tests" => execute_run_tests(task).await,
        _ => {
            // Simulate generic work
            let timeout = task.get("timeout").and_then(|v| v.as_u64()).unwrap_or(5);

            info!("Simulating work for {} seconds", timeout);
            sleep(Duration::from_secs(timeout)).await;

            Ok(json!({
                "action": action,
                "status": "completed",
                "simulation": true,
                "duration_seconds": timeout
            }))
        }
    }
}

/// Execute file creation task
async fn execute_create_file(task: &serde_json::Value) -> Result<serde_json::Value> {
    let spec = task
        .get("spec")
        .ok_or_else(|| anyhow::anyhow!("Missing spec"))?;

    let path = spec
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing file path"))?;

    let content = spec
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing file content"))?;

    info!("Creating file: {}", path);

    // Simulate file creation work
    sleep(Duration::from_millis(500)).await;

    // Actually create the file for demo
    tokio::fs::write(path, content).await?;

    Ok(json!({
        "action": "create_file",
        "file_path": path,
        "file_size": content.len(),
        "created_at": chrono::Utc::now().to_rfc3339(),
        "success": true
    }))
}

/// Execute patch application task
async fn execute_patch_apply(_task: &serde_json::Value) -> Result<serde_json::Value> {
    info!("Applying patch...");

    // Simulate patch application
    sleep(Duration::from_secs(2)).await;

    Ok(json!({
        "action": "patch_apply",
        "files_modified": ["src/main.rs", "tests/integration.rs"],
        "lines_added": 42,
        "lines_removed": 15,
        "commit_sha": "abc123def456",
        "success": true
    }))
}

/// Execute test run task
async fn execute_run_tests(_task: &serde_json::Value) -> Result<serde_json::Value> {
    info!("Running tests...");

    // Simulate test execution
    sleep(Duration::from_secs(3)).await;

    Ok(json!({
        "action": "run_tests",
        "framework": "cargo",
        "tests_total": 127,
        "tests_passed": 125,
        "tests_failed": 2,
        "duration_ms": 3000,
        "coverage_percent": 94.2,
        "success": true
    }))
}
