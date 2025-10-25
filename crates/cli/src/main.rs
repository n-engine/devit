// # -----------------------------
// # crates/cli/src/main.rs
// # -----------------------------
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tracing_subscriber::{fmt, EnvFilter};

const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " (", env!("DEVIT_GIT_SHA"), ")");
use devit_agent::Agent;
use devit_common::orchestration::OrchestrationMode;
use devit_common::{
    ApprovalLevel, Config, Event, PolicyCfg, SandboxProfile, StdError, StdResponse,
};
#[cfg(feature = "sandbox")]
use devit_sandbox as sandbox;
use devit_tools::git;
use std::time::Duration;
mod commit_msg;
mod merge_assist;
mod precommit;
mod recipes;
mod report;
mod test_runner;
use hmac::{Hmac, Mac};
use rand::RngCore;
use recipes::{list_recipes, run_recipe, RecipeRunError};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{stdin, Read, Write};
use std::path::{Path, PathBuf};
mod context;
mod sbom;

// Core Engine
use devit_cli::core::formats::OutputFormat;
use devit_cli::core::{CoreConfig, CoreEngine, DevItError};
use serde_json::{json, Value};

#[derive(Debug, Clone)]
struct LlmConfig {
    backend: Option<String>,
    model: Option<String>,
    endpoint: Option<String>,
    api_key: Option<String>,
}

#[derive(Parser, Debug)]
#[command(name = "devit", version = VERSION, about = "DevIt CLI - patch-only agent", long_about = None)]
struct Cli {
    /// Output format: pretty human-readable format (default is JSON)
    #[arg(long = "pretty", global = true)]
    pretty: bool,
    /// Legacy alias for JSON output (kept for compatibility)
    #[arg(long = "json-only", alias = "quiet-json", global = true, hide = true)]
    json_only: bool,
    /// Log level (trace, debug, info, warn, error, off). Overrides RUST_LOG if set.
    #[arg(long = "log-level", global = true, value_name = "LEVEL")]
    log_level: Option<String>,
    /// Enable structured JSON logging
    #[arg(long = "json-logs", global = true)]
    json_logs: bool,
    /// Assume yes for confirmation prompts (bypass interactive approval)
    #[arg(long = "yes", global = true)]
    yes: bool,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Propose a patch (unified diff)
    Suggest {
        #[arg(default_value = ".")]
        path: String,
        /// Goal to achieve (e.g., "add websocket support")
        #[arg(short, long)]
        goal: String,
        /// Use MCP for suggestions
        #[arg(long)]
        use_mcp: bool,
        /// LLM backend (openai|ollama|lmstudio)
        #[arg(long)]
        llm_backend: Option<String>,
        /// Model name (e.g., "gpt-4", "llama3.1:8b")
        #[arg(long)]
        model: Option<String>,
        /// LLM endpoint URL
        #[arg(long)]
        llm_endpoint: Option<String>,
        /// LLM API key (env var name or file path)
        #[arg(long)]
        llm_api_key: Option<String>,
    },

    /// Apply a unified diff to the workspace.
    ///
    /// Tips:
    /// - Generate patches with `git diff` so the `diff --git` / `---` / `+++` headers are present.
    /// - Use `--dry-run` to validate the patch before writing to disk.
    ///
    /// Examples:
    /// ```
    /// git diff HEAD~1..HEAD > /tmp/changes.diff
    /// devit apply --patch-file /tmp/changes.diff --dry_run
    /// ```
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
        /// Dry run - no side effects, only validation and preview
        #[arg(long)]
        dry_run: bool,
    },

    /// Chain: suggest -> (approval) -> apply -> commit -> test
    Run {
        /// Goal to achieve
        #[arg(short, long)]
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

    /// Initialize or update the workspace sandbox configuration
    Init {
        /// Sandbox root directory
        #[arg(long = "sandbox", value_name = "PATH")]
        sandbox: Option<String>,
        /// Allowed project patterns (repeatable)
        #[arg(long = "allow", value_name = "GLOB", value_delimiter = ',', action = clap::ArgAction::Append)]
        allow: Vec<String>,
        /// Default project to enter after initialization
        #[arg(long = "default-project", value_name = "PATH")]
        default_project: Option<String>,
    },

    /// Change the current workspace directory
    Cd {
        #[arg(value_name = "PATH")]
        path: String,
    },

    /// Print the current workspace directory
    Pwd,

    /// Delegate an orchestration task to a specific assistant
    Delegate {
        /// Goal to achieve (e.g., "ship feature X")
        #[arg(long)]
        goal: String,
        /// Target assistant identifier
        #[arg(long = "delegated-to", default_value = "claude_code")]
        delegated_to: String,
        /// Optional model override (otherwise falls back to context/default worker model)
        #[arg(long)]
        model: Option<String>,
        /// Timeout in seconds before the task is considered stale
        #[arg(long, value_name = "SECS")]
        timeout: Option<u64>,
        /// File watch patterns (comma-separated or repeated)
        #[arg(long = "watch", value_delimiter = ',', value_name = "GLOB")]
        watch: Vec<String>,
        /// Additional JSON context provided to the assistant
        #[arg(long, value_name = "JSON")]
        context: Option<String>,
        /// Working directory to use (relative to sandbox root)
        #[arg(long = "workdir", short = 'w')]
        working_dir: Option<String>,
        /// Response format (default|compact)
        #[arg(long = "format")]
        format: Option<String>,
    },

    /// Display orchestration status
    Status {
        /// Filter (all|active|completed|failed)
        #[arg(long)]
        filter: Option<String>,
        /// Output format (json|compact|table)
        #[arg(long)]
        format: Option<String>,
    },

    /// Record a notification for a delegated task
    Notify {
        /// Task identifier
        #[arg(long = "task")]
        task_id: String,
        /// New status value
        #[arg(long)]
        status: String,
        /// Short summary describing the update
        #[arg(long)]
        summary: String,
        /// Optional JSON payload with additional details
        #[arg(long, value_name = "JSON")]
        details: Option<String>,
        /// Optional JSON evidence payload
        #[arg(long, value_name = "JSON")]
        evidence: Option<String>,
    },

    /// Show details for a specific task
    Task {
        /// Task identifier to inspect
        #[arg(value_name = "TASK_ID")]
        task_id: String,
    },

    /// Tools (experimental): list and call
    Tool {
        #[command(subcommand)]
        action: ToolCmd,
    },

    /// Recipes runner
    Recipe {
        #[command(subcommand)]
        action: RecipeCmd,
    },

    /// TUI helpers
    Tui {
        #[command(subcommand)]
        action: TuiCmd,
    },

    /// Context utilities
    Context {
        #[command(subcommand)]
        action: CtxCmd,
    },

    /// Generate Conventional Commit message from staged or diff
    CommitMsg {
        /// Use staged changes (git diff --cached)
        #[arg(long = "from-staged", default_value_t = true)]
        from_staged: bool,
        /// Or compare from this ref to HEAD
        #[arg(long = "from-ref")]
        from_ref: Option<String>,
        /// Force type (feat|fix|refactor|docs|test|chore|perf|ci)
        #[arg(long = "type")]
        typ: Option<String>,
        /// Force scope (path or token)
        #[arg(long)]
        scope: Option<String>,
        /// Write to .git/COMMIT_EDITMSG instead of stdout
        #[arg(long)]
        write: bool,
        /// Include a small body template
        #[arg(long = "with-template")]
        with_template: bool,
    },

    /// Export reports (SARIF / JUnit)
    Report {
        #[command(subcommand)]
        kind: ReportCmd,
    },

    /// Quality gate: aggregate reports and check thresholds
    Quality {
        #[command(subcommand)]
        action: QualityCmd,
    },

    /// Merge assistance (explain/apply)
    Merge {
        #[command(subcommand)]
        action: MergeCmd,
    },

    /// Generate SBOM (CycloneDX JSON)
    Sbom {
        #[command(subcommand)]
        action: SbomCmd,
    },

    /// Apply a patch via JSON API (parity with tool call).
    ///
    /// Provide the full JSON payload expected by the MCP `devit_patch_apply` tool.
    /// The embedded diff must be the raw output of `git diff` (headers included).
    ///
    /// Example:
    /// ```bash
    /// git diff --staged > /tmp/changes.diff
    /// jq -n --rawfile diff /tmp/changes.diff '{diff:$diff}' | \
    ///   devit fs_patch_apply --json - --commit off --precommit off --tests-impacted off
    /// ```
    FsPatchApply {
        /// Read JSON from file or '-' for stdin
        #[arg(long = "json", default_value = "-")]
        json_input: String,
        /// commit mode: on|off|auto
        #[arg(long = "commit")]
        commit: Option<String>,
        /// precommit mode: on|off|auto
        #[arg(long = "precommit")]
        precommit: Option<String>,
        /// impacted tests mode: on|off|auto
        #[arg(long = "tests-impacted")]
        tests_impacted: Option<String>,
        /// attest diff enabled (default: on)
        #[arg(long = "attest-diff", default_value_t = false)]
        attest_diff: bool,
    },
}

#[derive(Subcommand, Debug)]
enum ToolCmd {
    /// List available tools (JSON)
    List,
    /// Call a tool
    Call {
        /// Tool name (fs_patch_apply | shell_exec)
        name: String,
        /// Read diff from file, or '-' for stdin (fs_patch_apply), or command for shell_exec after '--'
        #[arg(default_value = "-")]
        input: String,
        /// Auto-approve (no prompt)
        #[arg(long)]
        yes: bool,
        /// Skip precommit gate (only for fs_patch_apply)
        #[arg(long = "no-precommit")]
        no_precommit: bool,
        /// Only run precommit pipeline and exit (only for fs_patch_apply)
        #[arg(long = "precommit-only")]
        precommit_only: bool,
    },
}

#[derive(Subcommand, Debug)]
enum RecipeCmd {
    /// List available recipes (JSON)
    List,
    /// Run a recipe by id
    Run {
        #[arg(value_name = "ID")]
        id: String,
        #[arg(long = "dry-run", default_value_t = false)]
        dry_run: bool,
    },
}

#[derive(Subcommand, Debug)]
enum TuiCmd {
    /// Open a unified diff in the TUI
    OpenDiff { path: String },
    /// Open a journal log in the TUI
    OpenLog { path: String },
}

