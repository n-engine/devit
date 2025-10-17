use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

use crate::types::{DelegateResult, DelegatedTask, OrchestrationStatus, StatusFilter};

#[async_trait]
pub trait OrchestrationBackend: Send + Sync {
    async fn delegate(
        &self,
        goal: String,
        delegated_to: String,
        model: Option<String>,
        timeout: Option<Duration>,
        watch_patterns: Option<Vec<String>>,
        context: Option<Value>,
        working_dir: Option<PathBuf>,
        response_format: Option<String>,
    ) -> Result<DelegateResult>;

    async fn notify(
        &self,
        task_id: &str,
        status: &str,
        summary: &str,
        details: Option<Value>,
        evidence: Option<Value>,
    ) -> Result<()>;

    async fn status(&self, filter: StatusFilter) -> Result<OrchestrationStatus>;

    async fn cleanup_expired(&self) -> Result<()>;

    async fn get_task(&self, task_id: &str) -> Result<Option<DelegatedTask>>;
}
