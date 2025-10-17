// devit_exec - Background/foreground process execution (MVP v3.1)
//
// Security:
// - Binary allowlist validation
// - Path traversal protection (canonicalize_with_nofollow)
// - RLIMITs (NOFILE, NPROC, AS, CPU)
// - PR_SET_NO_NEW_PRIVS
// - PR_SET_PDEATHSIG(SIGKILL)
// - Process groups (setpgid)
// - env_clear + safe baseline
//
// Registry:
// - Persistent ~/.devit/process_registry.json
// - PID + start_ticks validation
// - Atomic writes with fsync
//
// Ref: PROJECT_TRACKING/WORK_IN_PROGRESS/devit_exec_tool.md v3.1

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use devit_cli::core::config::ExecToolConfig;
use devit_common::process_utils::read_proc_stat;
#[cfg(windows)]
use devit_sandbox::{backend::windows::WindowsSandbox, ProcessHandle, SandboxBackend};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io;
#[cfg(target_family = "unix")]
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
#[cfg(windows)]
use std::thread;
use tokio::sync::Mutex;
#[cfg(windows)]
use tokio::time::sleep;
use tracing::info;

use mcp_core::{McpError, McpResult, McpTool};

/// Stdin mode for devit_exec
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum StdinMode {
    #[default]
    Null,
    Pipe,
}

/// Execution mode
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum ExecutionMode {
    Background,
    #[default]
    Foreground,
}

/// Execution configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecConfig {
    pub binary: String,
    pub args: Vec<String>,
    pub working_dir: Option<String>,
    pub env: Option<HashMap<String, String>>,
    #[serde(default)]
    pub mode: ExecutionMode,
    #[serde(default)]
    pub stdin: StdinMode,
    pub foreground_timeout_secs: Option<u64>,
}

/// Execution result (foreground)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecResult {
    pub exit_code: Option<i32>,
    pub terminated_by_signal: Option<i32>,
    pub duration_ms: u64,
    pub stdout_tail: String,
    pub stderr_tail: String,
}

/// Background process metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackgroundProcess {
    pub pid: u32,
    pub pgid: u32,
    pub start_ticks: u64,
    pub started_at: DateTime<Utc>,
}

/// DevIt Exec tool
pub struct DevitExec {
    registry: Arc<Mutex<devit_common::process_registry::Registry>>,
    config: ExecToolConfig,
    sandbox_root: PathBuf,
}

impl DevitExec {
    pub fn with_config(config: ExecToolConfig, sandbox_root: PathBuf) -> McpResult<Self> {
        let canonical_root = sandbox_root
            .canonicalize()
            .map_err(|err| McpError::ExecutionFailed(format!("Invalid sandbox root: {}", err)))?;

        let registry = devit_common::process_registry::load_registry()
            .unwrap_or_else(|_| devit_common::process_registry::Registry::new());

        Ok(Self {
            registry: Arc::new(Mutex::new(registry)),
            config,
            sandbox_root: canonical_root,
        })
    }

