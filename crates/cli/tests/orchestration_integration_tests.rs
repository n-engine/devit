use devit_cli::core::{config::CoreConfig, CoreEngine};
use std::time::Duration;

#[tokio::test]
async fn test_orchestration_integration_with_existing_systems() {
    let mut config = CoreConfig::default();
    let engine = CoreEngine::new(config).await.unwrap();

    // Test delegation qui utilise Agent existant
    let task_id = engine
        .orchestration_delegate(
            "implement feature X".to_string(),
            "claude_code".to_string(),
            None,
            Some(Duration::from_secs(300)),
            Some(vec!["*.rs".to_string()]),
            None,
            None,
            None,
        )
        .await
        .unwrap();

    assert!(!task_id.is_empty());

    // Test notification
    engine
        .orchestration_notify(
            task_id.clone(),
            "completed".to_string(),
            "Feature X implemented successfully".to_string(),
            None,
            Some(serde_json::json!({"tests_passed": true})),
        )
        .await
        .unwrap();

    // Test status avec format compression
    let status = engine
        .orchestration_status(
            &devit_cli::core::formats::OutputFormat::Compact,
            Some("completed".to_string()),
        )
        .await
        .unwrap();

    assert!(status.contains("Feature X"));
}

#[tokio::test]
async fn test_orchestration_task_lifecycle() {
    let config = CoreConfig::default();
    let engine = CoreEngine::new(config).await.unwrap();

    // Create task
    let task_id = engine
        .orchestration_delegate(
            "test task".to_string(),
            "claude_code".to_string(),
            None,
            Some(Duration::from_secs(60)),
            Some(vec!["*.rs".to_string(), "Cargo.toml".to_string()]),
            Some(serde_json::json!({"priority": "high"})),
            None,
            None,
        )
        .await
        .unwrap();

    // Check active status
    let active_status = engine
        .orchestration_status(
            &devit_cli::core::formats::OutputFormat::Json,
            Some("active".to_string()),
        )
        .await
        .unwrap();

    assert!(active_status.contains(&task_id));

    // Send progress notification
    engine
        .orchestration_notify(
            task_id.clone(),
            "progress".to_string(),
            "Working on implementation".to_string(),
            Some(serde_json::json!({"percentage": 50})),
            None,
        )
        .await
        .unwrap();

    // Complete task
    engine
        .orchestration_notify(
            task_id.clone(),
            "completed".to_string(),
            "Task completed successfully".to_string(),
            Some(serde_json::json!({"duration_ms": 5000})),
            Some(serde_json::json!({
                "files_changed": ["src/lib.rs", "tests/mod.rs"],
                "tests_passed": true,
                "compilation_success": true
            })),
        )
        .await
        .unwrap();

    // Check completed status
    let completed_status = engine
        .orchestration_status(
            &devit_cli::core::formats::OutputFormat::Json,
            Some("completed".to_string()),
        )
        .await
        .unwrap();

    assert!(completed_status.contains(&task_id));
    assert!(completed_status.contains("completed"));
}

#[tokio::test]
async fn test_orchestration_status_formats() {
    let config = CoreConfig::default();
    let engine = CoreEngine::new(config).await.unwrap();

    // Create a few tasks for testing formats
    let task1 = engine
        .orchestration_delegate(
            "task 1".to_string(),
            "claude_code".to_string(),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    let task2 = engine
        .orchestration_delegate(
            "task 2".to_string(),
            "cursor".to_string(),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    // Test JSON format
    let json_status = engine
        .orchestration_status(&devit_cli::core::formats::OutputFormat::Json, None)
        .await
        .unwrap();

    assert!(json_status.contains("active_tasks"));
    assert!(json_status.contains(&task1));
    assert!(json_status.contains(&task2));

    // Test compact format (should be shorter)
    let compact_status = engine
        .orchestration_status(&devit_cli::core::formats::OutputFormat::Compact, None)
        .await
        .unwrap();

    // Compact should be shorter than JSON
    assert!(compact_status.len() < json_status.len());

    // Test table format
    let table_status = engine
        .orchestration_status(&devit_cli::core::formats::OutputFormat::Table, None)
        .await
        .unwrap();

    assert!(table_status.contains("task_id|status|goal"));
    assert!(table_status.contains(&task1));
    assert!(table_status.contains(&task2));
}
