use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use chrono::{SecondsFormat, Utc};
use devitd_client::{DevitClient, Msg};
use mcp_core::McpResult;
use serde_json::{json, Map, Value};
use tokio::sync::Mutex;
use tokio::time::sleep;
use tracing::{debug, info, warn};

use crate::errors::internal_error;
use devit_common::orchestration::types::default_daemon_start_timeout_ms;
use devit_orchestration::DaemonBackend;

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(20);
const POLL_WAIT_TIMEOUT: Duration = Duration::from_secs(3);
const POLL_SLEEP_STEP: Duration = Duration::from_millis(300);
const DEFAULT_SECRET: &str = "change-me-in-production";

#[derive(Clone, Default)]
pub struct ToolOptions {
    pub worker_bridge: Option<Arc<WorkerBridge>>,
    pub exec_config: Option<devit_cli::core::config::ExecToolConfig>,
    pub sandbox_root: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub enum WorkerEvent {
    Assignment(WorkerTask),
    Cancelled {
        task_id: String,
        reason: Option<String>,
    },
    Timeout {
        task_id: String,
    },
}

#[derive(Clone, Debug)]
pub struct WorkerTask {
    pub task_id: String,
    pub goal: Option<String>,
    pub timeout_secs: Option<u64>,
    pub watch_patterns: Vec<String>,
    pub context: Option<Value>,
    pub working_dir: Option<String>,
    pub absolute_working_dir: PathBuf,
    pub received_at: String,
    pub raw_task: Value,
}

pub struct WorkerBridge {
    client: Arc<DevitClient>,
    sandbox_root: PathBuf,
    worker_id: String,
    current_event: Mutex<Option<WorkerEvent>>,
}

impl WorkerBridge {
    pub async fn connect<P, S, T>(
        sandbox_root: PathBuf,
        socket: P,
        worker_id: S,
        secret: Option<T>,
    ) -> Result<Arc<Self>>
    where
        P: AsRef<Path>,
        S: Into<String>,
        T: Into<String>,
    {
        let worker_id = worker_id.into();
        let secret = secret
            .map(Into::into)
            .or_else(|| std::env::var("DEVIT_SECRET").ok())
            .unwrap_or_else(|| DEFAULT_SECRET.to_string());

        let socket_path = socket.as_ref();
        let socket_str = socket_path.to_string_lossy().to_string();

        let client_version = env::var("DEVIT_CLIENT_VERSION")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| format!("mcp-server/{}", env!("CARGO_PKG_VERSION")));

        let auto_start_allowed =
            std::env::var("DEVIT_NO_AUTO_START").is_err() && std::env::var("CI").is_err();

        if auto_start_allowed {
            let timeout_ms = std::env::var("DEVIT_DAEMON_START_TIMEOUT_MS")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or_else(default_daemon_start_timeout_ms);

            if let Err(err) =
                DaemonBackend::ensure_daemon_running(&socket_str, true, timeout_ms).await
            {
                tracing::warn!(
                    %socket_str,
                    %worker_id,
                    "Impossible de lancer devitd automatiquement: {err}"
                );
                return Err(err);
            }
        }

        let client = DevitClient::connect_with_version(
            socket_path,
            &worker_id,
            &secret,
            Some(client_version.clone()),
        )
        .await?;

        if let Ok(expected_daemon) = env::var("DEVIT_EXPECTED_DAEMON_VERSION") {
            match client.daemon_version() {
                Some(actual) if actual == expected_daemon => {
                    info!(
                        %worker_id,
                        %actual,
                        "Daemon version validated"
                    );
                }
                Some(actual) => {
                    return Err(anyhow!(
                        "daemon version mismatch: expected '{}', got '{}'",
                        expected_daemon,
                        actual
                    ));
                }
                None => {
                    warn!(
                        %worker_id,
                        %expected_daemon,
                        "Daemon did not return version during registration"
                    );
                }
            }
        }

        let daemon_version = client.daemon_version().unwrap_or("unknown");
        info!(
            %worker_id,
            worker_version = %client_version,
            daemon_version = %daemon_version,
            "Registered worker with devitd"
        );

        let bridge = Arc::new(Self {
            client: Arc::new(client),
            sandbox_root,
            worker_id,
            current_event: Mutex::new(None),
        });

        bridge.spawn_heartbeat_loop();
        Ok(bridge)
    }