#[derive(Subcommand, Debug)]
enum CtxCmd {
    /// Build a file index at .devit/index.json
    Map {
        /// Root path (default: .)
        #[arg(default_value = ".")]
        path: String,
        /// Max bytes per file (default: 262144)
        #[arg(long = "max-bytes-per-file")]
        max_bytes_per_file: Option<usize>,
        /// Max files to index (default: 5000)
        #[arg(long = "max-files")]
        max_files: Option<usize>,
        /// Allowed extensions CSV (e.g., rs,toml,md)
        #[arg(long = "ext-allow")]
        ext_allow: Option<String>,
        /// Output JSON path (default: .devit/index.json)
        #[arg(long = "json-out")]
        json_out: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
enum TestCmd {
    /// Run all tests (auto-detected stack)
    All,
    /// Run only impacted tests based on changed files
    Impacted {
        /// Compare from this git ref to HEAD to detect changes (optional)
        #[arg(long = "changed-from")]
        changed_from: Option<String>,
        /// Framework: auto|cargo|npm|pnpm|pytest|ctest
        #[arg(long, default_value = "auto")]
        framework: String,
        /// Timeout seconds per run (default DEVIT_TIMEOUT_SECS or 300)
        #[arg(long = "timeout-secs")]
        timeout_secs: Option<u64>,
        /// Max jobs/threads (hint, not all frameworks use it)
        #[arg(long = "max-jobs")]
        max_jobs: Option<usize>,
    },
}

#[derive(Subcommand, Debug)]
enum ReportCmd {
    Sarif {
        /// Source selector (currently supports: latest)
        #[arg(long = "from", default_value = "latest")]
        from: String,
    },
    Junit {
        /// Source selector (currently supports: latest)
        #[arg(long = "from", default_value = "latest")]
        from: String,
    },
    /// Generate summary markdown
    Summary {
        #[arg(long = "junit", default_value = ".devit/reports/junit.xml")]
        junit: String,
        #[arg(long = "sarif", default_value = ".devit/reports/sarif.json")]
        sarif: String,
        #[arg(long = "out", default_value = ".devit/reports/summary.md")]
        out: String,
    },
}

#[derive(Subcommand, Debug)]
enum SbomCmd {
    /// Generate combined SBOM and write to file
    Gen {
        /// Output path (default: .devit/sbom.cdx.json)
        #[arg(long = "out", default_value = ".devit/sbom.cdx.json")]
        out: String,
    },
}

#[derive(Subcommand, Debug)]
enum QualityCmd {
    Gate {
        #[arg(long = "junit", default_value = ".devit/reports/junit.xml")]
        junit: String,
        #[arg(long = "sarif", default_value = ".devit/reports/sarif.json")]
        sarif: String,
        /// Config path with [quality] thresholds
        #[arg(long = "config", default_value = ".devit/devit.toml")]
        config: String,
        /// Print JSON summary
        #[arg(long = "json", default_value_t = true)]
        json: bool,
    },
}

#[derive(Subcommand, Debug)]
enum MergeCmd {
    /// Explain merge conflicts in files (auto-detect unmerged by default)
    Explain {
        /// Optional target files
        #[arg(value_delimiter = ' ')]
        paths: Vec<String>,
    },
    /// Apply a resolution plan (JSON path)
    Apply {
        #[arg(long = "plan")]
        plan: String,
    },
    /// One-shot resolve: explain -> auto plan -> apply
    Resolve {
        #[arg(long = "strategy", default_value = "auto")]
        strategy: String,
    },
}

/// Initialize logging based on CLI arguments and environment
fn init_logging(log_level: Option<&str>, json_logs: bool) -> Result<()> {
    // Determine log level: CLI arg overrides RUST_LOG env var
    let filter = if let Some(level) = log_level {
        match level.to_lowercase().as_str() {
            "off" => EnvFilter::new("off"),
            "error" => EnvFilter::new("error"),
            "warn" | "warning" => EnvFilter::new("warn"),
            "info" => EnvFilter::new("info"),
            "debug" => EnvFilter::new("debug"),
            "trace" => EnvFilter::new("trace"),
            _ => {
                eprintln!("Warning: Invalid log level '{}', using 'info'", level);
                EnvFilter::new("info")
            }
        }
    } else {
        // Use RUST_LOG if set, otherwise default to warn (less verbose than before)
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"))
    };

    if json_logs {
        // Structured JSON logging - use json feature if available, otherwise inform user
        fmt()
            .with_env_filter(filter)
            .with_target(true)
            .with_file(true)
            .with_line_number(true)
            .init();
        eprintln!("Note: JSON logging format requested but may not be available in this build");
    } else {
        // Human-readable logging (default)
        fmt()
            .with_env_filter(filter)
            .with_target(false) // Less noise for human reading
            .init();
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging based on CLI arguments
    init_logging(cli.log_level.as_deref(), cli.json_logs)?;

    tracing::debug!("DevIt CLI starting with version {}", VERSION);
    tracing::debug!("CLI arguments: {:?}", cli);

    let cfg: Config = load_cfg("devit.toml").context("load config")?;
    tracing::info!("Configuration loaded successfully");
    let _agent = Agent::new(cfg.clone());
    tracing::debug!("Agent initialized (no network calls made unless explicitly requested)");

    // Determine output format: JSON by default, pretty if requested, json_only for legacy
    let use_json_output = !cli.pretty || cli.json_only;
    let assume_yes = cli.yes
        || std::env::var("DEVIT_ASSUME_YES")
            .map(|v| {
                let v = v.trim().to_ascii_lowercase();
                matches!(v.as_str(), "1" | "true" | "yes")
            })
            .unwrap_or(false);
    let policy_requires_yes = cfg
        .policy
        .approval
        .trim()
        .eq_ignore_ascii_case("on-request");

    match cli.command {
        Some(Commands::Suggest {
            path,
            goal,
            use_mcp,
            llm_backend,
            model,
            llm_endpoint,
            llm_api_key,
        }) => {
            tracing::info!("Starting suggest command for goal: {}", goal);
            tracing::debug!("Suggest path: {}, use_mcp: {}", path, use_mcp);
            let llm_config = LlmConfig {
                backend: llm_backend,
                model,
                endpoint: llm_endpoint,
                api_key: llm_api_key,
            };
            let response = handle_suggest(goal, use_mcp, use_json_output, llm_config).await;
            output_response(response, use_json_output);
        }
        Some(Commands::Apply {
            patch_file,
            approval,
            sandbox,
            dry_run,
        }) => {
            tracing::info!("Starting apply command for patch: {:?}", patch_file);
            tracing::debug!(
                "Apply options - dry_run: {}, approval: {:?}, sandbox: {:?}",
                dry_run,
                approval,
                sandbox
            );
            let response =
                handle_apply(patch_file, approval, sandbox, dry_run, use_json_output).await;
            output_response(response, use_json_output);
        }
        Some(Commands::Run { goal, use_mcp }) => {
            if policy_requires_yes && !assume_yes {
                eprintln!("Policy 'on-request' requires --yes to run this command.");
                std::process::exit(1);
            }
            let response = handle_run(goal, use_mcp, use_json_output).await;
            output_response(response, use_json_output);
        }
        Some(Commands::Test {
            stack,
            cmd,
            timeout,
        }) => {
            let response = handle_test(stack, cmd, timeout, use_json_output).await;
            output_response(response, use_json_output);
        }
        Some(Commands::Snapshot) => {
            let response = handle_snapshot(use_json_output).await;
            output_response(response, use_json_output);
        }
        Some(Commands::Init {
            sandbox,
            allow,
            default_project,
        }) => {
            handle_workspace_init(sandbox, allow, default_project)?;
        }
        Some(Commands::Cd { path }) => {
            let new_dir = handle_workspace_cd(path).await?;
            println!("{}", new_dir.display());
        }
        Some(Commands::Pwd) => {
            let dir = handle_workspace_pwd().await?;
            println!("{}", dir.display());
        }
        Some(Commands::Delegate {
            goal,
            delegated_to,
            model,
            timeout,
            watch,
            context,
            working_dir,
            format,
        }) => {
            let payload = handle_orchestration_delegate(
                goal,
                delegated_to,
                model,
                timeout,
                watch,
                context,
                working_dir,
                format,
            )
            .await?;
            print_orchestration_payload(&payload, use_json_output, |value| {
                let id = value.get("task_id").and_then(Value::as_str).unwrap_or("-");
                let target = value
                    .get("delegated_to")
                    .and_then(Value::as_str)
                    .unwrap_or("-");
                let goal = value.get("goal").and_then(Value::as_str).unwrap_or("-");
                let timeout = value
                    .get("timeout_secs")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let workdir = value
                    .get("working_dir")
                    .and_then(Value::as_str)
                    .unwrap_or(".");
                let format = value
                    .get("format")
                    .and_then(Value::as_str)
                    .unwrap_or("default");
                let model = value
                    .get("model")
                    .and_then(Value::as_str)
                    .unwrap_or("<default>");
                format!(
                    "Task delegated successfully.\nTask ID: {}\nDelegated to: {}\nModel: {}\nTimeout: {}s\nWorking dir: {}\nFormat: {}\nGoal: {}",
                    id, target, model, timeout, workdir, format, goal
                )
            })?;
        }
        Some(Commands::Status { filter, format }) => {
            handle_orchestration_status(filter, format, use_json_output).await?;
        }
        Some(Commands::Notify {
            task_id,
            status,
            summary,
            details,
            evidence,
        }) => {
            let payload =
                handle_orchestration_notify(task_id, status, summary, details, evidence).await?;
            print_orchestration_payload(&payload, use_json_output, |value| {
                let status = value.get("status").and_then(Value::as_str).unwrap_or("-");
                let task = value.get("task_id").and_then(Value::as_str).unwrap_or("-");
                let summary = value.get("summary").and_then(Value::as_str).unwrap_or("-");
                format!(
                    "Notification recorded.\nTask ID: {}\nStatus: {}\nSummary: {}",
                    task, status, summary
                )
            })?;
        }
        Some(Commands::Task { task_id }) => {
            let task = handle_orchestration_task(task_id).await?;
            print_orchestration_payload(&task, use_json_output, |value| {
                format_task_details(value)
            })?;
        }
        Some(Commands::Tool { action }) => match action {
            ToolCmd::List => {
                let tools = serde_json::json!([
                    {"name": "fs_patch_apply", "args": {"patch": "string", "mode": "index|worktree", "check_only": "bool"}, "description": "Apply unified diff (index/worktree), or --check-only"},
                    {"name": "shell_exec", "args": {"cmd": "string"}, "description": "Execute command via sandboxed shell (safe-list)"},
                    {"name": "server.approve", "args": {"name": "string", "scope": "once|session|always", "plugin_id": "string?"}, "description": "Approve on-request tools (once/session/always)"}
                ]);
                let payload = serde_json::json!({"tools": tools});
                emit_json(&payload)?;
            }
            ToolCmd::Call {
                name,
                input,
                yes,
                no_precommit,
                precommit_only,
            } => {
                if name == "-" {
                    let mut s = String::new();
                    stdin().lock().read_to_string(&mut s)?;
                    let req: serde_json::Value =
                        serde_json::from_str(&s).context("tool call: JSON invalide sur stdin")?;
                    let tname = req.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let args = req.get("args").cloned().unwrap_or(serde_json::json!({}));
                    let yes_flag = req.get("yes").and_then(|v| v.as_bool()).unwrap_or(yes);
                    let res = tool_call_json(&cfg, tname, args, yes_flag);
                    match res {
                        Ok(v) => emit_json(&serde_json::json!({"ok": true, "result": v}))?,
                        Err(e) => emit_json(&serde_json::json!({
                            "ok": false,
                            "error": e.to_string()
                        }))?,
                    }
                } else {
                    let out = tool_call_legacy(
                        &cfg,
                        &name,
                        &input,
                        yes,
                        no_precommit,
                        precommit_only,
                        use_json_output,
                    );
                    if let Err(e) = out {
                        anyhow::bail!(e);
                    }
                }
            }
        },
        Some(Commands::Tui { action }) => match action {
            TuiCmd::OpenDiff { path } => {
                run_tui_command(&["--open", path.as_str()])?;
            }
            TuiCmd::OpenLog { path } => {
                run_tui_command(&["--open-log", path.as_str()])?;
            }
        },
        Some(Commands::Recipe { action }) => match action {
            RecipeCmd::List => {
                let recipes = list_recipes()?;
                emit_json(&serde_json::json!({"recipes": recipes}))?;
            }
            RecipeCmd::Run { id, dry_run } => match run_recipe(&id, dry_run) {
                Ok(report) => {
                    emit_json(&serde_json::json!({"ok": true, "recipe": report}))?;
                }
                Err(RecipeRunError { payload, exit_code }) => {
                    emit_json(&serde_json::json!({"type":"tool.error","payload": payload}))?;
                    std::process::exit(exit_code);
                }
            },
        },
        Some(Commands::Context { action }) => match action {
            CtxCmd::Map {
                path,
                max_bytes_per_file,
                max_files,
                ext_allow,
                json_out,
            } => {
                let written = build_context_index_adv(
                    &path,
                    max_bytes_per_file,
                    max_files,
                    ext_allow.as_deref(),
                    json_out.as_deref(),
                )?;
                println!("index written: {}", written.display());
            }
        },
        Some(Commands::CommitMsg {
            from_staged,
            from_ref,
            typ,
            scope,
            write,
            with_template,
        }) => {
            let opts = commit_msg::Options {
                from_staged,
                change_from: from_ref,
                typ,
                scope,
                with_template,
            };
            let msg = commit_msg::generate(&opts)?;
            if write {
                let path = ".git/COMMIT_EDITMSG";
                std::fs::write(path, msg)?;
                println!("wrote: {}", path);
            } else {
                println!("{}", msg);
            }
        }
        Some(Commands::Report { kind }) => match kind {
            ReportCmd::Sarif { from } => {
                let p = if from == "latest" {
                    report::sarif_latest()?
                } else {
                    std::path::PathBuf::from(from)
                };
                println!("{}", p.display());
            }
            ReportCmd::Junit { from } => {
                let p = if from == "latest" {
                    report::junit_latest()?
                } else {
                    std::path::PathBuf::from(from)
                };
                println!("{}", p.display());
            }
            ReportCmd::Summary { junit, sarif, out } => {
                report::summary_markdown(
                    std::path::Path::new(&junit),
                    std::path::Path::new(&sarif),
                    std::path::Path::new(&out),
                )?;
                println!("{}", out);
            }
        },
        Some(Commands::Quality { action }) => match action {
            QualityCmd::Gate {
                junit,
                sarif,
                config,
                json: _,
            } => {
                // load quality cfg
                let cfg_text = std::fs::read_to_string(&config).unwrap_or_default();
                let tbl: toml::Value =
                    toml::from_str(&cfg_text).unwrap_or(toml::Value::Table(Default::default()));
                let qcfg: devit_common::QualityCfg = tbl
                    .get("quality")
                    .and_then(|v| v.clone().try_into().ok())
                    .unwrap_or_default();
                // flaky list (optional)
                let flaky_path = ".devit/flaky_tests.txt";
                let flaky = std::fs::read_to_string(flaky_path).ok().map(|s| {
                    s.lines()
                        .map(|l| l.trim().to_string())
                        .filter(|l| !l.is_empty())
                        .collect::<Vec<_>>()
                });
                let flaky_ref = flaky.as_deref();
                let sum = report::summarize(
                    std::path::Path::new(&junit),
                    std::path::Path::new(&sarif),
                    &qcfg,
                    flaky_ref,
                )?;
                let pass = report::check_thresholds(&sum, &qcfg);
                if pass {
                    println!(
                        "{}",
                        serde_json::to_string(&serde_json::json!({
                            "type":"tool.result",
                            "payload": { "ok": true, "summary": sum, "pass": pass }
                        }))?
                    );
                    std::process::exit(0);
                } else {
                    println!(
                        "{}",
                        serde_json::to_string(&serde_json::json!({
                            "type":"tool.error",
                            "payload": { "ok": false, "summary": sum, "pass": pass, "reason":"thresholds_exceeded" }
                        }))?
                    );
                    std::process::exit(1);
                }
            }
        },
        Some(Commands::Merge { action }) => match action {
            MergeCmd::Explain { paths } => {
                let conf = merge_assist::explain(&paths)?;
                println!(
                    "{}",
                    serde_json::to_string(&serde_json::json!({
                        "type":"tool.result",
                        "payload": {"ok": true, "conflicts": conf}
                    }))?
                );
            }
            MergeCmd::Apply { plan } => {
                let txt = std::fs::read_to_string(&plan).context("read plan.json")?;
                let p: merge_assist::Plan =
                    serde_json::from_str(&txt).context("parse plan.json")?;
                merge_assist::apply_plan(&p).map_err(|e| anyhow::anyhow!(e.to_string()))?;
                println!(
                    "{}",
                    serde_json::to_string(&serde_json::json!({
                        "type":"tool.result",
                        "payload": {"ok": true}
                    }))?
                );
            }
            MergeCmd::Resolve { strategy: _ } => {
                let conf = merge_assist::explain(&Vec::new())?;
                if conf.is_empty() {
                    println!(
                        "{}",
                        serde_json::to_string(&serde_json::json!({
                            "type":"tool.result",
                            "payload": {"ok": true, "resolved": false, "reason": "no_conflict"}
                        }))?
                    );
                } else {
                    let plan = merge_assist::propose_auto(&conf);
                    let files = plan.len() as u32;
                    merge_assist::apply_plan(&plan).map_err(|e| anyhow::anyhow!(e.to_string()))?;
                    println!(
                        "{}",
                        serde_json::to_string(&serde_json::json!({
                            "type":"tool.result",
                            "payload": {"ok": true, "resolved": true, "files": files, "backups_dir": ".devit/merge_backups"}
                        }))?
                    );
                }
            }
        },
        Some(Commands::Sbom { action }) => match action {
            SbomCmd::Gen { out } => {
                let outp = std::path::Path::new(&out);
                if let Some(dir) = outp.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                sbom::generate(outp)?;
                println!("{}", out);
            }
        },
        Some(Commands::FsPatchApply {
            json_input,
            commit,
            precommit,
            tests_impacted,
            attest_diff,
        }) => {
            // Read JSON request (fs_patch_apply args) and merge CLI overrides
            let mut s = String::new();
            if json_input == "-" || json_input == "@-" {
                stdin().read_to_string(&mut s)?;
            } else {
                s = std::fs::read_to_string(&json_input)?;
            }
            let req: serde_json::Value =
                serde_json::from_str(&s).context("parse JSON input for fs_patch_apply")?;
            // Accept both top-level args or raw args
            let mut args = if let Some(a) = req.get("args").cloned() {
                a
            } else {
                req.clone()
            };
            if let Some(v) = commit {
                args["commit"] = serde_json::Value::String(v);
            }
            if let Some(v) = precommit {
                args["precommit"] = serde_json::Value::String(v);
            }
            if let Some(v) = tests_impacted {
                args["tests_impacted"] = serde_json::Value::String(v);
            }
            if attest_diff {
                args["attest_diff"] = serde_json::Value::Bool(true);
            }
            let out = tool_call_json(&cfg, "fs_patch_apply", args, true)?;
            println!("{}", serde_json::to_string(&out)?);
        }
        _ => {
            eprintln!(
                "Usage:\n  devit suggest --goal \"...\" [PATH]\n  devit apply [-|PATCH.diff] [--yes] [--force]\n  devit run --goal \"...\" [PATH] [--yes] [--force]\n  devit test"
            );
        }
    }

    Ok(())
}

fn load_cfg(path: &str) -> Result<Config> {
    // Permettre un override via variable d'environnement
    let cfg_path = std::env::var("DEVIT_CONFIG").unwrap_or_else(|_| path.to_string());
    let s = fs::read_to_string(&cfg_path)
        .with_context(|| format!("unable to read config at {}", cfg_path))?;
    let cfg: Config = toml::from_str(&s)?;
    Ok(cfg)
}

fn collect_context(path: &str) -> Result<String> {
    // MVP: naive — list a few files with content; later: git-aware, size limits
    let mut out = String::new();
    for entry in walkdir::WalkDir::new(path).max_depth(2) {
        let entry = entry?;
        if entry.file_type().is_file() {
            let p = entry.path().display().to_string();
            if p.ends_with(".rs") || p.ends_with("Cargo.toml") {
                if let Ok(content) = std::fs::read_to_string(entry.path()) {
                    out.push_str(&format!("\n>>> FILE: {}\n{}\n", p, content));
                }
            }
        }
    }
    Ok(out)
}

fn read_patch(input: &str) -> Result<String> {
    if input == "-" {
        let mut s = String::new();
        stdin().lock().read_to_string(&mut s)?;
        Ok(s)
    } else {
        Ok(fs::read_to_string(input)?)
    }
}

fn ensure_git_repo() -> Result<()> {
    if !git::is_git_available() {
        anyhow::bail!("git is not available in PATH.");
    }
    if !git::in_repo() {
        anyhow::bail!("not inside a git repository (git rev-parse --is-inside-work-tree).");
    }
    Ok(())
}

fn ask_approval() -> Result<bool> {
    use std::io::{self, Write};
    eprint!("Appliquer le patch et committer ? [y/N] ");
    io::stderr().flush().ok();
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    let ans = buf.trim().to_lowercase();
    Ok(ans == "y" || ans == "yes")
}

fn emit_json(value: &serde_json::Value) -> Result<()> {
    let mut stdout = std::io::stdout().lock();
    serde_json::to_writer(&mut stdout, value)?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(())
}

fn run_tui_command(args: &[&str]) -> Result<()> {
    let mut candidate = std::env::current_exe()?;
    candidate.set_file_name("devit-tui");

    let status = if candidate.exists() {
        std::process::Command::new(&candidate).args(args).status()
    } else {
        std::process::Command::new("devit-tui").args(args).status()
    }
    .with_context(|| format!("spawn devit-tui with args {:?}", args))?;

    if !status.success() {
        return Err(anyhow::anyhow!(
            "devit-tui exited with status {}",
            status.code().unwrap_or(-1)
        ));
    }
    Ok(())
}

fn requires_approval_tool(policy: &PolicyCfg, tool: &str, yes_flag: bool, action: &str) -> bool {
    let eff = policy
        .approvals
        .as_ref()
        .and_then(|m| {
            m.get(&tool.to_ascii_lowercase())
                .map(|s| s.to_ascii_lowercase())
        })
        .unwrap_or_else(|| policy.approval.to_ascii_lowercase());
    match (eff.as_str(), action) {
        ("never", _) => false,
        ("untrusted", _) => true,
        ("on-request", _) => !yes_flag,
        ("on-failure", "write") => !yes_flag,
        ("on-failure", _) => false,
        _ => !yes_flag,
    }
}

fn compute_attest_hash(patch: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(patch.as_bytes());
    let out = hasher.finalize();
    hex::encode(out)
}

fn compute_call_attest(tool: &str, args: &serde_json::Value) -> Result<String> {
    // HMAC(tool_name, sha256(args_json), timestamp_ms)
    let ts_ms: u128 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let args_json = serde_json::to_string(args)?;
    let mut hasher = Sha256::new();
    hasher.update(args_json.as_bytes());
    let args_sha = hex::encode(hasher.finalize());
    let key = hmac_key()?;
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(&key).expect("HMAC key");
    let material = format!("{}:{}:{}", tool, args_sha, ts_ms);
    mac.update(material.as_bytes());
    Ok(hex::encode(mac.finalize().into_bytes()))
}

fn ensure_devit_dir() -> Result<PathBuf> {
    let p = Path::new(".devit");
    if !p.exists() {
        fs::create_dir_all(p)?;
    }
    Ok(p.to_path_buf())
}

fn hmac_key() -> Result<Vec<u8>> {
    let dir = ensure_devit_dir()?;
    let key_path = dir.join("hmac.key");
    if key_path.exists() {
        return Ok(fs::read(key_path)?);
    }
    let mut key = vec![0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    fs::write(&key_path, &key)?;
    Ok(key)
}

fn journal_event(ev: &Event) -> Result<()> {
    let dir = ensure_devit_dir()?;
    let jpath = dir.join("journal.jsonl");
    let key = hmac_key()?;
    let ev_json = serde_json::to_vec(ev)?;
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(&key).expect("HMAC key");
    mac.update(&ev_json);
    let sig = hex::encode(mac.finalize().into_bytes());
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let rec = serde_json::json!({ "ts": ts, "event": ev, "sig": sig });
    let line = serde_json::to_string(&rec)? + "\n";
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(jpath)?
        .write_all(line.as_bytes())?;
    Ok(())
}

fn build_context_index_adv(
    root: &str,
    max_bytes_per_file: Option<usize>,
    max_files: Option<usize>,
    ext_allow: Option<&str>,
    json_out: Option<&Path>,
) -> Result<PathBuf> {
    let dir = ensure_devit_dir()?;
    let out = json_out
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| dir.join("index.json"));
    // Timeout support
    let timeout = std::env::var("DEVIT_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_secs);
    let opts = crate::context::ContextOpts {
        max_bytes_per_file: max_bytes_per_file.unwrap_or(262_144),
        max_files: max_files.unwrap_or(5000),
        ext_allow: ext_allow.map(|s| {
            s.split(',')
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect()
        }),
        timeout,
        out_path: out.clone(),
    };
    match crate::context::generate_index(Path::new(root), &opts) {
        Ok(w) => Ok(w),
        Err(e) => {
            if e.to_string().contains("timeout") {
                eprintln!("error: context map timeout");
                std::process::exit(124);
            }
            Err(e)
        }
    }
}

// legacy helper removed; scanning now handled in context module

fn tool_call_json(
    cfg: &Config,
    name: &str,
    args: serde_json::Value,
    yes: bool,
) -> Result<serde_json::Value> {
    match name {
        "fs_patch_apply" => {
            ensure_git_repo()?;
            if cfg.policy.sandbox.to_lowercase() == "read-only" {
                anyhow::bail!("policy.sandbox=read-only: apply denied (no write operations allowed)");
            }
            let patch = args.get("patch").and_then(|v| v.as_str()).unwrap_or("");
            let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("index");
            let no_precommit = args
                .get("no_precommit")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let precommit_only = args
                .get("precommit_only")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let precommit_mode = args
                .get("precommit")
                .and_then(|v| v.as_str())
                .unwrap_or("auto")
                .to_lowercase();
            let tests_mode = args
                .get("tests_impacted")
                .and_then(|v| v.as_str())
                .unwrap_or("auto")
                .to_lowercase();
            let tests_timeout_secs = args
                .get("tests_timeout_secs")
                .and_then(|v| v.as_u64())
                .unwrap_or(300);
            let allow_apply_on_tests_fail = args
                .get("allow_apply_on_tests_fail")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let check_only = args
                .get("check_only")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let commit_mode = args
                .get("commit")
                .and_then(|v| v.as_str())
                .unwrap_or("auto")
                .to_lowercase();
            let commit_type = args
                .get("commit_type")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let commit_scope = args
                .get("commit_scope")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let commit_body_template = args
                .get("commit_body_template")
                .and_then(|v| v.as_str())
                .map(|p| std::fs::read_to_string(p).unwrap_or_default());
            let commit_dry_run = args
                .get("commit_dry_run")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let commit_signoff = args
                .get("signoff")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let no_prov_footer = args
                .get("no_provenance_footer")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if patch.is_empty() {
                anyhow::bail!("fs_patch_apply: 'patch' field is required (diff content)");
            }
            // Precommit gate
            if precommit_only {
                match precommit::run(cfg) {
                    Ok(()) => return Ok(serde_json::json!({"precommit_ok": true})),
                    Err(f) => anyhow::bail!(format!(
                        "{}",
                        serde_json::json!({
                            "precommit_failed": true, "tool": f.tool, "exit_code": f.exit_code, "stderr": f.stderr
                        })
                    )),
                }
            }
            // decide precommit enabled
            let profile = cfg
                .policy
                .profile
                .clone()
                .unwrap_or_else(|| "std".into())
                .to_lowercase();
            let precommit_enabled = match precommit_mode.as_str() {
                "on" => true,
                "off" => false,
                _ => profile != "danger",
            };
            if no_precommit && precommit_enabled {
                // Bypass policy check
                if !yes || !precommit::bypass_allowed(cfg) {
                    anyhow::bail!(format!(
                        "{}",
                        serde_json::json!({
                            "approval_required": true, "policy": "on_request", "phase": "pre", "reason": "precommit_bypass"
                        })
                    ));
                }
            } else if precommit_enabled {
                if let Err(f) = precommit::run(cfg) {
                    // write precommit report
                    let _ = std::fs::create_dir_all(".devit/reports");
                    let _ = std::fs::write(
                        ".devit/reports/precommit.json",
                        serde_json::to_vec(&serde_json::json!({
                            "precommit_failed": true, "tool": f.tool, "exit_code": f.exit_code
                        }))
                        .unwrap_or_default(),
                    );
                    anyhow::bail!(format!(
                        "{}",
                        serde_json::json!({
                            "precommit_failed": true, "tool": f.tool, "exit_code": f.exit_code, "stderr": f.stderr
                        })
                    ));
                }
                let _ = std::fs::create_dir_all(".devit/reports");
                let _ = std::fs::write(
                    ".devit/reports/precommit.json",
                    serde_json::to_vec(&serde_json::json!({
                        "ok": true
                    }))
                    .unwrap_or_default(),
                );
            }
            git::apply_check(patch)?;
            if check_only {
                return Ok(serde_json::json!({"checked": true}));
            }
            let ask = requires_approval_tool(&cfg.policy, "git", yes, "write");
            if ask && !ask_approval()? {
                anyhow::bail!("Cancelled by user.");
            }
            let ok = match mode {
                "worktree" => git::apply_worktree(patch)?,
                _ => git::apply_index(patch)?,
            };
            if !ok {
                anyhow::bail!(format!("git apply failed ({mode})"));
            }
            // tests impacted pipeline
            let tests_enabled = match tests_mode.as_str() {
                "on" => true,
                "off" => false,
                _ => profile != "danger",
            };
            if tests_enabled {
                let ns = git::numstat(patch).unwrap_or_default();
                let changed: Vec<String> = ns.into_iter().map(|e| e.path).collect();
                let opts = test_runner::ImpactedOpts {
                    changed_from: None,
                    changed_paths: Some(changed),
                    max_jobs: None,
                    framework: Some("auto".into()),
                    timeout_secs: Some(tests_timeout_secs),
                };
                match test_runner::run_impacted(&opts) {
                    Ok(rep) => {
                        let _ = std::fs::write(".devit/reports/impacted.json", serde_json::to_vec(&serde_json::json!({
                            "ok": true, "framework": rep.framework, "ran": rep.ran, "failed": rep.failed, "logs_path": rep.logs_path
                        })).unwrap_or_default());
                        if rep.failed > 0 {
                            if !allow_apply_on_tests_fail {
                                // revert
                                use std::io::Write as _;
                                use std::process::{Command, Stdio};
                                let mut child = Command::new("git")
                                    .args(["apply", "-R", "-"])
                                    .stdin(Stdio::piped())
                                    .stdout(Stdio::null())
                                    .stderr(Stdio::piped())
                                    .spawn()
                                    .ok();
                                let mut reverted = false;
                                if let Some(ref mut ch) = child {
                                    if let Some(stdin) = ch.stdin.as_mut() {
                                        let _ = stdin.write_all(patch.as_bytes());
                                    }
                                    if let Ok(status) = ch.wait() {
                                        reverted = status.success();
                                    }
                                }
                                anyhow::bail!(format!(
                                    "{}",
                                    serde_json::json!({
                                        "tests_failed": true, "reverted": reverted, "report": ".devit/reports/junit.xml"
                                    })
                                ));
                            } else {
                                anyhow::bail!(format!(
                                    "{}",
                                    serde_json::json!({
                                        "tests_failed": true, "report": ".devit/reports/junit.xml"
                                    })
                                ));
                            }
                        }
                    }
                    Err(e) => {
                        let s = e.to_string();
                        if s.contains("\"timeout\":true") {
                            anyhow::bail!(format!("{}", serde_json::json!({"timeout": true})));
                        } else {
                            anyhow::bail!(format!(
                                "{}",
                                serde_json::json!({"tests_failed": true, "report": ".devit/reports/junit.xml"})
                            ));
                        }
                    }
                }
            }
            // Commit stage
            let profile = cfg
                .policy
                .profile
                .clone()
                .unwrap_or_else(|| "std".into())
                .to_lowercase();
            let commit_default_on = matches!(profile.as_str(), "safe" | "std");
            let commit_enabled = match commit_mode.as_str() {
                "on" => true,
                "off" => false,
                _ => commit_default_on,
            };
            // gather staged paths
            let staged_list = std::process::Command::new("git")
                .args(["diff", "--name-only", "--cached"])
                .output()
                .ok()
                .map(|o| {
                    String::from_utf8_lossy(&o.stdout)
                        .lines()
                        .map(|s| s.to_string())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let staged_paths: Vec<std::path::PathBuf> =
                staged_list.iter().map(std::path::PathBuf::from).collect();
            let max_subject = cfg
                .commit
                .as_ref()
                .map(|c| c.max_subject)
                .unwrap_or(72usize);
            let template_body = match commit_body_template {
                Some(s) => Some(s),
                None => cfg
                    .commit
                    .as_ref()
                    .and_then(|c| c.template_body.as_ref())
                    .and_then(|p| std::fs::read_to_string(p).ok()),
            };
            // scope alias mapping
            let scopes_alias = cfg.commit.as_ref().map(|c| c.scopes_alias.clone());
            let input = crate::commit_msg::MsgInput {
                staged_paths,
                diff_summary: None,
                forced_type: commit_type.clone(),
                forced_scope: commit_scope.clone(),
                max_subject,
                template_body,
                scopes_alias,
            };
            let mut msg = crate::commit_msg::generate_struct(&input)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
            // Optional LLM subject synthesis (2s timeout; fallback heuristic)
            if msg.subject.trim().is_empty() || msg.subject.len() < 12 {
                let ns = git::numstat(patch).unwrap_or_default();
                let files = ns.len();
                let added: u64 = ns.iter().map(|e| e.added).sum();
                let deleted: u64 = ns.iter().map(|e| e.deleted).sum();
                let summary_llm = format!("{} file(s), +{}, -{}", files, added, deleted);
                let diff_head = patch.lines().take(120).collect::<Vec<_>>().join("\n");
                let agent = devit_agent::Agent::new(cfg.clone());
                let fut = agent.commit_message("", &summary_llm, &diff_head);
                if let Ok(Ok(s)) = tokio::runtime::Handle::current().block_on(async {
                    tokio::time::timeout(std::time::Duration::from_secs(2), fut).await
                }) {
                    if !s.trim().is_empty() {
                        msg.subject = s.trim().to_string();
                    }
                }
            }
            // provenance footer
            if cfg.provenance.footer && !no_prov_footer {
                let hash = compute_attest_hash(patch);
                msg.footers.push(format!("DevIt-Attest: {}", hash));
                let _ = journal_event(&Event::Attest { hash });
            }
            let msg_path = ".git/COMMIT_EDITMSG";
            // build commit message text
            let subject_line = if let Some(sc) = &msg.scope {
                format!("{}({}): {}", msg.ctype, sc, msg.subject)
            } else {
                format!("{}: {}", msg.ctype, msg.subject)
            };
            let body = msg.body.clone();
            let foot = if msg.footers.is_empty() {
                String::new()
            } else {
                format!("\n{}", msg.footers.join("\n"))
            };
            let full = if body.trim().is_empty() {
                format!("{}{}\n", subject_line, foot)
            } else {
                format!("{}\n\n{}{}\n", subject_line, body.trim(), foot)
            };
            if commit_dry_run || !commit_enabled {
                // write only if not dry-run? Spec: dry-run should not touch git; off should write.
                if !commit_dry_run {
                    let _ = std::fs::write(msg_path, &full);
                }
                // Write commit_meta.json for PR summary enrichment
                let _ = std::fs::create_dir_all(".devit/reports");
                let meta = serde_json::json!({
                    "subject": msg.subject,
                    "type": msg.ctype,
                    "scope": msg.scope,
                    "committed": false,
                    "sha": serde_json::Value::Null
                });
                let _ = std::fs::write(
                    ".devit/reports/commit_meta.json",
                    serde_json::to_vec(&meta).unwrap_or_default(),
                );
                return Ok(serde_json::json!({
                    "ok": true,
                    "committed": false,
                    "type": msg.ctype,
                    "scope": msg.scope,
                    "subject": msg.subject,
                    "msg_path": msg_path
                }));
            }
            // approval for commit step (safe requires --yes)
            if profile == "safe" && !yes {
                anyhow::bail!(format!(
                    "{}",
                    serde_json::json!({
                        "approval_required": true, "policy": "on_request", "phase": "pre", "reason": "commit"
                    })
                ));
            }
            // write message file
            std::fs::write(msg_path, &full)
                .map_err(|_| anyhow::anyhow!("commit_msg_failed: write_failed"))?;
            // git commit
            let mut cmd = std::process::Command::new("git");
            cmd.args(["commit", "-F", msg_path]);
            if commit_signoff {
                cmd.arg("--signoff");
            }
            let out = cmd.output().map_err(|e| anyhow::anyhow!(e))?;
            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                anyhow::bail!(format!(
                    "{}",
                    serde_json::json!({
                        "git_commit_failed": true, "exit_code": out.status.code().unwrap_or(1), "stderr": stderr
                    })
                ));
            }
            let sha = git::head_short().unwrap_or_default();
            // Write commit_meta.json reflecting committed SHA
            let _ = std::fs::create_dir_all(".devit/reports");
            let meta = serde_json::json!({
                "subject": msg.subject,
                "type": msg.ctype,
                "scope": msg.scope,
                "committed": true,
                "sha": sha
            });
            let _ = std::fs::write(
                ".devit/reports/commit_meta.json",
                serde_json::to_vec(&meta).unwrap_or_default(),
            );
            Ok(serde_json::json!({
                "ok": true,
                "committed": true,
                "commit_sha": sha,
                "type": msg.ctype,
                "scope": msg.scope,
                "subject": msg.subject,
                "msg_path": msg_path
            }))
        }
        "shell_exec" => {
            let cmd = args.get("cmd").and_then(|v| v.as_str()).unwrap_or("");
            if cmd.is_empty() {
                anyhow::bail!("shell_exec: 'cmd' field is required");
            }
            let ask = requires_approval_tool(&cfg.policy, "shell", yes, "exec");
            if ask && !ask_approval()? {
                anyhow::bail!("Cancelled by user.");
            }
            #[cfg(feature = "sandbox")]
            let (code, out) = sandbox::run_shell_sandboxed_capture(cmd, &cfg.policy, &cfg.sandbox)?;
            #[cfg(not(feature = "sandbox"))]
            let (code, out) = (1, "Sandbox feature not enabled".to_string());
            // provenance: attest shell_exec call (tool+args+ts)
            if let Ok(hash) = compute_call_attest("shell_exec", &args) {
                let _ = journal_event(&Event::Attest { hash });
            }
            Ok(serde_json::json!({"exit_code": code, "output": out}))
        }
        _ => anyhow::bail!(format!("unknown tool: {name}")),
    }
}

fn tool_call_legacy(
    cfg: &Config,
    name: &str,
    input: &str,
    yes: bool,
    no_precommit: bool,
    precommit_only: bool,
    json_only: bool,
) -> Result<()> {
    if json_only {
        anyhow::bail!("--json-only requires '-' as tool name with JSON stdin");
    }
    match name {
        "fs_patch_apply" => {
            ensure_git_repo()?;
            if cfg.policy.sandbox.to_lowercase() == "read-only" {
                anyhow::bail!("policy.sandbox=read-only: apply denied (no write operations allowed)");
            }
            let patch = read_patch(input)?;
            if precommit_only {
                match precommit::run(cfg) {
                    Ok(()) => {
                        println!("precommit_ok: true");
                        return Ok(());
                    }
                    Err(f) => anyhow::bail!(format!(
                        "{}",
                        serde_json::json!({
                            "precommit_failed": true, "tool": f.tool, "exit_code": f.exit_code, "stderr": f.stderr
                        })
                    )),
                }
            }
            if no_precommit {
                if !yes || !precommit::bypass_allowed(cfg) {
                    anyhow::bail!(format!(
                        "{}",
                        serde_json::json!({
                            "approval_required": true, "policy": "on_request", "phase": "pre", "reason": "precommit_bypass"
                        })
                    ));
                }
            } else if let Err(f) = precommit::run(cfg) {
                anyhow::bail!(format!(
                    "{}",
                    serde_json::json!({
                        "precommit_failed": true, "tool": f.tool, "exit_code": f.exit_code, "stderr": f.stderr
                    })
                ));
            }
            git::apply_check(&patch)?;
            let ask = requires_approval_tool(&cfg.policy, "git", yes, "write");
            if ask && !ask_approval()? {
                anyhow::bail!("Cancelled by user.");
            }
            if !git::apply_index(&patch)? {
                anyhow::bail!("git apply --index failed (patch-only).");
            }
            // run impacted tests (auto on for non-danger profiles)
            let profile = cfg
                .policy
                .profile
                .clone()
                .unwrap_or_else(|| "std".into())
                .to_lowercase();
            if profile != "danger" {
                let ns = git::numstat(&patch).unwrap_or_default();
                let changed: Vec<String> = ns.into_iter().map(|e| e.path).collect();
                let opts = test_runner::ImpactedOpts {
                    changed_from: None,
                    changed_paths: Some(changed),
                    max_jobs: None,
                    framework: Some("auto".into()),
                    timeout_secs: Some(300),
                };
                if let Ok(rep) = test_runner::run_impacted(&opts) {
                    if rep.failed > 0 {
                        anyhow::bail!(format!(
                            "{}",
                            serde_json::json!({"tests_failed": true, "report": ".devit/reports/junit.xml"})
                        ));
                    }
                }
            }
            let attest = compute_attest_hash(&patch);
            journal_event(&Event::Attest { hash: attest })?;
            println!("ok: patch applied to index (no commit)");
            Ok(())
        }
        "shell_exec" => {
            let ask = requires_approval_tool(&cfg.policy, "shell", yes, "exec");
            if ask && !ask_approval()? {
                anyhow::bail!("Cancelled by user.");
            }
            let cmd = if input == "-" {
                anyhow::bail!("shell_exec requires a command string as input");
            } else {
                input.to_string()
            };
            #[cfg(feature = "sandbox")]
            let code = sandbox::run_shell_sandboxed(&cmd, &cfg.policy, &cfg.sandbox)?;
            #[cfg(not(feature = "sandbox"))]
            let code = 1;
            if code != 0 {
                anyhow::bail!(format!("shell_exec exit code {code}"));
            }
            // provenance: attest shell_exec legacy call
            if let Ok(hash) = compute_call_attest("shell_exec", &serde_json::json!({"cmd": cmd})) {
                let _ = journal_event(&Event::Attest { hash });
            }
            Ok(())
        }
        _ => anyhow::bail!(format!("unknown tool: {name}")),
    }
}

// Nouveaux handlers Core Engine

async fn handle_suggest(
    goal: String,
    use_mcp: bool,
    _json_only: bool,
    llm_config: LlmConfig,
) -> StdResponse<String> {
    use chrono::Utc;
    use uuid::Uuid;

    let request_id = Uuid::new_v4();
    let timestamp = Utc::now();

    // Pour MVP, ignore use_mcp pour l'instant
    if use_mcp {
        tracing::warn!("MCP integration not yet implemented, using direct LLM");
    }

    // Résoudre la configuration LLM avec priorités : CLI > env > TOML > défauts
    let resolved_config = resolve_llm_config(&llm_config).await;

    // Charger la configuration
    let cfg = match load_cfg("devit.toml") {
        Ok(cfg) => cfg,
        Err(e) => {
            let error = StdError::new(
                "E_CONFIG_LOAD".to_string(),
                format!("Failed to load config: {}", e),
            )
            .with_hint("Check devit.toml exists and is valid".to_string());

            return StdResponse {
                success: false,
                timestamp,
                request_id: Some(request_id),
                error: Some(error),
                data: None,
            };
        }
    };

    // Collecter le contexte du workspace
    let context = match collect_context(".") {
        Ok(ctx) => ctx,
        Err(e) => {
            let error = StdError::new(
                "E_CONTEXT_COLLECT".to_string(),
                format!("Failed to collect context: {}", e),
            )
            .with_hint("Ensure you're in a valid project directory".to_string());

            return StdResponse {
                success: false,
                timestamp,
                request_id: Some(request_id),
                error: Some(error),
                data: None,
            };
        }
    };

    // Créer l'agent et générer le patch
    let agent = Agent::new(cfg);
    match agent.suggest_patch(&goal, &context).await {
        Ok(patch) => {
            let summary = if _json_only {
                // JSON output: inclure llm.backend, llm.model
                serde_json::json!({
                    "patch": patch,
                    "llm": {
                        "backend": resolved_config.backend,
                        "model": resolved_config.model,
                        "endpoint": resolved_config.endpoint,
                        "timeout_s": resolved_config.timeout_s
                    },
                    "goal": goal
                })
                .to_string()
            } else {
                // Pretty output: afficher résumé (backend, modèle, timeout)
                format!(
                    "🤖 LLM: {} ({})\n📡 Endpoint: {}\n⏱️ Timeout: {}s\n🎯 Goal: {}\n\n{}",
                    resolved_config.backend,
                    resolved_config.model,
                    resolved_config.endpoint,
                    resolved_config.timeout_s,
                    goal,
                    patch
                )
            };

            StdResponse {
                success: true,
                timestamp,
                request_id: Some(request_id),
                error: None,
                data: Some(summary),
            }
        }
        Err(e) => {
            let details = json!({
                "goal": goal,
                "backend": resolved_config.backend,
                "model": resolved_config.model
            });
            let error = StdError::new(
                "E_SUGGEST_FAILED".to_string(),
                format!("Failed to generate patch: {}", e),
            )
            .with_hint("Check your LLM configuration and connectivity".to_string())
            .with_details(details);

            StdResponse {
                success: false,
                timestamp,
                request_id: Some(request_id),
                error: Some(error),
                data: None,
            }
        }
    }
}

async fn handle_apply(
    patch_file: PathBuf,
    approval: Option<String>,
    sandbox: Option<String>,
    dry_run: bool,
    use_json_output: bool,
) -> StdResponse<String> {
    use chrono::Utc;
    use uuid::Uuid;

    let request_id = Uuid::new_v4();
    let timestamp = Utc::now();

    let patch_content = match fs::read_to_string(&patch_file) {
        Ok(content) => content,
        Err(err) => {
            let error = StdError::new(
                "E_IO".to_string(),
                format!(
                    "Failed to read patch file {}: {}",
                    patch_file.display(),
                    err
                ),
            )
            .with_hint("Verify that the path exists and the file is readable".to_string())
            .with_details(serde_json::Value::String(err.to_string()));

            return StdResponse {
                success: false,
                timestamp,
                request_id: Some(request_id),
                error: Some(error),
                data: None,
            };
        }
    };

    let mut config = load_core_config_with_env();
    let mut active_sandbox_profile = config.sandbox.default_profile.clone();

    if let Some(raw_profile) = sandbox.as_deref() {
        match parse_sandbox_profile_cli(raw_profile) {
            Some(profile) => {
                active_sandbox_profile = profile.clone();
                config.sandbox.default_profile = profile.clone();
                config.policy.sandbox_profile_default = profile;
            }
            None => {
                let error = StdError::new(
                    "E_VALIDATION".to_string(),
                    format!("Invalid sandbox profile '{}'", raw_profile),
                )
                .with_hint("Use 'strict' or 'permissive'".to_string());

                return StdResponse {
                    success: false,
                    timestamp,
                    request_id: Some(request_id),
                    error: Some(error),
                    data: None,
                };
            }
        }
    }

    let effective_approval = match approval.as_deref() {
        Some(raw) => match parse_approval_level_cli(raw) {
            Some(level) => level,
            None => {
                let error = StdError::new(
                    "E_VALIDATION".to_string(),
                    format!("Invalid approval level '{}'", raw),
                )
                .with_hint("Use one of: untrusted, ask, moderate, trusted".to_string());

                return StdResponse {
                    success: false,
                    timestamp,
                    request_id: Some(request_id),
                    error: Some(error),
                    data: None,
                };
            }
        },
        None => config.policy.default_approval_level.clone(),
    };

    let engine = match CoreEngine::new(config).await {
        Ok(engine) => engine,
        Err(err) => {
            return StdResponse {
                success: false,
                timestamp,
                request_id: Some(request_id),
                error: Some(std_error_from_core(err)),
                data: None,
            }
        }
    };

    let patch_result = match engine
        .patch_apply(&patch_content, effective_approval.clone(), dry_run, None)
        .await
    {
        Ok(result) => result,
        Err(err) => {
            return StdResponse {
                success: false,
                timestamp,
                request_id: Some(request_id),
                error: Some(std_error_from_core(err)),
                data: None,
            }
        }
    };

    let approval_label = approval_level_label(&effective_approval).to_string();
    let sandbox_label = sandbox_profile_label(&active_sandbox_profile).to_string();
    let modified_files: Vec<String> = patch_result
        .modified_files
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    let snapshot_str = patch_result
        .resulting_snapshot
        .as_ref()
        .map(|id| id.to_string());
    let warnings = patch_result.warnings.clone();
    let info_messages = patch_result.info_messages.clone();
    let commit_sha = patch_result.commit_sha.clone();
    let rollback_cmd = patch_result.rollback_cmd.clone();
    let test_results = patch_result.test_results.clone();
    let reverted_sha = patch_result.reverted_sha.clone();
    let execution_time = patch_result.execution_time;

    let human_output = if use_json_output {
        String::new()
    } else {
        let mut lines = Vec::new();
        if patch_result.success {
            if dry_run {
                lines
                    .push("🔍 Dry run completed successfully — no filesystem changes.".to_string());
            } else {
                lines.push("✅ Patch applied successfully.".to_string());
            }
        } else {
            lines.push("⚠️ Patch completed with warnings.".to_string());
        }

        lines.push(format!("Patch file: {}", patch_file.display()));
        lines.push(format!(
            "Approval level: {}   •   Sandbox: {}   •   Dry run: {}",
            approval_label,
            sandbox_label,
            if dry_run { "yes" } else { "no" }
        ));
        lines.push(format!(
            "Execution time: {:.2}s",
            execution_time.as_secs_f64()
        ));

        if !modified_files.is_empty() {
            lines.push(String::new());
            lines.push("Modified files:".to_string());
            for file in &modified_files {
                lines.push(format!("  • {}", file));
            }
        }

        if let Some(snapshot) = &snapshot_str {
            if !dry_run {
                lines.push(String::new());
                lines.push(format!("Snapshot captured: {}", snapshot));
            }
        }

        if let Some(commit) = &commit_sha {
            lines.push(format!("Commit created: {}", commit));
        }

        if let Some(rollback) = &rollback_cmd {
            lines.push(format!("Rollback command: {}", rollback));
        }

        if !warnings.is_empty() {
            lines.push(String::new());
            lines.push("Warnings:".to_string());
            for warning in &warnings {
                lines.push(format!("  ⚠️ {}", warning));
            }
        }

        if !info_messages.is_empty() {
            lines.push(String::new());
            lines.push("Details:".to_string());
            for info in &info_messages {
                lines.push(format!("  • {}", info));
            }
        }

        if let Some(results) = &test_results {
            if let Ok(serialized) = serde_json::to_string_pretty(results) {
                lines.push(String::new());
                lines.push("Test results:".to_string());
                lines.push(serialized);
            }
        }

        if patch_result.auto_reverted {
            lines.push(String::new());
            lines.push("Auto-revert triggered due to failures.".to_string());
            if let Some(revert) = &reverted_sha {
                lines.push(format!("Revert commit: {}", revert));
            }
        }

        lines.join("\n")
    };

    let test_results_value = test_results
        .as_ref()
        .and_then(|results| serde_json::to_value(results).ok())
        .unwrap_or(Value::Null);

    let data_content = if use_json_output {
        let payload = json!({
            "success": patch_result.success,
            "dry_run": dry_run,
            "approval_level": approval_label,
            "sandbox_profile": sandbox_label,
            "warnings": warnings,
            "info_messages": info_messages,
            "modified_files": modified_files,
            "snapshot": snapshot_str,
            "execution_time_ms": execution_time.as_millis(),
            "required_elevation": patch_result.required_elevation,
            "commit_sha": commit_sha,
            "rollback_cmd": rollback_cmd,
            "test_results": test_results_value,
            "auto_reverted": patch_result.auto_reverted,
            "reverted_sha": reverted_sha,
            "patch_file": patch_file.display().to_string(),
        });
        serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string())
    } else {
        human_output
    };

    StdResponse {
        success: patch_result.success,
        timestamp,
        request_id: Some(request_id),
        error: None,
        data: Some(data_content),
    }
}

fn parse_approval_level_cli(raw: &str) -> Option<ApprovalLevel> {
    match raw.to_ascii_lowercase().as_str() {
        "untrusted" => Some(ApprovalLevel::Untrusted),
        "ask" => Some(ApprovalLevel::Ask),
        "moderate" => Some(ApprovalLevel::Moderate),
        "trusted" => Some(ApprovalLevel::Trusted),
        _ => None,
    }
}

fn parse_sandbox_profile_cli(raw: &str) -> Option<SandboxProfile> {
    match raw.to_ascii_lowercase().as_str() {
        "strict" => Some(SandboxProfile::Strict),
        "permissive" => Some(SandboxProfile::Permissive),
        _ => None,
    }
}

fn approval_level_label(level: &ApprovalLevel) -> &'static str {
    match level {
        ApprovalLevel::Untrusted => "untrusted",
        ApprovalLevel::Ask => "ask",
        ApprovalLevel::Moderate => "moderate",
        ApprovalLevel::Trusted => "trusted",
        ApprovalLevel::Privileged { .. } => "privileged",
    }
}

fn sandbox_profile_label(profile: &SandboxProfile) -> &'static str {
    match profile {
        SandboxProfile::Strict => "strict",
        SandboxProfile::Permissive => "permissive",
    }
}

fn std_error_from_core(err: DevItError) -> StdError {
    let hint = err.recovery_hints().into_iter().next();
    let mut error = StdError::new(err.error_code().to_string(), err.to_string());
    if let Some(h) = hint {
        error = error.with_hint(h);
    }
    error.with_details(serde_json::Value::String(format!("{:?}", err)))
}

async fn handle_run(goal: String, use_mcp: bool, _json_only: bool) -> StdResponse<String> {
    use chrono::Utc;
    use std::fs;
    use uuid::Uuid;

    let request_id = Uuid::new_v4();
    let timestamp = Utc::now();

    // Pour MVP, ignore use_mcp pour l'instant
    if use_mcp {
        tracing::warn!("MCP integration not yet implemented, using direct LLM");
    }

    // 1. SUGGEST: Générer le patch
    tracing::info!("Step 1/3: Generating patch for goal: {}", goal);

    let cfg = match load_cfg("devit.toml") {
        Ok(cfg) => cfg,
        Err(e) => {
            let error = StdError::new(
                "E_CONFIG_LOAD".to_string(),
                format!("Failed to load config: {}", e),
            )
            .with_hint("Check devit.toml exists and is valid".to_string());

            return StdResponse {
                success: false,
                timestamp,
                request_id: Some(request_id),
                error: Some(error),
                data: None,
            };
        }
    };

    let context = match collect_context(".") {
        Ok(ctx) => ctx,
        Err(e) => {
            let error = StdError::new(
                "E_CONTEXT_COLLECT".to_string(),
                format!("Failed to collect context: {}", e),
            )
            .with_hint("Ensure you're in a valid project directory".to_string());

            return StdResponse {
                success: false,
                timestamp,
                request_id: Some(request_id),
                error: Some(error),
                data: None,
            };
        }
    };

    let agent = Agent::new(cfg.clone());
    let patch = match agent.suggest_patch(&goal, &context).await {
        Ok(patch) => patch,
        Err(e) => {
            let error = StdError::new(
                "E_SUGGEST_FAILED".to_string(),
                format!("Failed to generate patch: {}", e),
            )
            .with_hint("Check your LLM configuration and connectivity".to_string())
            .with_details(json!({ "goal": goal }));

            return StdResponse {
                success: false,
                timestamp,
                request_id: Some(request_id),
                error: Some(error),
                data: None,
            };
        }
    };

    // Sauvegarder le patch généré
    let patch_file = format!(
        ".devit/run_{}.patch",
        request_id.to_string().replace('-', "")[..8].to_string()
    );
    if let Err(e) = fs::write(&patch_file, &patch) {
        let error = StdError::new(
            "E_PATCH_SAVE".to_string(),
            format!("Failed to save patch: {}", e),
        )
        .with_hint("Check filesystem permissions".to_string());

        return StdResponse {
            success: false,
            timestamp,
            request_id: Some(request_id),
            error: Some(error),
            data: None,
        };
    };

    // 2. APPLY: Appliquer le patch
    tracing::info!("Step 2/3: Applying patch");

    if !git::is_git_available() || !git::in_repo() {
        let error = StdError::new(
            "E_GIT_REQUIRED".to_string(),
            "Git repository required for apply operation".to_string(),
        )
        .with_hint("Initialize git repository with 'git init'".to_string());

        return StdResponse {
            success: false,
            timestamp,
            request_id: Some(request_id),
            error: Some(error),
            data: None,
        };
    };

    if let Err(e) = git::apply_check(&patch) {
        let error = StdError::new(
            "E_PATCH_INVALID".to_string(),
            format!("Generated patch is invalid: {}", e),
        )
        .with_hint("Try refining your goal or check project structure".to_string())
        .with_details(serde_json::Value::String(format!(
            "Patch saved to: {}",
            patch_file
        )));

        return StdResponse {
            success: false,
            timestamp,
            request_id: Some(request_id),
            error: Some(error),
            data: None,
        };
    };

    // Appliquer à l'index (patch-only mode)
    if let Ok(false) | Err(_) = git::apply_index(&patch) {
        let error = StdError::new(
            "E_APPLY_FAILED".to_string(),
            "Failed to apply patch to index".to_string(),
        )
        .with_hint("Check for conflicts or invalid patch content".to_string())
        .with_details(serde_json::Value::String(format!(
            "Patch saved to: {}",
            patch_file
        )));

        return StdResponse {
            success: false,
            timestamp,
            request_id: Some(request_id),
            error: Some(error),
            data: None,
        };
    };

    // 3. TEST: Exécuter les tests (optionnel, basique pour MVP)
    tracing::info!("Step 3/3: Running tests");

    let test_result = if std::path::Path::new("Cargo.toml").exists() {
        // Projet Rust - exécuter cargo test basique
        let output = std::process::Command::new("cargo")
            .args(["test", "--no-fail-fast"])
            .output();

        match output {
            Ok(out) => {
                if out.status.success() {
                    format!("✅ Tests passed (cargo test)")
                } else {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    format!(
                        "⚠️ Some tests failed:\n{}",
                        stderr.lines().take(5).collect::<Vec<_>>().join("\n")
                    )
                }
            }
            Err(e) => format!("⚠️ Could not run tests: {}", e),
        }
    } else {
        "ℹ️ No test framework detected - skipping tests".to_string()
    };

    StdResponse {
        success: true,
        timestamp,
        request_id: Some(request_id),
        error: None,
        data: Some(format!(
            "🎯 Goal: {}\n✅ Patch generated and applied to index\n📄 Patch saved: {}\n🧪 {}\n\n💡 Use 'git diff --cached' to review changes, then 'git commit' to save",
            goal, patch_file, test_result
        )),
    }
}

async fn handle_test(
    stack: Option<String>,
    _cmd: Option<String>,
    timeout: Option<u64>,
    _json_only: bool,
) -> StdResponse<String> {
    use chrono::Utc;
    use uuid::Uuid;

    let request_id = Uuid::new_v4();
    let timestamp = Utc::now();

    // Detect stack if not provided
    let detected_stack = match stack.as_deref() {
        Some("auto") | None => detect_test_stack(),
        Some(s) => s.to_string(),
    };

    let timeout_secs = timeout.unwrap_or(60);

    // Execute based on detected or specified stack
    let result = match detected_stack.as_str() {
        "cargo" => run_cargo_tests(timeout_secs).await,
        "npm" => run_npm_tests(timeout_secs).await,
        "pytest" => run_pytest_tests(timeout_secs).await,
        "none" => Ok("ℹ️ No test framework detected".to_string()),
        _ => Err(format!("Unknown test stack: {}", detected_stack)),
    };

    match result {
        Ok(output) => StdResponse {
            success: true,
            timestamp,
            request_id: Some(request_id),
            error: None,
            data: Some(format!(
                "🧪 Test Framework: {}\n⏱️ Timeout: {}s\n\n{}",
                detected_stack, timeout_secs, output
            )),
        },
        Err(e) => {
            let error = StdError::new(
                "E_TEST_FAILED".to_string(),
                format!("Test execution failed: {}", e),
            )
            .with_hint("Check test configuration and dependencies".to_string())
            .with_details(json!({
                "stack": detected_stack,
                "timeout_secs": timeout_secs
            }));

            StdResponse {
                success: false,
                timestamp,
                request_id: Some(request_id),
                error: Some(error),
                data: None,
            }
        }
    }
}

async fn handle_orchestration_delegate(
    goal: String,
    delegated_to: String,
    model: Option<String>,
    timeout: Option<u64>,
    watch: Vec<String>,
    context: Option<String>,
    working_dir: Option<String>,
    format: Option<String>,
) -> Result<Value> {
    let config = load_core_config_with_env();
    let default_timeout = config.orchestration.base.default_timeout_secs;
    let mode = config.orchestration.base.mode;

    let watch_for_output = watch.clone();
    let watch_patterns = if watch_for_output.is_empty() {
        None
    } else {
        Some(watch_for_output.clone())
    };

    let model = model
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty());

    let context_value = match context {
        Some(raw) => Some(
            serde_json::from_str(&raw)
                .with_context(|| format!("invalid JSON for --context: {}", raw))?,
        ),
        None => None,
    };
    let context_for_output = context_value.clone();

    let timeout_duration = timeout.map(Duration::from_secs);
    let timeout_secs = timeout.unwrap_or(default_timeout);

    let engine = CoreEngine::new(config)
        .await
        .map_err(|err| anyhow::anyhow!("failed to initialize orchestration core: {}", err))?;

    if let Ok(socket) = std::env::var("DEVIT_DAEMON_SOCKET") {
        if !engine.orchestration_uses_daemon().await {
            anyhow::bail!(
                "devitd daemon not available at {} (set DEVIT_ORCHESTRATION_MODE=local to work offline)",
                socket
            );
        }
    }

    let resolved_relative = if let Some(ref dir) = working_dir {
        Some(
            engine
                .workspace_resolve_relative(dir)
                .await
                .map_err(|err| anyhow::anyhow!(err.to_string()))?,
        )
    } else {
        let current = engine
            .workspace_current_relative()
            .await
            .map_err(|err| anyhow::anyhow!(err.to_string()))?;
        if current.components().next().is_none() || current == PathBuf::from(".") {
            None
        } else {
            Some(current)
        }
    };

    let working_dir_str = resolved_relative.as_ref().map(|p| {
        if p.components().next().is_none() {
            ".".to_string()
        } else {
            p.to_string_lossy().replace('\\', "/")
        }
    });

    let sanitized_format = match format.as_deref() {
        Some("compact") => Some(String::from("compact")),
        Some("default") => None,
        Some(other) => {
            anyhow::bail!(
                "Invalid format '{}'. Supported values: compact, default",
                other
            );
        }
        None => None,
    };

    let task_id = engine
        .orchestration_delegate(
            goal.clone(),
            delegated_to.clone(),
            model.clone(),
            timeout_duration,
            watch_patterns,
            context_value,
            working_dir_str.clone(),
            sanitized_format.clone(),
        )
        .await
        .map_err(|err| anyhow::anyhow!("delegation failed: {}", err))?;

    let watch_field = if watch_for_output.is_empty() {
        Value::Null
    } else {
        serde_json::to_value(&watch_for_output).unwrap_or(Value::Null)
    };
    let context_field = context_for_output.unwrap_or(Value::Null);
    let mode_field = match mode {
        OrchestrationMode::Local => "local",
        OrchestrationMode::Daemon => "daemon",
        OrchestrationMode::Auto => "auto",
    };
    let working_dir_field = working_dir_str
        .as_ref()
        .map(|s| Value::String(s.clone()))
        .unwrap_or(Value::Null);
    let format_field = sanitized_format
        .as_ref()
        .map(|s| Value::String(s.clone()))
        .unwrap_or(Value::Null);
    let model_label = model.clone().unwrap_or_else(|| "<default>".to_string());
    let model_field = Value::String(model_label.clone());

    Ok(serde_json::json!({
        "task_id": task_id,
        "goal": goal,
        "delegated_to": delegated_to,
        "model": model_field,
        "timeout_secs": timeout_secs,
        "mode": mode_field,
        "watch_patterns": watch_field,
        "context": context_field,
        "working_dir": working_dir_field,
        "format": format_field,
    }))
}

async fn handle_orchestration_status(
    filter: Option<String>,
    format: Option<String>,
    use_json_output: bool,
) -> Result<()> {
    let config = load_core_config_with_env();
    let format_choice = format
        .or_else(|| {
            if use_json_output {
                Some("json".to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "compact".to_string());

    let output_format = match format_choice.to_lowercase().as_str() {
        "json" => OutputFormat::Json,
        "table" => OutputFormat::Table,
        "compact" => OutputFormat::Compact,
        other => {
            tracing::warn!("Unknown format '{}', falling back to compact", other);
            OutputFormat::Compact
        }
    };

    let engine = CoreEngine::new(config)
        .await
        .map_err(|err| anyhow::anyhow!("failed to initialize orchestration core: {}", err))?;

    if let Ok(socket) = std::env::var("DEVIT_DAEMON_SOCKET") {
        if !engine.orchestration_uses_daemon().await {
            anyhow::bail!(
                "devitd daemon not available at {} (set DEVIT_ORCHESTRATION_MODE=local to work offline)",
                socket
            );
        }
    }

    let status = engine
        .orchestration_status(&output_format, filter.clone())
        .await
        .map_err(|err| anyhow::anyhow!("failed to fetch orchestration status: {}", err))?;

    if matches!(output_format, OutputFormat::Json) {
        match serde_json::from_str::<Value>(&status) {
            Ok(parsed) => {
                print_orchestration_payload(&parsed, true, |_| String::new())?;
            }
            Err(err) => {
                tracing::warn!("Invalid JSON status payload: {}", err);
                println!("{}", status);
            }
        }
    } else if use_json_output {
        let payload = serde_json::json!({
            "format": match output_format {
                OutputFormat::Table => "table",
                OutputFormat::Compact => "compact",
                OutputFormat::MessagePack => "messagepack",
                OutputFormat::Json => "json",
            },
            "filter": filter,
            "output": status,
        });
        print_orchestration_payload(&payload, true, |_| String::new())?;
    } else {
        println!("{}", status);
    }

    Ok(())
}

fn handle_workspace_init(
    sandbox: Option<String>,
    allow: Vec<String>,
    default_project: Option<String>,
) -> Result<()> {
    use toml::Value as TomlValue;

    let config_path =
        std::env::var("DEVIT_CORE_CONFIG").unwrap_or_else(|_| "devit.core.toml".to_string());
    let mut doc = if std::path::Path::new(&config_path).exists() {
        let contents = fs::read_to_string(&config_path)
            .with_context(|| format!("failed to read config at {}", config_path))?;
        toml::from_str::<TomlValue>(&contents)
            .with_context(|| format!("failed to parse TOML at {}", config_path))?
    } else {
        TomlValue::Table(toml::map::Map::new())
    };

    let table = doc
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("configuration root must be a TOML table"))?;

    let workspace_entry = table
        .entry("workspace".to_string())
        .or_insert_with(|| TomlValue::Table(toml::map::Map::new()));
    let workspace_table = workspace_entry
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("[workspace] must be a table"))?;

    let sandbox_value = if let Some(root) = sandbox {
        workspace_table.insert("sandbox_root".to_string(), TomlValue::String(root.clone()));
        root
    } else {
        workspace_table
            .get("sandbox_root")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                anyhow::anyhow!("--sandbox is required when no sandbox_root is configured")
            })?
            .to_string()
    };

