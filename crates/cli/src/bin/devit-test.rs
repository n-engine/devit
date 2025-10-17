// Binaire de test pour les nouvelles commandes Core Engine
use chrono::Utc;
use clap::{Parser, Subcommand};
use devit_common::StdResponse;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Parser, Debug)]
#[command(
    name = "devit-test",
    version,
    about = "DevIt CLI Test - Core Engine commands"
)]
struct Cli {
    #[arg(long = "json-only", global = true)]
    json_only: bool,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Propose a patch (unified diff)
    Suggest {
        /// Goal to achieve (e.g., "add websocket support")
        #[arg(long)]
        goal: String,
        /// Use MCP for suggestions
        #[arg(long)]
        use_mcp: bool,
    },

    /// Apply a unified diff to the workspace
    Apply {
        /// Patch file path
        #[arg(long)]
        patch_file: PathBuf,
        /// Approval level (untrusted|ask|moderate|trusted|privileged)
        #[arg(long)]
        approval: Option<String>,
        /// Sandbox profile (strict|permissive)
        #[arg(long)]
        sandbox: Option<String>,
    },

    /// Chain: suggest -> (approval) -> apply -> commit -> test
    Run {
        /// Goal to achieve
        #[arg(long)]
        goal: String,
        /// Use MCP for execution
        #[arg(long)]
        use_mcp: bool,
    },

    /// Run tests according to detected stack (Cargo/npm/CMake)
    Test {
        /// Test stack (cargo|pytest|npm)
        #[arg(long)]
        stack: Option<String>,
        /// Custom test command
        #[arg(long)]
        cmd: Option<String>,
        /// Timeout in seconds
        #[arg(long)]
        timeout: Option<u64>,
    },

    /// Create a snapshot of the current state
    Snapshot,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let response = match cli.command {
        Commands::Suggest { goal, use_mcp } => handle_suggest(goal, use_mcp).await,
        Commands::Apply {
            patch_file,
            approval,
            sandbox,
        } => handle_apply(patch_file, approval, sandbox).await,
        Commands::Run { goal, use_mcp } => handle_run(goal, use_mcp).await,
        Commands::Test {
            stack,
            cmd,
            timeout,
        } => handle_test(stack, cmd, timeout).await,
        Commands::Snapshot => handle_snapshot().await,
    };

    output_response(response, cli.json_only);
}

async fn handle_suggest(goal: String, use_mcp: bool) -> StdResponse<String> {
    StdResponse {
        success: true,
        timestamp: Utc::now(),
        request_id: Some(Uuid::new_v4()),
        error: None,
        data: Some(format!(
            "Mock patch for goal: {}, use_mcp: {}",
            goal, use_mcp
        )),
    }
}

async fn handle_apply(
    patch_file: PathBuf,
    approval: Option<String>,
    sandbox: Option<String>,
) -> StdResponse<String> {
    StdResponse {
        success: true,
        timestamp: Utc::now(),
        request_id: Some(Uuid::new_v4()),
        error: None,
        data: Some(format!(
            "Mock apply result for patch: {:?}, approval: {:?}, sandbox: {:?}",
            patch_file, approval, sandbox
        )),
    }
}

async fn handle_run(goal: String, use_mcp: bool) -> StdResponse<String> {
    StdResponse {
        success: true,
        timestamp: Utc::now(),
        request_id: Some(Uuid::new_v4()),
        error: None,
        data: Some(format!(
            "Mock run result for goal: {}, use_mcp: {}",
            goal, use_mcp
        )),
    }
}

async fn handle_test(
    stack: Option<String>,
    cmd: Option<String>,
    timeout: Option<u64>,
) -> StdResponse<String> {
    StdResponse {
        success: true,
        timestamp: Utc::now(),
        request_id: Some(Uuid::new_v4()),
        error: None,
        data: Some(format!(
            "Mock test result for stack: {:?}, cmd: {:?}, timeout: {:?}",
            stack, cmd, timeout
        )),
    }
}

async fn handle_snapshot() -> StdResponse<String> {
    StdResponse {
        success: true,
        timestamp: Utc::now(),
        request_id: Some(Uuid::new_v4()),
        error: None,
        data: Some(format!(
            "snapshot_{}",
            Uuid::new_v4().to_string().replace('-', "")[..8].to_string()
        )),
    }
}

fn output_response<T: serde::Serialize + std::fmt::Debug>(
    response: StdResponse<T>,
    json_only: bool,
) {
    if json_only {
        if let Ok(json) = serde_json::to_string_pretty(&response) {
            println!("{}", json);
        }
    } else {
        if response.success {
            if let Some(data) = response.data {
                if let Ok(json) = serde_json::to_string_pretty(&data) {
                    println!("{}", json);
                } else {
                    println!("{:?}", data);
                }
            }
        } else {
            if let Some(error) = response.error {
                eprintln!("Error {}: {}", error.code, error.message);
            }
        }
    }
}
