use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Error;
use devit_common::orchestration::format_status as common_format_status;
use devit_common::orchestration::TaskStatus as CommonTaskStatus;
pub use devit_common::orchestration::{
    DelegateResult, DelegatedTask, OrchestrationConfig, OrchestrationContext, OrchestrationStatus,
    OrchestrationSummary, StatusFormat, TaskNotification, TaskStatus,
};
use serde_json::Value;
use uuid::Uuid;

use crate::core::errors::{DevItError, DevItResult};

#[derive(Clone)]
pub struct OrchestrationManager {
    context: Arc<OrchestrationContext>,
}

impl OrchestrationManager {
    pub async fn new(config: OrchestrationConfig) -> DevItResult<Self> {
        let context = OrchestrationContext::new(config)
            .await
            .map_err(convert_error)?;
        Ok(Self {
            context: Arc::new(context),
        })
    }

    pub async fn create_task(
        &self,
        goal: String,
        delegated_to: String,
        model: Option<String>,
        timeout: Duration,
        watch_patterns: Vec<String>,
        context: Option<Value>,
        working_dir: Option<PathBuf>,
        response_format: Option<String>,
    ) -> DevItResult<String> {
        let result = self
            .context
            .delegate(
                goal,
                delegated_to,
                model,
                Some(timeout),
                Some(watch_patterns),
                context,
                working_dir,
                response_format,
            )
            .await
            .map_err(convert_error)?;

        Ok(result.task_id)
    }

    pub async fn receive_notification(
        &self,
        task_id: String,
        status: String,
        summary: String,
        details: Option<Value>,
        evidence: Option<Value>,
    ) -> DevItResult<()> {
        self.context
            .notify(
                task_id.as_str(),
                status.as_str(),
                summary.as_str(),
                details,
                evidence,
            )
            .await
            .map_err(convert_error)
    }

    pub async fn get_status(&self, filter: Option<&str>) -> DevItResult<OrchestrationStatus> {
        self.context.status(filter).await.map_err(convert_error)
    }

    pub fn is_using_daemon(&self) -> bool {
        self.context.is_using_daemon()
    }
}

pub fn format_status(status: &OrchestrationStatus, format: StatusFormat) -> DevItResult<String> {
    match format {
        StatusFormat::Compact => Ok(status_to_string(status)),
        _ => common_format_status(status, format).map_err(convert_error),
    }
}

pub fn status_to_string(status: &OrchestrationStatus) -> String {
    let summary = &status.summary;
    let mut lines = vec![format!(
        "🎛️ Orchestration — actives: {}, terminées: {}, échouées: {}",
        summary.total_active, summary.total_completed, summary.total_failed
    )];

    if !status.active_tasks.is_empty() {
        lines.push("--- Tâches actives ---".to_string());
        for task in &status.active_tasks {
            let dir = task
                .working_dir
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| ".".to_string());
            lines.push(format!(
                "• {} → {} ({}) [{}]",
                task.id,
                task.delegated_to,
                describe_task_status(&task.status),
                dir
            ));
        }
    }

    if !status.completed_tasks.is_empty() {
        lines.push("--- Tâches terminées ---".to_string());
        for task in &status.completed_tasks {
            let dir = task
                .working_dir
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| ".".to_string());
            lines.push(format!(
                "• {} [{}] [{}]",
                task.id,
                describe_task_status(&task.status),
                dir
            ));
        }
    }

    lines.join("\n")
}

fn describe_task_status(status: &CommonTaskStatus) -> &'static str {
    match status {
        CommonTaskStatus::Pending => "⏳ pending",
        CommonTaskStatus::InProgress => "▶️ in progress",
        CommonTaskStatus::Completed => "✅ completed",
        CommonTaskStatus::Failed => "❌ failed",
        CommonTaskStatus::Cancelled => "🛑 cancelled",
    }
}

fn convert_error(err: Error) -> DevItError {
    match err.downcast::<std::io::Error>() {
        Ok(io_err) => DevItError::io(None, "orchestration", io_err),
        Err(other) => DevItError::Internal {
            component: "orchestration".to_string(),
            message: other.to_string(),
            cause: None,
            correlation_id: Uuid::new_v4().to_string(),
        },
    }
}