    if !allow.is_empty() {
        workspace_table.insert(
            "allowed_projects".to_string(),
            TomlValue::Array(allow.iter().map(|p| TomlValue::String(p.clone())).collect()),
        );
    }

    if let Some(default) = default_project {
        workspace_table.insert("default_project".to_string(), TomlValue::String(default));
    }

    let serialized = toml::to_string_pretty(&doc)?;
    fs::write(&config_path, serialized)
        .with_context(|| format!("failed to write config at {}", config_path))?;

    println!(
        "Workspace sandbox configured at {} (config: {})",
        sandbox_value, config_path
    );
    Ok(())
}

async fn handle_workspace_cd(path: String) -> Result<PathBuf> {
    let config = load_core_config_with_env();
    let engine = CoreEngine::new(config)
        .await
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let relative = engine
        .workspace_resolve_relative(&path)
        .await
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let absolute = engine
        .workspace_resolve_path(&path)
        .await
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;

    let relative_opt = if relative.components().next().is_none() {
        None
    } else {
        Some(relative.to_string_lossy().replace('\\', "/"))
    };

    let config_path =
        std::env::var("DEVIT_CORE_CONFIG").unwrap_or_else(|_| "devit.core.toml".to_string());
    let mut doc = if std::path::Path::new(&config_path).exists() {
        let contents = fs::read_to_string(&config_path)
            .with_context(|| format!("failed to read config at {}", config_path))?;
        toml::from_str::<toml::Value>(&contents)
            .with_context(|| format!("failed to parse TOML at {}", config_path))?
    } else {
        toml::Value::Table(toml::map::Map::new())
    };

    let table = doc
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("configuration root must be a TOML table"))?;
    let workspace_entry = table
        .entry("workspace".to_string())
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    let workspace_table = workspace_entry
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("[workspace] must be a table"))?;

    match relative_opt {
        Some(rel) => {
            workspace_table.insert("default_project".to_string(), toml::Value::String(rel));
        }
        None => {
            workspace_table.remove("default_project");
        }
    }

    let serialized = toml::to_string_pretty(&doc)?;
    fs::write(&config_path, serialized)
        .with_context(|| format!("failed to write config at {}", config_path))?;

    Ok(absolute)
}

