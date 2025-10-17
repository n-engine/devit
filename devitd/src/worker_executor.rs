use std::collections::{HashMap, HashSet};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde::Serialize;
use serde_json::{json, Value};
use strip_ansi_escapes::strip;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::process::{ChildStdin, ChildStdout, Command as TokioCommand};
use tokio::task;
use tokio::time;
use tracing::{debug, info, warn};
use uuid::Uuid;

use devit_common::orchestration::{CapabilityRateLimit, OrchestrationCapabilities};

use crate::DAEMON_VERSION;

/// Default timeout for worker execution (seconds)
const DEFAULT_TIMEOUT_SECS: u64 = 300;
const LOG_SNIPPET_LIMIT: usize = 2048;
pub const DEFAULT_APPROVER_TARGET: &str = "client:approver";

/// Worker configuration loaded from TOML.
#[derive(Debug, Clone, Deserialize)]
pub struct WorkerConfig {
    #[serde(rename = "type")]
    pub worker_type: WorkerType,
    pub binary: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub parse_mode: ParseMode,
    #[serde(default)]
    pub working_dir: Option<String>,
    #[serde(default)]
    pub max_response_chars: Option<usize>,
    #[serde(default)]
    pub mcp_tool: Option<String>,
    #[serde(default)]
    pub mcp_arguments: Option<Value>,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub allowed_models: Option<Vec<String>>,
}

