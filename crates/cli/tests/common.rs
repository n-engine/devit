//! Common helpers for RC1 integration tests

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

pub struct Tmp {
    pub dir: tempfile::TempDir,
}

impl Tmp {
    pub fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    pub fn write(&self, rel: &str, data: &str) {
        let p = self.path().join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).ok();
        }
        fs::write(p, data).unwrap();
    }

    pub fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.path().join(rel)).unwrap()
    }
}

pub fn run<S: AsRef<str>>(cmd: S, cwd: &Path) -> Output {
    let cmd_str = cmd.as_ref();

    // Replace devit commands with cargo run equivalents
    if cmd_str.starts_with("devit ") || cmd_str.contains(" devit ") {
        let workspace_root = std::env::var("CARGO_MANIFEST_DIR")
            .map(|p| {
                PathBuf::from(p)
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .to_path_buf()
            })
            .unwrap_or_else(|_| PathBuf::from("."));

        // Replace devit with full cargo run command, but preserve shell structure
        let cargo_cmd = "cargo run -p devit-cli --bin devit --target-dir target -- ";
        let full_cmd = cmd_str.replace("devit ", cargo_cmd);

        // Use shell to execute with proper working directory setup
        Command::new("sh")
            .arg("-c")
            .arg(format!("cd {} && {}", workspace_root.display(), full_cmd))
            .current_dir(cwd)
            .output()
            .expect("spawn cargo via shell")
    } else {
        Command::new("sh")
            .arg("-lc")
            .arg(cmd_str)
            .current_dir(cwd)
            .output()
            .expect("spawn")
    }
}

pub fn wait_until<F: Fn() -> bool>(timeout: Duration, f: F) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

/// Vérifie un journal via l'outil CLI `devit-jverify`.
pub fn jverify(journal: &Path, secret: &str) -> bool {
    let cmd = format!(
        "devit-jverify -- {} --secret {}",
        journal.to_string_lossy(),
        shell_escape::escape(secret.into())
    );
    run(cmd, Path::new(".")).status.success()
}
