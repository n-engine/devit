use std::{
    collections::HashMap,
    fs,
    future::Future,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use chrono::{SecondsFormat, Utc};
use devitd_client::{DevitClient, Msg, DEFAULT_SOCK};
use serde_json::{Map, Value};
use tokio::sync::Mutex;
use tokio::time::{sleep, timeout, Duration as TokioDuration, Instant as TokioInstant};
use tracing::{debug, info, warn};

use crate::backend::OrchestrationBackend;
use crate::types::{
    default_daemon_start_timeout_ms, DelegateResult, DelegatedTask, OrchestrationConfig,
    OrchestrationMode, OrchestrationStatus, OrchestrationSummary, StatusFilter, TaskNotification,
    TaskStatus,
};

#[cfg(not(unix))]
use tokio::net::windows::named_pipe::ClientOptions;
#[cfg(not(unix))]
use tokio::net::TcpStream;
#[cfg(unix)]
use tokio::net::UnixStream;

const DEFAULT_SECRET: &str = "change-me-in-production";

struct DaemonState {
    active: HashMap<String, DelegatedTask>,
    completed: HashMap<String, DelegatedTask>,
    last_cleanup: Option<Instant>,
}

impl Default for DaemonState {
    fn default() -> Self {
        Self {
            active: HashMap::new(),
            completed: HashMap::new(),
            last_cleanup: None,
        }
    }
}

pub struct DaemonBackend {
    config: OrchestrationConfig,
    socket_path: String,
    ident: String,
    secret: String,
    client: Arc<Mutex<Option<Arc<DevitClient>>>>,
    state: Arc<Mutex<DaemonState>>,
}

impl DaemonBackend {
    pub async fn new(config: OrchestrationConfig) -> Result<Self> {
        let socket_path = config
            .daemon_socket
            .clone()
            .unwrap_or_else(|| DEFAULT_SOCK.to_string());
        let secret = std::env::var("DEVIT_SECRET").unwrap_or_else(|_| DEFAULT_SECRET.to_string());
        let ident = std::env::var("DEVIT_IDENT")
            .unwrap_or_else(|_| format!("devit-cli:{}", std::process::id()));

        let backend = Self {
            config,
            socket_path,
            ident,
            secret,
            client: Arc::new(Mutex::new(None)),
            state: Arc::new(Mutex::new(DaemonState::default())),
        };

        match backend.ensure_connected().await {
            Ok(_) => info!("Connected to devitd daemon at {}", backend.socket_path),
            Err(err) => {
                warn!("Unable to connect to devitd daemon: {}", err);
                return Err(err);
            }
        }

        Ok(backend)
    }

    pub async fn ensure_daemon_running(
        socket: &str,
        auto_start: bool,
        timeout_ms: u64,
    ) -> Result<()> {
        #[cfg(unix)]
        {
            if Self::ping_socket(socket).await.is_ok() {
                return Ok(());
            }

            if !auto_start {
                bail!("devitd daemon not running and auto_start is disabled");
            }

            info!("Auto-starting devitd daemon at {}", socket);

            if let Some(parent) = Path::new(socket).parent() {
                if !parent.exists() {
                    fs::create_dir_all(parent)?;
                }
            }

            let binary = Self::find_devitd_binary()?;
            let secret =
                std::env::var("DEVIT_SECRET").unwrap_or_else(|_| DEFAULT_SECRET.to_string());

            let mut cmd = Command::new(&binary);
            cmd.arg("--socket")
                .arg(socket)
                .arg("--secret")
                .arg(&secret)
                .env(
                    "RUST_LOG",
                    std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
                )
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .stdin(Stdio::null());

            let mut child = cmd
                .spawn()
                .with_context(|| format!("failed to spawn devitd binary at {}", binary))?;

            info!("Spawned devitd (pid={})", child.id());

            let timeout_ms = if timeout_ms == 0 {
                default_daemon_start_timeout_ms()
            } else {
                timeout_ms
            };
            let deadline = TokioInstant::now() + TokioDuration::from_millis(timeout_ms);

            loop {
                if Self::ping_socket(socket).await.is_ok() {
                    info!("devitd daemon ready at {}", socket);
                    return Ok(());
                }

                if let Some(status) = child.try_wait()? {
                    bail!(
                        "devitd daemon exited prematurely with status {} while starting",
                        status
                    );
                }

                if TokioInstant::now() >= deadline {
                    bail!("devitd daemon failed to start within {}ms", timeout_ms);
                }

                sleep(TokioDuration::from_millis(200)).await;
            }
        }

        #[cfg(not(unix))]
        {
            let is_pipe = Self::is_pipe_addr(socket);
            let addr = if is_pipe {
                socket.to_string()
            } else if socket.contains(':') {
                socket.to_string()
            } else {
                devitd_client::DEFAULT_SOCK.to_string()
            };

            if Self::ping_socket(&addr).await.is_ok() {
                return Ok(());
            }

            if !auto_start {
                bail!("devitd daemon not running and auto_start is disabled");
            }

            info!("Auto-starting devitd daemon at {}", addr);

            let binary = Self::find_devitd_binary()?;
            let secret =
                std::env::var("DEVIT_SECRET").unwrap_or_else(|_| DEFAULT_SECRET.to_string());

            let mut cmd = Command::new(&binary);
            cmd.arg("--socket")
                .arg(if is_pipe { socket } else { addr.as_str() })
                .arg("--secret")
                .arg(&secret)
                .env(
                    "RUST_LOG",
                    std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
                )
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .stdin(Stdio::null());

            let mut child = cmd
                .spawn()
                .with_context(|| format!("failed to spawn devitd binary at {}", binary))?;

            info!("Spawned devitd (pid={})", child.id());

            let timeout_ms = if timeout_ms == 0 {
                default_daemon_start_timeout_ms()
            } else {
                timeout_ms
            };
            let deadline = TokioInstant::now() + TokioDuration::from_millis(timeout_ms);

            loop {
                if Self::ping_socket(&addr).await.is_ok() {
                    info!("devitd daemon ready at {}", addr);
                    return Ok(());
                }

                if let Some(status) = child.try_wait()? {
                    bail!(
                        "devitd daemon exited prematurely with status {} while starting",
                        status
                    );
                }

                if TokioInstant::now() >= deadline {
                    bail!("devitd daemon failed to start within {}ms", timeout_ms);
                }

                sleep(TokioDuration::from_millis(200)).await;
            }
        }
    }

    pub fn find_devitd_binary() -> Result<String> {
        if let Ok(path) = std::env::var("DEVITD_BINARY") {
            if Path::new(&path).is_file() {
                debug!("Using devitd binary from DEVITD_BINARY: {}", path);
                return Ok(path);
            }
        }

        let mut candidates: Vec<PathBuf> = vec![
            PathBuf::from("./target/debug/devitd"),
            PathBuf::from("./target/release/devitd"),
        ];

        if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
            let dir = PathBuf::from(manifest_dir);
            candidates.push(dir.join("../../target/debug/devitd"));
            candidates.push(dir.join("../../target/release/devitd"));
        }

        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(parent) = exe_path.parent() {
                candidates.push(parent.join("devitd"));
                candidates.push(parent.join("devitd.exe"));
                candidates.push(parent.join("../devitd"));
                candidates.push(parent.join("../devitd.exe"));
                candidates.push(parent.join("bin/devitd"));
                candidates.push(parent.join("bin/devitd.exe"));
            }
        }

        for candidate in &candidates {
            if candidate.is_file() {
                let path = candidate
                    .canonicalize()
                    .unwrap_or_else(|_| candidate.clone())
                    .to_string_lossy()
                    .to_string();
                debug!("Found devitd binary at {}", path);
                return Ok(path);
            }
        }

        warn!(
            ?candidates,
            "devitd binary not found; install devitd or set DEVITD_BINARY"
        );

        #[cfg(unix)]
        {
            if let Ok(output) = Command::new("which").arg("devitd").output() {
                if output.status.success() {
                    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !path.is_empty() {
                        debug!("Found devitd binary in PATH at {}", path);
                        return Ok(path);
                    }
                }
            }
        }

        bail!(
            "devitd binary not found. Build it with `cargo build -p devitd` or set DEVITD_BINARY"
        );
    }

    #[cfg(unix)]
    async fn ping_socket(socket: &str) -> Result<()> {
        timeout(TokioDuration::from_millis(100), UnixStream::connect(socket))
            .await
            .map_err(|_| anyhow!("timeout connecting to devitd socket {}", socket))?
            .map_err(|err| anyhow!("{}", err))?;

        Ok(())
    }

    #[cfg(not(unix))]
    async fn ping_socket(socket: &str) -> Result<()> {
        if Self::is_pipe_addr(socket) {
            let name = Self::normalize_pipe_name(socket);
            // Named pipe client open is sync; wrap in timeout by spawning a task
            let fut = async move { ClientOptions::new().open(&name) };
            timeout(TokioDuration::from_millis(100), fut)
                .await
                .map_err(|_| anyhow!("timeout connecting to devitd pipe"))??;
            Ok(())
        } else {
            let addr = if socket.contains(':') {
                socket.to_string()
            } else {
                devitd_client::DEFAULT_SOCK.to_string()
            };
            timeout(TokioDuration::from_millis(100), TcpStream::connect(addr))
                .await
                .map_err(|_| anyhow!("timeout connecting to devitd tcp socket"))?
                .map_err(|err| anyhow!("{}", err))?;
            Ok(())
        }
    }

    #[cfg(not(unix))]
    fn is_pipe_addr(addr: &str) -> bool {
        let a = addr.trim();
        a.starts_with(r"\\.\pipe\") || a.starts_with("pipe:") || (!a.contains(':'))
    }

    #[cfg(not(unix))]
    fn normalize_pipe_name(addr: &str) -> String {
        let trimmed = addr.trim();
        if trimmed.starts_with(r"\\.\pipe\") || trimmed.starts_with("\\\\.\\pipe\\") {
            trimmed.to_string()
        } else if trimmed.starts_with("pipe:") {
            let name = &trimmed[5..];
            format!(r"\\.\pipe\{}", name)
        } else {
            format!(r"\\.\pipe\{}", trimmed)
        }
    }

    pub async fn ping(&self) -> Result<()> {
        let client = self.ensure_connected().await?;
        if let Err(err) = client.heartbeat().await {
            self.invalidate_client().await;
            return Err(err);
        }
        Ok(())
    }

    async fn ensure_connected(&self) -> Result<Arc<DevitClient>> {
        {
            let guard = self.client.lock().await;
            if let Some(client) = guard.as_ref() {
                return Ok(client.clone());
            }
        }

        let client_version = format!("devit-cli/{}", env!("CARGO_PKG_VERSION"));
        let client = Arc::new(
            DevitClient::connect_with_version(
                &self.socket_path,
                &self.ident,
                &self.secret,
                Some(client_version),
            )
            .await
            .with_context(|| {
                format!(
                    "failed to connect to devitd at {} (mode: {:?})",
                    self.socket_path, self.config.mode
                )
            })?,
        );
        let mut guard = self.client.lock().await;
        *guard = Some(client.clone());
        Ok(client)
    }

    async fn invalidate_client(&self) {
        let mut guard = self.client.lock().await;
        *guard = None;
    }

    async fn with_client<F, Fut, T>(&self, op: F) -> Result<T>
    where
        F: FnOnce(Arc<DevitClient>) -> Fut + Send,
        Fut: Future<Output = Result<T>> + Send,
        T: Send,
    {
        let client = self.ensure_connected().await?;
        match op(client.clone()).await {
            Ok(value) => Ok(value),
            Err(err) => {
                self.invalidate_client().await;
                Err(err)
            }
        }
    }

    async fn record_task(&self, task: DelegatedTask) {
        let mut state = self.state.lock().await;
        state.active.insert(task.id.clone(), task);
    }

    async fn record_notification(
        &self,
        task_id: &str,
        status: &str,
        summary: &str,
        details: Option<Value>,
        evidence: Option<Value>,
    ) -> Result<()> {
        let mut state = self.state.lock().await;
        let now = Utc::now();
        let notification = TaskNotification {
            received_at: now,
            status: status.to_string(),
            summary: summary.to_string(),
            details,
            evidence,
            auto_generated: false,
            metadata: None,
        };

        let mut task = match state.active.remove(task_id) {
            Some(mut task) => {
                task.notifications.push(notification);
                task.last_activity = now;
                task
            }
            None => state
                .completed
                .remove(task_id)
                .unwrap_or_else(|| DelegatedTask {
                    id: task_id.to_string(),
                    goal: summary.to_string(),
                    delegated_to: String::new(),
                    created_at: now,
                    timeout_secs: self.config.default_timeout_secs,
                    status: TaskStatus::Pending,
                    context: None,
                    watch_patterns: Vec::new(),
                    last_activity: now,
                    notifications: vec![notification],
                    working_dir: None,
                    response_format: None,
                    model: None,
                    model_resolved: None,
                }),
        };

        task.status = map_status(status);
        task.last_activity = now;
        if !task
            .notifications
            .last()
            .map(|n| n.summary == summary)
            .unwrap_or(false)
        {
            task.notifications.push(TaskNotification {
                received_at: now,
                status: status.to_string(),
                summary: summary.to_string(),
                details: None,
                evidence: None,
                auto_generated: true,
                metadata: None,
            });
        }

        match task.status {
            TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled => {
                state.completed.insert(task_id.to_string(), task);
            }
            _ => {
                state.active.insert(task_id.to_string(), task);
            }
        }

        Ok(())
    }

    async fn refresh_notifications(&self) -> Result<()> {
        let client = self.ensure_connected().await?;

        if let Some(snapshot) = client.status_snapshot().await? {
            if let Err(err) = self.handle_daemon_message(snapshot).await {
                self.invalidate_client().await;
                return Err(err);
            }
        }

        if let Err(err) = self.consume_notification(client.as_ref(), true).await {
            self.invalidate_client().await;
            return Err(err);
        }

        Ok(())
    }

    async fn consume_notification(&self, client: &DevitClient, heartbeat: bool) -> Result<()> {
        if heartbeat {
            if let Some(message) = client.heartbeat().await? {
                self.handle_daemon_message(message).await?;
            }
        }

        loop {
            match client.poll().await? {
                Some(message) => self.handle_daemon_message(message).await?,
                None => break,
            }
        }

        Ok(())
    }

    async fn handle_daemon_message(&self, message: Msg) -> Result<()> {
        if DevitClient::is_notify(&message) {
            self.process_notification(message).await
        } else if message.msg_type == "STATUS_RESPONSE" {
            self.process_snapshot(message).await
        } else if DevitClient::is_delegate(&message) {
            debug!("Ignoring delegate message addressed to {}", message.to);
            Ok(())
        } else {
            debug!("Ignoring daemon message type: {}", message.msg_type);
            Ok(())
        }
    }

    async fn process_notification(&self, msg: Msg) -> Result<()> {
        let payload = msg.payload;
        let task_id = payload
            .get("task_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing task_id in daemon notification"))?;
        let status = payload
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("progress");

        let artifacts = payload.get("artifacts").cloned().unwrap_or(Value::Null);

        let (summary, details, evidence) = extract_notification_parts(artifacts);

        self.record_notification(task_id, status, &summary, details, evidence)
            .await
    }

    async fn process_snapshot(&self, msg: Msg) -> Result<()> {
        let status: OrchestrationStatus = serde_json::from_value(msg.payload)
            .map_err(|err| anyhow!("invalid status snapshot from daemon: {}", err))?;

        let mut state = self.state.lock().await;
        state.active = status
            .active_tasks
            .into_iter()
            .map(|task| (task.id.clone(), task))
            .collect();
        state.completed = status
            .completed_tasks
            .into_iter()
            .map(|task| (task.id.clone(), task))
            .collect();

        Ok(())
    }

    async fn cleanup_state(&self) {
        let mut state = self.state.lock().await;
        let should_cleanup = state
            .last_cleanup
            .map(|instant| instant.elapsed() > Duration::from_secs(30))
            .unwrap_or(true);

        if !should_cleanup {
            return;
        }

        state.last_cleanup = Some(Instant::now());
        let threshold = Utc::now()
            .checked_sub_signed(chrono::Duration::hours(2))
            .unwrap_or_else(Utc::now);
        state
            .completed
            .retain(|_, task| task.last_activity > threshold);
    }
}