fn default_timeout_secs() -> u64 {
    DEFAULT_TIMEOUT_SECS
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WorkerType {
    Cli,
    Mcp,
}

impl WorkerType {
    pub fn as_str(&self) -> &'static str {
        match self {
            WorkerType::Cli => "cli",
            WorkerType::Mcp => "mcp",
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ParseMode {
    Json,
    Text,
}

impl Default for ParseMode {
    fn default() -> Self {
        ParseMode::Json
    }
}

/// Runtime task metadata passed to the worker executor.
#[derive(Debug, Clone)]
pub struct WorkerTask {
    pub id: String,
    pub goal: String,
    pub delegated_to: String,
    pub working_dir: Option<String>,
    pub timeout_secs: Option<u64>,
    pub return_to: Option<String>,
    pub response_format: Option<String>,
    pub time_queued: DateTime<Utc>,
    pub model: Option<String>,
    pub context: Option<Value>,
}

/// Structured outcome returned by worker execution.
#[derive(Debug, Clone)]
pub struct WorkerOutcome {
    pub status: WorkerStatus,
    pub summary: String,
    pub details: Option<Value>,
    pub evidence: Option<Value>,
    pub truncated: bool,
    pub original_size: Option<usize>,
    pub metadata: TaskMetadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerStatus {
    Completed,
    Failed,
}

impl WorkerStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            WorkerStatus::Completed => "completed",
            WorkerStatus::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskMetadata {
    pub time_queued: DateTime<Utc>,
    pub time_started: DateTime<Utc>,
    pub time_completed: DateTime<Utc>,
    pub duration_total_ms: u64,
    pub duration_execution_ms: u64,
    pub worker_type: String,
    pub worker_version: Option<String>,
    pub model_requested: Option<String>,
    pub model_used: Option<String>,
    pub exit_code: i32,
    pub exit_reason: String,
    pub tokens_input: Option<u64>,
    pub tokens_output: Option<u64>,
    pub tokens_reasoning: Option<u64>,
    pub cost_estimate_usd: Option<f64>,
}

impl Default for TaskMetadata {
    fn default() -> Self {
        let now = Utc::now();
        Self {
            time_queued: now,
            time_started: now,
            time_completed: now,
            duration_total_ms: 0,
            duration_execution_ms: 0,
            worker_type: "unknown".into(),
            worker_version: None,
            model_requested: None,
            model_used: None,
            exit_code: -1,
            exit_reason: "unknown".into(),
            tokens_input: None,
            tokens_output: None,
            tokens_reasoning: None,
            cost_estimate_usd: None,
        }
    }
}

impl TaskMetadata {
    fn new(
        time_queued: DateTime<Utc>,
        time_started: DateTime<Utc>,
        time_completed: DateTime<Utc>,
        worker_type: &WorkerType,
        telemetry: ExecutionTelemetry,
    ) -> Self {
        let duration_total_ms = (time_completed - time_queued).num_milliseconds().max(0) as u64;
        let duration_execution_ms =
            (time_completed - time_started).num_milliseconds().max(0) as u64;

        Self {
            time_queued,
            time_started,
            time_completed,
            duration_total_ms,
            duration_execution_ms,
            worker_type: worker_type.as_str().to_string(),
            worker_version: telemetry.worker_version,
            model_requested: telemetry.model_requested,
            model_used: telemetry.model_used,
            exit_code: telemetry.exit_code.unwrap_or(-1),
            exit_reason: telemetry
                .exit_reason
                .unwrap_or_else(|| "unknown".to_string()),
            tokens_input: telemetry.tokens_input,
            tokens_output: telemetry.tokens_output,
            tokens_reasoning: telemetry.tokens_reasoning,
            cost_estimate_usd: telemetry.cost_estimate_usd,
        }
    }
}

/// Global worker settings loaded from configuration.
#[derive(Debug)]
pub struct WorkerSettings {
    pub configs: HashMap<String, WorkerConfig>,
    pub workspace_root: Option<PathBuf>,
    pub expected_worker_version: Option<String>,
    pub capabilities: OrchestrationCapabilities,
    pub screenshot: ScreenshotSettings,
    pub approval_target: String,
}

impl Default for WorkerSettings {
    fn default() -> Self {
        Self {
            configs: HashMap::new(),
            workspace_root: None,
            expected_worker_version: None,
            capabilities: OrchestrationCapabilities::default(),
            screenshot: ScreenshotSettings::default(),
            approval_target: DEFAULT_APPROVER_TARGET.to_string(),
        }
    }
}

#[derive(Debug, Default, Clone)]
struct ExecutionTelemetry {
    exit_code: Option<i32>,
    exit_reason: Option<String>,
    worker_version: Option<String>,
    model_requested: Option<String>,
    model_used: Option<String>,
    tokens_input: Option<u64>,
    tokens_output: Option<u64>,
    tokens_reasoning: Option<u64>,
    cost_estimate_usd: Option<f64>,
}

impl ExecutionTelemetry {
    fn apply_usage(&mut self, usage: UsageStats) {
        if self.tokens_input.is_none() {
            self.tokens_input = usage.tokens_input;
        }
        if self.tokens_output.is_none() {
            self.tokens_output = usage.tokens_output;
        }
        if self.tokens_reasoning.is_none() {
            self.tokens_reasoning = usage.tokens_reasoning;
        }
        if self.cost_estimate_usd.is_none() {
            self.cost_estimate_usd = usage.cost_estimate_usd;
        }
    }
}

#[derive(Default)]
struct UsageStats {
    tokens_input: Option<u64>,
    tokens_output: Option<u64>,
    tokens_reasoning: Option<u64>,
    cost_estimate_usd: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct ScreenshotSettings {
    pub enabled: bool,
    pub backend: ScreenshotBackend,
    pub format: String,
    pub output_dir: Option<PathBuf>,
}

impl Default for ScreenshotSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            backend: ScreenshotBackend::default(),
            format: "png".to_string(),
            output_dir: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenshotBackend {
    Scrot,
    Imagemagick,
    Native,
}

impl Default for ScreenshotBackend {
    fn default() -> Self {
        #[cfg(windows)]
        {
            ScreenshotBackend::Native
        }
        #[cfg(not(windows))]
        {
            ScreenshotBackend::Scrot
        }
    }
}

#[derive(Debug, Deserialize)]
struct ConfigFile {
    #[serde(default)]
    workspace: Option<WorkspaceSection>,
    #[serde(default)]
    workers: HashMap<String, WorkerConfig>,
}

#[derive(Debug, Deserialize)]
struct WorkspaceSection {
    #[serde(default)]
    sandbox_root: Option<String>,
}

/// Loader for worker configuration from the primary TOML file.
pub fn load_worker_settings(config_path: Option<&Path>) -> WorkerSettings {
    let Some(path) = config_path else {
        return WorkerSettings::default();
    };

    let contents = match fs::read_to_string(path) {
        Ok(value) => value,
        Err(err) => {
            warn!("Failed to read config file {}: {}", path.display(), err);
            return WorkerSettings::default();
        }
    };

    let parsed_value: Option<toml::Value> = match contents.parse::<toml::Value>() {
        Ok(value) => Some(value),
        Err(err) => {
            warn!(
                "Failed to parse {} as TOML value: {} (version enforcement disabled)",
                path.display(),
                err
            );
            None
        }
    };

    let parsed: ConfigFile = match toml::from_str(&contents) {
        Ok(cfg) => cfg,
        Err(err) => {
            warn!("Failed to parse {}: {}", path.display(), err);
            return WorkerSettings::default();
        }
    };

    let expected_worker_version = parsed_value
        .as_ref()
        .and_then(|v| v.get("daemon"))
        .and_then(|daemon| daemon.get("expected_worker_version"))
        .and_then(|value| value.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let workspace_root = parsed
        .workspace
        .and_then(|ws| ws.sandbox_root)
        .and_then(|raw| resolve_workspace_root(path, &raw));

    let mut configs = HashMap::new();
    for (name, cfg) in parsed.workers.into_iter() {
        match validate_worker_config(&name, &cfg) {
            Ok(_) => {
                configs.insert(name, cfg);
            }
            Err(err) => {
                warn!("Worker config '{}' skipped: {}", name, err);
            }
        }
    }

    if configs.is_empty() {
        warn!(
            "No worker configuration entries found in {}. Workers disabled.",
            path.display()
        );
    } else {
        info!(
            count = configs.len(),
            "Loaded {} worker definition(s) from {}",
            configs.len(),
            path.display()
        );
    }

    let capabilities = parsed_value
        .as_ref()
        .map(parse_capabilities)
        .unwrap_or_default();
    let screenshot = parsed_value
        .as_ref()
        .map(|value| parse_screenshot_settings(value, workspace_root.as_deref()))
        .unwrap_or_else(ScreenshotSettings::default);
    let approval_target = parsed_value
        .as_ref()
        .and_then(|value| value.get("daemon"))
        .and_then(|table| table.get("approvals"))
        .and_then(|approvals| approvals.get("default_target"))
        .and_then(|value| value.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_APPROVER_TARGET.to_string());

    WorkerSettings {
        configs,
        workspace_root,
        expected_worker_version,
        capabilities,
        screenshot,
        approval_target,
    }
}

fn resolve_workspace_root(config_path: &Path, raw: &str) -> Option<PathBuf> {
    let expanded = expand_home(raw);
    let path = PathBuf::from(expanded);
    let resolved = if path.is_absolute() {
        path
    } else if let Some(parent) = config_path.parent() {
        parent.join(&path)
    } else {
        path
    };

    match resolved.canonicalize() {
        Ok(canonical) => Some(canonical),
        Err(_) => Some(resolved),
    }
}

fn expand_home(raw: &str) -> String {
    if let Some(stripped) = raw.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{}/{}", home, stripped);
        }
    }
    raw.to_string()
}

fn validate_worker_config(name: &str, config: &WorkerConfig) -> Result<()> {
    if config.binary.trim().is_empty() {
        bail!("binary path empty");
    }

    let binary_path = Path::new(&config.binary);
    if binary_path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        bail!("binary contains parent directory traversal");
    }

    if config.timeout_secs == 0 {
        bail!("timeout must be greater than zero");
    }

    if let Some(limit) = config.max_response_chars {
        if limit == 0 {
            bail!("max_response_chars must be greater than zero");
        }
    }

    match config.worker_type {
        WorkerType::Cli => {}
        WorkerType::Mcp => {
            if config.parse_mode != ParseMode::Json {
                warn!(
                    "worker '{}' uses MCP backend; forcing JSON parse mode",
                    name
                );
            }

            if let Some(extra) = &config.mcp_arguments {
                if !extra.is_object() {
                    bail!("worker '{}' mcp_arguments must be a JSON object", name);
                }
            }
        }
    }

    Ok(())
}

fn parse_capabilities(root: &toml::Value) -> OrchestrationCapabilities {
    let mut capabilities = OrchestrationCapabilities::default();
    let Some(orchestration) = root.get("orchestration") else {
        return capabilities;
    };
    let Some(cap_table) = orchestration
        .get("capabilities")
        .and_then(|value| value.as_table())
    else {
        return capabilities;
    };

    if let Some(screenshot_cfg) = cap_table
        .get("screenshot")
        .and_then(|value| value.as_table())
    {
        if let Some(enabled) = screenshot_cfg
            .get("enabled")
            .and_then(|value| value.as_bool())
        {
            capabilities.screenshot.enabled = enabled;
        }

        if let Some(rate_value) = screenshot_cfg.get("rate_limit") {
            if let Some(rate) = parse_rate_limit_value(rate_value) {
                capabilities.screenshot.rate_limit = rate;
            }
        }
    }

    capabilities
}

fn parse_rate_limit_value(value: &toml::Value) -> Option<CapabilityRateLimit> {
    match value {
        toml::Value::String(raw) => CapabilityRateLimit::parse_str(raw),
        toml::Value::Integer(num) if *num > 0 => Some(CapabilityRateLimit::per_minute(*num as u32)),
        toml::Value::Float(num) if *num > 0.0 => {
            Some(CapabilityRateLimit::per_minute(num.round() as u32))
        }
        toml::Value::Table(table) => {
            let count = table
                .get("count")
                .or_else(|| table.get("max"))
                .or_else(|| table.get("max_actions"))
                .and_then(|v| v.as_integer())
                .unwrap_or(10);
            let window = table
                .get("per_seconds")
                .or_else(|| table.get("window_secs"))
                .or_else(|| table.get("interval"))
                .or_else(|| table.get("every"))
                .and_then(|v| v.as_integer())
                .unwrap_or(60);

            if count <= 0 || window <= 0 {
                None
            } else {
                Some(CapabilityRateLimit::new(count as u32, window as u64))
            }
        }
        _ => None,
    }
}

fn parse_screenshot_settings(
    root: &toml::Value,
    workspace_root: Option<&Path>,
) -> ScreenshotSettings {
    let mut settings = ScreenshotSettings::default();
    let default_dir = default_screenshot_dir(workspace_root);
    settings.output_dir = Some(default_dir.clone());

    let tools_table = root
        .get("tools")
        .and_then(|value| value.as_table())
        .and_then(|table| table.get("screenshot"))
        .and_then(|value| value.as_table());

    if let Some(table) = tools_table {
        if let Some(enabled) = table.get("enabled").and_then(|v| v.as_bool()) {
            settings.enabled = enabled;
        }
        if let Some(backend) = table.get("backend").and_then(|v| v.as_str()) {
            settings.backend = match backend.trim().to_lowercase().as_str() {
                "imagemagick" | "import" => ScreenshotBackend::Imagemagick,
                "native" => ScreenshotBackend::Native,
                "scrot" => ScreenshotBackend::Scrot,
                other => {
                    warn!(
                        "Unknown screenshot backend '{}', falling back to default",
                        other
                    );
                    ScreenshotBackend::default()
                }
            };
        }
        if let Some(format) = table.get("format").and_then(|v| v.as_str()) {
            let normalized = format.trim().to_lowercase();
            if !normalized.is_empty() {
                settings.format = normalized;
            }
        }
        if let Some(dir) = table.get("output_dir").and_then(|v| v.as_str()) {
            let resolved = resolve_screenshot_dir(dir, workspace_root);
            settings.output_dir = Some(resolved);
        }
    }

    settings
}

fn resolve_screenshot_dir(raw: &str, workspace_root: Option<&Path>) -> PathBuf {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return default_screenshot_dir(workspace_root);
    }

    let candidate = PathBuf::from(trimmed);
    let default_dir = default_screenshot_dir(workspace_root);
    let fallback_dir = fallback_screenshot_dir();

    if candidate.is_absolute() {
        if let Some(root) = workspace_root {
            if candidate.starts_with(root) {
                return candidate;
            }
        }
        if candidate.starts_with(&fallback_dir) {
            return candidate;
        }
        return default_dir;
    }

    if candidate
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return default_dir;
    }

    if let Some(root) = workspace_root {
        return root.join(candidate);
    }

    fallback_dir.join(candidate)
}

fn default_screenshot_dir(workspace_root: Option<&Path>) -> PathBuf {
    if let Some(root) = workspace_root {
        root.join(".devit").join("screenshots")
    } else {
        fallback_screenshot_dir()
    }
}

fn fallback_screenshot_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(local) = env::var_os("LOCALAPPDATA") {
            return PathBuf::from(local).join("DevIt").join("screenshots");
        }
    }
    env::temp_dir().join("devit-screenshots")
}

