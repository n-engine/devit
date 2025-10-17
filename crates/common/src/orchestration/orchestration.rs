use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Context, Result};
#[cfg(feature = "daemon")]
use devit_orchestration::DaemonBackend;
use devit_orchestration::{LocalBackend, OrchestrationBackend};
use serde_json::Value;
use tracing::{info, warn};

use super::types::{
    default_daemon_start_timeout_ms, DelegateResult, DelegatedTask, OrchestrationConfig,
    OrchestrationMode, OrchestrationStatus, StatusFilter, TaskStatus, DEFAULT_DAEMON_SOCKET,
};

pub struct OrchestrationContext {
    config: OrchestrationConfig,
    backend: Box<dyn OrchestrationBackend>,
    using_daemon: bool,
}

impl OrchestrationContext {
    pub async fn new(mut config: OrchestrationConfig) -> Result<Self> {
        if config.daemon_socket.is_none() {
            config.daemon_socket = Some(DEFAULT_DAEMON_SOCKET.to_string());
        }

        let auto_start = should_auto_start(&config);
        let timeout_ms = if config.daemon_start_timeout_ms == 0 {
            default_daemon_start_timeout_ms()
        } else {
            config.daemon_start_timeout_ms
        };

        config.auto_start_daemon = auto_start;
        config.daemon_start_timeout_ms = timeout_ms;

        let socket = config
            .daemon_socket
            .clone()
            .unwrap_or_else(|| DEFAULT_DAEMON_SOCKET.to_string());

        let (backend, using_daemon): (Box<dyn OrchestrationBackend>, bool) = match config.mode {
            OrchestrationMode::Local => (Box::new(LocalBackend::new(config.clone())), false),
            OrchestrationMode::Daemon => {
                #[cfg(feature = "daemon")]
                {
                    DaemonBackend::ensure_daemon_running(&socket, auto_start, timeout_ms).await?;
                    let backend = DaemonBackend::new(config.clone()).await?;
                    info!("Using devitd daemon backend at {}", socket);
                    (Box::new(backend), true)
                }
                #[cfg(not(feature = "daemon"))]
                {
                    warn!(
                        "devitd daemon support not compiled; falling back to local orchestration"
                    );
                    (Box::new(LocalBackend::new(config.clone())), false)
                }
            }
            OrchestrationMode::Auto => {
                #[cfg(feature = "daemon")]
                {
                    match DaemonBackend::ensure_daemon_running(&socket, auto_start, timeout_ms)
                        .await
                    {
                        Ok(_) => match DaemonBackend::new(config.clone()).await {
                            Ok(backend) => {
                                info!("Using devitd daemon backend (auto) at {}", socket);
                                (Box::new(backend), true)
                            }
                            Err(err) => {
                                warn!(
                                    "Failed to connect to devitd daemon: {}. Falling back to local orchestration",
                                    err
                                );
                                (Box::new(LocalBackend::new(config.clone())), false)
                            }
                        },
                        Err(err) => {
                            if auto_start {
                                warn!(
                                    "Failed to auto-start devitd daemon at {}: {}. Using local orchestration",
                                    socket,
                                    err
                                );
                            } else {
                                info!(
                                    "devitd daemon not available (auto_start disabled): {}. Using local orchestration",
                                    err
                                );
                            }
                            (Box::new(LocalBackend::new(config.clone())), false)
                        }
                    }
                }
                #[cfg(not(feature = "daemon"))]
                {
                    warn!(
                        "devitd daemon support not compiled; using local orchestration (auto mode)"
                    );
                    (Box::new(LocalBackend::new(config.clone())), false)
                }
            }
        };

        Ok(Self {
            config,
            backend,
            using_daemon,
        })
    }

    pub fn config(&self) -> &OrchestrationConfig {
        &self.config
    }

    pub fn is_using_daemon(&self) -> bool {
        self.using_daemon
    }

    pub async fn delegate(
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
        self.backend
            .delegate(
                goal,
                delegated_to,
                model,
                timeout,
                watch_patterns,
                context,
                working_dir,
                response_format,
            )
            .await
    }

    pub async fn notify(
        &self,
        task_id: &str,
        status: &str,
        summary: &str,
        details: Option<Value>,
        evidence: Option<Value>,
    ) -> Result<()> {
        self.backend
            .notify(task_id, status, summary, details, evidence)
            .await
    }

    pub async fn status(&self, filter: Option<&str>) -> Result<OrchestrationStatus> {
        let filter = StatusFilter::from_str(filter);
        self.backend.status(filter).await
    }

    pub async fn cleanup_expired(&self) -> Result<()> {
        self.backend.cleanup_expired().await
    }

    pub async fn task(&self, task_id: &str) -> Result<Option<DelegatedTask>> {
        self.backend.get_task(task_id).await
    }
}

