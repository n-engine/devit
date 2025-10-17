use std::{
    env, fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result};
use async_trait::async_trait;
use clap::{ArgAction, Parser};
use mcp_core::{McpResult, McpTool};
use mcp_server::{
    transport::{self, CliTransportOptions, Transport},
    McpServer, ToolRegistry,
};
use mcp_tools::{default_tools_with_options, ToolOptions, WorkerBridge};
use serde_json::{json, Value};
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser, Debug)]
#[command(name = "devit-mcp-server", version = env!("CARGO_PKG_VERSION"))]
struct Args {
    /// Override the sandbox/workspace root path
    #[arg(long = "working-dir", value_name = "PATH")]
    working_dir: Option<PathBuf>,

    /// Explicit path to devit core configuration (devit.core.toml)
    #[arg(long = "config", value_name = "FILE")]
    config_path: Option<PathBuf>,

    /// Optional log filter (e.g. info, debug)
    #[arg(long = "log-level", value_name = "LEVEL")]
    log_level: Option<String>,

    /// Transport backend (stdio | http | https)
    #[arg(long = "transport", value_name = "TYPE")]
    transport: Option<String>,

    /// Host binding for HTTP transport
    #[arg(long = "host", value_name = "HOST")]
    host: Option<String>,

    /// Port binding for HTTP transport
    #[arg(long = "port", value_name = "PORT")]
    port: Option<u16>,

    /// Enable Server-Sent Events for HTTP transport
    #[arg(long = "enable-sse", action = ArgAction::SetTrue)]
    enable_sse: bool,

    /// Disable Server-Sent Events for HTTP transport
    #[arg(long = "disable-sse", action = ArgAction::SetTrue)]
    disable_sse: bool,

    /// Bearer tokens provided via CLI
    #[arg(long = "auth-token", value_name = "TOKEN")]
    auth_tokens: Vec<String>,

    /// Path to JSON file containing authorized tokens
    #[arg(long = "tokens-file", value_name = "FILE")]
    tokens_file: Option<PathBuf>,

    /// Additional CORS allowed origins
    #[arg(long = "cors-origin", value_name = "ORIGIN")]
    cors_origins: Vec<String>,

    /// Run the MCP server in worker mode (connect to devitd as a worker)
    #[arg(long = "worker-mode")]
    worker_mode: bool,

    /// Unique worker identifier when worker-mode is enabled
    #[arg(long = "worker-id", requires = "worker_mode")]
    worker_id: Option<String>,

    /// Path to the devitd daemon socket when worker-mode is enabled
    #[arg(long = "daemon-socket", value_name = "PATH", requires = "worker_mode")]
    daemon_socket: Option<PathBuf>,

    /// Secret used to authenticate against devitd (worker-mode)
    #[arg(long = "secret", value_name = "SECRET", requires = "worker_mode")]
    secret: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    init_tracing(&args);
    tracing::info!(
        "{}",
        devit_build_info::formatted_banner("devit-mcp-server", SERVER_VERSION)
    );

    env::set_var("NO_COLOR", "1");
    env::set_var("CLICOLOR", "0");
    env::set_var("CLICOLOR_FORCE", "0");

    if let Some(config) = &args.config_path {
        env::set_var("DEVIT_CORE_CONFIG", config);
    }

    let working_dir = match &args.working_dir {
        Some(dir) => {
            let expanded = expand_tilde(dir.clone());
            let canonical = expanded.canonicalize().with_context(|| {
                format!(
                    "Failed to canonicalize working directory override: {}",
                    expanded.display()
                )
            })?;
            canonical
        }
        None => resolve_working_dir(args.config_path.as_ref())?,
    };

    env::set_var("DEVIT_WORKDIR", &working_dir);
    env::set_current_dir(&working_dir)?;
    env::set_var("DEVIT_FORCE_ROOT", &working_dir);
    let core_config_path = env::var("DEVIT_CORE_CONFIG").ok().map(PathBuf::from);

    let worker_bridge = if args.worker_mode {
        let worker_id = args
            .worker_id
            .or_else(|| env::var("DEVIT_IDENT").ok())
            .unwrap_or_else(|| "claude_code".to_string());

        let socket = args
            .daemon_socket
            .or_else(|| env::var("DEVIT_DAEMON_SOCKET").ok().map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("/tmp/devitd.sock"));

        env::set_var("DEVIT_IDENT", &worker_id);
        env::set_var("DEVIT_DAEMON_SOCKET", &socket);
        if let Some(secret) = args.secret.as_ref() {
            env::set_var("DEVIT_SECRET", secret);
        }

        let bridge = WorkerBridge::connect(
            working_dir.clone(),
            &socket,
            worker_id.clone(),
            args.secret.clone(),
        )
        .await
        .with_context(|| format!("Failed to initialize worker bridge for {}", worker_id))?;
        Some(bridge)
    } else {
        None
    };

    let mut tool_options = ToolOptions::default();
    if let Some(bridge) = worker_bridge {
        tool_options.worker_bridge = Some(bridge);
    }

    let mut tools = default_tools_with_options(working_dir.clone(), tool_options)
        .await
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;

    if let Some(help_tool) = tools
        .iter()
        .find(|tool| tool.name() == "devit_help")
        .cloned()
    {
        let aliases = [
            (
                "devit_help_all",
                "all",
                "Static overview for all DevIt MCP tools",
            ),
            (
                "devit_file_read_help_ext",
                "devit_file_read_ext",
                "Help details for devit_file_read_ext",
            ),
        ];

        for (name, topic, description) in aliases {
            tools.push(Arc::new(HelpAlias::new(
                Arc::clone(&help_tool),
                name,
                topic,
                description,
            )));
        }
    }