/// Executes a task using the configured worker backend.
pub struct WorkerExecutor {
    config: WorkerConfig,
    workspace_root: Option<PathBuf>,
}

impl WorkerExecutor {
    pub fn new(config: WorkerConfig, workspace_root: Option<PathBuf>) -> Self {
        Self {
            config,
            workspace_root,
        }
    }

    pub async fn execute_task(&self, task: &WorkerTask) -> Result<WorkerOutcome> {
        let time_started = Utc::now();
        let result = match self.config.worker_type {
            WorkerType::Cli => self.execute_cli(task).await,
            WorkerType::Mcp => self.execute_mcp(task).await,
        };
        let time_completed = Utc::now();

        match result {
            Ok((mut outcome, telemetry)) => {
                outcome.metadata = TaskMetadata::new(
                    task.time_queued,
                    time_started,
                    time_completed,
                    &self.config.worker_type,
                    telemetry,
                );
                Ok(outcome)
            }
            Err(err) => {
                let telemetry = ExecutionTelemetry {
                    exit_code: Some(-1),
                    exit_reason: Some(err.to_string()),
                    ..ExecutionTelemetry::default()
                };
                let metadata = TaskMetadata::new(
                    task.time_queued,
                    time_started,
                    time_completed,
                    &self.config.worker_type,
                    telemetry,
                );
                let outcome = WorkerOutcome {
                    status: WorkerStatus::Failed,
                    summary: format!("Worker execution error: {}", err),
                    details: None,
                    evidence: None,
                    truncated: false,
                    original_size: None,
                    metadata,
                };
                Ok(outcome)
            }
        }
    }

    fn effective_timeout(&self, task: &WorkerTask) -> Duration {
        let mut secs = self.config.timeout_secs;
        if let Some(requested) = task.timeout_secs {
            if requested > 0 {
                secs = secs.min(requested);
            }
        }
        if secs == 0 {
            Duration::from_secs(DEFAULT_TIMEOUT_SECS)
        } else {
            Duration::from_secs(secs)
        }
    }

    fn resolve_model(&self, task: &WorkerTask) -> Result<(Option<String>, Option<String>)> {
        let explicit = task
            .model
            .as_ref()
            .map(|m| m.trim())
            .filter(|m| !m.is_empty())
            .map(|m| m.to_string());

        let context_model = extract_model_from_context(&task.context);

        let requested = explicit.or(context_model);
        let resolved = requested
            .clone()
            .or_else(|| self.config.default_model.clone());

        if let Some(ref allowed) = self.config.allowed_models {
            if let Some(ref model_name) = resolved {
                if !allowed.iter().any(|entry| entry == model_name) {
                    bail!(
                        "model '{}' n'est pas autorisé pour le worker '{}'",
                        model_name,
                        task.delegated_to
                    );
                }
            }
        }

        Ok((requested, resolved))
    }

