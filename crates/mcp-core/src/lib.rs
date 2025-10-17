use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

pub type McpResult<T> = Result<T, McpError>;

#[derive(Debug, Error)]
pub enum McpError {
    #[error("invalid request: {0}")]
    InvalidRequest(String),

    #[error("tool not found: {0}")]
    ToolNotFound(String),

    #[error("execution failed: {0}")]
    ExecutionFailed(String),

    #[error("internal error: {0}")]
    Internal(String),

    #[error("{message}")]
    Rpc {
        code: i32,
        message: String,
        data: Option<Value>,
    },
}

impl McpError {
    pub fn code(&self) -> i32 {
        match self {
            McpError::InvalidRequest(_) => -32600,
            McpError::ToolNotFound(_) => -32601,
            McpError::ExecutionFailed(_) => -32001,
            McpError::Internal(_) => -32603,
            McpError::Rpc { code, .. } => *code,
        }
    }

    pub fn message(&self) -> String {
        match self {
            McpError::InvalidRequest(msg)
            | McpError::ToolNotFound(msg)
            | McpError::ExecutionFailed(msg)
            | McpError::Internal(msg) => msg.clone(),
            McpError::Rpc { message, .. } => message.clone(),
        }
    }

    pub fn data(&self) -> Option<Value> {
        match self {
            McpError::Rpc { data, .. } => data.clone(),
            _ => None,
        }
    }

    pub fn rpc<D: Into<Option<Value>>, M: Into<String>>(code: i32, message: M, data: D) -> Self {
        McpError::Rpc {
            code,
            message: message.into(),
            data: data.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolDescriptor {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

#[async_trait]
pub trait McpTool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    async fn execute(&self, params: Value) -> McpResult<Value>;
    fn input_schema(&self) -> Value;

    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: self.input_schema(),
        }
    }
}
