use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use mcp_core::{McpError, McpTool, ToolDescriptor};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::broadcast;

mod auth;
mod http_server;
pub mod transport;
use crate::transport::HttpTransportConfig;
#[derive(Clone)]
pub struct NotificationHub {
    sender: broadcast::Sender<Value>,
}

impl NotificationHub {
    pub fn new(buffer: usize) -> Self {
        let (sender, _receiver) = broadcast::channel(buffer);
        Self { sender }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Value> {
        self.sender.subscribe()
    }

    pub fn publish(&self, payload: Value) {
        if let Err(err) = self.sender.send(payload) {
            tracing::debug!("no subscribers for notification broadcast: {}", err);
        }
    }
}

#[derive(Clone)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn McpTool>>,
    order: Vec<String>,
}

impl ToolRegistry {
    pub fn new(tools: Vec<Arc<dyn McpTool>>) -> Self {
        let mut map = HashMap::new();
        let mut order = Vec::new();

        for tool in tools {
            let name = tool.name().to_string();
            if map.insert(name.clone(), tool).is_none() {
                order.push(name);
            }
        }

        Self { tools: map, order }
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn McpTool>> {
        self.tools.get(name).cloned()
    }

    pub fn descriptors(&self) -> Vec<ToolDescriptor> {
        self.order
            .iter()
            .filter_map(|name| self.tools.get(name))
            .map(|tool| tool.descriptor())
            .collect()
    }
}

pub struct McpServer {
    registry: ToolRegistry,
    notifier: NotificationHub,
}

impl McpServer {
    pub fn new(registry: ToolRegistry) -> Self {
        Self {
            registry,
            notifier: NotificationHub::new(128),
        }
    }

    pub fn notifier(&self) -> NotificationHub {
        self.notifier.clone()
    }

    pub async fn serve_stdio(&self) -> Result<()> {
        eprintln!("🔍 DEBUG: Starting MCP server on STDIN/STDOUT");
        let stdin = io::stdin();
        let mut reader = BufReader::new(stdin);
        let mut stdout = io::stdout();
        let mut line = String::new();

        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => {
                    eprintln!("🔍 DEBUG: Client disconnected (EOF)");
                    break;
                }
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    eprintln!("🔍 DEBUG: Received line: {}", trimmed);

                    match serde_json::from_str::<Value>(trimmed) {
                        Ok(request) => {
                            eprintln!("🔍 DEBUG: JSON parsed successfully");
                            match self.handle_jsonrpc(request).await {
                                Ok(Some(response)) => {
                                    let response_line = serde_json::to_string(&response)? + "\n";
                                    eprintln!("🔍 DEBUG: Sending: {}", response_line.trim());
                                    stdout.write_all(response_line.as_bytes()).await?;
                                    stdout.flush().await?;
                                    eprintln!("🔍 DEBUG: Response sent successfully");
                                }
                                Ok(None) => {
                                    eprintln!("🔍 DEBUG: No response emitted for this request");
                                }
                                Err(err) => {
                                    eprintln!("🚨 ERROR in handle_jsonrpc: {}", err);
                                    let error_response = json!({
                                        "jsonrpc": "2.0",
                                        "error": {"code": -32603, "message": "Internal error"}
                                    });
                                    let error_line = serde_json::to_string(&error_response)? + "\n";
                                    stdout.write_all(error_line.as_bytes()).await?;
                                    stdout.flush().await?;
                                }
                            }
                        }
                        Err(err) => {
                            eprintln!("🚨 JSON PARSE ERROR: {}", err);
                            eprintln!("🚨 Raw line was: '{}'", trimmed);
                            let error_response = json!({
                                "jsonrpc": "2.0",
                                "error": {"code": -32700, "message": "Parse error"}
                            });
                            let error_line = serde_json::to_string(&error_response)? + "\n";
                            stdout.write_all(error_line.as_bytes()).await?;
                            stdout.flush().await?;
                        }
                    }
                }
                Err(err) => {
                    eprintln!("🚨 READ ERROR: {}", err);
                    break;
                }
            }
        }

        eprintln!("🔍 DEBUG: Client handler exiting");
        Ok(())
    }

    pub async fn handle_jsonrpc(&self, request: Value) -> Result<Option<Value>> {
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("Missing method field"))?;

        if let Some(id) = request.get("id") {
            tracing::debug!("handling JSON-RPC method '{}' with id {}", method, id);
        } else {
            tracing::debug!("handling JSON-RPC method '{}'", method);
        }

        if method.starts_with("notifications/") {
            let payload = json!({
                "event": method,
                "params": request.get("params").cloned().unwrap_or(Value::Null),
            });
            self.notifier.publish(payload);
            return Ok(None);
        }

        let request_struct: JsonRpcRequest = serde_json::from_value(request.clone())?;
        let response = handle_request(request_struct, &self.registry).await;
        let value = serde_json::to_value(&response)?;
        tracing::debug!(
            "JSON-RPC response for '{}': {}",
            method,
            serde_json::to_string(&value).unwrap_or_else(|_| "<unserializable>".into())
        );
        Ok(Some(value))
    }

    pub async fn serve_http(self: Arc<Self>, config: HttpTransportConfig) -> Result<()> {
        http_server::run_http_transport(self, config).await
    }
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

