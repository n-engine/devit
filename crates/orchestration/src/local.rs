use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;
use uuid::Uuid;

use crate::backend::OrchestrationBackend;
use crate::types::{
    DelegateResult, DelegatedTask, OrchestrationConfig, OrchestrationStatus, OrchestrationSummary,
    StatusFilter, TaskNotification, TaskStatus,
};

#[derive(Default)]
struct OrchestrationState {
    active_tasks: HashMap<String, DelegatedTask>,
    completed_tasks: HashMap<String, DelegatedTask>,
    last_cleanup: Option<Instant>,
}

pub struct LocalBackend {
    config: OrchestrationConfig,
    state: Mutex<OrchestrationState>,
}

impl LocalBackend {
    pub fn new(config: OrchestrationConfig) -> Self {
        Self {
            config,
            state: Mutex::new(OrchestrationState::default()),
        }
    }

    fn remove_inactive_tasks(state: &mut OrchestrationState) {
        if let Some(last_cleanup) = state.last_cleanup {
            if last_cleanup.elapsed() < Duration::from_secs(30) {
                return;
            }
        }

        state.last_cleanup = Some(Instant::now());

        let threshold = Utc::now()
            .checked_sub_signed(chrono::Duration::seconds(2 * 60 * 60))
            .unwrap_or_else(Utc::now);

        state
            .completed_tasks
            .retain(|_, task| task.last_activity > threshold);
    }
}

#[async_trait]
impl OrchestrationBackend for LocalBackend {
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
    ) -> Result<DelegateResult> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow!("orchestration context poisoned"))?;

        if state.active_tasks.len() >= self.config.max_concurrent_tasks {
            bail!(
                "Nombre maximum de tâches déléguées atteint, veuillez en clôturer avant d'en créer de nouvelles"
            );
        }

        let task_id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let timeout_secs = timeout
            .map(|d| d.as_secs())
            .unwrap_or(self.config.default_timeout_secs);

        let task = DelegatedTask {
            id: task_id.clone(),
            goal,
            delegated_to,
            created_at: now,
            timeout_secs,
            status: TaskStatus::Pending,
            context,
            watch_patterns: watch_patterns
                .unwrap_or_else(|| self.config.default_watch_patterns.clone()),
            last_activity: now,
            notifications: Vec::new(),
            working_dir,
            response_format,
            model,
            model_resolved: None,
        };

        state.active_tasks.insert(task_id.clone(), task);

        Ok(DelegateResult {
            task_id,
            timeout_secs,
        })
    }

    async fn notify(
        &self,
        task_id: &str,
        status: &str,
        summary: &str,
        details: Option<Value>,
        evidence: Option<Value>,
    ) -> Result<()> {
        // ACK is a UI/daemon handshake and must not mutate local orchestration state.
        if status.eq_ignore_ascii_case("ack") {
            return Ok(());
        }

        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow!("orchestration context poisoned"))?;
        let mut task = state
            .active_tasks
            .remove(task_id)
            .ok_or_else(|| anyhow!("Tâche inconnue: {task_id}"))?;

        let notification = TaskNotification {
            received_at: Utc::now(),
            status: status.to_string(),
            summary: summary.to_string(),
            details,
            evidence,
            auto_generated: false,
            metadata: None,
        };

        task.notifications.push(notification);
        task.last_activity = Utc::now();

        task.status = match status {
            "completed" => TaskStatus::Completed,
            "failed" => TaskStatus::Failed,
            "cancelled" => TaskStatus::Cancelled,
            _ => TaskStatus::InProgress,
        };

        if matches!(
            task.status,
            TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled
        ) {
            state.completed_tasks.insert(task_id.to_string(), task);
        } else {
            state.active_tasks.insert(task_id.to_string(), task);
        }

        Ok(())
    }

    async fn status(&self, filter: StatusFilter) -> Result<OrchestrationStatus> {
        let state = self
            .state
            .lock()
            .map_err(|_| anyhow!("orchestration context poisoned"))?;

        let active_tasks: Vec<_> = match filter {
            StatusFilter::Active => state.active_tasks.values().cloned().collect(),
            StatusFilter::Failed => state
                .completed_tasks
                .values()
                .filter(|task| matches!(task.status, TaskStatus::Failed))
                .cloned()
                .collect(),
            StatusFilter::Completed => Vec::new(),
            StatusFilter::All => state.active_tasks.values().cloned().collect(),
        };

        let completed_tasks: Vec<_> = match filter {
            StatusFilter::Completed => state
                .completed_tasks
                .values()
                .filter(|task| matches!(task.status, TaskStatus::Completed))
                .cloned()
                .collect(),
            StatusFilter::Active => Vec::new(),
            StatusFilter::All | StatusFilter::Failed => {
                state.completed_tasks.values().cloned().collect()
            }
        };

        let summary = OrchestrationSummary {
            total_active: state.active_tasks.len(),
            total_completed: state
                .completed_tasks
                .values()
                .filter(|task| matches!(task.status, TaskStatus::Completed))
                .count(),
            total_failed: state
                .completed_tasks
                .values()
                .filter(|task| matches!(task.status, TaskStatus::Failed))
                .count(),
            oldest_active_task: state
                .active_tasks
                .values()
                .min_by_key(|task| task.created_at)
                .map(|task| task.id.clone()),
        };

        Ok(OrchestrationStatus {
            active_tasks,
            completed_tasks,
            summary,
        })
    }

    async fn cleanup_expired(&self) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow!("orchestration context poisoned"))?;
        Self::remove_inactive_tasks(&mut state);
        Ok(())
    }

    async fn get_task(&self, task_id: &str) -> Result<Option<DelegatedTask>> {
        let state = self
            .state
            .lock()
            .map_err(|_| anyhow!("orchestration context poisoned"))?;

        if let Some(task) = state.active_tasks.get(task_id) {
            return Ok(Some(task.clone()));
        }

        if let Some(task) = state.completed_tasks.get(task_id) {
            return Ok(Some(task.clone()));
        }

        Ok(None)
    }
}