    #[cfg(windows)]
    async fn execute_foreground_windows(&self, config: &ExecConfig) -> McpResult<Value> {
        use std::io::Read;

        let timeout_secs = config
            .foreground_timeout_secs
            .unwrap_or(self.config.rlimit_cpu_secs);
        let timeout = std::time::Duration::from_secs(timeout_secs);

        let working_dir = if let Some(ref wd) = config.working_dir {
            self.resolve_in_sandbox(Path::new(wd))
                .map_err(|e| McpError::ExecutionFailed(e.to_string()))?
        } else {
            self.sandbox_root.clone()
        };

        if !working_dir.is_dir() {
            return Err(McpError::ExecutionFailed(format!(
                "Working directory not found: {}",
                working_dir.display()
            )));
        }

        let binary_path = if Path::new(&config.binary).is_absolute() {
            devit_common::process_utils::canonicalize_with_nofollow(Path::new(&config.binary))
        } else {
            self.resolve_in_sandbox(Path::new(&config.binary))
        }
        .map_err(|e| McpError::ExecutionFailed(e.to_string()))?;

        self.validate_binary(&binary_path)
            .map_err(|e| McpError::ExecutionFailed(e.to_string()))?;

        let safe_env = self
            .build_safe_env(config.env.clone())
            .map_err(|e| McpError::ExecutionFailed(e.to_string()))?;

        fn join_output(
            handle: Option<thread::JoinHandle<io::Result<Vec<u8>>>>,
            stream: &str,
        ) -> McpResult<Vec<u8>> {
            if let Some(h) = handle {
                match h.join() {
                    Ok(Ok(buf)) => Ok(buf),
                    Ok(Err(err)) => Err(McpError::ExecutionFailed(format!(
                        "Failed to read {stream}: {err}"
                    ))),
                    Err(_) => Err(McpError::ExecutionFailed(format!(
                        "{stream} reader thread panicked"
                    ))),
                }
            } else {
                Ok(Vec::new())
            }
        }

        let mut cmd = Command::new(&binary_path);
        cmd.args(&config.args)
            .current_dir(&working_dir)
            .env_clear()
            .envs(&safe_env)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(match config.stdin {
                StdinMode::Null => Stdio::null(),
                StdinMode::Pipe => Stdio::piped(),
            });

        let mut sandbox = WindowsSandbox::new()
            .map_err(|e| McpError::ExecutionFailed(format!("Failed to create sandbox: {}", e)))?;

        if self.config.rlimit_as_gb > 0 {
            let bytes = self.config.rlimit_as_gb.saturating_mul(1024 * 1024 * 1024);
            if bytes > 0 {
                sandbox.set_memory_limit(bytes).map_err(|e| {
                    McpError::ExecutionFailed(format!("Failed to set memory limit: {}", e))
                })?;
            }
        }

        let mut child = sandbox
            .spawn(cmd)
            .map_err(|e| McpError::ExecutionFailed(format!("Failed to spawn process: {}", e)))?;

        fn spawn_reader(
            stream: Option<std::process::ChildStdout>,
        ) -> Option<thread::JoinHandle<io::Result<Vec<u8>>>> {
            stream.map(|mut handle| {
                thread::spawn(move || {
                    let mut buf = Vec::new();
                    handle.read_to_end(&mut buf)?;
                    Ok(buf)
                })
            })
        }

        fn spawn_err_reader(
            stream: Option<std::process::ChildStderr>,
        ) -> Option<thread::JoinHandle<io::Result<Vec<u8>>>> {
            stream.map(|mut handle| {
                thread::spawn(move || {
                    let mut buf = Vec::new();
                    handle.read_to_end(&mut buf)?;
                    Ok(buf)
                })
            })
        }

        let mut stdout_handle = spawn_reader(child.take_stdout());
        let mut stderr_handle = spawn_err_reader(child.take_stderr());

        let start = std::time::Instant::now();

        let exit_status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    if start.elapsed() > timeout {
                        let _ = child.kill();
                        let _ = join_output(stdout_handle.take(), "stdout").ok();
                        let _ = join_output(stderr_handle.take(), "stderr").ok();
                        return Err(McpError::ExecutionFailed(
                            "Foreground execution timeout".into(),
                        ));
                    }
                    sleep(std::time::Duration::from_millis(50)).await;
                }
                Err(e) => {
                    let _ = child.kill();
                    let _ = join_output(stdout_handle.take(), "stdout").ok();
                    let _ = join_output(stderr_handle.take(), "stderr").ok();
                    return Err(McpError::ExecutionFailed(format!(
                        "Process wait failed: {}",
                        e
                    )));
                }
            }
        };

        let stdout_bytes = join_output(stdout_handle.take(), "stdout")?;
        let stderr_bytes = join_output(stderr_handle.take(), "stderr")?;

        let stdout_tail = String::from_utf8_lossy(&stdout_bytes).into_owned();
        let stderr_tail = String::from_utf8_lossy(&stderr_bytes).into_owned();
        let duration_ms = start.elapsed().as_millis() as u64;
        let exit_code = exit_status.code();
        let terminated_by_signal: Option<i32> = None;

        let exit_summary = match exit_code {
            Some(code) => format!("exit {}", code),
            None => "completed".to_string(),
        };

        let summary = format!(
            "✅ `{}` terminé — {} ({} ms)",
            config.binary, exit_summary, duration_ms
        );

        info!(
            target: "devit_mcp_tools",
            "tool devit_exec foreground completed | binary={} exit_code={:?} duration_ms={} stdout_len={} stderr_len={}",
            config.binary,
            exit_code,
            duration_ms,
            stdout_tail.len(),
            stderr_tail.len()
        );

        let mut content = vec![json!({
            "type": "text",
            "text": summary
        })];

        if !stdout_tail.trim().is_empty() {
            content.push(json!({
                "type": "text",
                "text": format!("stdout (tail):\n{}", stdout_tail)
            }));
        }
        if !stderr_tail.trim().is_empty() {
            content.push(json!({
                "type": "text",
                "text": format!("stderr (tail):\n{}", stderr_tail)
            }));
        }

        let stdin_mode = match config.stdin {
            StdinMode::Null => "null",
            StdinMode::Pipe => "pipe",
        };

        let structured = json!({
            "exec": {
                "mode": "foreground",
                "binary": config.binary.clone(),
                "args": config.args.clone(),
                "working_dir": config.working_dir.clone(),
                "stdin": stdin_mode,
                "foreground_timeout_secs": config.foreground_timeout_secs,
                "duration_ms": duration_ms,
                "exit_code": exit_code,
                "terminated_by_signal": terminated_by_signal,
                "stdout_tail": stdout_tail,
                "stderr_tail": stderr_tail
            }
        });

        Ok(json!({
            "content": content,
            "structuredContent": structured
        }))
    }

    #[cfg(windows)]
    async fn execute_background_windows(&self, config: &ExecConfig) -> McpResult<Value> {
        let working_dir = if let Some(ref wd) = config.working_dir {
            self.resolve_in_sandbox(Path::new(wd))
                .map_err(|e| McpError::ExecutionFailed(e.to_string()))?
        } else {
            self.sandbox_root.clone()
        };

        if !working_dir.is_dir() {
            return Err(McpError::ExecutionFailed(format!(
                "Working directory not found: {}",
                working_dir.display()
            )));
        }

        let binary_path = if Path::new(&config.binary).is_absolute() {
            devit_common::process_utils::canonicalize_with_nofollow(Path::new(&config.binary))
        } else {
            self.resolve_in_sandbox(Path::new(&config.binary))
        }
        .map_err(|e| McpError::ExecutionFailed(e.to_string()))?;

        self.validate_binary(&binary_path)
            .map_err(|e| McpError::ExecutionFailed(e.to_string()))?;

        let safe_env = self
            .build_safe_env(config.env.clone())
            .map_err(|e| McpError::ExecutionFailed(e.to_string()))?;

        let mut cmd = Command::new(&binary_path);
        cmd.args(&config.args)
            .current_dir(&working_dir)
            .env_clear()
            .envs(&safe_env)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(match config.stdin {
                StdinMode::Null => Stdio::null(),
                StdinMode::Pipe => Stdio::piped(),
            });

        let mut sandbox = WindowsSandbox::new()
            .map_err(|e| McpError::ExecutionFailed(format!("Failed to create sandbox: {}", e)))?;

        if self.config.rlimit_as_gb > 0 {
            let bytes = self.config.rlimit_as_gb.saturating_mul(1024 * 1024 * 1024);
            if bytes > 0 {
                sandbox.set_memory_limit(bytes).map_err(|e| {
                    McpError::ExecutionFailed(format!("Failed to set memory limit: {}", e))
                })?;
            }
        }

        let mut child = sandbox
            .spawn(cmd)
            .map_err(|e| McpError::ExecutionFailed(format!("Failed to spawn process: {}", e)))?;

        let pid = child.id();
        child.disable_kill_on_drop();
        drop(child);

        let proc_stat = read_proc_stat(pid).map_err(|e| {
            McpError::ExecutionFailed(format!("Failed to query process {}: {}", pid, e))
        })?;

        let pgid = pid;
        let started_at = Utc::now();

        let record = devit_common::process_registry::ProcessRecord {
            pid,
            pgid,
            start_ticks: proc_stat.starttime,
            started_at,
            command: config.binary.clone(),
            args: config.args.clone(),
            status: devit_common::process_registry::ProcessStatus::Running,
            exit_code: None,
            terminated_by_signal: None,
            auto_kill_at: Some(
                started_at + chrono::Duration::seconds(self.config.max_lifetime_secs as i64),
            ),
        };

        let mut registry = self.registry.lock().await;
        registry.insert(pid, record.clone());
        devit_common::process_registry::save_registry(&*registry)
            .map_err(|e| McpError::ExecutionFailed(format!("Failed to save registry: {}", e)))?;
        drop(registry);

        let bg_proc = BackgroundProcess {
            pid,
            pgid,
            start_ticks: proc_stat.starttime,
            started_at,
        };

        let record_json = serde_json::to_value(&record).unwrap();
        let auto_kill_at_value = record.auto_kill_at;
        let auto_kill_at_text = auto_kill_at_value.as_ref().map(|ts| ts.to_rfc3339());
        let process_json = serde_json::to_value(&bg_proc).unwrap();

        let summary = if let Some(ref auto) = auto_kill_at_text {
            format!(
                "🚀 `{}` lancé en background — pid {} (pgid {}), auto-kill {}",
                config.binary, pid, pgid, auto
            )
        } else {
            format!(
                "🚀 `{}` lancé en background — pid {} (pgid {})",
                config.binary, pid, pgid
            )
        };

        info!(
            target: "devit_mcp_tools",
            "tool devit_exec background started | binary={} pid={} pgid={} auto_kill_at={:?}",
            config.binary,
            pid,
            pgid,
            auto_kill_at_value
        );

        let content = vec![json!({
            "type": "text",
            "text": summary
        })];

        let structured = json!({
            "exec": {
                "mode": "background",
                "binary": config.binary.clone(),
                "args": config.args.clone(),
                "working_dir": config.working_dir.clone(),
                "pid": pid,
                "pgid": pgid,
                "start_ticks": proc_stat.starttime,
                "started_at": started_at,
                "auto_kill_at": auto_kill_at_value,
                "record": record_json,
                "process": process_json
            }
        });

        Ok(json!({
            "content": content,
            "structuredContent": structured
        }))
    }

    #[cfg(target_family = "unix")]
    async fn execute_background_unix(&self, config: &ExecConfig) -> McpResult<Value> {
        let child = self
            .spawn_process(config)
            .await
            .map_err(|e| McpError::ExecutionFailed(e.to_string()))?;

        let pid = child.id();
        drop(child);

        let proc_stat = read_proc_stat(pid).map_err(|e| {
            McpError::ExecutionFailed(format!("Failed to read /proc/{}/stat: {}", pid, e))
        })?;

        let pgid = pid;
        let started_at = Utc::now();

        let record = devit_common::process_registry::ProcessRecord {
            pid,
            pgid,
            start_ticks: proc_stat.starttime,
            started_at,
            command: config.binary.clone(),
            args: config.args.clone(),
            status: devit_common::process_registry::ProcessStatus::Running,
            exit_code: None,
            terminated_by_signal: None,
            auto_kill_at: Some(
                started_at + chrono::Duration::seconds(self.config.max_lifetime_secs as i64),
            ),
        };

        let mut registry = self.registry.lock().await;
        registry.insert(pid, record.clone());
        devit_common::process_registry::save_registry(&*registry)
            .map_err(|e| McpError::ExecutionFailed(format!("Failed to save registry: {}", e)))?;
        drop(registry);

        let bg_proc = BackgroundProcess {
            pid,
            pgid,
            start_ticks: proc_stat.starttime,
            started_at,
        };

        let record_json = serde_json::to_value(&record).unwrap();
        let auto_kill_at_value = record.auto_kill_at;
        let auto_kill_at_text = auto_kill_at_value.as_ref().map(|ts| ts.to_rfc3339());
        let process_json = serde_json::to_value(&bg_proc).unwrap();

        let summary = if let Some(ref auto) = auto_kill_at_text {
            format!(
                "🚀 `{}` lancé en background — pid {} (pgid {}), auto-kill {}",
                config.binary, pid, pgid, auto
            )
        } else {
            format!(
                "🚀 `{}` lancé en background — pid {} (pgid {})",
                config.binary, pid, pgid
            )
        };

        info!(
            target: "devit_mcp_tools",
            "tool devit_exec background started | binary={} pid={} pgid={} auto_kill_at={:?}",
            config.binary,
            pid,
            pgid,
            auto_kill_at_value
        );

        let content = vec![json!({
            "type": "text",
            "text": summary
        })];

        let structured = json!({
            "exec": {
                "mode": "background",
                "binary": config.binary.clone(),
                "args": config.args.clone(),
                "working_dir": config.working_dir.clone(),
                "pid": pid,
                "pgid": pgid,
                "start_ticks": proc_stat.starttime,
                "started_at": started_at,
                "auto_kill_at": auto_kill_at_value,
                "record": record_json,
                "process": process_json
            }
        });

        Ok(json!({
            "content": content,
            "structuredContent": structured
        }))
    }

    /// Validate binary against allowlist
    fn validate_binary(&self, binary_path: &Path) -> io::Result<()> {
        let binary_str = binary_path.to_string_lossy();

        for pattern in &self.config.binary_allowlist {
            // Simple glob matching (* wildcard)
            if pattern.contains('*') {
                let prefix = pattern.trim_end_matches('*');
                if binary_str.starts_with(prefix) {
                    return Ok(());
                }
            } else if binary_str == pattern.as_str() {
                return Ok(());
            }
        }

        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("Binary not in allowlist: {}", binary_str),
        ))
    }

    /// Build safe environment baseline
    fn build_safe_env(
        &self,
        user_env: Option<HashMap<String, String>>,
    ) -> io::Result<HashMap<String, String>> {
        let mut env = HashMap::new();

        let mut safe_vars = vec![
            "LANG",
            "LC_ALL",
            "LC_CTYPE",
            "TZ",
            "TERM",
            "COLORTERM",
            "RUST_LOG",
            "RUST_BACKTRACE",
        ];

        if cfg!(target_os = "windows") {
            safe_vars.extend([
                "SystemRoot",
                "SYSTEMROOT",
                "ComSpec",
                "COMSPEC",
                "PATHEXT",
                "TEMP",
                "TMP",
            ]);
        }

        for var in &safe_vars {
            if let Ok(val) = std::env::var(var) {
                env.insert(var.to_string(), val);
            }
        }

        let default_path = if cfg!(target_os = "windows") {
            std::env::var("PATH").unwrap_or_else(|_| r"C:\Windows\System32;C:\Windows".to_string())
        } else {
            "/usr/bin:/bin".into()
        };
        env.insert("PATH".into(), default_path);

        // Sandboxed HOME
        let home = self.sandbox_root.join(".home");
        #[cfg(target_family = "unix")]
        {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new()
                .mode(0o700)
                .recursive(true)
                .create(&home)?;
        }
        #[cfg(not(target_family = "unix"))]
        {
            std::fs::create_dir_all(&home)?;
        }

        env.insert("HOME".into(), home.to_string_lossy().into());

        // Merge user env (limited)
        if let Some(user) = user_env {
            for (k, v) in user {
                if safe_vars.contains(&k.as_str()) || k.starts_with("DEVIT_") {
                    env.insert(k, v);
                }
            }
        }

        Ok(env)
    }

    fn resolve_in_sandbox(&self, path: &Path) -> io::Result<PathBuf> {
        devit_common::process_utils::canonicalize_within_root(&self.sandbox_root, path)
    }

    /// Spawn process with security hardening
    #[cfg(target_family = "unix")]
    async fn spawn_process(&self, config: &ExecConfig) -> io::Result<std::process::Child> {
        let working_dir = if let Some(ref wd) = config.working_dir {
            self.resolve_in_sandbox(Path::new(wd))?
        } else {
            self.sandbox_root.clone()
        };

        if !working_dir.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("Working directory not found: {}", working_dir.display()),
            ));
        }

        let binary_path = if Path::new(&config.binary).is_absolute() {
            devit_common::process_utils::canonicalize_with_nofollow(Path::new(&config.binary))?
        } else {
            self.resolve_in_sandbox(Path::new(&config.binary))?
        };
        self.validate_binary(&binary_path)?;

        let safe_env = self.build_safe_env(config.env.clone())?;

        let limits = self.config.clone();

        let child = unsafe {
            Command::new(&binary_path)
                .args(&config.args)
                .current_dir(&working_dir)
                .env_clear()
                .envs(&safe_env)
                .stdin(match config.stdin {
                    StdinMode::Null => Stdio::null(),
                    StdinMode::Pipe => Stdio::piped(),
                })
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .pre_exec(move || {
                    if libc::setpgid(0, 0) != 0 {
                        return Err(io::Error::last_os_error());
                    }

                    Self::apply_resource_limits(&limits)?;
                    Ok(())
                })
                .spawn()?
        };

        Ok(child)
    }

    /// Apply resource limits (called in before_exec)
    #[cfg(target_family = "unix")]
    fn apply_resource_limits(limits: &ExecToolConfig) -> io::Result<()> {
        use libc::{rlim_t, rlimit, setrlimit};
        use libc::{RLIMIT_AS, RLIMIT_CPU, RLIMIT_NOFILE, RLIMIT_NPROC};

        // NOFILE
        let nofile = rlimit {
            rlim_cur: limits.rlimit_nofile as rlim_t,
            rlim_max: limits.rlimit_nofile as rlim_t,
        };
        let ret = unsafe { setrlimit(RLIMIT_NOFILE, &nofile) };
        if ret != 0 {
            return Err(io::Error::last_os_error());
        }

        // NPROC
        let nproc = rlimit {
            rlim_cur: limits.rlimit_nproc as rlim_t,
            rlim_max: limits.rlimit_nproc as rlim_t,
        };
        let ret = unsafe { setrlimit(RLIMIT_NPROC, &nproc) };
        if ret != 0 {
            return Err(io::Error::last_os_error());
        }

        // AS (address space)
        let as_bytes = limits.rlimit_as_gb * 1024 * 1024 * 1024;
        let as_limit = rlimit {
            rlim_cur: as_bytes as rlim_t,
            rlim_max: as_bytes as rlim_t,
        };
        let ret = unsafe { setrlimit(RLIMIT_AS, &as_limit) };
        if ret != 0 {
            return Err(io::Error::last_os_error());
        }

        // CPU
        let cpu = rlimit {
            rlim_cur: limits.rlimit_cpu_secs as rlim_t,
            rlim_max: limits.rlimit_cpu_secs as rlim_t,
        };
        let ret = unsafe { setrlimit(RLIMIT_CPU, &cpu) };
        if ret != 0 {
            return Err(io::Error::last_os_error());
        }

        // PR_SET_NO_NEW_PRIVS
        let ret = unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
        if ret != 0 {
            return Err(io::Error::last_os_error());
        }

        // PR_SET_PDEATHSIG
        let ret = unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) };
        if ret != 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(())
    }

    /// Execute foreground (with timeout)
    async fn execute_foreground(&self, config: &ExecConfig) -> McpResult<Value> {
        #[cfg(target_family = "unix")]
        {
            return self.execute_foreground_unix(config).await;
        }
        #[cfg(windows)]
        {
            return self.execute_foreground_windows(config).await;
        }
        #[allow(unreachable_code)]
        Err(McpError::ExecutionFailed(
            "devit_exec not supported on this platform".into(),
        ))
    }

    #[cfg(target_family = "unix")]
    async fn execute_foreground_unix(&self, config: &ExecConfig) -> McpResult<Value> {
        let timeout_secs = config
            .foreground_timeout_secs
            .unwrap_or(self.config.rlimit_cpu_secs);
        let timeout = std::time::Duration::from_secs(timeout_secs);

        let start = std::time::Instant::now();
        let child = self
            .spawn_process(config)
            .await
            .map_err(|e| McpError::ExecutionFailed(e.to_string()))?;

        let pid = child.id();

        let output_result = tokio::time::timeout(
            timeout,
            tokio::task::spawn_blocking(move || child.wait_with_output()),
        )
        .await;

        let duration_ms = start.elapsed().as_millis() as u64;

        match output_result {
            Ok(Ok(Ok(output))) => {
                let stdout_tail = String::from_utf8_lossy(&output.stdout).into_owned();
                let stderr_tail = String::from_utf8_lossy(&output.stderr).into_owned();

                #[cfg(target_family = "unix")]
                let terminated_by_signal = output.status.signal();
                #[cfg(not(target_family = "unix"))]
                let terminated_by_signal = None;

                let result = ExecResult {
                    exit_code: output.status.code(),
                    terminated_by_signal,
                    duration_ms,
                    stdout_tail,
                    stderr_tail,
                };

                let ExecResult {
                    exit_code,
                    terminated_by_signal,
                    duration_ms,
                    stdout_tail,
                    stderr_tail,
                } = result;

                let exit_summary = match (exit_code, terminated_by_signal) {
                    (Some(code), None) => format!("exit {}", code),
                    (None, Some(sig)) => format!("signal {}", sig),
                    (Some(code), Some(sig)) => format!("exit {} (signal {})", code, sig),
                    (None, None) => "completed".to_string(),
                };

                let summary = format!(
                    "✅ `{}` terminé — {} ({} ms)",
                    config.binary, exit_summary, duration_ms
                );

                info!(
                    target: "devit_mcp_tools",
                    "tool devit_exec foreground completed | binary={} exit_code={:?} signal={:?} duration_ms={} stdout_len={} stderr_len={}",
                    config.binary,
                    exit_code,
                    terminated_by_signal,
                    duration_ms,
                    stdout_tail.len(),
                    stderr_tail.len()
                );

                let mut content = vec![json!({
                    "type": "text",
                    "text": summary
                })];

                if !stdout_tail.trim().is_empty() {
                    content.push(json!({
                        "type": "text",
                        "text": format!("stdout (tail):\n{}", stdout_tail)
                    }));
                }
                if !stderr_tail.trim().is_empty() {
                    content.push(json!({
                        "type": "text",
                        "text": format!("stderr (tail):\n{}", stderr_tail)
                    }));
                }

                let stdin_mode = match config.stdin {
                    StdinMode::Null => "null",
                    StdinMode::Pipe => "pipe",
                };

                let structured = json!({
                    "exec": {
                        "mode": "foreground",
                        "binary": config.binary.clone(),
                        "args": config.args.clone(),
                        "working_dir": config.working_dir.clone(),
                        "stdin": stdin_mode,
                        "foreground_timeout_secs": config.foreground_timeout_secs,
                        "duration_ms": duration_ms,
                        "exit_code": exit_code,
                        "terminated_by_signal": terminated_by_signal,
                        "stdout_tail": stdout_tail,
                        "stderr_tail": stderr_tail
                    }
                });

                Ok(json!({
                    "content": content,
                    "structuredContent": structured
                }))
            }
            Ok(Ok(Err(e))) => Err(McpError::ExecutionFailed(format!(
                "Process wait failed: {}",
                e
            ))),
            Ok(Err(e)) => Err(McpError::ExecutionFailed(format!(
                "Blocking task failed: {}",
                e
            ))),
            Err(_timeout) => {
                unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
                Err(McpError::ExecutionFailed(
                    "Foreground execution timeout".into(),
                ))
            }
        }
    }

    /// Execute background (register in registry)
    async fn execute_background(&self, config: &ExecConfig) -> McpResult<Value> {
        #[cfg(target_family = "unix")]
        {
            return self.execute_background_unix(config).await;
        }
        #[cfg(windows)]
        {
            return self.execute_background_windows(config).await;
        }
        #[allow(unreachable_code)]
        Err(McpError::ExecutionFailed(
            "devit_exec not supported on this platform".into(),
        ))
    }
}

