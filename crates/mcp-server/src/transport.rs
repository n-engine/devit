use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone)]
pub enum Transport {
    Stdio,
    Http(HttpTransportConfig),
}

#[derive(Debug, Clone)]
pub struct HttpTransportConfig {
    pub host: String,
    pub port: u16,
    pub sse_enabled: bool,
    pub auth: Option<HttpAuthConfig>,
    pub cors: Option<HttpCorsConfig>,
}

#[derive(Debug, Clone)]
pub struct HttpAuthConfig {
    pub tokens: HashSet<String>,
}

#[derive(Debug, Clone)]
pub struct HttpCorsConfig {
    pub allowed_origins: Vec<String>,
}

#[derive(Debug, Default, Clone)]
pub struct CliTransportOptions {
    pub transport: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub enable_sse: Option<bool>,
    pub tokens: Vec<String>,
    pub tokens_file: Option<PathBuf>,
    pub cors_origins: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct FileTransportConfig {
    pub transport: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub sse_enabled: Option<bool>,
    pub auth: Option<FileAuthConfig>,
    pub cors: Option<FileCorsConfig>,
}

#[derive(Debug, Clone)]
pub struct FileAuthConfig {
    pub enabled: Option<bool>,
    pub auth_type: Option<String>,
    pub tokens_file: Option<PathBuf>,
    pub tokens: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct FileCorsConfig {
    pub allowed_origins: Vec<String>,
}

#[derive(Deserialize)]
struct RootConfig {
    #[serde(default)]
    mcp_server: Option<RawFileTransportConfig>,
}

#[derive(Deserialize, Default)]
struct RawFileTransportConfig {
    transport: Option<String>,
    host: Option<String>,
    port: Option<u16>,
    sse_enabled: Option<bool>,
    auth: Option<RawFileAuthConfig>,
    cors: Option<RawFileCorsConfig>,
}

#[derive(Deserialize, Default)]
struct RawFileAuthConfig {
    enabled: Option<bool>,
    #[serde(rename = "type")]
    auth_type: Option<String>,
    tokens_file: Option<String>,
    tokens: Option<Vec<String>>,
}

#[derive(Deserialize, Default)]
struct RawFileCorsConfig {
    allowed_origins: Option<Vec<String>>,
}

pub fn load_file_config(path: Option<&Path>) -> Result<Option<FileTransportConfig>> {
    let Some(path) = path else {
        return Ok(None);
    };

    if !path.exists() {
        return Ok(None);
    }

    let contents = fs::read_to_string(path)
        .with_context(|| format!("Failed to read MCP config from {}", path.display()))?;
    let parsed: RootConfig = toml::from_str(&contents)
        .with_context(|| format!("Failed to parse TOML config {}", path.display()))?;

    Ok(parsed
        .mcp_server
        .map(|raw| raw.into_runtime_config(path.parent().unwrap_or(Path::new(".")))))
}

pub fn determine_transport(
    cli: &CliTransportOptions,
    file_cfg: Option<&FileTransportConfig>,
    working_dir: &Path,
) -> Result<Transport> {
    let transport_choice = cli
        .transport
        .as_deref()
        .map(str::to_string)
        .or_else(|| file_cfg.and_then(|cfg| cfg.transport.clone()))
        .unwrap_or_else(|| "stdio".to_string());

    match transport_choice.as_str() {
        "stdio" => Ok(Transport::Stdio),
        "http" => {
            let host = cli
                .host
                .clone()
                .or_else(|| file_cfg.and_then(|cfg| cfg.host.clone()))
                .unwrap_or_else(|| "127.0.0.1".to_string());

            let port = cli
                .port
                .or_else(|| file_cfg.and_then(|cfg| cfg.port))
                .unwrap_or(3000);

            let default_sse = file_cfg.and_then(|cfg| cfg.sse_enabled).unwrap_or(true);
            let sse_enabled = cli.enable_sse.unwrap_or(default_sse);

            let auth_config = build_auth_config(cli, file_cfg, working_dir)?;
            let cors_config = build_cors_config(cli, file_cfg);

            Ok(Transport::Http(HttpTransportConfig {
                host,
                port,
                sse_enabled,
                auth: auth_config,
                cors: cors_config,
            }))
        }
        "https" => Err(anyhow!(
            "HTTPS transport is not implemented yet. Please use HTTP or stdio."
        )),
        other => Err(anyhow!("Unsupported transport '{}'", other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::tempdir;

    #[test]
    fn default_transport_is_stdio() {
        let cli = CliTransportOptions::default();
        let transport =
            determine_transport(&cli, None, Path::new(".")).expect("determine transport");
        assert!(matches!(transport, Transport::Stdio));
    }

    #[test]
    fn file_config_drives_http_transport() {
        let file_cfg = FileTransportConfig {
            transport: Some("http".to_string()),
            host: Some("0.0.0.0".to_string()),
            port: Some(4000),
            sse_enabled: Some(false),
            auth: Some(FileAuthConfig {
                enabled: Some(true),
                auth_type: Some("bearer".to_string()),
                tokens_file: None,
                tokens: vec!["abc123".to_string()],
            }),
            cors: Some(FileCorsConfig {
                allowed_origins: vec!["https://example.com".to_string()],
            }),
        };

        let cli = CliTransportOptions::default();
        let transport = determine_transport(&cli, Some(&file_cfg), Path::new("."))
            .expect("determine http transport");

        match transport {
            Transport::Http(http) => {
                assert_eq!(http.host, "0.0.0.0");
                assert_eq!(http.port, 4000);
                assert!(!http.sse_enabled);
                let auth = http.auth.expect("auth config");
                assert!(auth.tokens.contains("abc123"));
                let cors = http.cors.expect("cors config");
                assert_eq!(
                    cors.allowed_origins,
                    vec!["https://example.com".to_string()]
                );
            }
            other => panic!("expected HTTP transport, got {other:?}"),
        }
    }

    #[test]
    fn cli_overrides_host_and_port() {
        let mut cli = CliTransportOptions::default();
        cli.transport = Some("http".to_string());
        cli.host = Some("127.0.0.1".to_string());
        cli.port = Some(9000);
        cli.enable_sse = Some(false);

        let transport =
            determine_transport(&cli, None, Path::new(".")).expect("determine http transport");

        match transport {
            Transport::Http(http) => {
                assert_eq!(http.host, "127.0.0.1");
                assert_eq!(http.port, 9000);
                assert!(!http.sse_enabled);
            }
            other => panic!("expected HTTP transport, got {other:?}"),
        }
    }

    #[test]
    fn cli_tokens_file_is_loaded() {
        let dir = tempdir().expect("tempdir");
        let tokens_path = dir.path().join("tokens.json");
        std::fs::write(&tokens_path, r#"{"tokens":[{"token":"secret-token"}]}"#)
            .expect("write tokens file");

        let mut cli = CliTransportOptions::default();
        cli.transport = Some("http".to_string());
        cli.tokens_file = Some(tokens_path.clone());

        let transport =
            determine_transport(&cli, None, dir.path()).expect("determine http transport");

        match transport {
            Transport::Http(http) => {
                let auth = http.auth.expect("auth config");
                assert!(auth.tokens.contains("secret-token"));
            }
            other => panic!("expected HTTP transport, got {other:?}"),
        }
    }
}

fn build_auth_config(
    cli: &CliTransportOptions,
    file_cfg: Option<&FileTransportConfig>,
    working_dir: &Path,
) -> Result<Option<HttpAuthConfig>> {
    let mut tokens: HashSet<String> = HashSet::new();
    let mut auth_enabled = false;

    if let Some(cfg) = file_cfg.and_then(|cfg| cfg.auth.as_ref()) {
        if let Some(auth_type) = cfg.auth_type.as_deref() {
            if auth_type != "bearer" {
                return Err(anyhow!(
                    "Unsupported MCP auth type '{}'. Only 'bearer' is supported.",
                    auth_type
                ));
            }
        }

        if let Some(enabled) = cfg.enabled {
            auth_enabled = enabled;
        }

        if !cfg.tokens.is_empty() {
            tokens.extend(cfg.tokens.iter().cloned());
            auth_enabled = cfg.enabled.unwrap_or(true);
        }

        if let Some(path) = cfg.tokens_file.as_ref() {
            let loaded = load_tokens_from_file(path)?;
            if !loaded.is_empty() {
                auth_enabled = true;
            }
            tokens.extend(loaded);
        }
    }

    if let Some(path) = cli.tokens_file.as_ref() {
        let resolved = resolve_relative(working_dir, path);
        let loaded = load_tokens_from_file(&resolved)?;
        if !loaded.is_empty() {
            auth_enabled = true;
        }
        tokens.extend(loaded);
    }

    if !cli.tokens.is_empty() {
        auth_enabled = true;
        tokens.extend(cli.tokens.iter().cloned());
    }

    if auth_enabled && tokens.is_empty() {
        tracing::warn!("MCP HTTP auth enabled but no tokens configured");
    }

    if auth_enabled {
        Ok(Some(HttpAuthConfig { tokens }))
    } else {
        Ok(None)
    }
}

fn build_cors_config(
    cli: &CliTransportOptions,
    file_cfg: Option<&FileTransportConfig>,
) -> Option<HttpCorsConfig> {
    let mut origins: Vec<String> = Vec::new();

    if let Some(cfg) = file_cfg.and_then(|cfg| cfg.cors.as_ref()) {
        origins.extend(cfg.allowed_origins.iter().cloned());
    }

    origins.extend(cli.cors_origins.iter().cloned());

    if origins.is_empty() {
        return None;
    }

    origins.sort();
    origins.dedup();

    Some(HttpCorsConfig {
        allowed_origins: origins,
    })
}

fn load_tokens_from_file(path: &Path) -> Result<Vec<String>> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("Failed to read tokens file {}", path.display()))?;
    let parsed: TokenFile = serde_json::from_str(&contents)
        .with_context(|| format!("Failed to parse tokens file {}", path.display()))?;
    Ok(parsed.tokens.into_iter().map(|entry| entry.token).collect())
}

fn resolve_relative(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

impl RawFileTransportConfig {
    fn into_runtime_config(self, base: &Path) -> FileTransportConfig {
        FileTransportConfig {
            transport: self.transport,
            host: self.host,
            port: self.port,
            sse_enabled: self.sse_enabled,
            auth: self.auth.map(|raw| raw.into_runtime_config(base)),
            cors: self.cors.map(|raw| raw.into_runtime_config()),
        }
    }
}

impl RawFileAuthConfig {
    fn into_runtime_config(self, base: &Path) -> FileAuthConfig {
        let tokens_file = self
            .tokens_file
            .map(|value| resolve_relative(base, Path::new(&value)));

        FileAuthConfig {
            enabled: self.enabled,
            auth_type: self.auth_type,
            tokens_file,
            tokens: self.tokens.unwrap_or_default(),
        }
    }
}

impl RawFileCorsConfig {
    fn into_runtime_config(self) -> FileCorsConfig {
        FileCorsConfig {
            allowed_origins: self.allowed_origins.unwrap_or_default(),
        }
    }
}

#[derive(Deserialize)]
struct TokenFile {
    tokens: Vec<TokenEntry>,
}

#[derive(Deserialize)]
struct TokenEntry {
    token: String,
}
