//! DevIt Client Library
//!
//! Client library for communicating with the DevIt orchestration daemon.
//! Provides high-level API for task delegation and notifications.

pub mod compact;

use anyhow::{anyhow, Result};
use base64::{engine::general_purpose, Engine as _};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
#[cfg(not(unix))]
use tokio::net::TcpStream;
#[cfg(unix)]
use tokio::net::UnixStream;
use tokio::time::{sleep, timeout};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

#[cfg(unix)]
pub const DEFAULT_SOCK: &str = "/tmp/devitd.sock";
#[cfg(not(unix))]
pub const DEFAULT_SOCK: &str = "127.0.0.1:60459";

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("disconnected")]
    Disconnected,
    #[error("invalid response: {0}")]
    InvalidResponse(String),
    #[error("timeout")]
    Timeout,
    #[error("daemon not available")]
    DaemonUnavailable,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Msg {
    pub msg_type: String, // REGISTER, HEARTBEAT, DELEGATE, NOTIFY, ACK, ERR
    pub msg_id: String,
    pub from: String,
    pub to: String,
    pub ts: u64,
    pub nonce: String,
    pub hmac: String,
    pub payload: serde_json::Value,
}

/// DevIt client for communicating with the orchestration daemon
pub struct DevitClient {
    sock_path: String,
    ident: String,
    secret: String,
    use_compact: bool,
    client_version: Option<String>,
    daemon_version: Option<String>,
    caps: Vec<String>,
}

impl DevitClient {
    /// Connect to the DevIt daemon
    pub async fn connect<P: AsRef<Path>>(sock: P, ident: &str, secret: &str) -> Result<Self> {
        let version_env = std::env::var("DEVIT_CLIENT_VERSION").ok();
        Self::connect_with_version(sock, ident, secret, version_env).await
    }

    /// Connect to the daemon while advertising a specific client version
    pub async fn connect_with_version<P: AsRef<Path>>(
        sock: P,
        ident: &str,
        secret: &str,
        version: Option<String>,
    ) -> Result<Self> {
        Self::connect_with_capabilities(sock, ident, secret, version, vec![]).await
    }

    /// Connect to the daemon advertising explicit capabilities
    pub async fn connect_with_capabilities<P: AsRef<Path>>(
        sock: P,
        ident: &str,
        secret: &str,
        version: Option<String>,
        caps: Vec<String>,
    ) -> Result<Self> {
        let sock_path = sock.as_ref().to_string_lossy().to_string();
        let use_compact = std::env::var("USE_COMPACT").ok().as_deref() == Some("1");

        let default_caps = vec!["delegate".to_string(), "notify".to_string()];
        let mut merged_caps = if caps.is_empty() {
            default_caps.clone()
        } else {
            caps
        };
        merged_caps.extend(default_caps);
        merged_caps.sort();
        merged_caps.dedup();

        let mut cli = Self {
            sock_path,
            ident: ident.to_string(),
            secret: secret.to_string(),
            use_compact,
            client_version: version,
            daemon_version: None,
            caps: merged_caps,
        };

        cli.register().await?;
        Ok(cli)
    }

    /// Get client identifier
    pub fn ident(&self) -> &str {
        &self.ident
    }