    async fn execute_cli(&self, task: &WorkerTask) -> Result<(WorkerOutcome, ExecutionTelemetry)> {
        let workspace_path = self.resolve_workspace_dir(task)?;
        let workspace_placeholder = workspace_path
            .as_ref()
            .map(|path| path.to_string_lossy().to_string())
            .or_else(|| {
                self.workspace_root
                    .as_ref()
                    .map(|root| root.to_string_lossy().to_string())
            });

        let (requested_model, resolved_model) = self.resolve_model(task)?;

        let args = self.prepare_args(
            task,
            workspace_placeholder.as_deref(),
            true,
            resolved_model.as_deref(),
        )?;
        let args_display: Vec<String> = args
            .iter()
            .map(|value| value.to_string_lossy().to_string())
            .collect();

        let mut command = StdCommand::new(&self.config.binary);
        command.args(&args);

        let mut current_dir = None;
        if let Some(ref dir) = self.config.working_dir {
            let resolved = self.replace_placeholders(
                dir,
                task,
                workspace_placeholder.as_deref(),
                resolved_model.as_deref(),
            )?;
            current_dir = Some(PathBuf::from(&resolved));
            command.current_dir(PathBuf::from(resolved));
        } else if let Some(ref dir) = workspace_path {
            current_dir = Some(dir.clone());
            command.current_dir(dir);
        }

        command.env("DEVIT_TASK_ID", &task.id);
        command.env("DEVIT_GOAL", &task.goal);
        if let Some(ref workspace) = workspace_placeholder {
            command.env("DEVIT_WORKSPACE", workspace);
        }

        if let Some(ref model) = requested_model {
            command.env("DEVIT_MODEL_REQUESTED", model);
        }
        if let Some(ref model) = resolved_model {
            command.env("DEVIT_MODEL_USED", model);
        }

        info!(
            worker = task.delegated_to,
            task_id = task.id,
            binary = %self.config.binary,
            args = ?args_display,
            current_dir = current_dir.as_ref().map(|p| p.display().to_string()),
            model = resolved_model.as_deref(),
            "Invoking worker subprocess"
        );

        let timeout = self.effective_timeout(task);
        let binary_display = self.config.binary.clone();
        let join_result = time::timeout(timeout, task::spawn_blocking(move || command.output()))
            .await
            .map_err(|_| anyhow!("worker timed out after {}s", timeout.as_secs()))?;

        let output = join_result
            .context("failed to join worker thread")?
            .context("failed to execute worker process")?;

        let stdout_text = decode_and_strip(&output.stdout);
        let stderr_text = decode_and_strip(&output.stderr);

        let stdout_summary = summarize_for_log(&stdout_text);
        let stderr_summary = summarize_for_log(&stderr_text);

        debug!(
            worker = task.delegated_to,
            task_id = task.id,
            stdout = %stdout_summary,
            stderr = %stderr_summary,
            "Worker process finished"
        );

        let response_format = task.response_format.as_deref();

        let exit_code = output.status.code().unwrap_or(-1);
        let telemetry = ExecutionTelemetry {
            exit_code: Some(exit_code),
            exit_reason: Some(if output.status.success() {
                "success".to_string()
            } else if let Some(code) = output.status.code() {
                format!("exit code {}", code)
            } else {
                "terminated by signal".to_string()
            }),
            model_requested: requested_model.clone(),
            model_used: resolved_model.clone(),
            ..ExecutionTelemetry::default()
        };

        if !output.status.success() {
            warn!(
                worker = task.delegated_to,
                task_id = task.id,
                code = ?output.status.code(),
                "Worker exited with non-zero status"
            );
            let mut outcome = WorkerOutcome {
                status: WorkerStatus::Failed,
                summary: format!("Worker '{}' failed: {}", binary_display, stderr_text.trim()),
                details: Some(json!({
                    "stdout": stdout_text,
                    "stderr": stderr_text,
                })),
                evidence: None,
                truncated: false,
                original_size: None,
                metadata: TaskMetadata::default(),
            };

            if let Some(limit) = self.config.max_response_chars {
                self.enforce_max_chars(&mut outcome, &stdout_text, response_format, limit);
            }

            return Ok((outcome, telemetry));
        }

        let mut outcome = match self.config.parse_mode {
            ParseMode::Json => self.parse_json_output(task, &stdout_text, &stderr_text)?,
            ParseMode::Text => self.parse_text_output(task, &stdout_text, &stderr_text),
        };

        if let Some(format) = response_format {
            if format.eq_ignore_ascii_case("compact") {
                self.apply_compact_format(&mut outcome, &stdout_text, &stderr_text);
            }
        }

        if let Some(limit) = self.config.max_response_chars {
            self.enforce_max_chars(&mut outcome, &stdout_text, response_format, limit);
        }

        Ok((outcome, telemetry))
    }

    async fn execute_mcp(&self, task: &WorkerTask) -> Result<(WorkerOutcome, ExecutionTelemetry)> {
        let workspace_path = self.resolve_workspace_dir(task)?;
        let workspace_placeholder = workspace_path
            .as_ref()
            .map(|path| path.to_string_lossy().to_string())
            .or_else(|| {
                self.workspace_root
                    .as_ref()
                    .map(|root| root.to_string_lossy().to_string())
            });

        let (requested_model, resolved_model) = self.resolve_model(task)?;

        let args = self.prepare_args(
            task,
            workspace_placeholder.as_deref(),
            false,
            resolved_model.as_deref(),
        )?;
        let args_display: Vec<String> = args
            .iter()
            .map(|value| value.to_string_lossy().to_string())
            .collect();

        let mut command = TokioCommand::new(&self.config.binary);
        command.args(&args);
        command.stdin(Stdio::piped());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::inherit());

        let mut current_dir = None;
        if let Some(ref dir) = self.config.working_dir {
            let resolved = self.replace_placeholders(
                dir,
                task,
                workspace_placeholder.as_deref(),
                resolved_model.as_deref(),
            )?;
            current_dir = Some(PathBuf::from(&resolved));
            command.current_dir(PathBuf::from(resolved));
        } else if let Some(ref dir) = workspace_path {
            current_dir = Some(dir.clone());
            command.current_dir(dir);
        }

        command.env("DEVIT_TASK_ID", &task.id);
        command.env("DEVIT_GOAL", &task.goal);
        if let Some(ref workspace) = workspace_placeholder {
            command.env("DEVIT_WORKSPACE", workspace);
        }
        if let Some(ref model) = requested_model {
            command.env("DEVIT_MODEL_REQUESTED", model);
        }
        if let Some(ref model) = resolved_model {
            command.env("DEVIT_MODEL_USED", model);
        }

        info!(
            worker = task.delegated_to,
            task_id = task.id,
            binary = %self.config.binary,
            args = ?args_display,
            current_dir = current_dir.as_ref().map(|p| p.display().to_string()),
            model = resolved_model.as_deref(),
            "Starting MCP worker subprocess"
        );

        let timeout = self.effective_timeout(task);
        let mut child = command
            .spawn()
            .context("failed to spawn MCP worker process")?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("MCP worker stdin unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("MCP worker stdout unavailable"))?;

        let mut session = match McpSession::new(stdin, stdout, timeout).await {
            Ok(session) => session,
            Err(err) => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return Err(err);
            }
        };
        let rpc_response = session
            .call_delegate(&self.config, task, resolved_model.as_deref())
            .await
            .context("MCP delegate call failed")?;

