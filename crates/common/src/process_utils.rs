use std::io;
use std::path::{Component, Path, PathBuf};

#[cfg(windows)]
use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
#[cfg(windows)]
use windows_sys::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION,
};

/// Stat data from /proc/[pid]/stat
#[derive(Debug, Clone)]
pub struct ProcStat {
    pub starttime: u64,
}

/// Read starttime from /proc/[pid]/stat (field 22 / index 21).
#[cfg(target_family = "unix")]
pub fn read_proc_stat(pid: u32) -> io::Result<ProcStat> {
    let path = format!("/proc/{}/stat", pid);
    let content = std::fs::read_to_string(&path)?;
    let parts: Vec<&str> = content.split_whitespace().collect();

    let starttime = parts
        .get(21)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Missing starttime field in {}", path),
            )
        })?
        .parse::<u64>()
        .map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Failed to parse starttime in {}: {}", path, err),
            )
        })?;

    Ok(ProcStat { starttime })
}

/// Read process creation time on Windows (100ns units since 1601).
#[cfg(windows)]
pub fn read_proc_stat(pid: u32) -> io::Result<ProcStat> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_INFORMATION, 0, pid);
        let handle = if handle.is_null() {
            let limited = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if limited.is_null() {
                return Err(io::Error::last_os_error());
            }
            limited
        } else {
            handle
        };

        let mut creation_time: FILETIME = std::mem::zeroed();
        let mut exit_time: FILETIME = std::mem::zeroed();
        let mut kernel_time: FILETIME = std::mem::zeroed();
        let mut user_time: FILETIME = std::mem::zeroed();

        let result = GetProcessTimes(
            handle,
            &mut creation_time,
            &mut exit_time,
            &mut kernel_time,
            &mut user_time,
        );
        let get_times_error = if result == 0 {
            Some(io::Error::last_os_error())
        } else {
            None
        };

        if CloseHandle(handle) == 0 {
            return Err(io::Error::last_os_error());
        }

        if let Some(err) = get_times_error {
            return Err(err);
        }

        let starttime =
            ((creation_time.dwHighDateTime as u64) << 32) | creation_time.dwLowDateTime as u64;

        Ok(ProcStat { starttime })
    }
}

/// Canonicalise path without following symlinks (root `/`).
pub fn canonicalize_with_nofollow(path: &Path) -> io::Result<PathBuf> {
    let normalized = normalize_path(path)?;
    enforce_no_symlinks(Path::new("/"), &normalized)?;
    Ok(normalized)
}

/// Canonicalise `candidate` relative to `root`, preventing traversal outside.
pub fn canonicalize_within_root(root: &Path, candidate: &Path) -> io::Result<PathBuf> {
    let root_norm = normalize_path(root)?;
    let combined = if candidate.is_absolute() {
        normalize_path(candidate)?
    } else {
        normalize_path(&root_norm.join(candidate))?
    };

    if !combined.starts_with(&root_norm) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "Path {} escapes sandbox {}",
                combined.display(),
                root_norm.display()
            ),
        ));
    }

    enforce_no_symlinks(&root_norm, &combined)?;
    Ok(combined)
}

/// Verify PGID leader matches expected start_ticks.
#[cfg(target_family = "unix")]
pub fn verify_pgid_leader(pgid: u32, expected_start_ticks: u64) -> bool {
    read_proc_stat(pgid)
        .ok()
        .map(|stat| stat.starttime == expected_start_ticks)
        .unwrap_or(false)
}

/// Windows does not expose PGID; treat validation as best-effort success.
#[cfg(windows)]
pub fn verify_pgid_leader(_pgid: u32, _expected_start_ticks: u64) -> bool {
    true
}

/// Check if process exists using `/proc`.
#[cfg(target_family = "unix")]
pub fn process_exists(pid: u32) -> bool {
    Path::new("/proc").join(pid.to_string()).exists()
}

/// Check if a process exists on Windows via OpenProcess.
#[cfg(windows)]
pub fn process_exists(pid: u32) -> bool {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            false
        } else {
            CloseHandle(handle);
            true
        }
    }
}

fn normalize_path(path: &Path) -> io::Result<PathBuf> {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::RootDir => normalized.push(Component::RootDir.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
        }
    }

    if normalized.as_os_str().is_empty() {
        normalized.push(".");
    }

    Ok(normalized)
}

fn enforce_no_symlinks(root: &Path, target: &Path) -> io::Result<()> {
    let mut current = PathBuf::new();

    for component in target.components() {
        push_component(&mut current, component);

        if matches!(component, Component::Prefix(_) | Component::RootDir) {
            continue;
        }

        if !current.starts_with(root) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "Path {} escapes sandbox {}",
                    current.display(),
                    root.display()
                ),
            ));
        }

        match std::fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("Symlink detected in path: {}", current.display()),
                    ));
                }
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err),
        }
    }

    Ok(())
}

fn push_component(path: &mut PathBuf, component: Component<'_>) {
    match component {
        Component::RootDir => *path = PathBuf::from(Component::RootDir.as_os_str()),
        Component::CurDir => {}
        Component::ParentDir => {
            path.pop();
        }
        Component::Normal(part) => path.push(part),
        Component::Prefix(prefix) => path.push(prefix.as_os_str()),
    }
}

#[cfg(all(test, target_family = "unix"))]
mod tests {
    use super::*;

    #[test]
    fn test_read_proc_stat_self() {
        let pid = std::process::id();
        let stat = read_proc_stat(pid).expect("Failed to read own stat");
        assert!(stat.starttime > 0);
    }

    #[test]
    fn test_process_exists_self() {
        let pid = std::process::id();
        assert!(process_exists(pid));
    }

    #[test]
    fn test_process_exists_invalid() {
        assert!(!process_exists(999999));
    }

    #[test]
    fn test_canonicalize_within_root_rejects_escape() {
        let root = Path::new("/tmp");
        let result = canonicalize_within_root(root, Path::new("../etc/passwd"));
        assert!(result.is_err());
    }
}
