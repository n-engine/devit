use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

use chrono::Utc;

fn main() {
    println!("cargo:rerun-if-env-changed=DEVIT_BUILD_ID_OVERRIDE");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs");

    if let Some(path) = git_head_path() {
        println!("cargo:rerun-if-changed={}", path);
    }

    let override_id = env::var("DEVIT_BUILD_ID_OVERRIDE").ok();
    let build_time = Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string();
    let git_label = git_describe().unwrap_or_else(|| "unknown".to_string());

    let build_id = override_id.unwrap_or_else(|| format!("{} | {}", build_time, git_label));

    println!("cargo:rustc-env=DEVIT_BUILD_ID={}", build_id);
    println!("cargo:rustc-env=DEVIT_BUILD_TIME={}", build_time);
    println!("cargo:rustc-env=DEVIT_BUILD_GIT={}", git_label);
}

fn git_head_path() -> Option<String> {
    let head_path = Path::new(".git/HEAD");
    if head_path.exists() {
        if let Ok(head_ref) = fs::read_to_string(head_path) {
            if let Some(path) = head_ref.strip_prefix("ref: ") {
                let ref_path = format!(".git/{}", path.trim());
                if Path::new(&ref_path).exists() {
                    return Some(ref_path);
                }
            }
        }
        return Some(head_path.display().to_string());
    }
    None
}

fn git_describe() -> Option<String> {
    let output = Command::new("git")
        .args(["describe", "--tags", "--dirty", "--always"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    let label = raw.trim();
    if label.is_empty() {
        None
    } else {
        Some(label.to_string())
    }
}