        let mut telemetry = ExecutionTelemetry {
            exit_code: Some(0),
            exit_reason: Some("success".to_string()),
            model_requested: requested_model.clone(),
            model_used: resolved_model.clone(),
            ..ExecutionTelemetry::default()
        };

        if let Some(result_value) = rpc_response.result.as_ref() {
            let usage = extract_usage_stats(result_value);
            telemetry.apply_usage(usage);
        }

        let cleaned_result = rpc_response.result.as_ref().map(clean_mcp_result);

        let mut outcome = if let Some(error) = rpc_response.error {
            telemetry.exit_code = Some(-1);
            telemetry.exit_reason = Some(format!("rpc error {}", error.code));
            WorkerOutcome {
                status: WorkerStatus::Failed,
                summary: format!("MCP worker error {}: {}", error.code, error.message),
                details: error
                    .data
                    .map(|data| json!({ "error": { "code": error.code, "message": error.message, "data": data } }))
                    .or_else(|| {
                        Some(json!({
                            "error": {
                                "code": error.code,
                                "message": error.message,
                            }
                        }))
                    }),
                evidence: None,
                truncated: false,
                original_size: None,
                metadata: TaskMetadata::default(),
            }
        } else if let Some(result) = cleaned_result {
            let summary = Self::summarize_mcp_result(&result);
            WorkerOutcome {
                status: WorkerStatus::Completed,
                summary,
                details: Some(result),
                evidence: None,
                truncated: false,
                original_size: None,
                metadata: TaskMetadata::default(),
            }
        } else {
            telemetry.exit_code = Some(-1);
            telemetry.exit_reason = Some("empty result".to_string());
            WorkerOutcome {
                status: WorkerStatus::Failed,
                summary: "MCP worker returned empty result".to_string(),
                details: None,
                evidence: None,
                truncated: false,
                original_size: None,
                metadata: TaskMetadata::default(),
            }
        };

        if let Some(limit) = self.config.max_response_chars {
            self.enforce_max_chars(&mut outcome, "", task.response_format.as_deref(), limit);
        }

        if let Some(id) = child.id() {
            debug!(
                worker = task.delegated_to,
                task_id = task.id,
                pid = id,
                "Stopping MCP worker subprocess"
            );
        }
        let _ = child.start_kill();
        let _ = child.wait().await;

