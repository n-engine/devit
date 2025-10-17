//! Basic task delegation with DevIt orchestration.
//! Run with: `cargo run --example basic_delegation`

use anyhow::Result;
use devit_common::orchestration::{OrchestrationConfig, OrchestrationContext};
use devit_orchestration::types::TaskStatus;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    println!("\n🚀 DevIt Orchestration – Basic Delegation");

    let ctx = OrchestrationContext::new(OrchestrationConfig::default()).await?;
    println!(
        "Mode: {}",
        if ctx.is_using_daemon() {
            "daemon"
        } else {
            "local"
        }
    );

    let result = ctx
        .delegate(
            "Create a Hello World REST API in Rust".into(),
            "claude_code".into(),
            None,
            Some(Duration::from_secs(300)),
            Some(vec!["*.rs".into(), "Cargo.toml".into()]),
            None,
            None,
            None,
        )
        .await?;

    println!("✅ Task delegated: {}", result.task_id);
    println!("Simulating completion in 2 seconds...");

    tokio::time::sleep(Duration::from_secs(2)).await;
    ctx.notify(
        &result.task_id,
        "completed",
        "Hello World API implemented",
        None,
        None,
    )
    .await?;

    let status = ctx.status(Some("all")).await?;
    if let Some(task) = status
        .completed_tasks
        .iter()
        .find(|t| t.id == result.task_id && t.status == TaskStatus::Completed)
    {
        println!("🎉 Task '{}' is {:?}", task.id, task.status);
    }

    Ok(())
}