fn should_auto_start(config: &OrchestrationConfig) -> bool {
    if std::env::var("DEVIT_NO_AUTO_START").is_ok() {
        return false;
    }
    if std::env::var("CI").is_ok() {
        return false;
    }
    config.auto_start_daemon
}

#[cfg(all(test, unix, feature = "daemon"))]
mod tests {
    use super::*;
    use devit_orchestration::DaemonBackend;
    use std::process::Command;
    use tempfile::tempdir;

    #[tokio::test(flavor = "multi_thread")]
    async fn auto_launch_daemon_backend() -> anyhow::Result<()> {
        if std::env::var("DEVIT_SKIP_DAEMON_TESTS").is_ok() || std::env::var("CI").is_ok() {
            return Ok(());
        }

        let binary = match DaemonBackend::find_devitd_binary() {
            Ok(path) => path,
            Err(err) => {
                eprintln!("skipping auto-launch test: {}", err);
                return Ok(());
            }
        };

        let previous_binary = std::env::var("DEVITD_BINARY").ok();
        std::env::set_var("DEVITD_BINARY", &binary);
        std::env::set_var("DEVIT_SECRET", "devit-auto-launch-test");

        let socket_dir = tempdir()?;
        let socket_path = socket_dir.path().join("devitd.sock");
        let socket = socket_path.to_string_lossy().to_string();

        let mut config = OrchestrationConfig::default();
        config.mode = OrchestrationMode::Auto;
        config.daemon_socket = Some(socket.clone());
        config.auto_start_daemon = true;
        config.daemon_start_timeout_ms = 5_000;

        let context = OrchestrationContext::new(config.clone()).await?;
        assert!(
            context.is_using_daemon(),
            "context should use daemon backend"
        );

        let result = context
            .delegate(
                "auto-launch smoke".to_string(),
                "worker:test".to_string(),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .await?;
        assert!(!result.task_id.is_empty());

        let _ = Command::new("pkill").arg("-f").arg(&socket).status();

        match previous_binary {
            Some(val) => std::env::set_var("DEVITD_BINARY", val),
            None => std::env::remove_var("DEVITD_BINARY"),
        }

        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub enum StatusFormat {
    Json,
    Compact,
    Table,
}

impl StatusFormat {
    pub fn parse(value: Option<&str>) -> Result<Self> {
        match value.unwrap_or("compact") {
            "json" => Ok(StatusFormat::Json),
            "compact" => Ok(StatusFormat::Compact),
            "table" => Ok(StatusFormat::Table),
            other => bail!(
                "Format invalide: {}. Formats supportés: json, compact, table",
                other
            ),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            StatusFormat::Json => "json",
            StatusFormat::Compact => "compact",
            StatusFormat::Table => "table",
        }
    }
}

pub fn format_status(status: &OrchestrationStatus, format: StatusFormat) -> Result<String> {
    match format {
        StatusFormat::Json => serde_json::to_string_pretty(status)
            .with_context(|| "Sérialisation JSON impossible".to_string()),
        StatusFormat::Compact => {
            let mut lines = Vec::new();
            lines.push(format!(
                "🎛️ Orchestration — actives: {}, terminées: {}, échouées: {}",
                status.summary.total_active,
                status.summary.total_completed,
                status.summary.total_failed
            ));

            if !status.active_tasks.is_empty() {
                lines.push("--- Tâches actives ---".to_string());
                for task in &status.active_tasks {
                    lines.push(format!(
                        "• {} → {} (objectif: {}…)",
                        task.id,
                        task.delegated_to,
                        task.goal.chars().take(40).collect::<String>()
                    ));
                }
            }

            if !status.completed_tasks.is_empty() {
                lines.push("--- Tâches terminées ---".to_string());
                for task in &status.completed_tasks {
                    lines.push(format!(
                        "• {} [{}]",
                        task.id,
                        match task.status {
                            TaskStatus::Completed => "✅",
                            TaskStatus::Failed => "❌",
                            TaskStatus::Cancelled => "🚫",
                            TaskStatus::Pending | TaskStatus::InProgress => "…",
                        }
                    ));
                }
            }

            Ok(lines.join("\n"))
        }
        StatusFormat::Table => {
            let mut table = String::from("task_id | status | delegated_to | créé | objectif\n");
            table.push_str("--------------------------------------------------------------\n");

            for task in status
                .active_tasks
                .iter()
                .chain(status.completed_tasks.iter())
            {
                table.push_str(&format!(
                    "{} | {:?} | {} | {} | {}\n",
                    task.id,
                    task.status,
                    task.delegated_to,
                    task.created_at.to_rfc3339(),
                    task.goal.chars().take(40).collect::<String>()
                ));
            }

            Ok(table)
        }
    }
}