    /// Send a message and optionally wait for response
    async fn send_message(&self, msg: Msg) -> Result<Option<Msg>> {
        #[cfg(unix)]
        {
            let stream = UnixStream::connect(&self.sock_path).await?;
            let (reader, mut writer) = stream.into_split();
            let mut buf_reader = BufReader::new(reader);

            // Send message
            let mut signed_msg = msg;
            sign_msg(&mut signed_msg, &self.secret);

            let line = if self.use_compact {
                let compact = compact::to_compact(&signed_msg);
                serde_json::to_string(&compact)? + "\n"
            } else {
                serde_json::to_string(&signed_msg)? + "\n"
            };

            writer.write_all(line.as_bytes()).await?;

            // Read response (if any)
            let mut response_line = String::new();
            match timeout(
                Duration::from_secs(1),
                buf_reader.read_line(&mut response_line),
            )
            .await
            {
                Err(_) => Ok(None),    // No response within timeout
                Ok(Ok(0)) => Ok(None), // Connection closed without response
                Ok(Ok(_)) => {
                    let response_str = response_line.trim();
                    if response_str.is_empty() {
                        return Ok(None);
                    }

                    // Try compact format first, then standard
                    match serde_json::from_str::<compact::MsgC>(response_str) {
                        Ok(compact_msg) => {
                            let response_msg = compact::from_compact(&compact_msg);
                            if verify_hmac(&response_msg, &self.secret).unwrap_or(false) {
                                Ok(Some(response_msg))
                            } else {
                                warn!("Invalid HMAC in compact response");
                                Ok(None)
                            }
                        }
                        Err(_) => {
                            // Try standard format
                            match serde_json::from_str::<Msg>(response_str) {
                                Ok(response_msg) => {
                                    if verify_hmac(&response_msg, &self.secret).unwrap_or(false) {
                                        Ok(Some(response_msg))
                                    } else {
                                        warn!("Invalid HMAC in standard response");
                                        Ok(None)
                                    }
                                }
                                Err(e) => {
                                    error!("Failed to parse response: {}", e);
                                    Ok(None)
                                }
                            }
                        }
                    }
                }
                Ok(Err(err)) => Err(err.into()),
            }
        }
        #[cfg(not(unix))]
        {
            use tokio::io::split;
            use tokio::net::windows::named_pipe::ClientOptions;

            // Decide transport: Named Pipe when the address looks like a pipe, otherwise TCP
            let is_pipe = is_pipe_addr(&self.sock_path);

            if is_pipe {
                let name = normalize_pipe_name(&self.sock_path);
                let pipe = ClientOptions::new().open(&name)?;
                let (reader, mut writer) = split(pipe);
                let mut buf_reader = BufReader::new(reader);

                // Send message
                let mut signed_msg = msg;
                sign_msg(&mut signed_msg, &self.secret);

                let line = if self.use_compact {
                    let compact = compact::to_compact(&signed_msg);
                    serde_json::to_string(&compact)? + "\n"
                } else {
                    serde_json::to_string(&signed_msg)? + "\n"
                };

                writer.write_all(line.as_bytes()).await?;

                // Read response (if any)
                let mut response_line = String::new();
                match timeout(
                    Duration::from_secs(1),
                    buf_reader.read_line(&mut response_line),
                )
                .await
                {
                    Err(_) => Ok(None),    // No response within timeout
                    Ok(Ok(0)) => Ok(None), // Connection closed without response
                    Ok(Ok(_)) => {
                        let response_str = response_line.trim();
                        if response_str.is_empty() {
                            return Ok(None);
                        }
                        // Parse standard then compact for symmetry
                        match serde_json::from_str::<Msg>(response_str) {
                            Ok(response_msg) => {
                                if verify_hmac(&response_msg, &self.secret).unwrap_or(false) {
                                    Ok(Some(response_msg))
                                } else {
                                    warn!("Invalid HMAC in standard response");
                                    Ok(None)
                                }
                            }
                            Err(e) => match serde_json::from_str::<compact::MsgC>(response_str) {
                                Ok(compact_msg) => {
                                    let response_msg = compact::from_compact(&compact_msg);
                                    if verify_hmac(&response_msg, &self.secret).unwrap_or(false) {
                                        Ok(Some(response_msg))
                                    } else {
                                        warn!("Invalid HMAC in compact response");
                                        Ok(None)
                                    }
                                }
                                Err(_) => {
                                    error!("Failed to parse response: {}", e);
                                    Ok(None)
                                }
                            },
                        }
                    }
                    Ok(Err(err)) => Err(err.into()),
                }
            } else {
                let addr = if self.sock_path.contains(':') {
                    self.sock_path.clone()
                } else {
                    DEFAULT_SOCK.to_string()
                };
                let stream = TcpStream::connect(&addr).await?;
                let (reader, mut writer) = stream.into_split();
                let mut buf_reader = BufReader::new(reader);

                // Send message
                let mut signed_msg = msg;
                sign_msg(&mut signed_msg, &self.secret);

                let line = if self.use_compact {
                    let compact = compact::to_compact(&signed_msg);
                    serde_json::to_string(&compact)? + "\n"
                } else {
                    serde_json::to_string(&signed_msg)? + "\n"
                };

                writer.write_all(line.as_bytes()).await?;

                // Read response (if any)
                let mut response_line = String::new();
                match timeout(
                    Duration::from_secs(1),
                    buf_reader.read_line(&mut response_line),
                )
                .await
                {
                    Err(_) => Ok(None),    // No response within timeout
                    Ok(Ok(0)) => Ok(None), // Connection closed without response
                    Ok(Ok(_)) => {
                        let response_str = response_line.trim();
                        if response_str.is_empty() {
                            return Ok(None);
                        }
                        match serde_json::from_str::<Msg>(response_str) {
                            Ok(response_msg) => {
                                if verify_hmac(&response_msg, &self.secret).unwrap_or(false) {
                                    Ok(Some(response_msg))
                                } else {
                                    warn!("Invalid HMAC in standard response");
                                    Ok(None)
                                }
                            }
                            Err(e) => {
                                // Try compact on Windows too for symmetry
                                match serde_json::from_str::<compact::MsgC>(response_str) {
                                    Ok(compact_msg) => {
                                        let response_msg = compact::from_compact(&compact_msg);
                                        if verify_hmac(&response_msg, &self.secret).unwrap_or(false)
                                        {
                                            Ok(Some(response_msg))
                                        } else {
                                            warn!("Invalid HMAC in compact response");
                                            Ok(None)
                                        }
                                    }
                                    Err(_) => {
                                        error!("Failed to parse response: {}", e);
                                        Ok(None)
                                    }
                                }
                            }
                        }
                    }
                    Ok(Err(err)) => Err(err.into()),
                }
            }
        }
    }

