//! DevIt Orchestration Daemon
//!
//! Central orchestrator for AI-to-AI communication via Unix domain sockets.
//! Handles task delegation, worker registration, heartbeats, and notifications.

mod compact;
mod journal;
mod policy;
mod process_registry;
mod process_utils;
mod reaper;
mod worker_executor;

use anyhow::{Context, Result};
use base64::{engine::general_purpose, Engine as _};
use chrono::{SecondsFormat, Utc};
use clap::Parser;
use hmac::{Hmac, Mac};
use screenshots::image::ImageFormat;
use screenshots::Screen;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::{HashMap, VecDeque};
use std::env;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
#[cfg(not(unix))]
use std::{ffi::c_void, mem, ptr};
use tokio::fs;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
#[cfg(not(unix))]
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
#[cfg(not(unix))]
use tokio::net::{TcpListener, TcpStream};
#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};
use tokio::process::Command as TokioCommand;
use tokio::signal;
use tokio::sync::Mutex;
use tokio::task::spawn_blocking;
use tracing::{debug, error, info, warn};
use uuid::Uuid;
#[cfg(not(unix))]
use windows_sys::Win32::{
    Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL},
    Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
    },
    Security::{
        GetTokenInformation, TokenUser, SECURITY_ATTRIBUTES, TOKEN_ACCESS_MASK,
        TOKEN_INFORMATION_CLASS, TOKEN_QUERY, TOKEN_USER,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

use devit_common::orchestration::types::{
    CapabilityRateLimit, DelegatedTask, OrchestrationStatus, OrchestrationSummary,
    TaskNotification as OrchestrationTaskNotification, TaskStatus, DEFAULT_TIMEOUT_SECS,
};

use worker_executor::{
    load_worker_settings, ScreenshotBackend, ScreenshotSettings, TaskMetadata, WorkerConfig,
    WorkerExecutor, WorkerOutcome, WorkerSettings, WorkerStatus, WorkerTask,
};

type HmacSha256 = Hmac<Sha256>;

#[cfg(unix)]
const DEFAULT_SOCK: &str = "/tmp/devitd.sock";
#[cfg(not(unix))]
const DEFAULT_SOCK: &str = "127.0.0.1:60459";
const DEFAULT_SECRET: &str = "change-me-in-production";
const LEASE_TTL: Duration = Duration::from_secs(900); // 15 minutes
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_COMPLETED_TASKS: usize = 1000;
const LOG_SNIPPET_LIMIT: usize = 512;
const DAEMON_VERSION: &str = env!("CARGO_PKG_VERSION");

static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

#[derive(Parser, Debug)]
#[command(name = "devitd", version = "0.1.0")]
#[command(about = "DevIt orchestration daemon")]
struct Cli {
    /// Socket path (Unix) or TCP address (Windows)
    #[arg(long, default_value = DEFAULT_SOCK)]
    socket: PathBuf,

    /// HMAC secret key (or use DEVIT_SECRET env var)
    #[arg(long)]
    secret: Option<String>,

    /// Enable debug logging
    #[arg(long)]
    debug: bool,

    /// Path to devit core configuration (devit.core.toml)
    #[arg(long)]
    config: Option<PathBuf>,

    /// Shut down automatically after N seconds with no active clients (set 0 to disable)
    #[arg(long)]
    auto_shutdown_after: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Msg {
    pub msg_type: String, // REGISTER, HEARTBEAT, DELEGATE, NOTIFY, ACK, ERR
    pub msg_id: String,
    pub from: String,
    pub to: String,
    pub ts: u64,
    pub nonce: String,
    pub hmac: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct Client {
    ident: String,
    last_heartbeat: Instant,
    capabilities: Vec<String>,
    version: Option<String>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct Lease {
    task_id: String,
    assigned_to: String,
    original_from: String,
    deadline: Instant,
    return_to: Option<String>,
    working_dir: Option<String>,
    response_format: Option<String>,
    model: Option<String>,
    model_resolved: Option<String>,
}

#[derive(Debug, Clone)]
struct TaskDetails {
    goal: String,
    timeout_secs: u64,
    context: Option<serde_json::Value>,
    watch_patterns: Vec<String>,
    working_dir: Option<String>,
    response_format: Option<String>,
    model: Option<String>,
}

fn parse_task_details(task: Option<&serde_json::Value>, fallback_id: &str) -> TaskDetails {
    let goal = task
        .and_then(|t| t.get("goal"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("Task {} delegated", fallback_id));

    let timeout_secs = task
        .and_then(|t| t.get("timeout"))
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_TIMEOUT_SECS);

    let context = task.and_then(|t| t.get("context")).cloned();

    let watch_patterns = task
        .and_then(|t| t.get("watch_patterns"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|value| value.as_str().map(|s| s.to_string()))
                .collect::<Vec<String>>()
        })
        .unwrap_or_default();

    let working_dir = task
        .and_then(|t| t.get("working_dir"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let response_format = task
        .and_then(|t| t.get("format"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let model = task
        .and_then(|t| t.get("model"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    TaskDetails {
        goal,
        timeout_secs,
        context,
        watch_patterns,
        working_dir,
        response_format,
        model,
    }
}

struct State {
    clients: HashMap<String, Client>,
    leases: HashMap<String, Lease>,         // task_id -> lease
    expired_leases: HashMap<String, Lease>, // task_id -> lease preserved after timeout
    pending_notifications: HashMap<String, Vec<Msg>>, // client_id -> notifications
    pending_approvals: HashMap<String, PendingApproval>, // approval_id -> pending approval
    // Pending ACK markers for notification hooks waiting for confirmation
    ack_markers: HashMap<String, PathBuf>, // task_id -> marker file path
    #[cfg(unix)]
    ack_sockets: HashMap<String, PathBuf>, // task_id -> unix socket path
    #[cfg(unix)]
    ack_writers: HashMap<String, tokio::net::unix::OwnedWriteHalf>, // task_id -> accepted writer
    #[cfg(not(unix))]
    ack_pipe_servers: HashMap<String, tokio::net::windows::named_pipe::NamedPipeServer>, // task_id -> connected server
    secret: String,
    journal: journal::Journal,
    worker_configs: HashMap<String, WorkerConfig>,
    workspace_root: Option<PathBuf>,
    tasks_active: HashMap<String, DelegatedTask>,
    tasks_completed: HashMap<String, DelegatedTask>,
    notify_hook: Option<NotifyHook>,
    daemon_version: String,
    expected_worker_version: Option<String>,
    screenshot: ScreenshotControl,
    approver_target: String,
}

#[derive(Clone, Debug)]
struct NotifyHook {
    command: String,
}

type HookInvocation = (NotifyHook, NotificationHookContext);

#[derive(Serialize)]
struct NotificationHookContext {
    task_id: String,
    status: String,
    worker: String,
    return_to: String,
    summary: String,
    working_dir: Option<String>,
    details: Option<serde_json::Value>,
    evidence: Option<serde_json::Value>,
    timestamp: String,
    metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct PendingApproval {
    task_id: String,
    original_msg: Msg,
    tool: String,
    requested_at: Instant,
}

struct ScreenshotControl {
    enabled: bool,
    backend: ScreenshotBackend,
    format: String,
    output_dir: PathBuf,
    rate_limit: CapabilityRateLimit,
    history: VecDeque<Instant>,
    workspace_root: Option<PathBuf>,
}

impl ScreenshotControl {
    fn new(
        settings: ScreenshotSettings,
        rate_limit: CapabilityRateLimit,
        capability_enabled: bool,
        workspace_root: Option<PathBuf>,
    ) -> Self {
        let output_dir = settings
            .output_dir
            .clone()
            .unwrap_or_else(|| default_screenshot_directory(workspace_root.as_deref()));

        Self {
            enabled: settings.enabled && capability_enabled,
            backend: settings.backend,
            format: if settings.format.trim().is_empty() {
                "png".to_string()
            } else {
                settings.format.to_lowercase()
            },
            output_dir,
            rate_limit,
            history: VecDeque::new(),
            workspace_root,
        }
    }

    fn prepare_job(&mut self) -> anyhow::Result<ScreenshotJob> {
        if !self.enabled {
            anyhow::bail!("Screenshot capability is disabled by configuration");
        }

        let now = Instant::now();
        self.prune(now);

        if self.rate_limit.max_actions > 0
            && self.history.len() as u32 >= self.rate_limit.max_actions
        {
            let window = self.rate_limit.per_seconds.max(1);
            anyhow::bail!(
                "Screenshot rate limit exceeded (max {} per {}s)",
                self.rate_limit.max_actions,
                window
            );
        }

        std::fs::create_dir_all(&self.output_dir)?;

        let filename = format!(
            "screenshot-{}.{}",
            Utc::now().format("%Y%m%dT%H%M%S%.3fZ"),
            self.format
        );
        let candidate = self.output_dir.join(filename);

        if !self.is_allowed(&candidate) {
            anyhow::bail!(
                "Screenshot path {} is outside of the allowed sandbox",
                candidate.display()
            );
        }

        self.history.push_back(now);

        Ok(ScreenshotJob {
            path: candidate,
            backend: self.backend,
            format: self.format.clone(),
        })
    }

    fn prune(&mut self, now: Instant) {
        if self.rate_limit.max_actions == 0 || self.rate_limit.per_seconds == 0 {
            self.history.clear();
            return;
        }
        let window = Duration::from_secs(self.rate_limit.per_seconds.max(1));
        while let Some(front) = self.history.front() {
            if now.duration_since(*front) > window {
                self.history.pop_front();
            } else {
                break;
            }
        }
    }

    fn is_allowed(&self, path: &Path) -> bool {
        if let Some(root) = &self.workspace_root {
            if path.starts_with(root) {
                return true;
            }
        }

        path.starts_with(&fallback_screenshot_directory())
    }
}

struct ScreenshotJob {
    path: PathBuf,
    backend: ScreenshotBackend,
    format: String,
}

struct ScreenshotResult {
    path: PathBuf,
    format: String,
    bytes: u64,
}

impl ScreenshotJob {
    async fn execute(self) -> anyhow::Result<ScreenshotResult> {
        let ScreenshotJob {
            path,
            backend,
            format,
        } = self;

        if let ScreenshotBackend::Native = backend {
            return Self::capture_native(path, format).await;
        }

        let path_str = path.to_string_lossy().to_string();
        let mut command = match backend {
            ScreenshotBackend::Scrot => {
                let mut cmd = TokioCommand::new("scrot");
                cmd.args(["--overwrite", "--silent", &path_str]);
                cmd
            }
            ScreenshotBackend::Imagemagick => {
                let mut cmd = TokioCommand::new("import");
                cmd.args(["-window", "root", &path_str]);
                cmd
            }
            ScreenshotBackend::Native => unreachable!(),
        };

        let status = command
            .status()
            .await
            .with_context(|| format!("Failed to execute screenshot backend {:?}", backend))?;

        if !status.success() {
            anyhow::bail!(
                "Screenshot backend {:?} exited with status {}",
                backend,
                status
            );
        }

        let metadata = fs::metadata(&path).await.with_context(|| {
            format!(
                "Screenshot captured but failed to stat output file {}",
                path.display()
            )
        })?;

        Ok(ScreenshotResult {
            path,
            format,
            bytes: metadata.len(),
        })
    }

    async fn capture_native(path: PathBuf, format: String) -> anyhow::Result<ScreenshotResult> {
        let format_lower = format.to_lowercase();
        let result = spawn_blocking(move || -> anyhow::Result<ScreenshotResult> {
            let screens =
                Screen::all().map_err(|e| anyhow::anyhow!("Failed to enumerate screens: {e}"))?;
            let screen = screens
                .into_iter()
                .next()
                .ok_or_else(|| anyhow::anyhow!("No screens detected for screenshot"))?;

            let image = screen
                .capture()
                .map_err(|e| anyhow::anyhow!("Screen capture failed: {e}"))?;

            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            let image_format = match format_lower.as_str() {
                "png" => ImageFormat::Png,
                "jpg" | "jpeg" => ImageFormat::Jpeg,
                "bmp" => ImageFormat::Bmp,
                other => {
                    return Err(anyhow::anyhow!(
                        "Unsupported screenshot format '{}' for native backend",
                        other
                    ));
                }
            };

            image.save_with_format(&path, image_format)?;
            let bytes = std::fs::metadata(&path)?.len();

            Ok(ScreenshotResult {
                path,
                format: format_lower,
                bytes,
            })
        })
        .await?;

        Ok(result?)
    }
}

fn default_screenshot_directory(workspace_root: Option<&Path>) -> PathBuf {
    workspace_root
        .map(|root| root.join(".devit").join("screenshots"))
        .unwrap_or_else(fallback_screenshot_directory)
}

fn fallback_screenshot_directory() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(local) = env::var_os("LOCALAPPDATA") {
            return PathBuf::from(local).join("DevIt").join("screenshots");
        }
    }
    env::temp_dir().join("devit-screenshots")
}

fn human_readable_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes == 0 {
        return "0 B".to_string();
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.2} {}", UNITS[unit])
}
impl State {
    fn new(
        secret: String,
        journal_path: &str,
        workers: WorkerSettings,
        notify_hook: Option<NotifyHook>,
    ) -> Result<Self> {
        let journal = journal::Journal::open(journal_path, secret.as_bytes())?;
        let WorkerSettings {
            configs,
            workspace_root,
            expected_worker_version,
            capabilities,
            screenshot,
            approval_target,
        } = workers;

        let screenshot_control = ScreenshotControl::new(
            screenshot,
            capabilities.screenshot.rate_limit,
            capabilities.screenshot.enabled,
            workspace_root.clone(),
        );

        Ok(Self {
            clients: HashMap::new(),
            leases: HashMap::new(),
            expired_leases: HashMap::new(),
            pending_notifications: HashMap::new(),
            pending_approvals: HashMap::new(),
            ack_markers: HashMap::new(),
            #[cfg(unix)]
            ack_sockets: HashMap::new(),
            #[cfg(unix)]
            ack_writers: HashMap::new(),
            #[cfg(not(unix))]
            ack_pipe_servers: HashMap::new(),
            secret,
            journal,
            worker_configs: configs,
            workspace_root,
            tasks_active: HashMap::new(),
            tasks_completed: HashMap::new(),
            notify_hook,
            daemon_version: DAEMON_VERSION.to_string(),
            expected_worker_version,
            screenshot: screenshot_control,
            approver_target: if approval_target.trim().is_empty() {
                worker_executor::DEFAULT_APPROVER_TARGET.to_string()
            } else {
                approval_target
            },
        })
    }

    fn has_live_clients(&self) -> bool {
        self.clients
            .values()
            .any(|client| client.last_heartbeat.elapsed() < HEARTBEAT_TIMEOUT)
    }

    fn has_inflight_work(&self) -> bool {
        !self.tasks_active.is_empty()
            || !self.leases.is_empty()
            || !self.pending_approvals.is_empty()
    }

    fn is_client_alive(&self, ident: &str) -> bool {
        self.clients
            .get(ident)
            .map(|c| c.last_heartbeat.elapsed() < HEARTBEAT_TIMEOUT)
            .unwrap_or(false)
    }

    fn add_notification(&mut self, client_id: &str, msg: Msg) {
        self.pending_notifications
            .entry(client_id.to_string())
            .or_insert_with(Vec::new)
            .push(msg);
    }

    fn get_notifications(&mut self, client_id: &str) -> Vec<Msg> {
        self.pending_notifications
            .remove(client_id)
            .unwrap_or_default()
    }

    fn cleanup_expired(&mut self) -> Vec<HookInvocation> {
        let now = Instant::now();
        let mut hook_invocations = Vec::new();

        // Remove expired leases
        let expired_leases: Vec<String> = self
            .leases
            .iter()
            .filter(|(_, lease)| now > lease.deadline)
            .map(|(k, _)| k.clone())
            .collect();

        for task_id in expired_leases {
            if let Some(lease) = self.leases.remove(&task_id) {
                self.expired_leases.insert(task_id.clone(), lease.clone());
                warn!(
                    task_id = %task_id,
                    worker = %lease.assigned_to,
                    "Lease expired for task"
                );

                let return_target = lease
                    .return_to
                    .clone()
                    .unwrap_or_else(|| lease.original_from.clone());
                let timestamp = Utc::now();
                let timestamp_rfc3339 = timestamp.to_rfc3339_opts(SecondsFormat::Millis, true);
                let summary = format!(
                    "Task lease expired after {}s without completion",
                    LEASE_TTL.as_secs()
                );
                let detail_payload = serde_json::json!({
                    "reason": "lease_timeout",
                    "timeout_secs": LEASE_TTL.as_secs(),
                    "expired_at": timestamp_rfc3339,
                    "worker": lease.assigned_to,
                });
                let metadata = serde_json::json!({
                    "failure": {
                        "reason": "lease_timeout",
                        "timeout_secs": LEASE_TTL.as_secs(),
                        "expired_at": timestamp_rfc3339,
                        "worker": lease.assigned_to,
                    }
                });

                let notification_record = OrchestrationTaskNotification {
                    received_at: timestamp,
                    status: "failed".to_string(),
                    summary: summary.clone(),
                    details: Some(detail_payload.clone()),
                    evidence: None,
                    auto_generated: true,
                    metadata: Some(metadata.clone()),
                };
                let hook_record = notification_record.clone();

                let mut task = self.tasks_active.remove(&task_id);
                if task.is_none() {
                    task = self.tasks_completed.remove(&task_id);
                }

                let updated_task = if let Some(mut existing) = task {
                    existing.status = TaskStatus::Failed;
                    existing.last_activity = timestamp;
                    existing.notifications.push(notification_record.clone());
                    if let Some(ref dir) = lease.working_dir {
                        existing.working_dir = Some(PathBuf::from(dir));
                    }
                    if existing.response_format.is_none() {
                        existing.response_format = lease.response_format.clone();
                    }
                    if existing.model.is_none() {
                        existing.model = lease.model.clone();
                    }
                    if existing.model_resolved.is_none() {
                        existing.model_resolved = lease.model_resolved.clone();
                    }
                    Some(existing)
                } else {
                    Some(DelegatedTask {
                        id: task_id.clone(),
                        goal: summary.clone(),
                        delegated_to: lease.assigned_to.clone(),
                        created_at: timestamp,
                        timeout_secs: DEFAULT_TIMEOUT_SECS,
                        status: TaskStatus::Failed,
                        context: None,
                        watch_patterns: Vec::new(),
                        last_activity: timestamp,
                        notifications: vec![notification_record.clone()],
                        working_dir: lease.working_dir.as_ref().map(PathBuf::from),
                        response_format: lease.response_format.clone(),
                        model: lease.model.clone(),
                        model_resolved: lease.model_resolved.clone(),
                    })
                };

                if let Some(task) = updated_task {
                    self.finalize_task(task);
                }

                let mut artifacts = serde_json::Map::new();
                artifacts.insert("summary".into(), serde_json::Value::String(summary.clone()));
                artifacts.insert("details".into(), detail_payload.clone());
                artifacts.insert(
                    "reported_at".into(),
                    serde_json::Value::String(timestamp_rfc3339.clone()),
                );

                let mut payload = serde_json::Map::new();
                payload.insert("task_id".into(), serde_json::Value::String(task_id.clone()));
                payload.insert(
                    "status".into(),
                    serde_json::Value::String("failed".to_string()),
                );
                payload.insert("artifacts".into(), serde_json::Value::Object(artifacts));
                payload.insert("metadata".into(), metadata.clone());
                payload.insert(
                    "return_to".into(),
                    serde_json::Value::String(return_target.clone()),
                );

                let mut notification = Msg {
                    msg_type: "NOTIFY".to_string(),
                    msg_id: Uuid::new_v4().to_string(),
                    from: lease.assigned_to.clone(),
                    to: return_target.clone(),
                    ts: now_ts(),
                    nonce: Uuid::new_v4().to_string(),
                    hmac: String::new(),
                    payload: serde_json::Value::Object(payload),
                };

                if let Err(err) = sign_msg(&mut notification, &self.secret) {
                    warn!(
                        task_id = %task_id,
                        worker = %lease.assigned_to,
                        "Failed to sign timeout notification: {}",
                        err
                    );
                } else {
                    self.add_notification(&return_target, notification);
                }

                let _ = self.journal.append(
                    "NOTIFY",
                    &task_id,
                    &lease.assigned_to,
                    &return_target,
                    serde_json::json!({
                        "status": "failed",
                        "summary": summary,
                        "details": detail_payload,
                        "metadata": metadata,
                        "reason": "lease_timeout"
                    }),
                );

                if let Some(hook) = self.notify_hook.clone() {
                    let context = NotificationHookContext {
                        task_id: task_id.clone(),
                        status: "failed".to_string(),
                        worker: lease.assigned_to.clone(),
                        return_to: return_target,
                        summary: hook_record.summary.clone(),
                        working_dir: lease.working_dir.clone(),
                        details: hook_record.details.clone(),
                        evidence: None,
                        timestamp: hook_record
                            .received_at
                            .to_rfc3339_opts(SecondsFormat::Millis, true),
                        metadata: hook_record.metadata.clone(),
                    };
                    hook_invocations.push((hook, context));
                }
            }
        }

        // Remove disconnected clients
        let dead_clients: Vec<String> = self
            .clients
            .iter()
            .filter(|(_, client)| client.last_heartbeat.elapsed() > HEARTBEAT_TIMEOUT)
            .map(|(k, _)| k.clone())
            .collect();

        for ident in dead_clients {
            warn!("Client timed out: {}", ident);
            self.clients.remove(&ident);
            self.pending_notifications.remove(&ident);
        }

        hook_invocations
    }

    fn insert_active_task(&mut self, task: DelegatedTask) {
        self.tasks_active.insert(task.id.clone(), task);
    }

    fn finalize_task(&mut self, task: DelegatedTask) {
        match task.status {
            TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled => {
                self.tasks_completed.insert(task.id.clone(), task);
                self.prune_completed();
            }
            _ => {
                self.tasks_active.insert(task.id.clone(), task);
            }
        }
    }

    fn prune_completed(&mut self) {
        if self.tasks_completed.len() <= MAX_COMPLETED_TASKS {
            return;
        }

        let mut tasks: Vec<_> = self
            .tasks_completed
            .values()
            .map(|task| (task.id.clone(), task.last_activity))
            .collect();
        tasks.sort_by_key(|(_, ts)| *ts);

        let overflow = self.tasks_completed.len() - MAX_COMPLETED_TASKS;
        for (task_id, _) in tasks.into_iter().take(overflow) {
            self.tasks_completed.remove(&task_id);
        }
    }

    fn build_snapshot(&self) -> OrchestrationStatus {
        let active_tasks: Vec<DelegatedTask> = self
            .tasks_active
            .values()
            .cloned()
            .collect::<Vec<DelegatedTask>>();
        let completed_tasks: Vec<DelegatedTask> = self
            .tasks_completed
            .values()
            .cloned()
            .collect::<Vec<DelegatedTask>>();

        let total_active = active_tasks.len();
        let total_completed = completed_tasks
            .iter()
            .filter(|task| matches!(task.status, TaskStatus::Completed))
            .count();
        let total_failed = completed_tasks
            .iter()
            .filter(|task| matches!(task.status, TaskStatus::Failed))
            .count();
        let oldest_active_task = active_tasks
            .iter()
            .min_by_key(|task| task.created_at)
            .map(|task| task.id.clone());

        OrchestrationStatus {
            active_tasks,
            completed_tasks,
            summary: OrchestrationSummary {
                total_active,
                total_completed,
                total_failed,
                oldest_active_task,
            },
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging
    let filter = if cli.debug { "debug" } else { "info" };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(io::stderr)
        .init();

    // Get secret from CLI or environment
    let secret = cli
        .secret
        .or_else(|| std::env::var("DEVIT_SECRET").ok())
        .unwrap_or_else(|| DEFAULT_SECRET.to_string());

    if secret == DEFAULT_SECRET {
        warn!("Using default secret - change DEVIT_SECRET in production!");
    }

    let config_path = cli
        .config
        .clone()
        .or_else(|| std::env::var("DEVIT_CORE_CONFIG").ok().map(PathBuf::from))
        .or_else(|| {
            let candidate = std::env::current_dir().ok()?.join("devit.core.toml");
            if candidate.is_file() {
                Some(candidate)
            } else {
                None
            }
        });

    let auto_shutdown_secs =
        cli.auto_shutdown_after
            .or_else(|| match std::env::var("DEVIT_AUTO_SHUTDOWN_AFTER") {
                Ok(raw) => {
                    let trimmed = raw.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        match trimmed.parse::<u64>() {
                            Ok(value) => Some(value),
                            Err(err) => {
                                warn!("Invalid DEVIT_AUTO_SHUTDOWN_AFTER value '{}': {}", raw, err);
                                None
                            }
                        }
                    }
                }
                Err(_) => None,
            });

    let auto_shutdown = auto_shutdown_secs.and_then(|secs| {
        if secs == 0 {
            None
        } else {
            Some(Duration::from_secs(secs))
        }
    });

    info!(
        "devitd version {} ({})",
        DAEMON_VERSION,
        devit_build_info::build_id()
    );
    let worker_settings = load_worker_settings(config_path.as_deref());
    let notify_hook = load_notify_hook();
    if let Some(hook) = notify_hook.as_ref() {
        info!("Notification hook enabled: {}", hook.command);
    }
    if let Some(duration) = auto_shutdown {
        info!(
            "Auto-shutdown after {}s of inactivity enabled",
            duration.as_secs()
        );
    }

    #[cfg(unix)]
    {
        // Remove existing socket
        let _ = std::fs::remove_file(&cli.socket);
        let listener = UnixListener::bind(&cli.socket)?;
        info!("DevIt daemon listening on {}", cli.socket.display());
        let journal_path = "/tmp/devitd.journal";
        let state = Arc::new(Mutex::new(State::new(
            secret,
            journal_path,
            worker_settings,
            notify_hook,
        )?));

        spawn_signal_handlers(state.clone());
        if let Some(duration) = auto_shutdown {
            spawn_idle_shutdown_task(state.clone(), duration);
        }

        // Cleanup task
        let state_cleanup = state.clone();
        let state_for_hooks = state.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            loop {
                interval.tick().await;
                let hook_invocations = {
                    let mut guard = state_cleanup.lock().await;
                    guard.cleanup_expired()
                };
                for (hook, context) in hook_invocations {
                    let state_ref = state_for_hooks.clone();
                    let task_id = context.task_id.clone();
                    let status = context.status.clone();
                    let worker = context.worker.clone();
                    tokio::spawn(async move {
                        info!(
                            %task_id,
                            %status,
                            %worker,
                            "Launching notification hook (lease timeout)"
                        );
                        match run_notify_hook(hook, context, &state_ref).await {
                            Ok(()) => {
                                info!(
                                    %task_id,
                                    %status,
                                    %worker,
                                    "Notification hook completed successfully (lease timeout)"
                                );
                            }
                            Err(err) => {
                                warn!(
                                    %task_id,
                                    %status,
                                    %worker,
                                    "Notification hook failed (lease timeout): {}",
                                    err
                                );
                            }
                        }
                    });
                }
            }
        });

        // Reaper task (devit_exec process monitoring)
        let registry = Arc::new(Mutex::new(
            process_registry::load_registry().unwrap_or_else(|_| process_registry::Registry::new()),
        ));
        reaper::spawn_reaper_task(registry);

        // Accept connections
        loop {
            let (stream, _addr) = listener.accept().await?;
            let state = state.clone();

            tokio::spawn(async move {
                if let Err(e) = handle_connection(stream, state).await {
                    error!("Connection error: {}", e);
                }
            });
        }
    }

    #[cfg(not(unix))]
    {
        let addr_str = cli.socket.to_string_lossy().to_string();
        let journal_path =
            std::env::var("DEVITD_JOURNAL").unwrap_or_else(|_| "devitd.journal".into());
        let state = Arc::new(Mutex::new(State::new(
            secret,
            &journal_path,
            worker_settings,
            notify_hook,
        )?));

        spawn_signal_handlers(state.clone());
        if let Some(duration) = auto_shutdown {
            spawn_idle_shutdown_task(state.clone(), duration);
        }

        // Cleanup task
        let state_cleanup = state.clone();
        let state_for_hooks = state.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            loop {
                interval.tick().await;
                let hook_invocations = {
                    let mut guard = state_cleanup.lock().await;
                    guard.cleanup_expired()
                };
                for (hook, context) in hook_invocations {
                    let state_ref = state_for_hooks.clone();
                    let task_id = context.task_id.clone();
                    let status = context.status.clone();
                    let worker = context.worker.clone();
                    tokio::spawn(async move {
                        info!(
                            %task_id,
                            %status,
                            %worker,
                            "Launching notification hook (lease timeout)"
                        );
                        match run_notify_hook(hook, context, &state_ref).await {
                            Ok(()) => {
                                info!(
                                    %task_id,
                                    %status,
                                    %worker,
                                    "Notification hook completed successfully (lease timeout)"
                                );
                            }
                            Err(err) => {
                                warn!(
                                    %task_id,
                                    %status,
                                    %worker,
                                    "Notification hook failed (lease timeout): {}",
                                    err
                                );
                            }
                        }
                    });
                }
            }
        });

        if is_pipe_addr(&addr_str) {
            let name = normalize_pipe_name(&addr_str);
            info!("DevIt daemon listening on winpipe://{}", name);

            let mut server = create_restricted_named_pipe(&name)?;
            loop {
                server.connect().await?;
                let connected =
                    std::mem::replace(&mut server, create_restricted_named_pipe(&name)?);
                let state = state.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_connection_pipe(connected, state).await {
                        error!("Pipe connection error: {}", e);
                    }
                });
            }
        } else {
            let addr = if addr_str.contains(':') {
                addr_str
            } else {
                DEFAULT_SOCK.to_string()
            };
            let listener = TcpListener::bind(&addr).await?;
            info!("DevIt daemon listening on tcp://{}", addr);

            loop {
                let (stream, _addr) = listener.accept().await?;
                let state = state.clone();

                tokio::spawn(async move {
                    if let Err(e) = handle_connection_tcp(stream, state).await {
                        error!("TCP connection error: {}", e);
                    }
                });
            }
        }
    }

    // Unreachable: covered by cfg-specific accept loops above
    #[allow(unreachable_code)]
    Ok(())
}

#[cfg(unix)]
async fn handle_connection(stream: UnixStream, state: Arc<Mutex<State>>) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut buf_reader = BufReader::new(reader);
    let mut line = String::new();

    loop {
        line.clear();
        match buf_reader.read_line(&mut line).await? {
            0 => break, // EOF
            _ => {
                let raw = line.trim();
                if raw.is_empty() {
                    continue;
                }

                // Try compact format first, then standard
                match serde_json::from_str::<compact::MsgC>(raw) {
                    Ok(compact_msg) => {
                        let msg = compact::from_compact(&compact_msg);
                        if let Some(mut response) = handle_message(msg, &state).await? {
                            let secret = {
                                let state_guard = state.lock().await;
                                state_guard.secret.clone()
                            };
                            sign_msg(&mut response, &secret)?;

                            // Send response in same format as received
                            let compact_response = compact::to_compact(&response);
                            let response_line = serde_json::to_string(&compact_response)? + "\n";
                            writer.write_all(response_line.as_bytes()).await?;
                        }
                    }
                    Err(_) => {
                        // Try standard format
                        match serde_json::from_str::<Msg>(raw) {
                            Ok(msg) => {
                                if let Some(mut response) = handle_message(msg, &state).await? {
                                    let secret = {
                                        let state_guard = state.lock().await;
                                        state_guard.secret.clone()
                                    };
                                    sign_msg(&mut response, &secret)?;

                                    let response_line = serde_json::to_string(&response)? + "\n";
                                    writer.write_all(response_line.as_bytes()).await?;
                                }
                            }
                            Err(e) => {
                                error!("JSON parse error: {}", e);
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(not(unix))]
async fn handle_connection_tcp(stream: TcpStream, state: Arc<Mutex<State>>) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut buf_reader = BufReader::new(reader);
    let mut line = String::new();

    loop {
        line.clear();
        match buf_reader.read_line(&mut line).await? {
            0 => break, // EOF
            _ => {
                let raw = line.trim();
                if raw.is_empty() {
                    continue;
                }

                // Try standard format then compact
                match serde_json::from_str::<Msg>(raw) {
                    Ok(msg) => {
                        if let Some(mut response) = handle_message(msg, &state).await? {
                            let secret = {
                                let state_guard = state.lock().await;
                                state_guard.secret.clone()
                            };
                            sign_msg(&mut response, &secret)?;

                            let response_line = serde_json::to_string(&response)? + "\n";
                            writer.write_all(response_line.as_bytes()).await?;
                        }
                    }
                    Err(_) => match serde_json::from_str::<compact::MsgC>(raw) {
                        Ok(compact_msg) => {
                            let msg = compact::from_compact(&compact_msg);
                            if let Some(mut response) = handle_message(msg, &state).await? {
                                let secret = {
                                    let state_guard = state.lock().await;
                                    state_guard.secret.clone()
                                };
                                sign_msg(&mut response, &secret)?;

                                let compact_response = compact::to_compact(&response);
                                let response_line =
                                    serde_json::to_string(&compact_response)? + "\n";
                                writer.write_all(response_line.as_bytes()).await?;
                            }
                        }
                        Err(e) => {
                            error!("JSON parse error: {}", e);
                        }
                    },
                }
            }
        }
    }
    Ok(())
}

#[cfg(not(unix))]
async fn handle_connection_pipe(
    mut stream: NamedPipeServer,
    state: Arc<Mutex<State>>,
) -> Result<()> {
    use tokio::io::split;
    let (reader, mut writer) = split(stream);
    let mut buf_reader = BufReader::new(reader);
    let mut line = String::new();

    loop {
        line.clear();
        match buf_reader.read_line(&mut line).await? {
            0 => break, // EOF
            _ => {
                let raw = line.trim();
                if raw.is_empty() {
                    continue;
                }

                // Try standard format then compact
                match serde_json::from_str::<Msg>(raw) {
                    Ok(msg) => {
                        if let Some(mut response) = handle_message(msg, &state).await? {
                            let secret = {
                                let state_guard = state.lock().await;
                                state_guard.secret.clone()
                            };
                            sign_msg(&mut response, &secret)?;

                            let response_line = serde_json::to_string(&response)? + "\n";
                            writer.write_all(response_line.as_bytes()).await?;
                        }
                    }
                    Err(_) => match serde_json::from_str::<compact::MsgC>(raw) {
                        Ok(compact_msg) => {
                            let msg = compact::from_compact(&compact_msg);
                            if let Some(mut response) = handle_message(msg, &state).await? {
                                let secret = {
                                    let state_guard = state.lock().await;
                                    state_guard.secret.clone()
                                };
                                sign_msg(&mut response, &secret)?;

                                let compact_response = compact::to_compact(&response);
                                let response_line =
                                    serde_json::to_string(&compact_response)? + "\n";
                                writer.write_all(response_line.as_bytes()).await?;
                            }
                        }
                        Err(e) => {
                            error!("JSON parse error: {}", e);
                        }
                    },
                }
            }
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn is_pipe_addr(addr: &str) -> bool {
    let a = addr.trim();
    a.starts_with(r"\\.\pipe\") || a.starts_with("pipe:") || (!a.contains(':'))
}

fn spawn_idle_shutdown_task(state: Arc<Mutex<State>>, idle_after: Duration) {
    tokio::spawn(async move {
        let mut idle_since: Option<Instant> = None;
        let mut ticker = tokio::time::interval(Duration::from_secs(5));

        loop {
            ticker.tick().await;

            let (has_clients, has_work) = {
                let guard = state.lock().await;
                (guard.has_live_clients(), guard.has_inflight_work())
            };

            if has_clients || has_work {
                idle_since = None;
                continue;
            }

            let start = idle_since.get_or_insert_with(Instant::now);
            if start.elapsed() >= idle_after {
                info!(
                    idle_timeout_secs = idle_after.as_secs(),
                    "No active clients or tasks detected; initiating auto-shutdown"
                );
                shutdown_daemon(state.clone()).await;
                break;
            }
        }
    });
}

fn spawn_signal_handlers(state: Arc<Mutex<State>>) {
    let ctrl_c_state = state.clone();
    tokio::spawn(async move {
        if signal::ctrl_c().await.is_ok() {
            info!("Received Ctrl+C; shutting down daemon");
            shutdown_daemon(ctrl_c_state).await;
        }
    });

    #[cfg(unix)]
    {
        let mut sigterm = match signal::unix::signal(signal::unix::SignalKind::terminate()) {
            Ok(stream) => stream,
            Err(err) => {
                warn!("Failed to install SIGTERM handler: {}", err);
                return;
            }
        };
        let state_clone = state.clone();
        tokio::spawn(async move {
            if sigterm.recv().await.is_some() {
                info!("Received SIGTERM; shutting down daemon");
                shutdown_daemon(state_clone).await;
            }
        });
    }
}

async fn shutdown_daemon(state: Arc<Mutex<State>>) {
    if SHUTDOWN_REQUESTED.swap(true, Ordering::SeqCst) {
        return;
    }

    let (clients, leases, active_tasks, approvals) = {
        let guard = state.lock().await;
        (
            guard.clients.len(),
            guard.leases.len(),
            guard.tasks_active.len(),
            guard.pending_approvals.len(),
        )
    };

    info!(
        connected_clients = clients,
        inflight_leases = leases,
        active_tasks,
        pending_approvals = approvals,
        "Graceful shutdown initiated"
    );

    // Give logs a moment to flush before exiting.
    tokio::time::sleep(Duration::from_millis(200)).await;
    std::process::exit(0);
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

#[cfg(not(unix))]
fn current_user_sid_sddl() -> io::Result<String> {
    unsafe {
        let process: HANDLE = GetCurrentProcess();
        let mut token: HANDLE = std::ptr::null_mut();
        let ok = OpenProcessToken(process, TOKEN_QUERY as TOKEN_ACCESS_MASK, &mut token);
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }

        let mut len: u32 = 0;
        let _ = GetTokenInformation(
            token,
            TokenUser as TOKEN_INFORMATION_CLASS,
            ptr::null_mut(),
            0,
            &mut len,
        );
        if len == 0 {
            let _ = CloseHandle(token);
            return Err(io::Error::last_os_error());
        }
        let mut buf: Vec<u8> = vec![0u8; len as usize];
        let ok2 = GetTokenInformation(
            token,
            TokenUser as TOKEN_INFORMATION_CLASS,
            buf.as_mut_ptr() as *mut c_void,
            len,
            &mut len,
        );
        let _ = CloseHandle(token);
        if ok2 == 0 {
            return Err(io::Error::last_os_error());
        }
        let token_user: *const TOKEN_USER = buf.as_ptr() as *const TOKEN_USER;
        let sid_ptr = (*token_user).User.Sid;

        let mut sid_str_ptr: *mut u16 = ptr::null_mut();
        let ok3 = ConvertSidToStringSidW(sid_ptr, &mut sid_str_ptr);
        if ok3 == 0 || sid_str_ptr.is_null() {
            return Err(io::Error::last_os_error());
        }
        let mut len_w = 0;
        while *sid_str_ptr.add(len_w) != 0 {
            len_w += 1;
        }
        let wslice = std::slice::from_raw_parts(sid_str_ptr, len_w);
        let sid_string = String::from_utf16_lossy(wslice);
        let _ = LocalFree(sid_str_ptr as HLOCAL);
        Ok(sid_string)
    }
}

#[cfg(not(unix))]
fn to_wide(s: &str) -> Vec<u16> {
    use std::os::windows::prelude::OsStrExt;
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(not(unix))]
fn create_restricted_named_pipe(name: &str) -> io::Result<NamedPipeServer> {
    unsafe {
        let sid = current_user_sid_sddl()?;
        let sddl = format!("D:P(A;;GA;;;{sid})");
        let sddl_w = to_wide(&sddl);

        let mut sd_ptr: *mut c_void = ptr::null_mut();
        let mut sd_size: u32 = 0;
        let ok = ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl_w.as_ptr(),
            1,
            &mut sd_ptr as *mut *mut c_void as *mut _,
            &mut sd_size,
        );
        if ok == 0 || sd_ptr.is_null() {
            return Err(io::Error::last_os_error());
        }

        let mut sa = SECURITY_ATTRIBUTES {
            nLength: mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd_ptr,
            bInheritHandle: 0,
        };

        let mut opts = ServerOptions::new();
        opts.reject_remote_clients(true);

        let result =
            opts.create_with_security_attributes_raw(name, &mut sa as *mut _ as *mut c_void);

        let _ = LocalFree(sd_ptr as HLOCAL);

        result
    }
}

async fn handle_message(msg: Msg, state: &Arc<Mutex<State>>) -> Result<Option<Msg>> {
    debug!("Received message: {} from {}", msg.msg_type, msg.from);

    // Verify HMAC
    let hmac_check = {
        let state_guard = state.lock().await;
        verify_hmac_detailed(&msg, &state_guard.secret)?
    };

    if !hmac_check.valid {
        warn!(
            from = %msg.from,
            to = %msg.to,
            msg_type = %msg.msg_type,
            msg_id = %msg.msg_id,
            ts = msg.ts,
            nonce = %msg.nonce,
            provided_sig = %shorten_sig(&hmac_check.provided),
            expected_sig = %shorten_sig(&hmac_check.expected),
            body = %summarize_body(&hmac_check.body),
            "Invalid HMAC from client"
        );
        return Ok(None);
    }

    match msg.msg_type.as_str() {
        "REGISTER" => handle_register(msg, state).await,
        "HEARTBEAT" => handle_heartbeat(msg, state).await,
        "DELEGATE" => handle_delegate(msg, state).await,
        "NOTIFY" => handle_notify(msg, state).await,
        "APPROVAL_DECISION" => handle_approval_decision(msg, state).await,
        "POLL" => handle_poll(msg, state).await,
        "STATUS_REQUEST" => handle_status_request(msg, state).await,
        "SCREENSHOT" => handle_screenshot(msg, state).await,
        _ => {
            warn!("Unknown message type: {}", msg.msg_type);
            Ok(None)
        }
    }
}

async fn handle_register(msg: Msg, state: &Arc<Mutex<State>>) -> Result<Option<Msg>> {
    let caps: Vec<String> = msg
        .payload
        .get("caps")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let client_version = msg
        .payload
        .get("version")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let pid_value = msg.payload.get("pid").cloned();

    let (response, log_msg);

    {
        let mut state_guard = state.lock().await;
        let expected_version = state_guard.expected_worker_version.clone();
        let daemon_version = state_guard.daemon_version.clone();

        if let Some(expected) = expected_version.as_ref() {
            match client_version.as_deref() {
                Some(actual) if actual == expected => {
                    debug!(
                        client = %msg.from,
                        expected,
                        actual,
                        "Worker version matches expectation"
                    );
                }
                Some(actual) => {
                    warn!(
                        client = %msg.from,
                        expected,
                        actual,
                        "Rejecting client due to worker version mismatch"
                    );
                    let payload = serde_json::json!({
                        "code": "E_VERSION_MISMATCH",
                        "message": format!(
                            "expected worker version '{}' but client reported '{}'",
                            expected, actual
                        ),
                        "expected_worker_version": expected,
                        "received_worker_version": actual,
                        "daemon_version": daemon_version,
                    });
                    let response = Msg {
                        msg_type: "ERR".to_string(),
                        msg_id: Uuid::new_v4().to_string(),
                        from: "orchestrator".to_string(),
                        to: msg.from.clone(),
                        ts: now_ts(),
                        nonce: Uuid::new_v4().to_string(),
                        hmac: String::new(),
                        payload,
                    };
                    return Ok(Some(response));
                }
                None => {
                    warn!(
                        client = %msg.from,
                        expected,
                        "Rejecting client missing worker version"
                    );
                    let payload = serde_json::json!({
                        "code": "E_VERSION_MISSING",
                        "message": format!(
                            "expected worker version '{}' but client omitted version field",
                            expected
                        ),
                        "expected_worker_version": expected,
                        "daemon_version": daemon_version,
                    });
                    let response = Msg {
                        msg_type: "ERR".to_string(),
                        msg_id: Uuid::new_v4().to_string(),
                        from: "orchestrator".to_string(),
                        to: msg.from.clone(),
                        ts: now_ts(),
                        nonce: Uuid::new_v4().to_string(),
                        hmac: String::new(),
                        payload,
                    };
                    return Ok(Some(response));
                }
            }
        }

        let client = Client {
            ident: msg.from.clone(),
            last_heartbeat: Instant::now(),
            capabilities: caps.clone(),
            version: client_version.clone(),
        };

        state_guard.clients.insert(msg.from.clone(), client);

        // Journal the registration
        let _ = state_guard.journal.append(
            "REGISTER",
            &msg.msg_id,
            &msg.from,
            "orchestrator",
            serde_json::json!({
                "capabilities": caps,
                "pid": pid_value,
                "version": client_version,
            }),
        );

        let payload = serde_json::json!({
            "daemon_version": daemon_version.clone(),
            "expected_worker_version": expected_version,
            "worker_version": msg.payload.get("version"),
        });

        response = Msg {
            msg_type: "ACK".to_string(),
            msg_id: Uuid::new_v4().to_string(),
            from: "orchestrator".to_string(),
            to: msg.from.clone(),
            ts: now_ts(),
            nonce: Uuid::new_v4().to_string(),
            hmac: String::new(),
            payload,
        };

        log_msg = format!(
            "Client registered: {} ({})",
            msg.from,
            client_version.as_deref().unwrap_or("unknown version")
        );
    }

    info!("{}", log_msg);
    debug!(
        client = %msg.from,
        version = %client_version.as_deref().unwrap_or("unknown"),
        daemon_version = %response.payload.get("daemon_version").and_then(|v| v.as_str()).unwrap_or("unknown"),
        "Handshake completed"
    );
    Ok(Some(response))
}

async fn handle_heartbeat(msg: Msg, state: &Arc<Mutex<State>>) -> Result<Option<Msg>> {
    let mut state_guard = state.lock().await;
    if let Some(client) = state_guard.clients.get_mut(&msg.from) {
        client.last_heartbeat = Instant::now();
        debug!("Heartbeat from {}", msg.from);
    }

    // Return any pending notifications
    let notifications = state_guard.get_notifications(&msg.from);
    if let Some(notification) = notifications.into_iter().next() {
        return Ok(Some(notification));
    }

    Ok(None)
}

async fn handle_delegate(msg: Msg, state: &Arc<Mutex<State>>) -> Result<Option<Msg>> {
    let task_id = msg.msg_id.clone();
    let worker = msg.to.clone();
    let return_to = msg
        .payload
        .get("return_to")
        .and_then(|v| v.as_str())
        .map(String::from);

    // Extract tool name for policy evaluation
    let tool = msg
        .payload
        .get("task")
        .and_then(|t| t.get("action"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let task_payload = msg.payload.get("task");
    let task_details = parse_task_details(task_payload, &task_id);

    info!("Delegating task {} (tool: {}) to {}", task_id, tool, worker);

    // Evaluate policy
    let policy_decision = policy::Policy::eval(&tool, &msg.payload);

    match policy_decision {
        policy::PolicyAction::Deny => {
            warn!("Task {} denied by policy", task_id);
            let detail_payload = serde_json::json!({
                "reason": "policy_denied",
                "tool": tool,
                "task_id": task_id,
                "worker": worker,
                "goal": task_details.goal
            });
            let metadata_payload = serde_json::json!({
                "failure": {
                    "reason": "policy_denied",
                    "tool": tool,
                    "task_id": task_id
                }
            });
            let summary = format!("Task denied by policy (tool: {})", tool);
            let artifacts = serde_json::json!({
                "summary": summary.clone(),
                "details": detail_payload.clone(),
                "metadata": metadata_payload.clone()
            });
            {
                let mut state_guard = state.lock().await;
                let _ = state_guard.journal.append(
                    "POLICY",
                    &task_id,
                    &msg.from,
                    "orchestrator",
                    serde_json::json!({
                        "decision": "deny",
                        "tool": &tool,
                        "reason": "policy_forbidden"
                    }),
                );
                let target = return_to.clone().unwrap_or_else(|| msg.from.clone());
                record_immediate_failure(
                    &mut state_guard,
                    &target,
                    &task_id,
                    &worker,
                    artifacts.clone(),
                    &task_details,
                );

                if target != msg.from {
                    queue_failure_notification(
                        &mut state_guard,
                        &msg.from,
                        &task_id,
                        artifacts.clone(),
                    );
                }
            }

            let response = Msg {
                msg_type: "ERR".to_string(),
                msg_id: Uuid::new_v4().to_string(),
                from: "orchestrator".to_string(),
                to: msg.from.clone(),
                ts: now_ts(),
                nonce: Uuid::new_v4().to_string(),
                hmac: String::new(),
                payload: serde_json::json!({
                    "code": "E_POLICY_DENIED",
                    "message": summary,
                    "details": detail_payload
                }),
            };
            return Ok(Some(response));
        }
        policy::PolicyAction::NeedApproval => {
            info!("Task {} requires approval", task_id);
            return handle_approval_request(msg, &task_id, &tool, state).await;
        }
        policy::PolicyAction::Allow => {
            info!("Task {} allowed by policy", task_id);
        }
    }

    let working_dir = task_details.working_dir.clone();
    let task_timeout = Some(task_details.timeout_secs);

    let (configured_worker, workspace_root, state_secret) = {
        let state_guard = state.lock().await;
        (
            state_guard.worker_configs.get(&worker).cloned(),
            state_guard.workspace_root.clone(),
            state_guard.secret.clone(),
        )
    };

    if let Some(worker_cfg) = configured_worker {
        let worker_task = WorkerTask {
            id: task_id.clone(),
            goal: task_details.goal.clone(),
            delegated_to: worker.clone(),
            working_dir: working_dir.clone(),
            timeout_secs: task_timeout,
            return_to: return_to.clone(),
            response_format: task_details.response_format.clone(),
            time_queued: Utc::now(),
            model: task_details.model.clone(),
            context: task_details.context.clone(),
        };

        if let Err(err) = proceed_with_delegation(
            msg.clone(),
            &task_id,
            &worker,
            return_to.clone(),
            &tool,
            false,
            task_details.clone(),
            state,
        )
        .await
        {
            error!(
                "Failed to record subprocess delegation for task {}: {}",
                task_id, err
            );
            return Ok(None);
        }

        let state_for_spawn = state.clone();
        let workspace_clone = workspace_root.clone();
        let secret_clone = state_secret.clone();
        tokio::spawn(async move {
            let executor = WorkerExecutor::new(worker_cfg, workspace_clone);
            let outcome = match executor.execute_task(&worker_task).await {
                Ok(outcome) => outcome,
                Err(err) => WorkerOutcome {
                    status: WorkerStatus::Failed,
                    summary: format!("Worker execution error: {}", err),
                    details: None,
                    evidence: None,
                    truncated: false,
                    original_size: None,
                    metadata: TaskMetadata::default(),
                },
            };

            let artifacts = build_worker_artifacts(&outcome);

            let mut payload = serde_json::Map::new();
            payload.insert("task_id".into(), serde_json::json!(&worker_task.id));
            payload.insert("status".into(), serde_json::json!(outcome.status.as_str()));
            payload.insert("artifacts".into(), artifacts);
            if let Some(rt) = worker_task.return_to.clone() {
                payload.insert("return_to".into(), serde_json::json!(rt));
            }

            let mut notify_msg = Msg {
                msg_type: "NOTIFY".to_string(),
                msg_id: worker_task.id.clone(),
                from: worker_task.delegated_to.clone(),
                to: "orchestrator".to_string(),
                ts: now_ts(),
                nonce: Uuid::new_v4().to_string(),
                hmac: String::new(),
                payload: serde_json::Value::Object(payload),
            };

            if let Err(err) = sign_msg(&mut notify_msg, &secret_clone) {
                error!(
                    "Failed to sign worker notification for task {}: {}",
                    worker_task.id, err
                );
                return;
            }

            if let Err(err) = handle_notify(notify_msg, &state_for_spawn).await {
                error!(
                    "Failed to record worker completion for task {}: {}",
                    worker_task.id, err
                );
            }
        });

        return Ok(None);
    }

    // Check if worker is alive in polling mode
    {
        let state_guard = state.lock().await;
        if !state_guard.is_client_alive(&worker) {
            warn!("Worker {} not available", worker);
            return Ok(None);
        }
    }

    // Proceed with delegation (polling workers)
    proceed_with_delegation(
        msg,
        &task_id,
        &worker,
        return_to,
        &tool,
        true,
        task_details,
        state,
    )
    .await
}

async fn handle_approval_request(
    msg: Msg,
    task_id: &str,
    tool: &str,
    state: &Arc<Mutex<State>>,
) -> Result<Option<Msg>> {
    let approval_id = Uuid::new_v4().to_string();
    let approver = {
        let state_guard = state.lock().await;
        state_guard.approver_target.clone()
    };
    let approver = if approver.trim().is_empty() {
        worker_executor::DEFAULT_APPROVER_TARGET.to_string()
    } else {
        approver
    };

    // Create approval request
    let approval_request =
        policy::create_approval_request(task_id, tool, &msg.msg_id, &msg.payload);

    let approval_msg = Msg {
        msg_type: "APPROVAL".to_string(),
        msg_id: approval_id.clone(),
        from: "orchestrator".to_string(),
        to: approver.to_string(),
        ts: now_ts(),
        nonce: Uuid::new_v4().to_string(),
        hmac: String::new(),
        payload: approval_request,
    };

    // Store pending approval
    let pending = PendingApproval {
        task_id: task_id.to_string(),
        original_msg: msg,
        tool: tool.to_string(),
        requested_at: Instant::now(),
    };

    {
        let mut state_guard = state.lock().await;
        state_guard
            .pending_approvals
            .insert(approval_id.clone(), pending);
        state_guard.add_notification(&approver, approval_msg);

        // Journal the approval request
        let _ = state_guard.journal.append(
            "POLICY",
            &approval_id,
            "orchestrator",
            &approver,
            serde_json::json!({
                "decision": "request_approval",
                "tool": tool,
                "task_id": task_id
            }),
        );
    }

    info!(
        "Approval requested for task {} (approval_id: {})",
        task_id, approval_id
    );
    Ok(None)
}

async fn proceed_with_delegation(
    msg: Msg,
    task_id: &str,
    worker: &str,
    return_to: Option<String>,
    tool: &str,
    notify_worker: bool,
    task_details: TaskDetails,
    state: &Arc<Mutex<State>>,
) -> Result<Option<Msg>> {
    let working_dir = task_details.working_dir.clone();
    let working_dir_path = working_dir.as_ref().map(PathBuf::from);
    let now = Utc::now();
    let delegated_task = DelegatedTask {
        id: task_id.to_string(),
        goal: task_details.goal.clone(),
        delegated_to: worker.to_string(),
        created_at: now,
        timeout_secs: task_details.timeout_secs,
        status: if notify_worker {
            TaskStatus::Pending
        } else {
            TaskStatus::InProgress
        },
        context: task_details.context.clone(),
        watch_patterns: task_details.watch_patterns.clone(),
        last_activity: now,
        notifications: Vec::new(),
        working_dir: working_dir_path,
        response_format: task_details.response_format.clone(),
        model: task_details.model.clone(),
        model_resolved: None,
    };

    // Create lease
    let lease = Lease {
        task_id: task_id.to_string(),
        assigned_to: worker.to_string(),
        original_from: msg.from.clone(),
        deadline: Instant::now() + LEASE_TTL,
        return_to: return_to.clone(),
        working_dir: working_dir.clone(),
        response_format: task_details.response_format.clone(),
        model: task_details.model.clone(),
        model_resolved: None,
    };

    let mut journal_payload = serde_json::json!({
        "original_msg_id": msg.msg_id,
        "return_to": return_to,
        "lease_ttl": LEASE_TTL.as_secs(),
        "task": msg.payload.get("task"),
        "tool": tool,
        "working_dir": working_dir,
        "format": task_details.response_format.clone(),
        "model": task_details.model.clone(),
        "policy_decision": "allow"
    });

    {
        let mut state_guard = state.lock().await;
        state_guard.leases.insert(task_id.to_string(), lease);
        state_guard.insert_active_task(delegated_task);
        if notify_worker {
            let task_msg = Msg {
                msg_type: "DELEGATE".to_string(),
                msg_id: task_id.to_string(),
                from: "orchestrator".to_string(),
                to: worker.to_string(),
                ts: now_ts(),
                nonce: Uuid::new_v4().to_string(),
                hmac: String::new(),
                payload: msg.payload.clone(),
            };
            state_guard.add_notification(worker, task_msg);
        } else if let Some(map) = journal_payload.as_object_mut() {
            map.insert(
                "delivery".into(),
                serde_json::Value::String("subprocess".into()),
            );
        }

        // Journal the delegation
        let _ = state_guard
            .journal
            .append("DELEGATE", task_id, &msg.from, worker, journal_payload);
    }

    Ok(None)
}

fn build_worker_artifacts(outcome: &WorkerOutcome) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    map.insert(
        "summary".into(),
        serde_json::Value::String(outcome.summary.clone()),
    );
    if let Some(details) = outcome.details.clone() {
        map.insert("details".into(), details);
    }
    if let Some(evidence) = outcome.evidence.clone() {
        map.insert("evidence".into(), evidence);
    }
    if outcome.truncated {
        map.insert("truncated".into(), serde_json::Value::Bool(true));
        if let Some(size) = outcome.original_size {
            map.insert(
                "original_size".into(),
                serde_json::Value::Number((size as u64).into()),
            );
        }
    }
    if let Ok(metadata) = serde_json::to_value(&outcome.metadata) {
        map.insert("metadata".into(), metadata);
    }
    map.insert(
        "reported_at".into(),
        serde_json::Value::String(Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)),
    );
    serde_json::Value::Object(map)
}

async fn handle_notify(msg: Msg, state: &Arc<Mutex<State>>) -> Result<Option<Msg>> {
    // Always prefer payload.task_id over msg_id for routing/state updates
    let payload_task_id = msg
        .payload
        .get("task_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let msg_id_fallback = msg.msg_id.clone();
    let task_id = payload_task_id.unwrap_or(msg_id_fallback);
    let mut hook_invocation: Option<(NotifyHook, NotificationHookContext)> = None;

    // Special case: ACK signal coming from a client (e.g., MCP) to acknowledge
    // reception/processing of a previous notification. This does not require a lease.
    let is_ack = msg
        .payload
        .get("status")
        .and_then(|v| v.as_str())
        .map(|s| s.eq_ignore_ascii_case("ack"))
        .unwrap_or(false);
    if is_ack {
        // Prefer payload.task_id for ACK routing
        let ack_task_id = task_id.clone();
        let mut marker_opt = {
            let mut guard = state.lock().await;
            guard.ack_markers.remove(&ack_task_id)
        };

        #[cfg(unix)]
        {
            let writer_opt = {
                let mut guard = state.lock().await;
                guard.ack_writers.remove(&ack_task_id)
            };
            if let Some(mut writer) = writer_opt {
                if let Err(err) = writer.write_all(&[1u8]).await {
                    warn!(
                        %ack_task_id,
                        "ACK socket write failed: {} (falling back to marker)",
                        err
                    );
                } else {
                    info!(task_id = %ack_task_id, "ACK signaled via socket");
                    // Cleanup socket file if recorded
                    let socket_path_opt = {
                        let mut guard = state.lock().await;
                        guard.ack_sockets.remove(&ack_task_id)
                    };
                    if let Some(path) = socket_path_opt {
                        let _ = std::fs::remove_file(&path);
                    }
                    // Remove any pending marker file if it already exists
                    if let Some(marker) = marker_opt.take() {
                        let _ = std::fs::remove_file(&marker);
                    }
                    return Ok(None);
                }
            }
        }

        #[cfg(not(unix))]
        {
            let server_opt = {
                let mut guard = state.lock().await;
                guard.ack_pipe_servers.remove(&ack_task_id)
            };
            if let Some(mut server) = server_opt {
                if let Err(err) = server.write_all(&[1u8]).await {
                    warn!(
                        %ack_task_id,
                        "ACK pipe write failed: {} (falling back to marker)",
                        err
                    );
                } else {
                    info!(task_id = %ack_task_id, "ACK signaled via pipe");
                    if let Some(marker) = marker_opt.take() {
                        let _ = std::fs::remove_file(&marker);
                    }
                    return Ok(None);
                }
            }
        }

        if let Some(marker) = marker_opt {
            if let Some(parent) = marker.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            // Touch marker file to release the waiting hook (best-effort)
            if let Err(err) = std::fs::write(&marker, b"ack") {
                warn!(
                    task_id = %ack_task_id,
                    path = %marker.display(),
                    "Failed to write ACK marker: {}",
                    err
                );
            } else {
                info!(
                    task_id = %ack_task_id,
                    path = %marker.display(),
                    "ACK received; created marker file"
                );
            }
        } else {
            debug!(task_id = %ack_task_id, "ACK received but no pending marker found");
        }
        #[cfg(unix)]
        {
            let socket_path_opt = {
                let mut guard = state.lock().await;
                guard.ack_sockets.remove(&ack_task_id)
            };
            if let Some(path) = socket_path_opt {
                let _ = std::fs::remove_file(&path);
            }
        }
        return Ok(None);
    }

    let mut state_guard = state.lock().await;
    let lease_opt = state_guard
        .leases
        .remove(&task_id)
        .or_else(|| state_guard.expired_leases.remove(&task_id));

    if let Some(lease) = lease_opt {
        info!("Task {} completed by {}", task_id, lease.assigned_to);

        let return_target = msg
            .payload
            .get("return_to")
            .and_then(|v| v.as_str())
            .unwrap_or(&lease.original_from)
            .to_string();

        let status_str = msg
            .payload
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("completed");
        let artifacts_value = msg
            .payload
            .get("artifacts")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let parts = extract_notification_parts(artifacts_value);
        let summary = parts.summary.clone();
        let details_value = parts.details.clone();
        let evidence_value = parts.evidence.clone();
        let metadata_value = parts.metadata.clone();
        let metadata_obj = metadata_value.as_ref().and_then(|meta| meta.as_object());
        let metadata_model_requested = metadata_obj
            .and_then(|map| map.get("model_requested"))
            .and_then(|value| value.as_str())
            .map(|s| s.to_string());
        let metadata_model_used = metadata_obj
            .and_then(|map| map.get("model_used"))
            .and_then(|value| value.as_str())
            .map(|s| s.to_string());

        let mut notification = Msg {
            msg_type: "NOTIFY".to_string(),
            msg_id: Uuid::new_v4().to_string(),
            from: "orchestrator".to_string(),
            to: return_target.clone(),
            ts: now_ts(),
            nonce: Uuid::new_v4().to_string(),
            hmac: String::new(),
            payload: msg.payload.clone(),
        };

        let status_enum = map_status(status_str);
        let timestamp = Utc::now();
        let timestamp_rfc3339 = timestamp.to_rfc3339_opts(SecondsFormat::Millis, true);
        let notification_record = OrchestrationTaskNotification {
            received_at: timestamp,
            status: status_str.to_string(),
            summary: summary.clone(),
            details: details_value.clone(),
            evidence: evidence_value.clone(),
            auto_generated: true,
            metadata: metadata_value.clone(),
        };

        let mut task = state_guard.tasks_active.remove(&task_id);
        if task.is_none() {
            task = state_guard.tasks_completed.remove(&task_id);
        }

        let updated_task = if let Some(mut existing) = task {
            existing.status = status_enum.clone();
            existing.last_activity = timestamp;
            existing.notifications.push(notification_record);
            if let Some(ref dir) = lease.working_dir {
                existing.working_dir = Some(PathBuf::from(dir));
            }
            if existing.response_format.is_none() {
                existing.response_format = lease.response_format.clone();
            }
            if existing.model.is_none() {
                if let Some(model_req) = metadata_model_requested.clone() {
                    existing.model = Some(model_req);
                } else {
                    existing.model = lease.model.clone();
                }
            }
            if let Some(model_used) = metadata_model_used.clone() {
                existing.model_resolved = Some(model_used);
            }
            Some(existing)
        } else {
            Some(DelegatedTask {
                id: task_id.to_string(),
                goal: summary.clone(),
                delegated_to: lease.assigned_to.clone(),
                created_at: timestamp,
                timeout_secs: DEFAULT_TIMEOUT_SECS,
                status: status_enum.clone(),
                context: None,
                watch_patterns: Vec::new(),
                last_activity: timestamp,
                notifications: vec![notification_record],
                working_dir: lease.working_dir.as_ref().map(PathBuf::from),
                response_format: lease.response_format.clone(),
                model: metadata_model_requested
                    .clone()
                    .or_else(|| lease.model.clone()),
                model_resolved: metadata_model_used.clone(),
            })
        };

        if let Some(task) = updated_task {
            state_guard.finalize_task(task);
        }

        if let Err(err) = sign_msg(&mut notification, &state_guard.secret) {
            error!(
                "Failed to sign notification for task {} destined to {}: {}",
                task_id, return_target, err
            );
            return Ok(None);
        }
        state_guard.add_notification(&return_target, notification);

        if let Some(hook) = state_guard.notify_hook.clone() {
            let context = NotificationHookContext {
                task_id: task_id.to_string(),
                status: status_str.to_string(),
                worker: lease.assigned_to.clone(),
                return_to: return_target.clone(),
                summary: summary.clone(),
                working_dir: lease.working_dir.clone(),
                details: details_value.clone(),
                evidence: evidence_value.clone(),
                timestamp: timestamp_rfc3339,
                metadata: metadata_value.clone(),
            };
            hook_invocation = Some((hook, context));
        }

        let _ = state_guard.journal.append(
            "NOTIFY",
            &task_id,
            &msg.from,
            &return_target,
            serde_json::json!({
                "status": status_str,
                "completed_by": lease.assigned_to,
                "summary": summary,
                "details": details_value,
                "evidence": evidence_value,
                "metadata": metadata_value
            }),
        );
    } else {
        let status_str = msg
            .payload
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("completed");
        let artifacts_value = msg
            .payload
            .get("artifacts")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let parts = extract_notification_parts(artifacts_value);
        let summary = parts.summary.clone();
        let details_value = parts.details.clone();
        let evidence_value = parts.evidence.clone();
        let metadata_value = parts.metadata.clone();

        let return_target = msg
            .payload
            .get("return_to")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| msg.from.clone());

        let mut notification = Msg {
            msg_type: "NOTIFY".to_string(),
            msg_id: Uuid::new_v4().to_string(),
            from: "orchestrator".to_string(),
            to: return_target.clone(),
            ts: now_ts(),
            nonce: Uuid::new_v4().to_string(),
            hmac: String::new(),
            payload: msg.payload.clone(),
        };

        let timestamp = Utc::now();
        let timestamp_rfc3339 = timestamp.to_rfc3339_opts(SecondsFormat::Millis, true);
        let notification_record = OrchestrationTaskNotification {
            received_at: timestamp,
            status: status_str.to_string(),
            summary: summary.clone(),
            details: details_value.clone(),
            evidence: evidence_value.clone(),
            auto_generated: true,
            metadata: metadata_value.clone(),
        };

        let mut task = state_guard.tasks_active.remove(&task_id);
        if task.is_none() {
            task = state_guard.tasks_completed.remove(&task_id);
        }

        let updated_task = if let Some(mut existing) = task {
            existing.status = map_status(status_str);
            existing.last_activity = timestamp;
            existing.notifications.push(notification_record);
            Some(existing)
        } else {
            Some(DelegatedTask {
                id: task_id.to_string(),
                goal: summary.clone(),
                delegated_to: msg.from.clone(),
                created_at: timestamp,
                timeout_secs: DEFAULT_TIMEOUT_SECS,
                status: map_status(status_str),
                context: None,
                watch_patterns: Vec::new(),
                last_activity: timestamp,
                notifications: vec![notification_record],
                working_dir: None,
                response_format: None,
                model: None,
                model_resolved: None,
            })
        };

        if let Some(task) = updated_task {
            state_guard.finalize_task(task);
        }

        if let Err(err) = sign_msg(&mut notification, &state_guard.secret) {
            error!(
                "Failed to sign late notification for task {} destined to {}: {}",
                task_id, return_target, err
            );
            return Ok(None);
        }
        state_guard.add_notification(&return_target, notification);

        if let Some(hook) = state_guard.notify_hook.clone() {
            let context = NotificationHookContext {
                task_id: task_id.to_string(),
                status: status_str.to_string(),
                worker: msg.from.clone(),
                return_to: return_target.clone(),
                summary: summary.clone(),
                working_dir: None,
                details: details_value.clone(),
                evidence: evidence_value.clone(),
                timestamp: timestamp_rfc3339,
                metadata: metadata_value.clone(),
            };
            hook_invocation = Some((hook, context));
        }

        let _ = state_guard.journal.append(
            "NOTIFY",
            &task_id,
            &msg.from,
            &return_target,
            serde_json::json!({
                "status": status_str,
                "summary": summary,
                "details": details_value,
                "evidence": evidence_value,
                "metadata": metadata_value,
                "note": "late_notification"
            }),
        );
    }

    drop(state_guard);

    if let Some((hook, context)) = hook_invocation {
        let task_id = context.task_id.clone();
        let status = context.status.clone();
        let worker = context.worker.clone();
        let state_for_spawn = state.clone();
        tokio::spawn(async move {
            info!(
                %task_id,
                %status,
                %worker,
                "Launching notification hook"
            );
            match run_notify_hook(hook, context, &state_for_spawn).await {
                Ok(()) => {
                    info!(
                        %task_id,
                        %status,
                        %worker,
                        "Notification hook completed successfully"
                    );
                }
                Err(err) => {
                    warn!(
                        %task_id,
                        %status,
                        %worker,
                        "Notification hook failed: {}",
                        err
                    );
                }
            }
        });
    }

    Ok(None)
}

async fn handle_poll(msg: Msg, state: &Arc<Mutex<State>>) -> Result<Option<Msg>> {
    let mut state_guard = state.lock().await;
    let notifications = state_guard.get_notifications(&msg.from);

    if let Some(notification) = notifications.into_iter().next() {
        return Ok(Some(notification));
    }

    Ok(None)
}

async fn handle_status_request(msg: Msg, state: &Arc<Mutex<State>>) -> Result<Option<Msg>> {
    let snapshot_value = {
        let state_guard = state.lock().await;
        serde_json::to_value(state_guard.build_snapshot())?
    };

    let response = Msg {
        msg_type: "STATUS_RESPONSE".to_string(),
        msg_id: Uuid::new_v4().to_string(),
        from: "orchestrator".to_string(),
        to: msg.from,
        ts: now_ts(),
        nonce: Uuid::new_v4().to_string(),
        hmac: String::new(),
        payload: snapshot_value,
    };

    Ok(Some(response))
}

async fn handle_screenshot(msg: Msg, state: &Arc<Mutex<State>>) -> Result<Option<Msg>> {
    let (job, workspace_root) = {
        let mut state_guard = state.lock().await;
        match state_guard.screenshot.prepare_job() {
            Ok(job) => (job, state_guard.workspace_root.clone()),
            Err(err) => {
                let response = build_error_response(&msg, "E_SCREENSHOT_DENIED", &err.to_string());
                return Ok(Some(response));
            }
        }
    };

    match job.execute().await {
        Ok(result) => {
            let relative_path = workspace_root
                .as_ref()
                .and_then(|root| result.path.strip_prefix(root).ok())
                .map(|p| p.display().to_string());
            let path_str = relative_path.unwrap_or_else(|| result.path.display().to_string());

            let payload = serde_json::json!({
                "status": "ok",
                "path": path_str,
                "format": result.format,
                "size": {
                    "bytes": result.bytes,
                    "human": human_readable_size(result.bytes),
                }
            });

            let response = Msg {
                msg_type: "ACK".to_string(),
                msg_id: Uuid::new_v4().to_string(),
                from: "orchestrator".to_string(),
                to: msg.from.clone(),
                ts: now_ts(),
                nonce: Uuid::new_v4().to_string(),
                hmac: String::new(),
                payload,
            };
            Ok(Some(response))
        }
        Err(err) => {
            let response = build_error_response(&msg, "E_SCREENSHOT_FAILED", &err.to_string());
            Ok(Some(response))
        }
    }
}

fn build_error_response(msg: &Msg, code: &str, message: &str) -> Msg {
    Msg {
        msg_type: "ERR".to_string(),
        msg_id: Uuid::new_v4().to_string(),
        from: "orchestrator".to_string(),
        to: msg.from.clone(),
        ts: now_ts(),
        nonce: Uuid::new_v4().to_string(),
        hmac: String::new(),
        payload: serde_json::json!({
            "code": code,
            "message": message,
        }),
    }
}

fn record_immediate_failure(
    state: &mut State,
    target: &str,
    task_id: &str,
    delegated_to: &str,
    artifacts: serde_json::Value,
    task_details: &TaskDetails,
) {
    let mut payload = serde_json::Map::new();
    payload.insert(
        "task_id".into(),
        serde_json::Value::String(task_id.to_string()),
    );
    payload.insert(
        "status".into(),
        serde_json::Value::String("failed".to_string()),
    );
    payload.insert("artifacts".into(), artifacts.clone());
    payload.insert(
        "return_to".into(),
        serde_json::Value::String(target.to_string()),
    );

    let mut notification = Msg {
        msg_type: "NOTIFY".to_string(),
        msg_id: Uuid::new_v4().to_string(),
        from: "orchestrator".to_string(),
        to: target.to_string(),
        ts: now_ts(),
        nonce: Uuid::new_v4().to_string(),
        hmac: String::new(),
        payload: serde_json::Value::Object(payload),
    };

    if let Err(err) = sign_msg(&mut notification, &state.secret) {
        warn!(
            "Failed to sign policy/approval failure notification for task {}: {}",
            task_id, err
        );
    } else {
        state.add_notification(target, notification);
    }

    let timestamp = Utc::now();
    let parts = extract_notification_parts(artifacts);
    let details_value = parts.details.clone();
    let metadata_value = parts.metadata.clone();
    let notification_record = OrchestrationTaskNotification {
        received_at: timestamp,
        status: "failed".to_string(),
        summary: parts.summary.clone(),
        details: details_value,
        evidence: None,
        auto_generated: true,
        metadata: metadata_value.clone(),
    };

    let delegated_task = DelegatedTask {
        id: task_id.to_string(),
        goal: task_details.goal.clone(),
        delegated_to: delegated_to.to_string(),
        created_at: timestamp,
        timeout_secs: task_details.timeout_secs,
        status: TaskStatus::Failed,
        context: task_details.context.clone(),
        watch_patterns: task_details.watch_patterns.clone(),
        last_activity: timestamp,
        notifications: vec![notification_record],
        working_dir: task_details
            .working_dir
            .as_ref()
            .map(|dir| PathBuf::from(dir)),
        response_format: task_details.response_format.clone(),
        model: task_details.model.clone(),
        model_resolved: None,
    };

    state.finalize_task(delegated_task);
}

fn queue_failure_notification(
    state: &mut State,
    target: &str,
    task_id: &str,
    artifacts: serde_json::Value,
) {
    let mut payload = serde_json::Map::new();
    payload.insert(
        "task_id".into(),
        serde_json::Value::String(task_id.to_string()),
    );
    payload.insert(
        "status".into(),
        serde_json::Value::String("failed".to_string()),
    );
    payload.insert("artifacts".into(), artifacts.clone());
    payload.insert(
        "return_to".into(),
        serde_json::Value::String(target.to_string()),
    );

    let mut notification = Msg {
        msg_type: "NOTIFY".to_string(),
        msg_id: Uuid::new_v4().to_string(),
        from: "orchestrator".to_string(),
        to: target.to_string(),
        ts: now_ts(),
        nonce: Uuid::new_v4().to_string(),
        hmac: String::new(),
        payload: serde_json::Value::Object(payload),
    };

    if let Err(err) = sign_msg(&mut notification, &state.secret) {
        warn!(
            "Failed to sign supplemental failure notification for task {}: {}",
            task_id, err
        );
    } else {
        state.add_notification(target, notification);
    }
}

fn load_notify_hook() -> Option<NotifyHook> {
    match std::env::var("DEVIT_NOTIFY_HOOK") {
        Ok(command) if !command.trim().is_empty() => Some(NotifyHook { command }),
        _ => None,
    }
}

async fn run_notify_hook(
    hook: NotifyHook,
    ctx: NotificationHookContext,
    state: &Arc<Mutex<State>>,
) -> Result<()> {
    let payload = serde_json::to_string(&ctx)?;
    let NotificationHookContext {
        task_id,
        status,
        worker,
        return_to,
        summary,
        working_dir,
        details,
        evidence,
        timestamp,
        metadata,
    } = ctx;

    #[cfg(unix)]
    let mut command = {
        let mut cmd = TokioCommand::new("bash");
        cmd.arg("-lc").arg(&hook.command);
        cmd
    };

    #[cfg(not(unix))]
    let mut command = {
        use std::path::Path;

        let ps_from_env = std::env::var("DEVIT_POWERSHELL")
            .ok()
            .filter(|value| !value.trim().is_empty());

        let system_root_candidate = std::env::var("SystemRoot")
            .ok()
            .map(|root| format!(r"{}\System32\WindowsPowerShell\v1.0\powershell.exe", root));

        let fallback_paths = [
            String::from(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"),
            String::from(r"C:\Windows\Sysnative\WindowsPowerShell\v1.0\powershell.exe"),
            String::from(r"C:\Windows\SysWOW64\WindowsPowerShell\v1.0\powershell.exe"),
        ];

        let mut candidates = std::iter::once(ps_from_env)
            .flatten()
            .chain(system_root_candidate.into_iter())
            .chain(fallback_paths.into_iter())
            .chain(std::iter::once(String::from("powershell.exe")));

        let ps_executable = candidates
            .find(|candidate| {
                let path = Path::new(candidate);
                path.is_file()
                    || (path.exists() && path.extension().map(|ext| ext == "exe").unwrap_or(false))
            })
            .unwrap_or_else(|| String::from("powershell.exe"));

        let ps_exists = Path::new(&ps_executable).exists();
        debug!(
            executable = %ps_executable,
            exists = ps_exists,
            "Resolved PowerShell executable for notify hook"
        );

        let mut cmd = TokioCommand::new(&ps_executable);
        cmd.arg("-NoProfile")
            .arg("-NonInteractive")
            .arg("-ExecutionPolicy")
            .arg("Bypass")
            .arg("-STA");

        let hook_path = Path::new(&hook.command);
        debug!(
            script = %hook.command,
            exists = hook_path.exists(),
            "Notify hook script path inspection"
        );
        if hook_path.exists() {
            cmd.arg("-File").arg(&hook.command);
        } else {
            cmd.arg("-Command").arg(&hook.command);
        }
        cmd
    };
    command.env("DEVIT_NOTIFY_TASK_ID", &task_id);
    command.env("DEVIT_NOTIFY_STATUS", &status);
    command.env("DEVIT_NOTIFY_WORKER", &worker);
    command.env("DEVIT_NOTIFY_RETURN_TO", &return_to);
    command.env("DEVIT_NOTIFY_SUMMARY", &summary);
    command.env("DEVIT_NOTIFY_TIMESTAMP", &timestamp);
    if let Some(ref dir) = working_dir {
        command.env("DEVIT_NOTIFY_WORKDIR", dir);
    }
    if let Some(ref details_value) = details {
        let details_str = serde_json::to_string(details_value)?;
        command.env("DEVIT_NOTIFY_DETAILS", details_str);
    }
    if let Some(ref evidence_value) = evidence {
        let evidence_str = serde_json::to_string(evidence_value)?;
        command.env("DEVIT_NOTIFY_EVIDENCE", evidence_str);
    }
    if let Some(ref metadata_value) = metadata {
        let metadata_str = serde_json::to_string(metadata_value)?;
        command.env("DEVIT_NOTIFY_METADATA", metadata_str);
    }
    // Prepare ACK marker path and record it so an incoming ACK can signal the hook
    let ack_marker = {
        let base = std::env::temp_dir().join("devit-notify");
        let path = base.join(format!("ack-{}-{}", task_id, std::process::id()));
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        {
            let mut guard = state.lock().await;
            guard.ack_markers.insert(task_id.clone(), path.clone());
        }
        path
    };
    command.env("DEVIT_ACK_MARKER", ack_marker.as_os_str());

    // Prepare ACK socket (Unix only) to allow blocking read in hook
    #[cfg(unix)]
    {
        let socket_path = {
            let base = std::env::temp_dir().join("devit-notify");
            let p = base.join(format!("acksock-{}-{}.sock", task_id, std::process::id()));
            // ensure dir exists, remove previous
            let _ = std::fs::create_dir_all(&base);
            let _ = std::fs::remove_file(&p);
            p
        };

        match UnixListener::bind(&socket_path) {
            Ok(listener) => {
                {
                    let mut guard = state.lock().await;
                    guard
                        .ack_sockets
                        .insert(task_id.clone(), socket_path.clone());
                }
                command.env("DEVIT_ACK_SOCKET", socket_path.as_os_str());

                // Accept a single connection and store writer for later ACK
                let state_for_accept = state.clone();
                let task_for_accept = task_id.clone();
                tokio::spawn(async move {
                    match listener.accept().await {
                        Ok((stream, _)) => {
                            let (_r, w) = stream.into_split();
                            let mut guard = state_for_accept.lock().await;
                            guard.ack_writers.insert(task_for_accept, w);
                        }
                        Err(err) => {
                            warn!("ACK socket accept failed: {}", err);
                        }
                    }
                });
            }
            Err(err) => {
                warn!("Failed to bind ACK socket: {} (marker fallback only)", err);
            }
        }
    }
    // Prepare ACK pipe (Windows only) using Named Pipes; hook will block waiting for connection
    #[cfg(not(unix))]
    {
        let pipe_name = format!(r"\\.\pipe\devit-ack-{}-{}", task_id, std::process::id());
        match create_restricted_named_pipe(&pipe_name) {
            Ok(server) => {
                // Provide the pipe name to the hook
                command.env("DEVIT_ACK_PIPE", &pipe_name);
                // Accept a single client connection in the background, then store the server for later ACK write
                let state_for_accept = state.clone();
                let task_for_accept = task_id.clone();
                tokio::spawn(async move {
                    let mut server = server;
                    if let Err(err) = server.connect().await {
                        warn!("ACK pipe connect failed: {}", err);
                        return;
                    }
                    let mut guard = state_for_accept.lock().await;
                    guard.ack_pipe_servers.insert(task_for_accept, server);
                });
            }
            Err(err) => {
                warn!("Failed to create ACK pipe: {} (marker fallback only)", err);
            }
        }
    }
    command.env("DEVIT_NOTIFY_PAYLOAD", &payload);
    command.stdin(Stdio::null());
    command.stdout(Stdio::null());
    command.stderr(Stdio::inherit());

    debug!(
        %task_id,
        %status,
        %worker,
        script = %hook.command,
        "Spawning notification hook"
    );

    let status = command.status().await?;
    if !status.success() {
        anyhow::bail!("notify hook exited with status {}", status);
    }

    debug!(
        %task_id,
        %status,
        %worker,
        "Notification hook exited with success"
    );

    Ok(())
}

struct NotificationParts {
    summary: String,
    details: Option<serde_json::Value>,
    evidence: Option<serde_json::Value>,
    metadata: Option<serde_json::Value>,
}

fn extract_notification_parts(artifacts: serde_json::Value) -> NotificationParts {
    match artifacts {
        serde_json::Value::Object(map) => {
            let metadata = map.get("metadata").cloned();
            let summary = map
                .get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or("daemon notification")
                .to_string();
            let mut details = map.get("details").cloned();
            if map.get("truncated").is_some() || map.get("original_size").is_some() {
                let mut merged = match details.take() {
                    Some(serde_json::Value::Object(obj)) => obj,
                    Some(other) => {
                        let mut new_map = serde_json::Map::new();
                        new_map.insert("payload".into(), other);
                        new_map
                    }
                    None => serde_json::Map::new(),
                };
                if let Some(flag) = map.get("truncated") {
                    merged.insert("truncated".into(), flag.clone());
                }
                if let Some(size) = map.get("original_size") {
                    merged.insert("original_size".into(), size.clone());
                }
                details = Some(serde_json::Value::Object(merged));
            } else {
                details = map.get("details").cloned();
            }

            NotificationParts {
                summary,
                details,
                evidence: map.get("evidence").cloned(),
                metadata,
            }
        }
        other => NotificationParts {
            summary: other.to_string(),
            details: None,
            evidence: None,
            metadata: None,
        },
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

async fn handle_approval_decision(msg: Msg, state: &Arc<Mutex<State>>) -> Result<Option<Msg>> {
    let approval_id = &msg.msg_id;
    let status = msg
        .payload
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    info!(
        "Received approval decision: {} for approval {}",
        status, approval_id
    );

    // Find pending approval
    let pending_approval = {
        let mut state_guard = state.lock().await;
        state_guard.pending_approvals.remove(approval_id)
    };

    if let Some(pending) = pending_approval {
        let task_id = &pending.task_id;
        let tool = pending.tool.clone();
        let original_msg = pending.original_msg;
        let original_task_details = parse_task_details(original_msg.payload.get("task"), task_id);

        match status {
            "granted" => {
                info!("Approval granted for task {}", task_id);

                // Journal the approval
                {
                    let state_guard = state.lock().await;
                    let _ = state_guard.journal.append(
                        "POLICY",
                        approval_id,
                        &msg.from,
                        "orchestrator",
                        serde_json::json!({
                            "decision": "granted",
                            "tool": &tool,
                            "task_id": task_id,
                            "note": msg.payload.get("note")
                        }),
                    );
                }

                // Extract original parameters
                let worker = original_msg.to.clone();
                let return_to = original_msg
                    .payload
                    .get("return_to")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let task_details = original_task_details.clone();

                // Proceed with delegation
                proceed_with_delegation(
                    original_msg,
                    task_id,
                    &worker,
                    return_to,
                    tool.as_str(),
                    true,
                    task_details,
                    state,
                )
                .await
            }
            "denied" => {
                warn!("Approval denied for task {}", task_id);
                let note_text = msg
                    .payload
                    .get("note")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
                let summary = note_text
                    .as_ref()
                    .map(|note| format!("Approval denied for task {}: {}", task_id, note))
                    .unwrap_or_else(|| {
                        format!("Approval denied for task {} (tool: {})", task_id, tool)
                    });
                let mut detail_obj = serde_json::Map::new();
                detail_obj.insert(
                    "reason".into(),
                    serde_json::Value::String("approval_denied".into()),
                );
                detail_obj.insert("tool".into(), serde_json::Value::String(tool.clone()));
                detail_obj.insert(
                    "task_id".into(),
                    serde_json::Value::String(task_id.to_string()),
                );
                detail_obj.insert(
                    "approver".into(),
                    serde_json::Value::String(msg.from.clone()),
                );
                if let Some(note) = note_text.as_ref() {
                    detail_obj.insert("note".into(), serde_json::Value::String(note.clone()));
                }
                let detail_payload = serde_json::Value::Object(detail_obj);

                let mut failure_meta = serde_json::Map::new();
                failure_meta.insert(
                    "reason".into(),
                    serde_json::Value::String("approval_denied".into()),
                );
                failure_meta.insert("tool".into(), serde_json::Value::String(tool.clone()));
                failure_meta.insert(
                    "approver".into(),
                    serde_json::Value::String(msg.from.clone()),
                );
                failure_meta.insert(
                    "approval_id".into(),
                    serde_json::Value::String(approval_id.to_string()),
                );
                if let Some(note) = note_text.as_ref() {
                    failure_meta.insert("note".into(), serde_json::Value::String(note.clone()));
                }
                let metadata_payload = serde_json::json!({
                    "failure": serde_json::Value::Object(failure_meta)
                });
                let artifacts = serde_json::json!({
                    "summary": summary.clone(),
                    "details": detail_payload.clone(),
                    "metadata": metadata_payload.clone()
                });

                {
                    let mut state_guard = state.lock().await;
                    let _ = state_guard.journal.append(
                        "POLICY",
                        approval_id,
                        &msg.from,
                        "orchestrator",
                        serde_json::json!({
                            "decision": "denied",
                            "tool": &tool,
                            "task_id": task_id,
                            "note": msg.payload.get("note")
                        }),
                    );

                    let return_target = original_msg
                        .payload
                        .get("return_to")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| original_msg.from.clone());

                    record_immediate_failure(
                        &mut state_guard,
                        &return_target,
                        task_id,
                        &original_msg.to,
                        artifacts.clone(),
                        &original_task_details,
                    );

                    if return_target != original_msg.from {
                        queue_failure_notification(
                            &mut state_guard,
                            &original_msg.from,
                            task_id,
                            artifacts,
                        );
                    }
                }

                Ok(None)
            }
            _ => {
                warn!("Unknown approval decision: {}", status);
                Ok(None)
            }
        }
    } else {
        warn!("Approval {} not found in pending approvals", approval_id);
        Ok(None)
    }
}

// HMAC utilities
fn now_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

struct HmacCheck {
    valid: bool,
    expected: String,
    provided: String,
    body: String,
}

fn verify_hmac_detailed(msg: &Msg, secret: &str) -> Result<HmacCheck> {
    let body = canonical_body(msg);
    let provided_bytes = general_purpose::STANDARD
        .decode(msg.hmac.as_bytes())
        .unwrap_or_default();

    let mut mac_verify = HmacSha256::new_from_slice(secret.as_bytes())?;
    mac_verify.update(body.as_bytes());
    let valid = mac_verify.verify_slice(&provided_bytes).is_ok();

    let mut mac_expected = HmacSha256::new_from_slice(secret.as_bytes())?;
    mac_expected.update(body.as_bytes());
    let expected_bytes = mac_expected.finalize().into_bytes();
    let expected = general_purpose::STANDARD.encode(expected_bytes);

    Ok(HmacCheck {
        valid,
        expected,
        provided: msg.hmac.clone(),
        body,
    })
}

fn canonical_body(msg: &Msg) -> String {
    let payload = serde_json::to_string(&msg.payload).unwrap_or("{}".to_string());
    format!(
        "{}|{}|{}|{}|{}|{}|{}",
        msg.msg_type, msg.msg_id, msg.from, msg.to, msg.ts, msg.nonce, payload
    )
}

fn sign_msg(msg: &mut Msg, secret: &str) -> Result<()> {
    let body = canonical_body(msg);
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())?;
    mac.update(body.as_bytes());
    let sig = mac.finalize().into_bytes();
    msg.hmac = general_purpose::STANDARD.encode(sig);
    Ok(())
}

fn shorten_sig(sig: &str) -> String {
    if sig.len() <= 12 {
        sig.to_string()
    } else {
        format!("{}…", &sig[..12])
    }
}

fn summarize_body(body: &str) -> String {
    let total_chars = body.chars().count();
    if total_chars <= LOG_SNIPPET_LIMIT {
        body.to_string()
    } else {
        let snippet: String = body.chars().take(LOG_SNIPPET_LIMIT).collect();
        format!(
            "{}… (truncated {} chars)",
            snippet,
            total_chars - LOG_SNIPPET_LIMIT
        )
    }
}