fn extract_notification_parts(artifacts: Value) -> (String, Option<Value>, Option<Value>) {
    match artifacts {
        Value::Object(map) => {
            let summary = map
                .get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or("daemon notification")
                .to_string();
            (
                summary,
                map.get("details").cloned(),
                map.get("evidence").cloned(),
            )
        }
        other => (other.to_string(), None, None),
    }
}

fn map_status(status: &str) -> TaskStatus {
    match status {
        "completed" => TaskStatus::Completed,
        "failed" => TaskStatus::Failed,
        "cancelled" => TaskStatus::Cancelled,
        "pending" => TaskStatus::Pending,
        _ => TaskStatus::InProgress,
    }
}

#[async_trait]
impl OrchestrationBackend for DaemonBackend {
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
        let timeout_secs = timeout
            .map(|d| d.as_secs())
            .unwrap_or(self.config.default_timeout_secs);
        let watch_patterns =
            watch_patterns.unwrap_or_else(|| self.config.default_watch_patterns.clone());

        let task_payload = assemble_task_payload(
            &goal,
            timeout_secs,
            &watch_patterns,
            model.clone(),
            context.clone(),
            working_dir.as_ref().map(|p| p.as_path()),
            response_format.as_deref(),
        );
        let delegated_to_for_daemon = delegated_to.clone();