    /// Register with the daemon
    pub async fn register(&mut self) -> Result<()> {
        let mut payload = serde_json::Map::new();
        payload.insert("caps".into(), serde_json::json!(self.caps));
        payload.insert("pid".into(), serde_json::json!(std::process::id()));
        if let Some(version) = &self.client_version {
            payload.insert("version".into(), serde_json::Value::String(version.clone()));
        }

        let msg = new_msg(
            "REGISTER",
            &self.ident,
            "orchestrator",
            &serde_json::Value::Object(payload),
            &self.secret,
        );
        match self.send_message(msg).await? {
            Some(response) => match response.msg_type.as_str() {
                "ACK" => {
                    if let Some(version) = response
                        .payload
                        .get("daemon_version")
                        .and_then(|v| v.as_str())
                    {
                        self.daemon_version = Some(version.to_string());
                        info!("Registered against devitd {}", version);
                    }
                    debug!("Registered client: {}", self.ident);
                    Ok(())
                }
                "ERR" => {
                    let message = response
                        .payload
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("daemon rejected registration");
                    Err(anyhow!(message.to_string()))
                }
                other => {
                    warn!("Unexpected response to REGISTER: {}", other);
                    Ok(())
                }
            },
            None => {
                debug!("Registered client: {}", self.ident);
                Ok(())
            }
        }
    }

    /// Send heartbeat and get any pending notifications
    pub async fn heartbeat(&self) -> Result<Option<Msg>> {
        let msg = new_msg(
            "HEARTBEAT",
            &self.ident,
            "orchestrator",
            &serde_json::json!({}),
            &self.secret,
        );
        self.send_message(msg).await
    }

    /// Version of the daemon reported during registration, when available.
    pub fn daemon_version(&self) -> Option<&str> {
        self.daemon_version.as_deref()
    }

    /// Delegate a task to a worker
    pub async fn delegate(
        &self,
        to_worker: &str,
        task: serde_json::Value,
        return_to: &str,
    ) -> Result<String> {
        let mut payload = serde_json::Map::new();
        payload.insert("task".into(), task);
        payload.insert(
            "return_to".into(),
            serde_json::Value::String(return_to.to_string()),
        );

        let msg = new_msg(
            "DELEGATE",
            &self.ident,
            to_worker,
            &serde_json::Value::Object(payload),
            &self.secret,
        );
        let id = msg.msg_id.clone();
        self.send_message(msg).await?;
        debug!("Delegated task {} to {}", id, to_worker);
        Ok(id)
    }

    /// Send notification about task completion
    pub async fn notify(
        &self,
        to: &str,
        task_id: &str,
        status: &str,
        artifacts: serde_json::Value,
        return_to: Option<&str>,
    ) -> Result<()> {
        let mut payload = serde_json::Map::new();
        payload.insert("task_id".into(), serde_json::json!(task_id));
        payload.insert("status".into(), serde_json::json!(status));
        payload.insert("artifacts".into(), artifacts);

        if let Some(rt) = return_to {
            payload.insert("return_to".into(), serde_json::json!(rt));
        }

        let msg = new_msg(
            "NOTIFY",
            &self.ident,
            to,
            &serde_json::Value::Object(payload),
            &self.secret,
        );
        self.send_message(msg).await?;
        debug!("Sent notification for task {} to {}", task_id, to);
        Ok(())
    }

    /// Request a full status snapshot from the daemon
    pub async fn status_snapshot(&self) -> Result<Option<Msg>> {
        let msg = new_msg(
            "STATUS_REQUEST",
            &self.ident,
            "orchestrator",
            &serde_json::json!({}),
            &self.secret,
        );
        self.send_message(msg).await
    }

