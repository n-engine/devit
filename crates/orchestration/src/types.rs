use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

pub const DEFAULT_TIMEOUT_SECS: u64 = 30 * 60; // 30 minutes
pub const DEFAULT_DAEMON_SOCKET: &str = "/tmp/devitd.sock";
pub const DEFAULT_DAEMON_START_TIMEOUT_MS: u64 = 3_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DelegatedTask {
    pub id: String,
    pub goal: String,
    pub delegated_to: String,
    pub created_at: DateTime<Utc>,
    pub timeout_secs: u64,
    pub status: TaskStatus,
    pub context: Option<Value>,
    pub watch_patterns: Vec<String>,
    pub last_activity: DateTime<Utc>,
    pub notifications: Vec<TaskNotification>,
    #[serde(default)]
    pub working_dir: Option<PathBuf>,
    #[serde(default)]
    pub response_format: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub model_resolved: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskNotification {
    pub received_at: DateTime<Utc>,
    pub status: String,
    pub summary: String,
    pub details: Option<Value>,
    pub evidence: Option<Value>,
    pub auto_generated: bool,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OrchestrationStatus {
    pub active_tasks: Vec<DelegatedTask>,
    pub completed_tasks: Vec<DelegatedTask>,
    pub summary: OrchestrationSummary,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OrchestrationSummary {
    pub total_active: usize,
    pub total_completed: usize,
    pub total_failed: usize,
    pub oldest_active_task: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestrationConfig {
    pub max_concurrent_tasks: usize,
    pub default_timeout_secs: u64,
    pub default_watch_patterns: Vec<String>,
    #[serde(default = "default_orchestration_mode")]
    pub mode: OrchestrationMode,
    #[serde(default)]
    pub daemon_socket: Option<String>,
    #[serde(default = "default_auto_start_daemon")]
    pub auto_start_daemon: bool,
    #[serde(default = "default_daemon_start_timeout_ms")]
    pub daemon_start_timeout_ms: u64,
    #[serde(default)]
    pub capabilities: OrchestrationCapabilities,
}

impl Default for OrchestrationConfig {
    fn default() -> Self {
        Self {
            max_concurrent_tasks: 5,
            default_timeout_secs: DEFAULT_TIMEOUT_SECS,
            default_watch_patterns: vec!["*.rs".into(), "Cargo.toml".into()],
            mode: OrchestrationMode::Auto,
            daemon_socket: Some(DEFAULT_DAEMON_SOCKET.to_string()),
            auto_start_daemon: true,
            daemon_start_timeout_ms: DEFAULT_DAEMON_START_TIMEOUT_MS,
            capabilities: OrchestrationCapabilities::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrchestrationMode {
    Local,
    Daemon,
    Auto,
}

pub fn default_orchestration_mode() -> OrchestrationMode {
    OrchestrationMode::Auto
}

pub fn default_auto_start_daemon() -> bool {
    true
}

pub fn default_daemon_start_timeout_ms() -> u64 {
    DEFAULT_DAEMON_START_TIMEOUT_MS
}

#[derive(Debug, Clone)]
pub struct DelegateResult {
    pub task_id: String,
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, Copy)]
pub enum StatusFilter {
    All,
    Active,
    Completed,
    Failed,
}

impl StatusFilter {
    pub fn from_str(value: Option<&str>) -> Self {
        match value.unwrap_or("all") {
            "active" => StatusFilter::Active,
            "completed" => StatusFilter::Completed,
            "failed" => StatusFilter::Failed,
            _ => StatusFilter::All,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            StatusFilter::All => "all",
            StatusFilter::Active => "active",
            StatusFilter::Completed => "completed",
            StatusFilter::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OrchestrationCapabilities {
    pub screenshot: CapabilityToggle,
}

impl Default for OrchestrationCapabilities {
    fn default() -> Self {
        Self {
            screenshot: CapabilityToggle::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CapabilityToggle {
    pub enabled: bool,
    #[serde(default)]
    #[serde(with = "capability_rate_limit_serde")]
    pub rate_limit: CapabilityRateLimit,
}

impl Default for CapabilityToggle {
    fn default() -> Self {
        Self {
            enabled: false,
            rate_limit: CapabilityRateLimit::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityRateLimit {
    pub max_actions: u32,
    pub per_seconds: u64,
}

impl CapabilityRateLimit {
    pub const fn new(max_actions: u32, per_seconds: u64) -> Self {
        Self {
            max_actions,
            per_seconds,
        }
    }

    pub const fn per_minute(max_actions: u32) -> Self {
        Self::new(max_actions, 60)
    }

    pub fn parse_str(input: &str) -> Option<Self> {
        capability_rate_limit_serde::parse_rate_limit_str(input).ok()
    }
}

impl Default for CapabilityRateLimit {
    fn default() -> Self {
        Self::per_minute(10)
    }
}

mod capability_rate_limit_serde {
    use super::CapabilityRateLimit;
    use serde::{de::Error, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &CapabilityRateLimit, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let window = match value.per_seconds {
            1 => "second",
            60 => "minute",
            3600 => "hour",
            other => return serializer.serialize_str(&format!("{}/{}s", value.max_actions, other)),
        };
        serializer.serialize_str(&format!("{}/{}", value.max_actions, window))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<CapabilityRateLimit, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = serde_json::Value::deserialize(deserializer)?;
        parse_value(&raw).map_err(D::Error::custom)
    }

    fn parse_value(value: &serde_json::Value) -> Result<CapabilityRateLimit, String> {
        match value {
            serde_json::Value::String(s) => parse_rate_limit_str(s),
            serde_json::Value::Object(map) => {
                let max_actions = map
                    .get("count")
                    .or_else(|| map.get("max"))
                    .or_else(|| map.get("max_actions"))
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| "missing count field".to_string())?;
                let per_seconds = map
                    .get("per_seconds")
                    .or_else(|| map.get("window_secs"))
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| "missing per_seconds field".to_string())?;
                Ok(CapabilityRateLimit::new(max_actions as u32, per_seconds))
            }
            serde_json::Value::Number(num) => num
                .as_u64()
                .map(|count| CapabilityRateLimit::per_minute(count as u32))
                .ok_or_else(|| "invalid numeric rate limit".to_string()),
            _ => Err("unsupported rate limit format".to_string()),
        }
    }

    pub(super) fn parse_rate_limit_str(raw: &str) -> Result<CapabilityRateLimit, String> {
        let normalized = raw.trim().to_lowercase();
        if normalized.is_empty() {
            return Ok(CapabilityRateLimit::default());
        }

        let (count_str, period_str) = if let Some((left, right)) = normalized.split_once('/') {
            (left.trim(), right.trim())
        } else if let Some(idx) = normalized.find("per") {
            let (left, right) = normalized.split_at(idx);
            (left.trim(), right.trim_start_matches("per").trim())
        } else {
            return Err(format!("invalid rate limit '{}'", raw));
        };

        let count: u32 = count_str
            .parse()
            .map_err(|_| format!("invalid rate limit count '{}'", count_str))?;

        let per_seconds = match period_str.trim_matches(|c: char| c == 's' || c == ' ') {
            "sec" | "second" => 1,
            "min" | "minute" => 60,
            "hour" | "hr" => 3600,
            other => parse_free_form_window(other)
                .ok_or_else(|| format!("invalid rate limit window '{}'", period_str))?,
        };

        Ok(CapabilityRateLimit::new(count, per_seconds))
    }

    fn parse_free_form_window(raw: &str) -> Option<u64> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Some(60);
        }

        let (num_part, multiplier) = if let Some(stripped) = trimmed.strip_suffix('h') {
            (stripped, 3600)
        } else if let Some(stripped) = trimmed.strip_suffix('m') {
            (stripped, 60)
        } else if let Some(stripped) = trimmed.strip_suffix('s') {
            (stripped, 1)
        } else {
            (trimmed, 1)
        };

        let value = num_part.trim().parse::<u64>().ok()?;
        let seconds = value.saturating_mul(multiplier);
        if seconds == 0 {
            None
        } else {
            Some(seconds)
        }
    }
}