        let task_id = self
            .with_client(move |client| {
                let payload = task_payload;
                async move {
                    client
                        .delegate(&delegated_to_for_daemon, payload, client.ident())
                        .await
                }
            })
            .await?;

        let now = Utc::now();
        let task = DelegatedTask {
            id: task_id.clone(),
            goal,
            delegated_to,
            created_at: now,
            timeout_secs,
            status: TaskStatus::Pending,
            context,
            watch_patterns,
            last_activity: now,
            notifications: Vec::new(),
            working_dir,
            response_format,
            model,
            model_resolved: None,
        };

        self.record_task(task).await;

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
        // Special-case ACK: forward to daemon for hook signaling but DO NOT mutate local state/cache.
        if status.eq_ignore_ascii_case("ack") {
            let artifacts = build_artifacts(summary, details.clone(), evidence.clone());
            let task_id_owned = task_id.to_string();
            self.with_client(move |client| {
                let payload = artifacts;
                async move {
                    client
                        .notify(
                            "orchestrator",
                            &task_id_owned,
                            status,
                            payload,
                            Some(client.ident()),
                        )
                        .await
                }
            })
            .await?;
            return Ok(());
        }

        let artifacts = build_artifacts(summary, details.clone(), evidence.clone());
        let task_id_owned = task_id.to_string();