    fn spawn_heartbeat_loop(self: &Arc<Self>) {
        let bridge = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                match bridge.client.heartbeat().await {
                    Ok(Some(msg)) => {
                        if let Err(err) = bridge.process_message(msg).await {
                            warn!("worker heartbeat message processing failed: {}", err);
                        }
                    }
                    Ok(None) => {}
                    Err(err) => {
                        warn!("worker heartbeat failed: {}", err);
                    }
                }

                sleep(HEARTBEAT_INTERVAL).await;
            }
        });
    }

    pub async fn poll_event(&self, wait: bool) -> McpResult<Option<WorkerEvent>> {
        if let Some(event) = self.snapshot_event().await? {
            return Ok(Some(event));
        }

        let deadline = if wait {
            Some(Instant::now() + POLL_WAIT_TIMEOUT)
        } else {
            None
        };

        loop {
            match self.client.poll().await {
                Ok(Some(msg)) => {
                    self.process_message(msg).await?;
                    if let Some(event) = self.snapshot_event().await? {
                        return Ok(Some(event));
                    }
                }
                Ok(None) => {}
                Err(err) => {
                    return Err(internal_error(format!(
                        "Impossible de récupérer les tâches auprès du daemon: {err}"
                    )));
                }
            }

            match deadline {
                Some(limit) if Instant::now() < limit => {
                    sleep(POLL_SLEEP_STEP).await;
                }
                Some(_) => return Ok(None),
                None => return Ok(None),
            }
        }
    }

    pub async fn notify(
        &self,
        task_id: &str,
        status: &str,
        summary: &str,
        details: Option<Value>,
        evidence: Option<Value>,
    ) -> McpResult<()> {
        let artifacts = build_artifacts(summary, details.clone(), evidence.clone());
        self.client
            .notify("orchestrator", task_id, status, artifacts, None)
            .await
            .map_err(|err| {
                internal_error(format!("Impossible d'envoyer la notification: {err}"))
            })?;

        if is_terminal_status(status) {
            let mut guard = self.current_event.lock().await;
            if let Some(WorkerEvent::Assignment(task)) = guard.as_ref() {
                if task.task_id == task_id {
                    *guard = None;
                }
            }
        }

        Ok(())
    }

    async fn snapshot_event(&self) -> McpResult<Option<WorkerEvent>> {
        let mut guard = self.current_event.lock().await;
        let result = match guard.as_ref() {
            Some(WorkerEvent::Assignment(task)) => Some(WorkerEvent::Assignment(task.clone())),
            Some(WorkerEvent::Cancelled { .. }) | Some(WorkerEvent::Timeout { .. }) => guard.take(),
            None => None,
        };
        Ok(result)
    }

    async fn process_message(&self, msg: Msg) -> McpResult<()> {
        match msg.msg_type.as_str() {
            "DELEGATE" => self.handle_delegate(msg).await,
            "NOTIFY" => self.handle_notify(msg).await,
            other => {
                debug!("worker received unsupported message type: {}", other);
                Ok(())
            }
        }
    }

    async fn handle_delegate(&self, msg: Msg) -> McpResult<()> {
        if msg.to != self.worker_id {
            debug!(
                "ignoring delegation intended for {} (current worker: {})",
                msg.to, self.worker_id
            );
            return Ok(());
        }

        let task = self.build_task(&msg)?;
        info!("Tâche reçue pour {}: {}", self.worker_id, task.task_id);

        let mut guard = self.current_event.lock().await;
        *guard = Some(WorkerEvent::Assignment(task));
        Ok(())
    }

    async fn handle_notify(&self, msg: Msg) -> McpResult<()> {
        let payload = &msg.payload;
        let task_id = payload
            .get("task_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let status = payload
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown");

        debug!(
            "worker received notify for task {} with status {}",
            task_id, status
        );

        match status {
            "cancelled" => {
                let reason = payload
                    .get("artifacts")
                    .and_then(|v| v.get("summary"))
                    .and_then(Value::as_str)
                    .map(|s| s.to_string());
                let mut guard = self.current_event.lock().await;
                *guard = Some(WorkerEvent::Cancelled { task_id, reason });
            }
            "timeout" => {
                let mut guard = self.current_event.lock().await;
                *guard = Some(WorkerEvent::Timeout { task_id });
            }
            _ => {}
        }

        Ok(())
    }

    fn build_task(&self, msg: &Msg) -> McpResult<WorkerTask> {
        let task_payload = msg
            .payload
            .get("task")
            .and_then(Value::as_object)
            .ok_or_else(|| internal_error("Payload de tâche invalide reçu du daemon"))?;

        let task_id = msg.msg_id.clone();
        let goal = task_payload
            .get("goal")
            .and_then(Value::as_str)
            .map(|s| s.to_string());
        let timeout_secs = task_payload.get("timeout").and_then(Value::as_u64);
        let watch_patterns = task_payload
            .get("watch_patterns")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let context = task_payload.get("context").cloned();
        let working_dir = task_payload
            .get("working_dir")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let absolute_working_dir = working_dir
            .as_ref()
            .map(|rel| self.sandbox_root.join(rel))
            .unwrap_or_else(|| self.sandbox_root.clone());

        let raw_task = Value::Object(task_payload.clone());

        Ok(WorkerTask {
            task_id,
            goal,
            timeout_secs,
            watch_patterns,
            context,
            working_dir,
            absolute_working_dir,
            received_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            raw_task,
        })
    }
}

