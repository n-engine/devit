use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

static MCP_SERVER_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Ensures the canonical mcp-server binary is built and returns its filesystem path.
pub fn mcp_server_path() -> PathBuf {
    MCP_SERVER_PATH
        .get_or_init(|| {
            build_mcp_server();
            let path = workspace_root().join("target/debug/mcp-server");
            assert!(
                path.exists(),
                "mcp-server binary not found at {}",
                path.display()
            );
            path
        })
        .clone()
}

fn build_mcp_server() {
    let status = Command::new("cargo")
        .args(["build", "-p", "mcp-server"])
        .status()
        .expect("failed to invoke cargo build for mcp-server");

    assert!(
        status.success(),
        "cargo build -p mcp-server exited with status {:?}",
        status.code()
    );
}

pub fn workspace_root() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("crates/cli manifest should have workspace parent")
        .to_path_buf()
}