        Ok((outcome, telemetry))
    }

    fn prepare_args(
        &self,
        task: &WorkerTask,
        workspace: Option<&str>,
        append_goal: bool,
        model: Option<&str>,
    ) -> Result<Vec<OsString>> {
        let mut args = Vec::new();
        let goal_placeholder = self.config.args.iter().any(|arg| arg.contains("{goal}"));
        let model_placeholder = self.config.args.iter().any(|arg| arg.contains("{model}"));

        if model_placeholder && model.is_none() {
            bail!(
                "worker '{}' requires a model but none was provided (configure default_model or pass model override)",
                self.config.binary
            );
        }

        for arg in &self.config.args {
            let replaced = self.replace_placeholders(arg, task, workspace, model)?;
            args.push(OsString::from(replaced));
        }

        if append_goal && !goal_placeholder {
            args.push(OsString::from(task.goal.clone()));
        }
        Ok(args)
    }

    fn replace_placeholders(
        &self,
        arg: &str,
        task: &WorkerTask,
        workspace: Option<&str>,
        model: Option<&str>,
    ) -> Result<String> {
        let mut replaced = arg.replace("{goal}", &task.goal);
        if let Some(ws) = workspace {
            replaced = replaced.replace("{workspace}", ws);
        } else if let Some(ref working_dir) = task.working_dir {
            replaced = replaced.replace("{workspace}", working_dir);
        }
        replaced = replaced.replace("{task_id}", &task.id);

        if replaced.contains("{model}") {
            let value = model.ok_or_else(|| {
                anyhow!(
                    "worker '{}' requires a model but none was resolved",
                    self.config.binary
                )
            })?;
            replaced = replaced.replace("{model}", value);
        }

        Ok(replaced)
    }

    fn summarize_mcp_result(result: &Value) -> String {
        if let Some(summary) = result.get("summary").and_then(Value::as_str) {
            let trimmed = summary.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }

        if let Some(details) = result.get("details") {
            if let Some(raw) = details
                .get("raw_excerpt")
                .and_then(Value::as_str)
                .map(|s| s.trim())
            {
                if !raw.is_empty() {
                    return raw.to_string();
                }
            }

            if let Some(Value::Object(structured)) = details.get("structured_data") {
                for value in structured.values() {
                    match value {
                        Value::Array(items) => {
                            if let Some(text) = items
                                .iter()
                                .filter_map(Value::as_str)
                                .map(|s| s.trim())
                                .find(|s| !s.is_empty())
                            {
                                return text.to_string();
                            }
                        }
                        Value::String(s) => {
                            let trimmed = s.trim();
                            if !trimmed.is_empty() {
                                return trimmed.to_string();
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        if let Some(content) = result.get("content").and_then(Value::as_array) {
            for item in content {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        return trimmed.to_string();
                    }
                }
            }
        }

        serde_json::to_string(result).unwrap_or_else(|_| "MCP worker response".to_string())
    }

    fn resolve_workspace_dir(&self, task: &WorkerTask) -> Result<Option<PathBuf>> {
        let Some(ref relative) = task.working_dir else {
            return Ok(self.workspace_root.clone());
        };

        let candidate = PathBuf::from(relative);
        if candidate.is_absolute() {
            return Ok(Some(candidate));
        }

        if let Some(root) = &self.workspace_root {
            let joined = root.join(&candidate);
            return Ok(Some(joined));
        }

        Ok(Some(candidate))
    }

    fn parse_json_output(
        &self,
        task: &WorkerTask,
        stdout: &str,
        stderr: &str,
    ) -> Result<WorkerOutcome> {
        let trimmed = stdout.trim();
        if trimmed.is_empty() {
            bail!("worker produced no JSON output");
        }

        let parsed = parse_json_payload(trimmed)
            .with_context(|| format!("invalid JSON output for task {}", task.id))?;

        let summary = extract_summary(&parsed).unwrap_or_else(|| "Worker completed".to_string());
        let mut details = serde_json::Map::new();
        details.insert("stdout".into(), parsed);
        if !stderr.trim().is_empty() {
            details.insert("stderr".into(), Value::String(stderr.trim().to_string()));
        }

        Ok(WorkerOutcome {
            status: WorkerStatus::Completed,
            summary,
            details: Some(Value::Object(details)),
            evidence: None,
            truncated: false,
            original_size: None,
            metadata: TaskMetadata::default(),
        })
    }

    fn parse_text_output(&self, task: &WorkerTask, stdout: &str, stderr: &str) -> WorkerOutcome {
        let trimmed = stdout.trim();
        let summary = if trimmed.is_empty() {
            format!("Worker {} completed without output", task.delegated_to)
        } else {
            truncate(trimmed, 240).to_string()
        };

        let details = Value::Object(
            vec![
                ("stdout".to_string(), Value::String(stdout.to_string())),
                ("stderr".to_string(), Value::String(stderr.to_string())),
            ]
            .into_iter()
            .collect(),
        );

        WorkerOutcome {
            status: WorkerStatus::Completed,
            summary,
            details: Some(details),
            evidence: None,
            truncated: false,
            original_size: None,
            metadata: TaskMetadata::default(),
        }
    }

    fn apply_compact_format(&self, outcome: &mut WorkerOutcome, stdout: &str, stderr: &str) {
        if stdout.trim().is_empty() {
            return;
        }

        if let Some(Value::Object(details)) = &outcome.details {
            if matches!(details.get("structured_data"), Some(Value::Object(_))) {
                return;
            }
            if matches!(
                details.get("stdout"),
                Some(Value::Object(_)) | Some(Value::Array(_))
            ) {
                return;
            }
        }

        let structured = build_compact_structured_data(stdout);
        outcome.summary = "Analysis complete. See structured_data.".to_string();

        let mut compact_details = serde_json::Map::new();
        compact_details.insert("structured_data".into(), Value::Object(structured));
        compact_details.insert(
            "raw_excerpt".into(),
            Value::String(make_excerpt(stdout, 600)),
        );
        if !stderr.trim().is_empty() {
            compact_details.insert("stderr".into(), Value::String(stderr.trim().to_string()));
        }
        compact_details.insert("format".into(), Value::String("compact".into()));

        if let Some(Value::Object(existing)) = outcome.details.take() {
            if let Some(evidence) = existing.get("evidence") {
                compact_details.insert("evidence".into(), evidence.clone());
            }
        }

        outcome.details = Some(Value::Object(compact_details));
    }

    fn enforce_max_chars(
        &self,
        outcome: &mut WorkerOutcome,
        stdout: &str,
        response_format: Option<&str>,
        limit: usize,
    ) {
        if stdout.len() <= limit {
            return;
        }

        outcome.truncated = true;
        outcome.original_size = Some(stdout.len());
        warn!(
            task_summary = %outcome.summary,
            original = stdout.len(),
            limit,
            "Worker stdout exceeded limit; truncating"
        );

        if matches!(response_format, Some(fmt) if fmt.eq_ignore_ascii_case("compact")) {
            if let Some(Value::Object(map)) = outcome.details.as_mut() {
                map.entry("original_size")
                    .or_insert(Value::Number((stdout.len() as u64).into()));
                map.entry("truncated").or_insert(Value::Bool(true));
            } else {
                let mut map = serde_json::Map::new();
                map.insert("truncated".into(), Value::Bool(true));
                map.insert(
                    "original_size".into(),
                    Value::Number((stdout.len() as u64).into()),
                );
                map.insert("excerpt".into(), Value::String(make_excerpt(stdout, limit)));
                outcome.details = Some(Value::Object(map));
            }
            return;
        }

        let truncated_value = create_truncated_value(stdout, limit);
        let mut map = match outcome.details.take() {
            Some(Value::Object(existing)) => existing,
            Some(other) => {
                let mut new_map = serde_json::Map::new();
                new_map.insert("data".into(), other);
                new_map
            }
            None => serde_json::Map::new(),
        };
        map.insert("stdout".into(), truncated_value);
        map.insert("truncated".into(), Value::Bool(true));
        map.insert(
            "original_size".into(),
            Value::Number((stdout.len() as u64).into()),
        );
        outcome.details = Some(Value::Object(map));
    }
}

fn decode_and_strip(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }

    let decode = |data: &[u8]| String::from_utf8_lossy(data).to_string();

    if !bytes.contains(&b'\x1b') {
        return decode(bytes);
    }

    match strip(bytes) {
        Ok(clean) => decode(&clean),
        Err(err) => {
            warn!(?err, "Failed to strip ANSI escapes from worker output");
            decode(bytes)
        }
    }
}

fn extract_model_from_context(context: &Option<Value>) -> Option<String> {
    let value = context.as_ref()?;
    match value {
        Value::Object(map) => map
            .get("model")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        _ => None,
    }
}

fn summarize_for_log(payload: &str) -> String {
    if payload.len() <= LOG_SNIPPET_LIMIT {
        payload.to_string()
    } else {
        let snippet = truncate(payload, LOG_SNIPPET_LIMIT);
        format!(
            "{}… (truncated {} chars)",
            snippet,
            payload.len() - LOG_SNIPPET_LIMIT
        )
    }
}

fn truncate(input: &str, max: usize) -> &str {
    if input.len() <= max {
        return input;
    }
    let mut end = max;
    while !input.is_char_boundary(end) {
        end -= 1;
    }
    &input[..end]
}

fn parse_json_payload(raw: &str) -> Result<Value> {
    if let Ok(value) = serde_json::from_str::<Value>(raw) {
        return Ok(value);
    }

    for line in raw.lines().rev() {
        let candidate = line.trim();
        if candidate.is_empty() {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<Value>(candidate) {
            return Ok(value);
        }
    }

    Err(anyhow!("unable to parse JSON payload"))
}

fn extract_summary(value: &Value) -> Option<String> {
    if let Some(result) = value.get("result").and_then(Value::as_str) {
        return Some(result.to_string());
    }
    if let Some(response) = value.get("response").and_then(Value::as_str) {
        return Some(response.to_string());
    }
    if let Some(content) = value.get("content") {
        if let Some(array) = content.as_array() {
            for item in array {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    return Some(text.to_string());
                }
            }
        }
    }
    None
}

fn build_compact_structured_data(text: &str) -> serde_json::Map<String, Value> {
    let mut new_funcs: HashSet<String> = HashSet::new();
    let mut modified_funcs: HashSet<String> = HashSet::new();
    let mut risks: Vec<String> = Vec::new();
    let mut highlights: Vec<String> = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lower = trimmed.to_lowercase();
        if lower.contains("new function") {
            if let Some(name) = extract_identifier(trimmed) {
                new_funcs.insert(name);
            }
        }
        if lower.contains("modified function")
            || lower.contains("updated function")
            || lower.contains("changes in")
        {
            if let Some(name) = extract_identifier(trimmed) {
                modified_funcs.insert(name);
            }
        }
        if lower.contains("risk") || lower.contains("concern") || lower.contains("issue") {
            if risks.len() < 5 {
                risks.push(trimmed.to_string());
            }
        }
        if (trimmed.starts_with('-') || trimmed.starts_with('*')) && highlights.len() < 5 {
            highlights.push(
                trimmed
                    .trim_start_matches(|c: char| c == '-' || c == '*' || c.is_whitespace())
                    .to_string(),
            );
        }
    }

    if highlights.is_empty() {
        for sentence in text.split_terminator(['.', '!', '?']) {
            let snippet = sentence.trim();
            if snippet.is_empty() {
                continue;
            }
            highlights.push(snippet.to_string());
            if highlights.len() >= 5 {
                break;
            }
        }
    }

    let mut map = serde_json::Map::new();

    if !new_funcs.is_empty() {
        let mut items: Vec<_> = new_funcs.into_iter().collect();
        items.sort();
        map.insert(
            "new_funcs".into(),
            Value::Array(items.into_iter().map(Value::String).collect()),
        );
    }

    if !modified_funcs.is_empty() {
        let mut items: Vec<_> = modified_funcs.into_iter().collect();
        items.sort();
        map.insert(
            "modified_funcs".into(),
            Value::Array(items.into_iter().map(Value::String).collect()),
        );
    }

    if !risks.is_empty() {
        map.insert(
            "risks".into(),
            Value::Array(risks.into_iter().map(Value::String).collect()),
        );
    }

    if map.is_empty() || !highlights.is_empty() {
        let limited: Vec<_> = highlights.into_iter().take(5).collect();
        map.insert(
            "highlights".into(),
            Value::Array(limited.into_iter().map(Value::String).collect()),
        );
    }

    map
}

fn extract_identifier(line: &str) -> Option<String> {
    for token in line.split_whitespace() {
        let cleaned = token.trim_matches(|c: char| "`".contains(c) || c.is_ascii_punctuation());
        if cleaned.is_empty() {
            continue;
        }
        if let Some(idx) = cleaned.find('(') {
            let name = &cleaned[..idx];
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    line.split_whitespace().next().map(|word| {
        word.trim_matches(|c: char| c.is_ascii_punctuation())
            .to_string()
    })
}

fn make_excerpt(text: &str, limit: usize) -> String {
    let trimmed = text.trim();
    if trimmed.len() <= limit {
        return trimmed.to_string();
    }
    let truncated = truncate(trimmed, limit);
    format!("{}… [excerpt]", truncated)
}

fn create_truncated_value(text: &str, limit: usize) -> Value {
    if serde_json::from_str::<Value>(text).is_ok() {
        Value::String(truncate_json_excerpt(text, limit))
    } else {
        Value::String(truncate_plain(text, limit))
    }
}

fn truncate_plain(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_string();
    }
    let truncated = truncate(text, limit);
    format!("{}… [truncated]", truncated)
}

fn truncate_json_excerpt(text: &str, limit: usize) -> String {
    let safe_limit = limit.min(text.len());
    let mut end = safe_limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let candidate = &text[..end];
    if let Some(idx) = candidate.rfind('}') {
        let slice = &candidate[..=idx];
        format!("{} … [truncated]", slice)
    } else {
        truncate_plain(text, limit)
    }
}

fn clean_mcp_result(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut cleaned = map.clone();
            if let Some(Value::Object(details)) = cleaned.get_mut("details") {
                if let Some(Value::String(raw)) = details.get_mut("raw_excerpt") {
                    *raw = raw.trim().to_string();
                }

                if let Some(Value::Object(structured)) = details.get_mut("structured_data") {
                    normalize_string_fields(structured);
                }

                details.remove("stderr");
            }
            Value::Object(cleaned)
        }
        _ => value.clone(),
    }
}

