use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

/// Minimal view of `/proc/[pid]/stat`
#[derive(Debug, Clone)]
pub struct ProcStat {
    pub starttime: u64,
}

/// Parse `/proc/[pid]/stat` and extract the `starttime` field (index 21).
pub fn read_proc_stat(pid: u32) -> io::Result<ProcStat> {
    let path = format!("/proc/{}/stat", pid);
    let content = fs::read_to_string(&path)?;
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

/// Canonicalise `candidate` relative to `root`, forbidding symlinks and traversal
/// outside of the sandbox.
pub fn canonicalize_within_root(root: &Path, candidate: &Path) -> io::Result<PathBuf> {
    let root = normalize_path(root)?;
    let combined = if candidate.is_absolute() {
        normalize_path(candidate)?
    } else {
        normalize_path(&root.join(candidate))?
    };

    if !combined.starts_with(&root) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "Path {} escapes sandbox root {}",
                combined.display(),
                root.display()
            ),
        ));
    }

    enforce_no_symlinks(&root, &combined)?;
    Ok(combined)
}

/// Canonicalise a path without following symlinks (root set to `/`).
pub fn canonicalize_with_nofollow(path: &Path) -> io::Result<PathBuf> {
    canonicalize_within_root(Path::new("/"), path)
}

/// Ensure the process group leader still matches the recorded start ticks.
#[allow(dead_code)]
pub fn verify_pgid_leader(pgid: u32, expected_start_ticks: u64) -> bool {
    read_proc_stat(pgid)
        .ok()
        .map(|stat| stat.starttime == expected_start_ticks)
        .unwrap_or(false)
}

/// Check for existence of a process in `/proc`.
pub fn process_exists(pid: u32) -> bool {
    Path::new("/proc").join(pid.to_string()).exists()
}

// --- helpers ----------------------------------------------------------------

fn normalize_path(path: &Path) -> io::Result<PathBuf> {
    if path == Path::new("") {
        return Ok(PathBuf::from("."));
    }

    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::RootDir => {
                normalized.push(Component::RootDir.as_os_str());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => {
                normalized.push(part);
            }
            Component::Prefix(prefix) => {
                normalized.push(prefix.as_os_str());
            }
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

        // Skip checks for prefixes and the virtual root itself.
        if matches!(component, Component::Prefix(_) | Component::RootDir) {
            continue;
        }

        if !current.starts_with(root) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "Path {} escapes sandbox root {}",
                    current.display(),
                    root.display()
                ),
            ));
        }

        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("Symlink detected in path: {}", current.display()),
                    ));
                }
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                // Parent may still exist; that's enough for our guarantees.
                continue;
            }
            Err(err) => return Err(err),
        }
    }

    Ok(())
}

fn push_component(path: &mut PathBuf, component: Component<'_>) {
    match component {
        Component::RootDir => {
            *path = PathBuf::from(Component::RootDir.as_os_str());
        }
        Component::CurDir => {}
        Component::ParentDir => {
            path.pop();
        }
        Component::Normal(part) => {
            path.push(part);
        }
        Component::Prefix(prefix) => {
            path.push(prefix.as_os_str());
        }
    }
}