async fn handle_workspace_pwd() -> Result<PathBuf> {
    let config = load_core_config_with_env();
    let engine = CoreEngine::new(config)
        .await
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    engine
        .workspace_current_dir()
        .await
        .map_err(|err| anyhow::anyhow!(err.to_string()))
}

async fn handle_orchestration_notify(
    task_id: String,
    status: String,
    summary: String,
    details: Option<String>,
    evidence: Option<String>,
) -> Result<Value> {
    let config = load_core_config_with_env();
    let details_value = match details {
        Some(raw) => Some(
            serde_json::from_str(&raw)
                .with_context(|| format!("invalid JSON for --details: {}", raw))?,
        ),
        None => None,
    };
    let evidence_value = match evidence {
        Some(raw) => Some(
            serde_json::from_str(&raw)
                .with_context(|| format!("invalid JSON for --evidence: {}", raw))?,
        ),
        None => None,
    };

    let engine = CoreEngine::new(config)
        .await
        .map_err(|err| anyhow::anyhow!("failed to initialize orchestration core: {}", err))?;

    if let Ok(socket) = std::env::var("DEVIT_DAEMON_SOCKET") {
        if !engine.orchestration_uses_daemon().await {
            anyhow::bail!(
                "devitd daemon not available at {} (set DEVIT_ORCHESTRATION_MODE=local to work offline)",
                socket
            );
        }
    }

    engine
        .orchestration_notify(
            task_id.clone(),
            status.clone(),
            summary.clone(),
            details_value.clone(),
            evidence_value.clone(),
        )
        .await
        .map_err(|err| anyhow::anyhow!("failed to record notification: {}", err))?;

    Ok(serde_json::json!({
        "task_id": task_id,
        "status": status,
        "summary": summary,
        "details": details_value.unwrap_or(Value::Null),
        "evidence": evidence_value.unwrap_or(Value::Null),
    }))
}