fn normalize_string_fields(map: &mut serde_json::Map<String, Value>) {
    for value in map.values_mut() {
        match value {
            Value::String(s) => {
                *s = s.trim().to_string();
            }
            Value::Array(items) => {
                for item in items.iter_mut() {
                    if let Value::String(s) = item {
                        *s = s.trim().to_string();
                    }
                }
            }
            Value::Object(nested) => normalize_string_fields(nested),
            _ => {}
        }
    }
}

fn extract_usage_stats(value: &Value) -> UsageStats {
    let mut stats = UsageStats::default();
    collect_usage_stats(value, &mut stats, None);
    stats
}

fn collect_usage_stats(value: &Value, stats: &mut UsageStats, parent_key: Option<&str>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let lower_key = key.to_ascii_lowercase();
                match lower_key.as_str() {
                    "input_tokens" | "tokens_input" | "prompt_tokens" | "prompt_token_count" => {
                        assign_u64(&mut stats.tokens_input, child);
                    }
                    "output_tokens"
                    | "tokens_output"
                    | "completion_tokens"
                    | "completion_token_count" => {
                        assign_u64(&mut stats.tokens_output, child);
                    }
                    "reasoning_tokens" | "tokens_reasoning" => {
                        assign_u64(&mut stats.tokens_reasoning, child);
                    }
                    "total_tokens" | "total_token_count" => {
                        if stats.tokens_output.is_none() {
                            assign_u64(&mut stats.tokens_output, child);
                        }
                    }
                    "cost_usd" | "estimated_cost_usd" | "total_cost_usd" | "cost_estimate_usd"
                    | "usage_cost_usd" => {
                        assign_f64(&mut stats.cost_estimate_usd, child);
                    }
                    "usd" | "total_cost" | "cost" | "price_usd" => {
                        if matches!(
                            parent_key,
                            Some("cost")
                                | Some("pricing")
                                | Some("usage")
                                | Some("billing")
                                | Some("estimate")
                                | Some("estimated_cost")
                                | Some("charge")
                        ) {
                            assign_f64(&mut stats.cost_estimate_usd, child);
                        }
                    }
                    _ => {}
                }
                collect_usage_stats(child, stats, Some(lower_key.as_str()));
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_usage_stats(item, stats, parent_key);
            }
        }
        _ => {}
    }
}

fn assign_u64(slot: &mut Option<u64>, value: &Value) {
    if slot.is_some() {
        return;
    }
    if let Some(number) = value_as_u64(value) {
        *slot = Some(number);
    }
}

fn assign_f64(slot: &mut Option<f64>, value: &Value) {
    if slot.is_some() {
        return;
    }
    if let Some(number) = value_as_f64(value) {
        *slot = Some(number);
    }
}

fn value_as_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(num) => {
            if let Some(val) = num.as_u64() {
                Some(val)
            } else if let Some(val) = num.as_i64() {
                (val >= 0).then_some(val as u64)
            } else {
                num.as_f64().map(|v| v.max(0.0).round() as u64)
            }
        }
        Value::String(text) => text.trim().parse::<u64>().ok(),
        _ => None,
    }
}

fn value_as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(num) => num.as_f64(),
        Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    }
}

struct McpSession {
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    timeout: Duration,
}