    /// Poll for pending messages
    pub async fn poll(&self) -> Result<Option<Msg>> {
        let msg = new_msg(
            "POLL",
            &self.ident,
            "orchestrator",
            &serde_json::json!({}),
            &self.secret,
        );
        self.send_message(msg).await
    }

    /// Start heartbeat loop that also checks for notifications
    pub async fn run_heartbeat_loop<F>(&self, mut handler: F) -> Result<()>
    where
        F: FnMut(Msg) -> Result<()>,
    {
        loop {
            if let Some(notification) = self.heartbeat().await? {
                if let Err(e) = handler(notification) {
                    error!("Error handling notification: {}", e);
                }
            }

            sleep(Duration::from_secs(5)).await;
        }
    }

    /// Check if a message is a task delegation
    pub fn is_delegate(msg: &Msg) -> bool {
        msg.msg_type == "DELEGATE"
    }

    /// Check if a message is a notification
    pub fn is_notify(msg: &Msg) -> bool {
        msg.msg_type == "NOTIFY"
    }

    /// Extract task from delegate message
    pub fn extract_task(msg: &Msg) -> Option<&serde_json::Value> {
        msg.payload.get("task")
    }

    /// Extract return_to from message
    pub fn extract_return_to(msg: &Msg) -> Option<String> {
        msg.payload
            .get("return_to")
            .and_then(|v| v.as_str())
            .map(String::from)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenshotResponse {
    pub path: String,
    pub format: String,
    pub bytes: u64,
    pub human_size: Option<String>,
}

impl DevitClient {
    /// Request a daemon-side screenshot capture.
    pub async fn capture_screenshot(&self) -> Result<ScreenshotResponse> {
        let payload = serde_json::json!({});
        let msg = new_msg(
            "SCREENSHOT",
            &self.ident,
            "orchestrator",
            &payload,
            &self.secret,
        );
        match self.send_message(msg).await? {
            Some(response) => match response.msg_type.as_str() {
                "ACK" => {
                    let path = response
                        .payload
                        .get("path")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| anyhow!("daemon response missing screenshot path"))?;
                    let format = response
                        .payload
                        .get("format")
                        .and_then(|v| v.as_str())
                        .unwrap_or("png");
                    let size_obj = response.payload.get("size");
                    let bytes = size_obj
                        .and_then(|value| value.get("bytes"))
                        .and_then(|v| v.as_u64())
                        .ok_or_else(|| anyhow!("daemon response missing size.bytes"))?;
                    let human = size_obj
                        .and_then(|value| value.get("human"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());

                    Ok(ScreenshotResponse {
                        path: path.to_string(),
                        format: format.to_string(),
                        bytes,
                        human_size: human,
                    })
                }
                "ERR" => {
                    let message = response
                        .payload
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("daemon rejected screenshot request");
                    Err(anyhow!(message.to_string()))
                }
                other => Err(anyhow!(format!("unexpected daemon response: {other}"))),
            },
            None => Err(anyhow!("daemon did not return a response")),
        }
    }
}

// HMAC utilities
fn now_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// Windows helpers for pipe address parsing
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

fn new_msg(typ: &str, from: &str, to: &str, payload: &serde_json::Value, secret: &str) -> Msg {
    let mut msg = Msg {
        msg_type: typ.to_string(),
        msg_id: Uuid::new_v4().to_string(),
        from: from.to_string(),
        to: to.to_string(),
        ts: now_ts(),
        nonce: Uuid::new_v4().to_string(),
        hmac: String::new(),
        payload: payload.clone(),
    };
    sign_msg(&mut msg, secret);
    msg
}

fn sign_msg(msg: &mut Msg, secret: &str) {
    let body = canonical_body(msg);
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC key error");
    mac.update(body.as_bytes());
    let sig = mac.finalize().into_bytes();
    msg.hmac = general_purpose::STANDARD.encode(sig);
}

fn verify_hmac(msg: &Msg, secret: &str) -> Result<bool> {
    let body = canonical_body(msg);
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())?;
    mac.update(body.as_bytes());
    let bytes = general_purpose::STANDARD
        .decode(msg.hmac.as_bytes())
        .unwrap_or_default();
    Ok(mac.verify_slice(&bytes).is_ok())
}

fn canonical_body(msg: &Msg) -> String {
    let payload = serde_json::to_string(&msg.payload).unwrap_or("{}".to_string());
    format!(
        "{}|{}|{}|{}|{}|{}|{}",
        msg.msg_type, msg.msg_id, msg.from, msg.to, msg.ts, msg.nonce, payload
    )
}