async fn handle_orchestration_task(task_id: String) -> Result<Value> {
    let config = load_core_config_with_env();
    let engine = CoreEngine::new(config)
        .await
        .map_err(|err| anyhow::anyhow!("failed to initialize orchestration core: {}", err))?;

    if let Ok(socket) = std::env::var("DEVIT_DAEMON_SOCKET") {
        if !engine.orchestration_uses_daemon().await {
            anyhow::bail!(
                "devitd daemon not available at {} (set DEVIT_ORCHESTRATION_MODE=local to work offline)",
                socket
            );
        }
    }

    let status_json = engine
        .orchestration_status(&OutputFormat::Json, None)
        .await
        .map_err(|err| anyhow::anyhow!("failed to fetch orchestration status: {}", err))?;

    let status_value: Value = serde_json::from_str(&status_json)
        .with_context(|| "failed to parse orchestration status JSON")?;

    if let Some(task) = find_task_by_id(&status_value, &task_id) {
        Ok(task)
    } else {
        anyhow::bail!("Task {} not found", task_id);
    }
}

fn print_orchestration_payload<F>(
    payload: &Value,
    use_json_output: bool,
    pretty_fn: F,
) -> Result<()>
where
    F: FnOnce(&Value) -> String,
{
    if use_json_output {
        println!("{}", serde_json::to_string_pretty(payload)?);
    } else {
        println!("{}", pretty_fn(payload));
    }
    Ok(())
}