#[derive(Clone, Debug)]
struct RpcError {
    code: i32,
    message: String,
    data: Option<Value>,
}

#[derive(Clone, Debug)]
struct RpcResponse {
    result: Option<Value>,
    error: Option<RpcError>,
}

#[derive(Deserialize)]
struct RawRpcResponse {
    #[serde(default)]
    jsonrpc: String,
    #[serde(default)]
    id: Value,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<RawRpcError>,
}

#[derive(Deserialize)]
struct RawRpcError {
    code: i32,
    message: String,
    #[serde(default)]
    data: Option<Value>,
}

impl From<RawRpcResponse> for RpcResponse {
    fn from(raw: RawRpcResponse) -> RpcResponse {
        RpcResponse {
            result: raw.result,
            error: raw.error.map(|err| RpcError {
                code: err.code,
                message: err.message,
                data: err.data,
            }),
        }
    }
}

impl McpSession {
    async fn new(stdin: ChildStdin, stdout: ChildStdout, timeout: Duration) -> Result<Self> {
        let mut session = Self {
            stdin: BufWriter::new(stdin),
            stdout: BufReader::new(stdout),
            timeout,
        };
        session.initialize().await?;
        Ok(session)
    }

    async fn initialize(&mut self) -> Result<()> {
        let init_params = json!({
            "protocolVersion": "1.0",
            "clientInfo": {
                "name": "devitd",
                "version": DAEMON_VERSION,
            },
            "serverInfo": {
                "name": "codex-mcp",
                "version": "0"
            },
            "capabilities": {
                "tools": {}
            }
        });
        if let Some(err) = self.rpc_call("initialize", Some(init_params)).await?.error {
            bail!("MCP initialize failed: {} ({})", err.message, err.code);
        }

        if let Some(err) = self.rpc_call("tools/list", Some(json!({}))).await?.error {
            bail!("MCP tools/list failed: {} ({})", err.message, err.code);
        }
        Ok(())
    }

    async fn call_delegate(
        &mut self,
        config: &WorkerConfig,
        task: &WorkerTask,
        model: Option<&str>,
    ) -> Result<RpcResponse> {
        let mut arguments = serde_json::Map::new();

        if let Some(extra) = config.mcp_arguments.as_ref() {
            if let Some(map) = extra.as_object() {
                for (key, value) in map {
                    arguments.insert(key.clone(), value.clone());
                }
            }
        }

        arguments
            .entry("goal".to_string())
            .or_insert(Value::String(task.goal.clone()));
        arguments
            .entry("prompt".to_string())
            .or_insert(Value::String(task.goal.clone()));

        if let Some(timeout) = task.timeout_secs {
            arguments
                .entry("timeout".to_string())
                .or_insert(Value::Number(timeout.into()));
        }
        if let Some(ref working_dir) = task.working_dir {
            arguments
                .entry("working_dir".to_string())
                .or_insert(Value::String(working_dir.clone()));
        }
        if let Some(ref format) = task.response_format {
            arguments
                .entry("format".to_string())
                .or_insert(Value::String(format.clone()));
        }
        if let Some(model_name) = model {
            arguments
                .entry("model".to_string())
                .or_insert(Value::String(model_name.to_string()));
        }

        let tool_name = config.mcp_tool.as_deref().unwrap_or("devit_delegate");

        let params = json!({
            "name": tool_name,
            "arguments": Value::Object(arguments),
        });

        self.rpc_call("tools/call", Some(params)).await
    }

    async fn rpc_call(&mut self, method: &str, params: Option<Value>) -> Result<RpcResponse> {
        let id_value = Value::String(Uuid::new_v4().to_string());
        let mut request = serde_json::Map::new();
        request.insert("jsonrpc".into(), Value::String("2.0".into()));
        request.insert("id".into(), id_value.clone());
        request.insert("method".into(), Value::String(method.to_string()));
        if let Some(params_value) = params {
            request.insert("params".into(), params_value);
        }

        let payload = Value::Object(request);
        self.write_json(&payload).await?;
        self.read_response_matching(&id_value).await
    }

    async fn write_json(&mut self, value: &Value) -> Result<()> {
        let serialized = serde_json::to_string(value)?;
        self.stdin.write_all(serialized.as_bytes()).await?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn read_response_matching(&mut self, expected_id: &Value) -> Result<RpcResponse> {
        loop {
            let mut line = String::new();
            let bytes_read = tokio::time::timeout(self.timeout, self.stdout.read_line(&mut line))
                .await
                .map_err(|_| anyhow!("MCP worker timed out after {}s", self.timeout.as_secs()))?
                .context("failed to read MCP worker response")?;

            if bytes_read == 0 {
                bail!("MCP worker closed the connection unexpectedly");
            }

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let parsed: RawRpcResponse =
                serde_json::from_str(trimmed).context("invalid JSON from MCP worker")?;

            if parsed.jsonrpc != "2.0" {
                warn!(
                    "Ignoring MCP response with unsupported jsonrpc value '{}'",
                    parsed.jsonrpc
                );
            }

            if parsed.id != *expected_id {
                debug!(
                    "Skipping MCP response with mismatched id (expected {:?}, got {:?})",
                    expected_id, parsed.id
                );
                continue;
            }

            return Ok(parsed.into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_handles_unicode() {
        let input = "éééé";
        assert_eq!(truncate(input, 3), "éé");
    }

    #[test]
    fn decode_and_strip_removes_ansi_sequences() {
        let input = b"\x1b[31mhello\x1b[0m";
        let output = decode_and_strip(input);
        assert_eq!(output, "hello");
    }

    #[test]
    fn decode_and_strip_passthrough_without_escape() {
        let input = "plain text";
        let output = decode_and_strip(input.as_bytes());
        assert_eq!(output, input);
    }

    #[test]
    fn task_metadata_new_computes_durations() {
        let time_queued = DateTime::<Utc>::from_timestamp(1, 0).unwrap();
        let time_started = DateTime::<Utc>::from_timestamp(2, 0).unwrap();
        let time_completed = DateTime::<Utc>::from_timestamp(4, 500_000_000).unwrap();

        let telemetry = ExecutionTelemetry {
            exit_code: Some(0),
            exit_reason: Some("success".into()),
            ..ExecutionTelemetry::default()
        };

        let metadata = TaskMetadata::new(
            time_queued,
            time_started,
            time_completed,
            &WorkerType::Cli,
            telemetry,
        );

        assert_eq!(metadata.duration_total_ms, 3500);
        assert_eq!(metadata.duration_execution_ms, 2500);
        assert_eq!(metadata.exit_code, 0);
        assert_eq!(metadata.exit_reason, "success");
        assert_eq!(metadata.worker_type, "cli");
        assert!(metadata.model_requested.is_none());
        assert!(metadata.model_used.is_none());
    }
}
