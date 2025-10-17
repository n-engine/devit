//! DevIt Smart Client
//!
//! Example "smart" AI client that delegates tasks to workers.
//! Represents Claude Desktop or other orchestrating AI.

use anyhow::Result;
use devitd_client::{DevitClient, DEFAULT_SOCK};
use serde_json::json;
use std::env;
use tokio::time::{sleep, Duration};
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt().with_env_filter("info").init();

    let ident = env::var("DEVIT_IDENT").unwrap_or_else(|_| "client:smart".to_string());

    let secret = env::var("DEVIT_SECRET").unwrap_or_else(|_| "change-me".to_string());

    let sock_path = env::var("DEVIT_SOCK").unwrap_or_else(|_| DEFAULT_SOCK.to_string());

    info!("Starting DevIt smart client: {}", ident);

    // Connect to daemon
    let client = DevitClient::connect(&sock_path, &ident, &secret).await?;

    // Example task delegation that will trigger approval policy
    let task = json!({
        "action": "devit_patch_apply",
        "path": "/unknown/test.rs",  // Unknown path -> needs approval
        "diff": "--- a/test.rs\n+++ b/test.rs\n@@ -1 +1,2 @@\n+// Hello from DevIt\n fn main() {}"
    });

    info!("Delegating task to worker:code");
    let task_id = client.delegate("worker:code", task, &ident).await?;
    info!("Task delegated with ID: {}", task_id);

    // Wait for completion notification via heartbeat
    let mut task_completed = false;
    let mut attempts = 0;
    const MAX_ATTEMPTS: u32 = 60; // 5 minutes

    while !task_completed && attempts < MAX_ATTEMPTS {
        if let Some(notification) = client.heartbeat().await? {
            if DevitClient::is_notify(&notification) {
                info!("Received notification: {}", notification.msg_type);

                if let Some(status) = notification.payload.get("status").and_then(|v| v.as_str()) {
                    info!("Task status: {}", status);
                }

                if let Some(artifacts) = notification.payload.get("artifacts") {
                    info!("Artifacts: {}", artifacts);
                }

                if let Some(task_id_field) =
                    notification.payload.get("task_id").and_then(|v| v.as_str())
                {
                    if task_id_field == task_id {
                        info!("Task {} completed successfully!", task_id);
                        task_completed = true;
                    }
                }
            } else {
                info!("Received other message: {}", notification.msg_type);
            }
        }

        if !task_completed {
            sleep(Duration::from_secs(5)).await;
            attempts += 1;
        }
    }

    if !task_completed {
        error!("Task did not complete within timeout");
    }

    info!("Smart client shutting down");
    Ok(())
}