fn format_task_details(task: &Value) -> String {
    let id = task.get("id").and_then(Value::as_str).unwrap_or("-");
    let status = task.get("status").and_then(Value::as_str).unwrap_or("-");
    let goal = task.get("goal").and_then(Value::as_str).unwrap_or("-");
    let delegated = task
        .get("delegated_to")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let created_at = task
        .get("created_at")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let timeout = task
        .get("timeout_secs")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let mut lines = vec![
        format!("Task ID: {}", id),
        format!("Status: {}", status),
        format!("Delegated to: {}", delegated),
        format!("Timeout: {}s", timeout),
        format!("Goal: {}", goal),
        format!("Created at: {}", created_at),
    ];

    if let Some(dir) = task.get("working_dir").and_then(Value::as_str) {
        lines.push(format!("Working dir: {}", dir));
    }

    if let Some(fmt) = task.get("response_format").and_then(Value::as_str) {
        lines.push(format!("Format: {}", fmt));
    }

    if let Some(notifications) = task.get("notifications").and_then(Value::as_array) {
        if !notifications.is_empty() {
            lines.push("Notifications:".to_string());
            for note in notifications {
                let status = note.get("status").and_then(Value::as_str).unwrap_or("-");
                let summary = note.get("summary").and_then(Value::as_str).unwrap_or("-");
                lines.push(format!("- {}: {}", status, summary));
            }
        }
    }

    lines.join("\n")
}