        self.with_client(move |client| {
            let payload = artifacts;
            async move {
                client
                    .notify(
                        "orchestrator",
                        &task_id_owned,
                        status,
                        payload,
                        Some(client.ident()),
                    )
                    .await
            }
        })
        .await?;

        self.record_notification(task_id, status, summary, details, evidence)
            .await
    }

    async fn status(&self, filter: StatusFilter) -> Result<OrchestrationStatus> {
        if let Err(err) = self.refresh_notifications().await {
            warn!("Failed to refresh notifications from daemon: {}", err);
        }

        let state = self.state.lock().await;

        let active: Vec<_> = match filter {
            StatusFilter::Active => state.active.values().cloned().collect(),
            StatusFilter::Failed => state
                .completed
                .values()
                .filter(|task| matches!(task.status, TaskStatus::Failed))
                .cloned()
                .collect(),
            StatusFilter::Completed => Vec::new(),
            StatusFilter::All => state.active.values().cloned().collect(),
        };

        let completed: Vec<_> = match filter {
            StatusFilter::Completed => state
                .completed
                .values()
                .filter(|task| matches!(task.status, TaskStatus::Completed))
                .cloned()
                .collect(),
            StatusFilter::Active => Vec::new(),
            StatusFilter::All | StatusFilter::Failed => state.completed.values().cloned().collect(),
        };

        let summary = OrchestrationSummary {
            total_active: state.active.len(),
            total_completed: state
                .completed
                .values()
                .filter(|task| matches!(task.status, TaskStatus::Completed))
                .count(),
            total_failed: state
                .completed
                .values()
                .filter(|task| matches!(task.status, TaskStatus::Failed))
                .count(),
            oldest_active_task: state
                .active
                .values()
                .min_by_key(|task| task.created_at)
                .map(|task| task.id.clone()),
        };

        Ok(OrchestrationStatus {
            active_tasks: active,
            completed_tasks: completed,
            summary,
        })
    }

    async fn cleanup_expired(&self) -> Result<()> {
        self.cleanup_state().await;
        Ok(())
    }

    async fn get_task(&self, task_id: &str) -> Result<Option<DelegatedTask>> {
        {
            let state = self.state.lock().await;
            if let Some(task) = state.active.get(task_id) {
                return Ok(Some(task.clone()));
            }
            if let Some(task) = state.completed.get(task_id) {
                return Ok(Some(task.clone()));
            }
        }

        if let Err(err) = self.refresh_notifications().await {
            warn!("Failed to refresh notifications from daemon: {}", err);
        }

        let state = self.state.lock().await;
        if let Some(task) = state.active.get(task_id) {
            return Ok(Some(task.clone()));
        }
        if let Some(task) = state.completed.get(task_id) {
            return Ok(Some(task.clone()));
        }

        Ok(None)
    }
}