    let registry = ToolRegistry::new(tools);
    let server = Arc::new(McpServer::new(registry));

    let cli_transport = CliTransportOptions {
        transport: args.transport.clone(),
        host: args.host.clone(),
        port: args.port,
        enable_sse: if args.disable_sse {
            Some(false)
        } else if args.enable_sse {
            Some(true)
        } else {
            None
        },
        tokens: args.auth_tokens.clone(),
        tokens_file: args.tokens_file.clone(),
        cors_origins: args.cors_origins.clone(),
    };

    let file_transport = transport::load_file_config(core_config_path.as_deref())?;
    let transport_mode =
        transport::determine_transport(&cli_transport, file_transport.as_ref(), &working_dir)?;

    match transport_mode {
        Transport::Stdio => {
            server.serve_stdio().await?;
        }
        Transport::Http(http_cfg) => {
            server.clone().serve_http(http_cfg).await?;
        }
    }

    Ok(())
}

fn init_tracing(args: &Args) {
    if let Some(level) = &args.log_level {
        env::set_var("RUST_LOG", level);
    }

    let builder = tracing_subscriber::fmt()
        .with_target(false)
        .with_writer(io::stderr);

    let _ = builder.try_init();
}

struct HelpAlias {
    base: Arc<dyn McpTool>,
    name: String,
    topic: String,
    description: String,
}

impl HelpAlias {
    fn new(base: Arc<dyn McpTool>, name: &str, topic: &str, description: &str) -> Self {
        Self {
            base,
            name: name.to_string(),
            topic: topic.to_string(),
            description: description.to_string(),
        }
    }
}

#[async_trait]
impl McpTool for HelpAlias {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    async fn execute(&self, _params: Value) -> McpResult<Value> {
        let params = json!({ "topic": self.topic });
        self.base.execute(params).await
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "format": {
                    "type": "string",
                    "enum": ["markdown"],
                    "description": "Output format for help content",
                    "default": "markdown"
                }
            },
            "additionalProperties": false
        })
    }
}

fn resolve_working_dir(config_path: Option<&PathBuf>) -> Result<PathBuf> {
    if let Some(from_env) = resolve_from_env(config_path)? {
        return Ok(from_env);
    }

    let current = env::current_dir()?;
    if let Some(root) = find_project_root(&current) {
        return Ok(root);
    }

    if let Ok(exe) = env::current_exe() {
        if let Some(dir) = exe.parent() {
            if let Some(root) = find_project_root(dir) {
                return Ok(root);
            }
        }
    }

    Ok(current)
}

fn resolve_from_env(config_path: Option<&PathBuf>) -> Result<Option<PathBuf>> {
    let candidates = [
        "DEVIT_WORKDIR",
        "DEVIT_SANDBOX_ROOT",
        "DEVIT_PROJECT_ROOT",
        "DEVIT_ROOT",
    ];

    for var in candidates {
        if let Ok(value) = env::var(var) {
            if value.trim().is_empty() {
                continue;
            }
            let path = expand_tilde(PathBuf::from(value));
            if path.exists() {
                return Ok(Some(path.canonicalize().unwrap_or(path)));
            }
        }
    }

    if let Some(config_root) = resolve_from_core_config(config_path)? {
        return Ok(Some(config_root));
    }

    Ok(None)
}

fn resolve_from_core_config(cli_config_path: Option<&PathBuf>) -> Result<Option<PathBuf>> {
    let config_path = if let Some(path) = cli_config_path {
        Some(path.clone())
    } else {
        env::var("DEVIT_CORE_CONFIG")
            .ok()
            .map(PathBuf::from)
            .or_else(|| {
                let cwd = env::current_dir().ok()?;
                let candidate = cwd.join("devit.core.toml");
                if candidate.exists() {
                    Some(candidate)
                } else {
                    None
                }
            })
    };

    let Some(path) = config_path else {
        return Ok(None);
    };

    if !path.exists() {
        return Ok(None);
    }

    let contents = match fs::read_to_string(&path) {
        Ok(data) => data,
        Err(_) => return Ok(None),
    };
    let parsed: toml::Value = match toml::from_str(&contents) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    let workspace = parsed
        .get("workspace")
        .and_then(|value| value.as_table())
        .and_then(|table| table.get("sandbox_root"))
        .and_then(|value| value.as_str());

    let Some(root) = workspace else {
        return Ok(None);
    };

    let expanded = expand_tilde(PathBuf::from(root));
    if expanded.exists() {
        Ok(Some(expanded.canonicalize().unwrap_or(expanded)))
    } else {
        Ok(None)
    }
}

fn expand_tilde(path: PathBuf) -> PathBuf {
    if !path.starts_with("~") {
        return path;
    }

    if path == PathBuf::from("~") {
        return env::var("HOME").map(PathBuf::from).unwrap_or(path);
    }

    if let Ok(home) = env::var("HOME") {
        if let Some(rest) = path.to_string_lossy().strip_prefix("~/") {
            let mut expanded = PathBuf::from(home);
            expanded.push(rest);
            return expanded;
        }
    }

    path
}

fn find_project_root(start: &Path) -> Option<PathBuf> {
    let mut current = Some(start);
    while let Some(dir) = current {
        if dir.join(".devit/devit.toml").exists()
            || dir.join("devit.toml").exists()
            || dir.join(".git").is_dir()
        {
            return Some(dir.to_path_buf());
        }
        current = dir.parent();
    }
    None
}