fn build_artifacts(summary: &str, details: Option<Value>, evidence: Option<Value>) -> Value {
    let mut map = Map::new();
    map.insert("summary".into(), Value::String(summary.to_string()));
    if let Some(details) = details {
        map.insert("details".into(), details);
    }
    if let Some(evidence) = evidence {
        map.insert("evidence".into(), evidence);
    }
    map.insert(
        "reported_at".into(),
        Value::String(Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)),
    );
    Value::Object(map)
}

fn is_terminal_status(status: &str) -> bool {
    matches!(status, "completed" | "failed" | "cancelled")
}

pub struct PollTasksTool {
    worker: Arc<WorkerBridge>,
}

impl PollTasksTool {
    pub fn new(worker: Arc<WorkerBridge>) -> Self {
        Self { worker }
    }
}

#[async_trait::async_trait]
impl mcp_core::McpTool for PollTasksTool {
    fn name(&self) -> &str {
        "devit_poll_tasks"
    }

    fn description(&self) -> &str {
        "Récupère la prochaine tâche assignée au worker connecté au daemon"
    }

    async fn execute(&self, params: Value) -> McpResult<Value> {
        let wait = params.get("wait").and_then(Value::as_bool).unwrap_or(false);

        match self.worker.poll_event(wait).await? {
            Some(WorkerEvent::Assignment(task)) => {
                let payload = json!({
                    "status": "assigned",
                    "task_id": task.task_id,
                    "goal": task.goal,
                    "timeout_secs": task.timeout_secs,
                    "watch_patterns": task.watch_patterns,
                    "working_dir": task.working_dir,
                    "absolute_working_dir": task.absolute_working_dir.display().to_string(),
                    "received_at": task.received_at,
                    "context": task.context,
                    "raw_task": task.raw_task,
                });

                Ok(json!({
                    "content": [
                        {
                            "type": "text",
                            "text": "✅ Nouvelle tâche assignée par le daemon"
                        },
                        {
                            "type": "json",
                            "json": payload
                        }
                    ]
                }))
            }
            Some(WorkerEvent::Cancelled { task_id, reason }) => Ok(json!({
                "content": [
                    {
                        "type": "text",
                        "text": format!(
                            "⚠️ Tâche {} annulée par l'orchestrateur{}",
                            task_id,
                            reason
                                .as_ref()
                                .map(|r| format!(" : {}", r))
                                .unwrap_or_default()
                        )
                    },
                    {
                        "type": "json",
                        "json": json!({
                            "status": "cancelled",
                            "task_id": task_id,
                            "reason": reason
                        })
                    }
                ]
            })),
            Some(WorkerEvent::Timeout { task_id }) => Ok(json!({
                "content": [
                    {
                        "type": "text",
                        "text": format!("⏱️ Tâche {} expirée côté orchestrateur", task_id)
                    },
                    {
                        "type": "json",
                        "json": json!({
                            "status": "timeout",
                            "task_id": task_id
                        })
                    }
                ]
            })),
            None => Ok(json!({
                "content": [{
                    "type": "text",
                    "text": "📭 Aucune tâche disponible"
                }],
                "metadata": {
                    "status": "idle"
                }
            })),
        }
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "wait": {
                    "type": "boolean",
                    "description": "Attend quelques secondes pour une tâche si aucune n'est disponible immédiatement",
                    "default": false
                }
            },
            "additionalProperties": false
        })
    }
}