fn assemble_task_payload(
    goal: &str,
    timeout_secs: u64,
    watch_patterns: &[String],
    model: Option<String>,
    context: Option<Value>,
    working_dir: Option<&Path>,
    response_format: Option<&str>,
) -> Value {
    let mut task = Map::new();
    task.insert("action".into(), Value::String("devit_delegate".into()));
    task.insert("goal".into(), Value::String(goal.to_string()));
    task.insert("timeout".into(), Value::from(timeout_secs));
    if !watch_patterns.is_empty() {
        task.insert(
            "watch_patterns".into(),
            Value::Array(
                watch_patterns
                    .iter()
                    .map(|p| Value::String(p.clone()))
                    .collect(),
            ),
        );
    }
    if let Some(ctx) = context {
        task.insert("context".into(), ctx);
    }
    if let Some(model) = model {
        task.insert("model".into(), Value::String(model));
    }
    if let Some(dir) = working_dir {
        if let Some(rel_str) = dir.to_str() {
            task.insert(
                "working_dir".into(),
                Value::String(rel_str.replace('\\', "/")),
            );
        }
    }
    if let Some(format) = response_format {
        task.insert("format".into(), Value::String(format.to_string()));
    }

    Value::Object(task)
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

impl DaemonBackend {
    pub fn mode(&self) -> OrchestrationMode {
        self.config.mode
    }
}