fn find_task_by_id(status: &Value, task_id: &str) -> Option<Value> {
    let search = |key: &str| -> Option<Value> {
        status.get(key).and_then(Value::as_array).and_then(|tasks| {
            tasks
                .iter()
                .find(|task| task.get("id").and_then(Value::as_str) == Some(task_id))
                .cloned()
        })
    };

    search("active_tasks").or_else(|| search("completed_tasks"))
}

fn load_core_config_with_env() -> CoreConfig {
    if let Ok(path) = std::env::var("DEVIT_CORE_CONFIG") {
        match CoreConfig::from_file(&path) {
            Ok(mut cfg) => {
                apply_orchestration_env_overrides(&mut cfg);
                return cfg;
            }
            Err(err) => {
                tracing::warn!(
                    "Failed to load core configuration from {}: {}. Using defaults.",
                    path,
                    err
                );
            }
        }
    }

    match CoreConfig::from_file("devit.core.toml") {
        Ok(mut cfg) => {
            apply_orchestration_env_overrides(&mut cfg);
            cfg
        }
        Err(_) => {
            let mut cfg = CoreConfig::default();
            apply_orchestration_env_overrides(&mut cfg);
            cfg
        }
    }
}

fn apply_orchestration_env_overrides(config: &mut CoreConfig) {
    let socket_env = std::env::var("DEVIT_DAEMON_SOCKET").ok();
    if let Some(socket) = socket_env.clone() {
        config.orchestration.base.daemon_socket = Some(socket);
    }

    let mut mode_override = None;
    if let Ok(mode) = std::env::var("DEVIT_ORCHESTRATION_MODE") {
        mode_override = match mode.to_lowercase().as_str() {
            "local" => Some(OrchestrationMode::Local),
            "daemon" => Some(OrchestrationMode::Daemon),
            "auto" => Some(OrchestrationMode::Auto),
            other => {
                tracing::warn!(
                    "Unknown DEVIT_ORCHESTRATION_MODE '{}', keeping existing setting",
                    other
                );
                None
            }
        };
    } else if socket_env.is_some() {
        // When a custom daemon socket is provided, favor daemon mode to avoid
        // silently falling back to the local in-memory backend.
        mode_override = Some(OrchestrationMode::Daemon);
    }

    if let Some(mode) = mode_override {
        config.orchestration.base.mode = mode;
    }

    if let Ok(timeout) = std::env::var("DEVIT_ORCHESTRATION_TIMEOUT") {
        match timeout.parse::<u64>() {
            Ok(value) => config.orchestration.base.default_timeout_secs = value,
            Err(err) => {
                tracing::warn!("Invalid DEVIT_ORCHESTRATION_TIMEOUT '{}': {}", timeout, err)
            }
        }
    }

    if let Ok(root) = std::env::var("DEVIT_SANDBOX_ROOT") {
        config.workspace.sandbox_root = Some(PathBuf::from(root));
    }
}