#[async_trait]
impl McpTool for DevitExec {
    fn name(&self) -> &str {
        "devit_exec"
    }

    fn description(&self) -> &str {
        "Execute a process in background or foreground with security hardening"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "binary": {
                    "type": "string",
                    "description": "Path to binary to execute"
                },
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Command line arguments"
                },
                "working_dir": {
                    "type": "string",
                    "description": "Working directory (optional)"
                },
                "env": {
                    "type": "object",
                    "description": "Environment variables (limited to safe vars)"
                },
                "mode": {
                    "type": "string",
                    "enum": ["background", "foreground"],
                    "description": "Execution mode (default: foreground)"
                },
                "stdin": {
                    "type": "string",
                    "enum": ["null", "pipe"],
                    "description": "Stdin mode (default: null)"
                },
                "foreground_timeout_secs": {
                    "type": "number",
                    "description": "Timeout for foreground execution (default: 300)"
                }
            },
            "required": ["binary", "args"]
        })
    }

    async fn execute(&self, params: Value) -> McpResult<Value> {
        let config: ExecConfig =
            serde_json::from_value(params).map_err(|e| McpError::InvalidRequest(e.to_string()))?;

        info!(
            target: "devit_mcp_tools",
            "tool devit_exec called | mode={:?} binary={} args={:?} cwd={:?}",
            config.mode, config.binary, config.args, config.working_dir
        );

        match config.mode {
            ExecutionMode::Foreground => self.execute_foreground(&config).await,
            ExecutionMode::Background => self.execute_background(&config).await,
        }
    }
}
