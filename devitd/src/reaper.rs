// Process reaper task - waitpid loop for devit_exec
// Version: v3.1

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{debug, error, warn};

use crate::process_registry::{
    load_registry, save_registry, validate_process, ProcessStatus, Registry,
};
use crate::process_utils::read_proc_stat;

#[cfg(target_family = "unix")]
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
#[cfg(target_family = "unix")]
use nix::unistd::Pid;

enum ProcessUpdate {
    Exited(i32),
    Signaled(i32),
}

/// Start reaper background task
pub fn spawn_reaper_task(registry: Arc<Mutex<Registry>>) {
    tokio::spawn(async move {
        reaper_loop(registry).await;
    });
}

async fn reaper_loop(registry: Arc<Mutex<Registry>>) {
    debug!("Reaper task started");

    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;

        // Reload registry from disk (might have been updated by devit_exec)
        let fresh_registry = match load_registry() {
            Ok(r) => r,
            Err(e) => {
                warn!("Reaper: failed to reload registry: {}", e);
                continue;
            }
        };

        let mut updates = Vec::new();

        for (pid, record) in fresh_registry.iter() {
            if record.status == ProcessStatus::Running {
                if !validate_process(record) {
                    updates.push((*pid, ProcessUpdate::Exited(-1)));
                    continue;
                }
                #[cfg(target_family = "unix")]
                {
                    let pid_nix = Pid::from_raw(*pid as i32);

                    match waitpid(pid_nix, Some(WaitPidFlag::WNOHANG)) {
                        Ok(WaitStatus::Exited(_, code)) => {
                            debug!("Process {} exited with code {}", pid, code);
                            updates.push((*pid, ProcessUpdate::Exited(code)));
                        }
                        Ok(WaitStatus::Signaled(_, sig, _)) => {
                            debug!("Process {} terminated by signal {}", pid, sig as i32);
                            updates.push((*pid, ProcessUpdate::Signaled(sig as i32)));
                        }
                        Ok(WaitStatus::StillAlive) => {
                            // Process still running - check auto_kill_at
                            if let Some(auto_kill_at) = record.auto_kill_at {
                                if chrono::Utc::now() > auto_kill_at {
                                    warn!("Process {} exceeded max lifetime, sending SIGKILL", pid);
                                    unsafe { libc::kill(*pid as i32, libc::SIGKILL) };
                                }
                            }
                        }
                        Ok(_) => {}
                        Err(e) => {
                            // Process may have been reaped by parent or doesn't exist
                            debug!("waitpid({}) failed: {}", pid, e);

                            // Double-check via /proc
                            if read_proc_stat(*pid).is_err() {
                                // Process gone, mark as exited with unknown code
                                updates.push((*pid, ProcessUpdate::Exited(-1)));
                            }
                        }
                    }
                }

                #[cfg(not(target_family = "unix"))]
                {
                    // Windows stub - not implemented
                }
            }
        }

        // Apply updates
        if !updates.is_empty() {
            let mut registry_guard = registry.lock().await;

            // Reload again to avoid race
            let mut fresh = match load_registry() {
                Ok(r) => r,
                Err(e) => {
                    error!("Reaper: failed to reload registry for updates: {}", e);
                    continue;
                }
            };

            for (pid, update) in updates {
                if let Some(record) = fresh.processes.get_mut(&pid) {
                    match update {
                        ProcessUpdate::Exited(code) => {
                            record.status = ProcessStatus::Exited;
                            record.exit_code = Some(code);
                        }
                        ProcessUpdate::Signaled(sig) => {
                            record.status = ProcessStatus::Exited;
                            record.terminated_by_signal = Some(sig);
                        }
                    }
                }
            }

            if let Err(e) = save_registry(&fresh) {
                error!("Reaper: failed to save registry: {}", e);
            } else {
                *registry_guard = fresh;
            }
        }
    }
}