async fn handle_snapshot(_json_only: bool) -> StdResponse<String> {
    // Stub - délègue au Core Engine
    use chrono::Utc;
    use uuid::Uuid;

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

fn output_response<T: serde::Serialize>(response: StdResponse<T>, use_json_output: bool) {
    if use_json_output {
        // JSON output mode (default)
        if let Ok(json) = serde_json::to_string_pretty(&response) {
            println!("{}", json);
        } else {
            eprintln!("Failed to serialize response to JSON");
            std::process::exit(1);
        }
    } else {
        // Pretty human-readable mode
        if response.success {
            if let Some(data) = response.data {
                // Try to deserialize as string first (for our enhanced messages)
                if let Ok(data_str) = serde_json::from_value::<String>(
                    serde_json::to_value(&data).unwrap_or_default(),
                ) {
                    println!("{}", data_str);
                } else {
                    // Fallback to JSON if it's not a string
                    if let Ok(json) = serde_json::to_string_pretty(&data) {
                        println!("{}", json);
                    }
                }
            } else {
                println!("✅ Operation completed successfully");
            }
        } else {
            // Error handling without stacktrace leaks
            if let Some(error) = response.error {
                eprintln!("❌ Error {}: {}", error.code, error.message);
                if let Some(hint) = error.hint {
                    eprintln!("💡 Hint: {}", hint);
                }
                if let Some(details) = error.details {
                    match details {
                        serde_json::Value::Object(map) => {
                            if let Some(additional_hint) = map.get("hint").and_then(|h| h.as_str())
                            {
                                eprintln!("💡 Additional: {}", additional_hint);
                            } else if !map.is_empty() {
                                if let Ok(pretty) = serde_json::to_string_pretty(&map) {
                                    eprintln!("ℹ️ Details: {}", pretty);
                                }
                            }
                        }
                        serde_json::Value::String(text) => {
                            eprintln!("ℹ️ Details: {}", text);
                        }
                        other => {
                            if let Ok(pretty) = serde_json::to_string_pretty(&other) {
                                eprintln!("ℹ️ Details: {}", pretty);
                            }
                        }
                    }
                }
            } else {
                eprintln!("❌ Operation failed");
            }
            std::process::exit(1);
        }
    }
}

// Stack detection functions
fn detect_test_stack() -> String {
    if std::path::Path::new("Cargo.toml").exists() {
        "cargo".to_string()
    } else if std::path::Path::new("package.json").exists() {
        "npm".to_string()
    } else if std::path::Path::new("pytest.ini").exists()
        || std::path::Path::new("pyproject.toml").exists()
        || std::path::Path::new("setup.py").exists()
    {
        "pytest".to_string()
    } else {
        "none".to_string()
    }
}

async fn run_cargo_tests(timeout_secs: u64) -> Result<String, String> {
    run_sandboxed_test("cargo test --color=never", timeout_secs, "cargo").await
}

async fn run_npm_tests(timeout_secs: u64) -> Result<String, String> {
    run_sandboxed_test("npm test", timeout_secs, "npm").await
}

async fn run_pytest_tests(timeout_secs: u64) -> Result<String, String> {
    run_sandboxed_test("pytest -v --tb=short", timeout_secs, "pytest").await
}

// Unified sandboxed test runner
async fn run_sandboxed_test(
    cmd: &str,
    timeout_secs: u64,
    framework: &str,
) -> Result<String, String> {
    use tokio::time::{timeout, Duration};

    // Load config for sandbox settings
    let cfg = match load_cfg("devit.toml") {
        Ok(cfg) => cfg,
        Err(e) => return Err(format!("Failed to load config for sandbox: {}", e)),
    };

    // Run test command in sandbox with timeout
    let result = timeout(
        Duration::from_secs(timeout_secs),
        tokio::task::spawn_blocking({
            let command = cmd.to_string();
            let policy = cfg.policy.clone();
            let sandbox_cfg = cfg.sandbox.clone();
            move || {
                #[cfg(feature = "sandbox")]
                {
                    use devit_sandbox as sandbox_mod;
                    sandbox_mod::run_shell_sandboxed_capture(&command, &policy, &sandbox_cfg)
                }
                #[cfg(not(feature = "sandbox"))]
                {
                    let _ = (&command, &policy, &sandbox_cfg);
                    Err::<(i32, String), anyhow::Error>(anyhow::anyhow!(
                        "Sandbox feature not enabled"
                    ))
                }
            }
        }),
    )
    .await;

    match result {
        Ok(Ok(Ok((exit_code, output)))) => {
            if exit_code == 0 {
                Ok(format!(
                    "✅ {} tests passed (sandboxed)\n\n{}",
                    framework, output
                ))
            } else {
                Ok(format!(
                    "⚠️ Some {} tests failed (sandboxed)\n\n{}",
                    framework,
                    output.lines().take(15).collect::<Vec<_>>().join("\n")
                ))
            }
        }
        Ok(Ok(Err(e))) => {
            // Fallback to non-sandboxed execution if binary not allowed
            if e.to_string().contains("unauthorized binary")
                || e.to_string().contains("binary not allowed")
            {
                tracing::warn!(
                    "Sandbox blocked test command, falling back to non-sandboxed execution"
                );
                run_test_fallback(cmd, timeout_secs, framework).await
            } else {
                Err(format!("Sandbox execution failed: {}", e))
            }
        }
        Ok(Err(e)) => Err(format!("Task join error: {}", e)),
        Err(_) => Err(format!(
            "{} test timed out after {}s",
            framework, timeout_secs
        )),
    }
}

// Fallback function for non-sandboxed test execution
async fn run_test_fallback(
    cmd: &str,
    timeout_secs: u64,
    framework: &str,
) -> Result<String, String> {
    use tokio::process::Command;
    use tokio::time::{timeout, Duration};

    let parts: Vec<&str> = cmd.split_whitespace().collect();
    if parts.is_empty() {
        return Err("Empty command".to_string());
    }

    let output = timeout(
        Duration::from_secs(timeout_secs),
        Command::new(parts[0]).args(&parts[1..]).output(),
    )
    .await;

    match output {
        Ok(Ok(out)) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);

            if out.status.success() {
                Ok(format!(
                    "✅ {} tests passed (non-sandboxed)\n\n{}",
                    framework, stdout
                ))
            } else {
                Ok(format!(
                    "⚠️ Some {} tests failed (non-sandboxed)\n\nSTDOUT:\n{}\nSTDERR:\n{}",
                    framework,
                    stdout.lines().take(10).collect::<Vec<_>>().join("\n"),
                    stderr.lines().take(5).collect::<Vec<_>>().join("\n")
                ))
            }
        }
        Ok(Err(e)) => Err(format!("Failed to execute {}: {}", cmd, e)),
        Err(_) => Err(format!(
            "{} test timed out after {}s",
            framework, timeout_secs
        )),
    }
}

// Configuration resolution with priorities: CLI > env > TOML > defaults
async fn resolve_llm_config(cli_config: &LlmConfig) -> ResolvedLlmConfig {
    let mut resolved = ResolvedLlmConfig {
        backend: "ollama".to_string(),                     // Default
        model: "llama3.1:8b".to_string(),                  // Default
        endpoint: "http://localhost:11434/v1".to_string(), // Default
        api_key: None,                                     // Default for Ollama
        timeout_s: 120,                                    // Default
        max_tokens: 4096,                                  // Default
    };

    // 1. Load from TOML (devit.toml [llm] section)
    if let Ok(cfg) = load_cfg("devit.toml") {
        if let Some(llm_cfg) = &cfg.llm {
            resolved.backend = llm_cfg.backend.clone();
            resolved.endpoint = llm_cfg.endpoint.clone();
            resolved.model = llm_cfg.model.clone();
            if let Some(timeout) = llm_cfg.timeout_s {
                resolved.timeout_s = timeout;
            }
            if let Some(max_tokens) = llm_cfg.max_tokens {
                resolved.max_tokens = max_tokens;
            }
        }
    }

    // 2. Override with environment variables
    if let Ok(backend) = std::env::var("DEVIT_LLM_BACKEND") {
        resolved.backend = backend;
    }
    if let Ok(model) = std::env::var("DEVIT_LLM_MODEL") {
        resolved.model = model;
    }
    if let Ok(endpoint) = std::env::var("DEVIT_LLM_ENDPOINT") {
        resolved.endpoint = endpoint;
    }
    if let Ok(api_key) = std::env::var("DEVIT_LLM_API_KEY") {
        resolved.api_key = Some(api_key);
    }

    // 3. Override with CLI flags (highest priority)
    if let Some(backend) = &cli_config.backend {
        resolved.backend = backend.clone();
        // Auto-adjust defaults based on backend
        match backend.as_str() {
            "ollama" => {
                resolved.endpoint = "http://localhost:11434/v1".to_string();
                resolved.api_key = None;
            }
            "openai" => {
                resolved.endpoint = "https://api.openai.com/v1".to_string();
            }
            "lmstudio" => {
                resolved.endpoint = "http://localhost:1234/v1".to_string();
            }
            _ => {} // Keep current endpoint
        }
    }
    if let Some(model) = &cli_config.model {
        resolved.model = model.clone();
    }
    if let Some(endpoint) = &cli_config.endpoint {
        resolved.endpoint = endpoint.clone();
    }
    if let Some(api_key) = &cli_config.api_key {
        resolved.api_key = Some(api_key.clone());
    }

    // Validation: pas d'API key requise pour Ollama
    if resolved.backend == "ollama" && resolved.api_key.is_some() {
        tracing::warn!("API key not required for Ollama backend, ignoring");
        resolved.api_key = None;
    }

    // Validation: API key requise pour OpenAI
    if resolved.backend == "openai" && resolved.api_key.is_none() {
        tracing::warn!("API key required for OpenAI backend");
    }

    resolved
}

#[derive(Debug, Clone)]
struct ResolvedLlmConfig {
    backend: String,
    model: String,
    endpoint: String,
    api_key: Option<String>,
    timeout_s: u64,
    max_tokens: u32,
}
