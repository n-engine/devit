use anyhow::Result;
use devit_common::orchestration::OrchestrationContext;
use devit_orchestration::backend::OrchestrationBackend;
use devit_orchestration::types::{
    OrchestrationConfig, OrchestrationMode, StatusFilter, TaskStatus,
};
use once_cell::sync::Lazy;
use std::{env, path::PathBuf, process::Command, time::Duration};
use tokio::time::{sleep, timeout};

#[cfg(unix)]
static DEVITD_BINARY: Lazy<PathBuf> = Lazy::new(|| {
    let status = Command::new("cargo")
        .args(["build", "--bin", "devitd"])
        .status()
        .expect("failed to build devitd binary");
    assert!(status.success(), "devitd build failed");

    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let mut path = PathBuf::from(manifest_dir);
    path.pop();
    path.pop();
    path.push("target");
    path.push("debug");
    path.push("devitd");
    path
});

#[cfg(unix)]
async fn kill_daemon_by_socket(socket: &str) {
    let _ = Command::new("pkill")
        .args(["-f", &format!("devitd.*{}", socket)])
        .status();
    sleep(Duration::from_millis(500)).await;
}

#[cfg(unix)]
mod daemon_tests {
    use super::*;
    use devit_orchestration::daemon::DaemonBackend;
    use tempfile::TempDir;

    async fn ensure_binary_env() {
        let path = DEVITD_BINARY.clone();
        env::set_var("DEVITD_BINARY", path);
    }

    #[tokio::test]
    async fn test_daemon_crash_recovery() -> Result<()> {
        if env::var("CI").is_ok() {
            return Ok(());
        }

        ensure_binary_env().await;

        let temp = TempDir::new()?;
        let socket = temp.path().join("crash.sock");
        let socket_str = socket.to_string_lossy().to_string();

        let mut config = OrchestrationConfig::default();
        config.mode = OrchestrationMode::Daemon;
        config.daemon_socket = Some(socket_str.clone());
        config.auto_start_daemon = true;
        config.daemon_start_timeout_ms = 5_000;

        DaemonBackend::ensure_daemon_running(&socket_str, true, 5_000).await?;
        let backend = DaemonBackend::new(config.clone()).await?;

        let result = backend
            .delegate(
                "crash test".into(),
                "test_ai".into(),
                None,
                Some(Duration::from_secs(30)),
                None,
                None,
                None,
                None,
            )
            .await?;
        assert!(!result.task_id.is_empty());

        kill_daemon_by_socket(&socket_str).await;

        DaemonBackend::ensure_daemon_running(&socket_str, true, 5_000).await?;
        let backend2 = DaemonBackend::new(config.clone()).await?;
        let recovery = backend2
            .delegate(
                "recovery".into(),
                "test_ai".into(),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .await?;
        assert!(!recovery.task_id.is_empty());

        kill_daemon_by_socket(&socket_str).await;
        Ok(())
    }

    #[tokio::test]
    async fn test_env_overrides() -> Result<()> {
        ensure_binary_env().await;

        env::set_var("DEVIT_NO_AUTO_START", "1");
        let socket = "/tmp/devitd_no_autostart.sock";
        let result = DaemonBackend::ensure_daemon_running(socket, true, 1_000).await;
        assert!(result.is_err());
        env::remove_var("DEVIT_NO_AUTO_START");

        env::set_var("CI", "1");
        let result = DaemonBackend::ensure_daemon_running(socket, true, 1_000).await;
        assert!(result.is_err());
        env::remove_var("CI");

        Ok(())
    }

    #[tokio::test]
    async fn test_daemon_start_timeouts() -> Result<()> {
        if env::var("CI").is_ok() {
            return Ok(());
        }

        ensure_binary_env().await;

        let temp = TempDir::new()?;
        let socket = temp.path().join("timeout.sock");
        let socket_str = socket.to_string_lossy().to_string();

        kill_daemon_by_socket(&socket_str).await;
        let fail = DaemonBackend::ensure_daemon_running(&socket_str, true, 1).await;
        assert!(fail.is_err(), "should time out with 1ms");

        let success = DaemonBackend::ensure_daemon_running(&socket_str, true, 5_000).await;
        assert!(success.is_ok(), "should succeed with reasonable timeout");

        kill_daemon_by_socket(&socket_str).await;
        Ok(())
    }

    #[tokio::test]
    async fn test_cross_session_delegation() -> Result<()> {
        if env::var("CI").is_ok() {
            return Ok(());
        }

        ensure_binary_env().await;

        let temp = TempDir::new()?;
        let socket = temp.path().join("session.sock");
        let socket_str = socket.to_string_lossy().to_string();

        let mut config = OrchestrationConfig::default();
        config.mode = OrchestrationMode::Daemon;
        config.daemon_socket = Some(socket_str.clone());
        config.auto_start_daemon = true;

        DaemonBackend::ensure_daemon_running(&socket_str, true, 5_000).await?;
        let session1 = DaemonBackend::new(config.clone()).await?;
        let task = session1
            .delegate(
                "shared task".into(),
                "claude_code".into(),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .await?;

        let session2 = DaemonBackend::new(config.clone()).await?;
        let status = session2.status(StatusFilter::All).await?;
        let found = status
            .active_tasks
            .iter()
            .any(|t| t.id == task.task_id && t.goal == "shared task");
        assert!(found, "second session should see delegated task");

        session2
            .notify(&task.task_id, "in_progress", "Working", None, None)
            .await?;

        timeout(Duration::from_secs(5), async {
            loop {
                let refreshed = session1.status(StatusFilter::All).await?;
                if let Some(updated) = refreshed.active_tasks.iter().find(|t| t.id == task.task_id)
                {
                    if updated.status == TaskStatus::InProgress {
                        break Ok::<_, anyhow::Error>(());
                    }
                }
                sleep(Duration::from_millis(200)).await;
            }
        })
        .await??;

        kill_daemon_by_socket(&socket_str).await;
        Ok(())
    }
}

mod local_tests {
    use super::*;
    use devit_orchestration::local::LocalBackend;

    #[tokio::test]
    async fn test_local_backend_isolation() -> Result<()> {
        let config = OrchestrationConfig {
            mode: OrchestrationMode::Local,
            ..Default::default()
        };

        let backend_a = LocalBackend::new(config.clone());
        let backend_b = LocalBackend::new(config.clone());

        let task = backend_a
            .delegate(
                "task_a".into(),
                "worker_a".into(),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .await?;
        assert!(!task.task_id.is_empty());

        let status_b = backend_b.status(StatusFilter::All).await?;
        assert_eq!(
            status_b.active_tasks.len() + status_b.completed_tasks.len(),
            0
        );

        let status_a = backend_a.status(StatusFilter::All).await?;
        assert_eq!(
            status_a.active_tasks.len() + status_a.completed_tasks.len(),
            1
        );

        Ok(())
    }
}

#[tokio::test]
async fn test_auto_mode_behavior() -> Result<()> {
    let config = OrchestrationConfig {
        mode: OrchestrationMode::Auto,
        auto_start_daemon: false,
        ..Default::default()
    };

    let context = OrchestrationContext::new(config).await?;
    assert!(
        !context.is_using_daemon(),
        "Auto mode should fall back to local when daemon unavailable"
    );

    let result = context
        .delegate(
            "auto mode test".into(),
            "local_ai".into(),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await?;
    assert!(!result.task_id.is_empty());

    Ok(())
}