async fn handle_request(request: JsonRpcRequest, registry: &ToolRegistry) -> JsonRpcResponse {
    let JsonRpcRequest {
        jsonrpc,
        id,
        method,
        params,
    } = request;

    if jsonrpc != "2.0" {
        return JsonRpcResponse {
            jsonrpc: "2.0",
            id: id.unwrap_or(Value::Null),
            result: None,
            error: Some(JsonRpcError {
                code: -32600,
                message: format!("Unsupported JSON-RPC version: {jsonrpc}"),
                data: None,
            }),
        };
    }

    match method.as_str() {
        "initialize" => respond_initialize(id),
        "tools/list" => respond_with_tools(id, registry),
        "tools/call" => handle_tools_call(id, params, registry).await,
        "resources/list" => respond_with_resources(id),
        "prompts/list" => respond_with_prompts(id),
        _ => JsonRpcResponse {
            jsonrpc: "2.0",
            id: id.unwrap_or(Value::Null),
            result: None,
            error: Some(JsonRpcError {
                code: -32601,
                message: format!("Unknown method: {method}"),
                data: None,
            }),
        },
    }
}

fn respond_with_tools(id: Option<Value>, registry: &ToolRegistry) -> JsonRpcResponse {
    let tools = registry.descriptors();
    JsonRpcResponse {
        jsonrpc: "2.0",
        id: id.unwrap_or(Value::Null),
        result: Some(json!({ "tools": tools })),
        error: None,
    }
}

fn respond_with_resources(id: Option<Value>) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0",
        id: id.unwrap_or(Value::Null),
        result: Some(json!({ "resources": [] })),
        error: None,
    }
}

fn respond_with_prompts(id: Option<Value>) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0",
        id: id.unwrap_or(Value::Null),
        result: Some(json!({ "prompts": [] })),
        error: None,
    }
}

fn respond_initialize(id: Option<Value>) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0",
        id: id.unwrap_or(Value::Null),
        result: Some(json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {
                "tools": { "listChanged": false },
                "prompts": { "listChanged": false },
                "resources": { "listChanged": false, "subscribe": false }
            },
            "serverInfo": {
                "name": "mcp-server",
                "version": env!("CARGO_PKG_VERSION"),
            }
        })),
        error: None,
    }
}

async fn handle_tools_call(
    id: Option<Value>,
    params: Option<Value>,
    registry: &ToolRegistry,
) -> JsonRpcResponse {
    let id = id.unwrap_or(Value::Null);

    let params = match params {
        Some(Value::Object(map)) => map,
        _ => {
            return rpc_error_response(
                id,
                McpError::InvalidRequest("Missing params object for tools/call".into()),
            );
        }
    };

    let name = match params.get("name").and_then(Value::as_str) {
        Some(name) => name,
        None => {
            return rpc_error_response(
                id,
                McpError::InvalidRequest("Missing 'name' in params".into()),
            );
        }
    };

    let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);

    let tool = match registry.get(name) {
        Some(tool) => tool,
        None => {
            return rpc_error_response(id, McpError::ToolNotFound(name.to_string()));
        }
    };

    match tool.execute(arguments).await {
        Ok(result) => JsonRpcResponse {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        },
        Err(err) => rpc_error_response(id, err),
    }
}

fn rpc_error_response(id: Value, error: McpError) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(JsonRpcError {
            code: error.code(),
            message: error.message(),
            data: error.data(),
        }),
    }
}
