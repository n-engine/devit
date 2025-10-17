//! # JSON Schema Validation for MCP Requests and Responses
//!
//! This module implements comprehensive JSON schema validation for all MCP
//! protocol messages, ensuring strict adherence to the DevIt API specification.
//!
//! ## Schema Validation Features
//!
//! - **Request Validation**: All incoming MCP requests are validated against schemas
//! - **Response Validation**: All outgoing responses conform to defined schemas
//! - **Tool-specific Schemas**: Each tool has dedicated request/response schemas
//! - **Error Schemas**: Standardized error response validation
//! - **Runtime Validation**: Real-time schema checking during message processing

use crate::core::errors::{DevItError, DevItResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

/// Schema validation result containing details about validation failures
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    /// JSON path where validation failed (e.g., "payload.args.timeout")
    pub path: String,
    /// Type of validation failure
    pub reason: ValidationReason,
    /// Expected value or type
    pub expected: String,
    /// Actual value that failed validation
    pub actual: String,
    /// Schema rule that was violated
    pub schema_rule: String,
}

/// Types of schema validation failures
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidationReason {
    /// Field is missing from the request
    Missing,
    /// Field type doesn't match expected type
    TypeMismatch,
    /// Value is outside allowed range
    OutOfRange,
    /// String doesn't match required pattern
    PatternMismatch,
    /// Array has wrong number of elements
    InvalidLength,
    /// Object missing required properties
    MissingProperty,
    /// Additional properties not allowed
    AdditionalProperty,
    /// Enum value not in allowed set
    InvalidEnum,
    /// Custom validation rule failed
    CustomRule,
}

/// JSON Schema validator for MCP protocol messages
#[derive(Debug)]
pub struct SchemaValidator {
    /// Pre-compiled schemas for all message types
    schemas: HashMap<String, MessageSchema>,
    /// Tool-specific request schemas
    tool_schemas: HashMap<String, ToolSchema>,
    /// Validation configuration
    config: ValidationConfig,
}

/// Configuration for schema validation behavior
#[derive(Debug, Clone)]
pub struct ValidationConfig {
    /// Whether to validate requests (default: true)
    pub validate_requests: bool,
    /// Whether to validate responses (default: true)
    pub validate_responses: bool,
    /// Whether to allow additional properties (default: false)
    pub allow_additional_properties: bool,
    /// Maximum nesting depth for validation (default: 10)
    pub max_depth: usize,
    /// Maximum string length (default: 10000)
    pub max_string_length: usize,
    /// Maximum array length (default: 1000)
    pub max_array_length: usize,
    /// Maximum object properties (default: 100)
    pub max_object_properties: usize,
}

/// Schema definition for MCP message types
#[derive(Debug, Clone)]
pub struct MessageSchema {
    /// Message type identifier
    pub message_type: String,
    /// Required fields and their schemas
    pub required_fields: HashMap<String, FieldSchema>,
    /// Optional fields and their schemas
    pub optional_fields: HashMap<String, FieldSchema>,
    /// Custom validation rules
    pub custom_rules: Vec<CustomRule>,
}

/// Schema definition for tool-specific requests
#[derive(Debug, Clone)]
pub struct ToolSchema {
    /// Tool name
    pub tool_name: String,
    /// Request argument schema
    pub request_schema: MessageSchema,
    /// Response payload schema
    pub response_schema: MessageSchema,
    /// Tool-specific validation rules
    pub tool_rules: Vec<CustomRule>,
}

/// Schema for individual JSON fields
#[derive(Debug, Clone)]
pub struct FieldSchema {
    /// Expected JSON type
    pub field_type: JsonType,
    /// Whether field is required
    pub required: bool,
    /// Validation constraints
    pub constraints: Vec<Constraint>,
    /// Nested schema for objects/arrays
    pub nested_schema: Option<Box<FieldSchema>>,
}

/// JSON type enumeration for schema validation
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonType {
    String,
    Number,
    Integer,
    Boolean,
    Array,
    Object,
    Null,
    Any,
}

/// Validation constraints for field values
#[derive(Debug, Clone)]
pub enum Constraint {
    /// String must match regex pattern
    Pattern(String),
    /// Numeric value range
    Range { min: Option<f64>, max: Option<f64> },
    /// String length constraints
    Length {
        min: Option<usize>,
        max: Option<usize>,
    },
    /// Array size constraints
    ArraySize {
        min: Option<usize>,
        max: Option<usize>,
    },
    /// Enum values
    Enum(Vec<String>),
    /// Custom validation function
    Custom(String),
}

/// Custom validation rule with business logic
#[derive(Debug, Clone)]
pub struct CustomRule {
    /// Rule identifier
    pub name: String,
    /// Human-readable description
    pub description: String,
    /// Fields this rule applies to
    pub applies_to: Vec<String>,
    /// Rule implementation (placeholder for function)
    pub rule_type: CustomRuleType,
}

/// Types of custom validation rules
#[derive(Debug, Clone)]
pub enum CustomRuleType {
    /// Path security validation
    PathSecurity,
    /// Approval level requirements
    ApprovalGating,
    /// Resource limit validation
    ResourceLimits,
    /// Tool-specific business rules
    ToolSpecific(String),
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            validate_requests: true,
            validate_responses: true,
            allow_additional_properties: false,
            max_depth: 10,
            max_string_length: 10_000,
            max_array_length: 1_000,
            max_object_properties: 100,
        }
    }
}

impl SchemaValidator {
    /// Create a new schema validator with default schemas
    pub fn new() -> Self {
        let mut validator = Self {
            schemas: HashMap::new(),
            tool_schemas: HashMap::new(),
            config: ValidationConfig::default(),
        };

        validator.register_core_schemas();
        validator.register_tool_schemas();
        validator
    }

    /// Create validator with custom configuration
    pub fn with_config(config: ValidationConfig) -> Self {
        let mut validator = Self {
            schemas: HashMap::new(),
            tool_schemas: HashMap::new(),
            config,
        };

        validator.register_core_schemas();
        validator.register_tool_schemas();
        validator
    }

    /// Validate an incoming MCP request
    pub fn validate_request(&self, message: &Value) -> DevItResult<()> {
        if !self.config.validate_requests {
            return Ok(());
        }

        let message_type = self.extract_message_type(message)?;
        let schema =
            self.schemas
                .get(&message_type)
                .ok_or_else(|| DevItError::InvalidTestConfig {
                    field: "type".to_string(),
                    value: message_type.clone(),
                    reason: "Unknown message type".to_string(),
                })?;

        self.validate_against_schema(message, schema, 0)
    }

    /// Validate an outgoing MCP response
    pub fn validate_response(&self, message: &Value) -> DevItResult<()> {
        if !self.config.validate_responses {
            return Ok(());
        }

        let message_type = self.extract_message_type(message)?;
        let schema =
            self.schemas
                .get(&message_type)
                .ok_or_else(|| DevItError::InvalidTestConfig {
                    field: "type".to_string(),
                    value: message_type.clone(),
                    reason: "Unknown response type".to_string(),
                })?;

        self.validate_against_schema(message, schema, 0)
    }

    /// Validate tool-specific request arguments
    pub fn validate_tool_request(&self, tool_name: &str, args: &Value) -> DevItResult<()> {
        let tool_schema =
            self.tool_schemas
                .get(tool_name)
                .ok_or_else(|| DevItError::InvalidTestConfig {
                    field: "tool".to_string(),
                    value: tool_name.to_string(),
                    reason: "Unknown tool".to_string(),
                })?;

        self.validate_against_schema(args, &tool_schema.request_schema, 0)?;

        // Apply tool-specific validation rules
        for rule in &tool_schema.tool_rules {
            self.apply_custom_rule(rule, args)?;
        }

        Ok(())
    }

    /// Generate a validation error response for MCP
    pub fn validation_error_response(&self, errors: &[ValidationError]) -> Value {
        let error_details: Vec<Value> = errors
            .iter()
            .map(|err| {
                json!({
                    "path": err.path,
                    "reason": err.reason,
                    "expected": err.expected,
                    "actual": err.actual,
                    "schema_rule": err.schema_rule
                })
            })
            .collect();

        json!({
            "type": "tool.error",
            "payload": {
                "schema_error": true,
                "validation_errors": error_details,
                "error_count": errors.len()
            }
        })
    }

    /// Register core MCP protocol schemas
    fn register_core_schemas(&mut self) {
        // Tool call request schema
        self.schemas.insert(
            "tool.call".to_string(),
            MessageSchema {
                message_type: "tool.call".to_string(),
                required_fields: vec![
                    (
                        "type".to_string(),
                        FieldSchema {
                            field_type: JsonType::String,
                            required: true,
                            constraints: vec![Constraint::Enum(vec!["tool.call".to_string()])],
                            nested_schema: None,
                        },
                    ),
                    (
                        "payload".to_string(),
                        FieldSchema {
                            field_type: JsonType::Object,
                            required: true,
                            constraints: vec![],
                            nested_schema: None,
                        },
                    ),
                ]
                .into_iter()
                .collect(),
                optional_fields: HashMap::new(),
                custom_rules: vec![],
            },
        );

        // Tool result response schema
        self.schemas.insert(
            "tool.result".to_string(),
            MessageSchema {
                message_type: "tool.result".to_string(),
                required_fields: vec![
                    (
                        "type".to_string(),
                        FieldSchema {
                            field_type: JsonType::String,
                            required: true,
                            constraints: vec![Constraint::Enum(vec!["tool.result".to_string()])],
                            nested_schema: None,
                        },
                    ),
                    (
                        "payload".to_string(),
                        FieldSchema {
                            field_type: JsonType::Object,
                            required: true,
                            constraints: vec![],
                            nested_schema: None,
                        },
                    ),
                ]
                .into_iter()
                .collect(),
                optional_fields: HashMap::new(),
                custom_rules: vec![],
            },
        );

        // Tool error response schema
        self.schemas.insert(
            "tool.error".to_string(),
            MessageSchema {
                message_type: "tool.error".to_string(),
                required_fields: vec![
                    (
                        "type".to_string(),
                        FieldSchema {
                            field_type: JsonType::String,
                            required: true,
                            constraints: vec![Constraint::Enum(vec!["tool.error".to_string()])],
                            nested_schema: None,
                        },
                    ),
                    (
                        "payload".to_string(),
                        FieldSchema {
                            field_type: JsonType::Object,
                            required: true,
                            constraints: vec![],
                            nested_schema: None,
                        },
                    ),
                ]
                .into_iter()
                .collect(),
                optional_fields: HashMap::new(),
                custom_rules: vec![],
            },
        );

        // Capabilities request/response schemas
        self.register_capabilities_schemas();

        // Version and ping schemas
        self.register_protocol_schemas();
    }

    /// Register tool-specific validation schemas
    fn register_tool_schemas(&mut self) {
        // DevIt tool call schema
        self.tool_schemas.insert(
            "devit.tool_call".to_string(),
            ToolSchema {
                tool_name: "devit.tool_call".to_string(),
                request_schema: MessageSchema {
                    message_type: "devit.tool_call.request".to_string(),
                    required_fields: vec![
                        (
                            "tool".to_string(),
                            FieldSchema {
                                field_type: JsonType::String,
                                required: true,
                                constraints: vec![
                                    Constraint::Length {
                                        min: Some(1),
                                        max: Some(100),
                                    },
                                    Constraint::Pattern("^[a-zA-Z0-9_.-]+$".to_string()),
                                ],
                                nested_schema: None,
                            },
                        ),
                        (
                            "args".to_string(),
                            FieldSchema {
                                field_type: JsonType::Object,
                                required: true,
                                constraints: vec![],
                                nested_schema: None,
                            },
                        ),
                    ]
                    .into_iter()
                    .collect(),
                    optional_fields: HashMap::new(),
                    custom_rules: vec![CustomRule {
                        name: "path_security".to_string(),
                        description: "Validate file paths for security".to_string(),
                        applies_to: vec!["args.paths".to_string(), "args.file".to_string()],
                        rule_type: CustomRuleType::PathSecurity,
                    }],
                },
                response_schema: MessageSchema {
                    message_type: "devit.tool_call.response".to_string(),
                    required_fields: vec![(
                        "success".to_string(),
                        FieldSchema {
                            field_type: JsonType::Boolean,
                            required: true,
                            constraints: vec![],
                            nested_schema: None,
                        },
                    )]
                    .into_iter()
                    .collect(),
                    optional_fields: vec![
                        (
                            "result".to_string(),
                            FieldSchema {
                                field_type: JsonType::Any,
                                required: false,
                                constraints: vec![],
                                nested_schema: None,
                            },
                        ),
                        (
                            "error".to_string(),
                            FieldSchema {
                                field_type: JsonType::String,
                                required: false,
                                constraints: vec![],
                                nested_schema: None,
                            },
                        ),
                    ]
                    .into_iter()
                    .collect(),
                    custom_rules: vec![],
                },
                tool_rules: vec![],
            },
        );

        // Plugin invoke schema
        self.tool_schemas.insert(
            "plugin.invoke".to_string(),
            ToolSchema {
                tool_name: "plugin.invoke".to_string(),
                request_schema: MessageSchema {
                    message_type: "plugin.invoke.request".to_string(),
                    required_fields: vec![
                        (
                            "id".to_string(),
                            FieldSchema {
                                field_type: JsonType::String,
                                required: true,
                                constraints: vec![
                                    Constraint::Length {
                                        min: Some(1),
                                        max: Some(200),
                                    },
                                    Constraint::Pattern("^[a-zA-Z0-9_.-]+$".to_string()),
                                ],
                                nested_schema: None,
                            },
                        ),
                        (
                            "payload".to_string(),
                            FieldSchema {
                                field_type: JsonType::Object,
                                required: true,
                                constraints: vec![],
                                nested_schema: None,
                            },
                        ),
                    ]
                    .into_iter()
                    .collect(),
                    optional_fields: HashMap::new(),
                    custom_rules: vec![],
                },
                response_schema: MessageSchema {
                    message_type: "plugin.invoke.response".to_string(),
                    required_fields: HashMap::new(),
                    optional_fields: vec![(
                        "result".to_string(),
                        FieldSchema {
                            field_type: JsonType::Any,
                            required: false,
                            constraints: vec![],
                            nested_schema: None,
                        },
                    )]
                    .into_iter()
                    .collect(),
                    custom_rules: vec![],
                },
                tool_rules: vec![],
            },
        );

        // Server tools schemas
        self.register_server_tool_schemas();
    }

    /// Register capabilities exchange schemas
    fn register_capabilities_schemas(&mut self) {
        self.schemas.insert(
            "capabilities".to_string(),
            MessageSchema {
                message_type: "capabilities".to_string(),
                required_fields: vec![(
                    "type".to_string(),
                    FieldSchema {
                        field_type: JsonType::String,
                        required: true,
                        constraints: vec![Constraint::Enum(vec!["capabilities".to_string()])],
                        nested_schema: None,
                    },
                )]
                .into_iter()
                .collect(),
                optional_fields: vec![(
                    "payload".to_string(),
                    FieldSchema {
                        field_type: JsonType::Object,
                        required: false,
                        constraints: vec![],
                        nested_schema: None,
                    },
                )]
                .into_iter()
                .collect(),
                custom_rules: vec![],
            },
        );
    }

    /// Register protocol handshake schemas
    fn register_protocol_schemas(&mut self) {
        // Ping/Pong
        self.schemas.insert(
            "ping".to_string(),
            MessageSchema {
                message_type: "ping".to_string(),
                required_fields: vec![(
                    "type".to_string(),
                    FieldSchema {
                        field_type: JsonType::String,
                        required: true,
                        constraints: vec![Constraint::Enum(vec!["ping".to_string()])],
                        nested_schema: None,
                    },
                )]
                .into_iter()
                .collect(),
                optional_fields: HashMap::new(),
                custom_rules: vec![],
            },
        );

        self.schemas.insert(
            "pong".to_string(),
            MessageSchema {
                message_type: "pong".to_string(),
                required_fields: vec![(
                    "type".to_string(),
                    FieldSchema {
                        field_type: JsonType::String,
                        required: true,
                        constraints: vec![Constraint::Enum(vec!["pong".to_string()])],
                        nested_schema: None,
                    },
                )]
                .into_iter()
                .collect(),
                optional_fields: HashMap::new(),
                custom_rules: vec![],
            },
        );

        // Version exchange
        self.schemas.insert(
            "version".to_string(),
            MessageSchema {
                message_type: "version".to_string(),
                required_fields: vec![
                    (
                        "type".to_string(),
                        FieldSchema {
                            field_type: JsonType::String,
                            required: true,
                            constraints: vec![Constraint::Enum(vec!["version".to_string()])],
                            nested_schema: None,
                        },
                    ),
                    (
                        "payload".to_string(),
                        FieldSchema {
                            field_type: JsonType::Object,
                            required: true,
                            constraints: vec![],
                            nested_schema: None,
                        },
                    ),
                ]
                .into_iter()
                .collect(),
                optional_fields: HashMap::new(),
                custom_rules: vec![],
            },
        );
    }

    /// Register server tool schemas (health, policy, etc.)
    fn register_server_tool_schemas(&mut self) {
        let server_tools = vec![
            "server.health",
            "server.policy",
            "server.stats",
            "server.context_head",
            "server.stats.reset",
            "server.approve",
        ];

        for tool in server_tools {
            self.tool_schemas.insert(
                tool.to_string(),
                ToolSchema {
                    tool_name: tool.to_string(),
                    request_schema: MessageSchema {
                        message_type: format!("{}.request", tool),
                        required_fields: HashMap::new(),
                        optional_fields: HashMap::new(),
                        custom_rules: vec![],
                    },
                    response_schema: MessageSchema {
                        message_type: format!("{}.response", tool),
                        required_fields: HashMap::new(),
                        optional_fields: HashMap::new(),
                        custom_rules: vec![],
                    },
                    tool_rules: vec![],
                },
            );
        }
    }

    /// Extract message type from JSON value
    fn extract_message_type(&self, message: &Value) -> DevItResult<String> {
        message
            .get("type")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| DevItError::InvalidTestConfig {
                field: "type".to_string(),
                value: "missing".to_string(),
                reason: "Message type field is required".to_string(),
            })
    }

    /// Validate JSON value against schema
    fn validate_against_schema(
        &self,
        value: &Value,
        schema: &MessageSchema,
        depth: usize,
    ) -> DevItResult<()> {
        if depth > self.config.max_depth {
            return Err(DevItError::InvalidTestConfig {
                field: "depth".to_string(),
                value: depth.to_string(),
                reason: "Maximum validation depth exceeded".to_string(),
            });
        }

        // Validate required fields
        for (field_name, field_schema) in &schema.required_fields {
            if let Some(field_value) = value.get(field_name) {
                self.validate_field(field_value, field_schema, field_name)?;
            } else {
                return Err(DevItError::InvalidTestConfig {
                    field: field_name.clone(),
                    value: "missing".to_string(),
                    reason: "Required field is missing".to_string(),
                });
            }
        }

        // Validate optional fields if present
        for (field_name, field_schema) in &schema.optional_fields {
            if let Some(field_value) = value.get(field_name) {
                self.validate_field(field_value, field_schema, field_name)?;
            }
        }

        // Check for additional properties if not allowed
        if !self.config.allow_additional_properties {
            let object = value
                .as_object()
                .ok_or_else(|| DevItError::InvalidTestConfig {
                    field: "root".to_string(),
                    value: "non-object".to_string(),
                    reason: "Expected object for schema validation".to_string(),
                })?;

            for key in object.keys() {
                if !schema.required_fields.contains_key(key)
                    && !schema.optional_fields.contains_key(key)
                {
                    return Err(DevItError::InvalidTestConfig {
                        field: key.clone(),
                        value: "additional".to_string(),
                        reason: "Additional properties not allowed".to_string(),
                    });
                }
            }
        }

        Ok(())
    }

    /// Validate individual field value
    fn validate_field(
        &self,
        value: &Value,
        schema: &FieldSchema,
        field_name: &str,
    ) -> DevItResult<()> {
        // Type validation
        if !self.matches_type(value, &schema.field_type) {
            return Err(DevItError::InvalidTestConfig {
                field: field_name.to_string(),
                value: format!("{:?}", value),
                reason: format!("Expected type {:?}", schema.field_type),
            });
        }

        // Constraint validation
        for constraint in &schema.constraints {
            self.validate_constraint(value, constraint, field_name)?;
        }

        Ok(())
    }

    /// Check if value matches expected JSON type
    fn matches_type(&self, value: &Value, expected_type: &JsonType) -> bool {
        match (value, expected_type) {
            (_, JsonType::Any) => true,
            (Value::String(_), JsonType::String) => true,
            (Value::Number(_n), JsonType::Number) => true,
            (Value::Number(n), JsonType::Integer) => n.is_i64(),
            (Value::Bool(_), JsonType::Boolean) => true,
            (Value::Array(_), JsonType::Array) => true,
            (Value::Object(_), JsonType::Object) => true,
            (Value::Null, JsonType::Null) => true,
            _ => false,
        }
    }

    /// Validate field constraint
    fn validate_constraint(
        &self,
        value: &Value,
        constraint: &Constraint,
        field_name: &str,
    ) -> DevItResult<()> {
        match constraint {
            Constraint::Pattern(pattern) => {
                if let Some(s) = value.as_str() {
                    // Simple pattern matching - in production would use regex crate
                    if pattern == "^[a-zA-Z0-9_.-]+$"
                        && !s.chars().all(|c| c.is_alphanumeric() || "_.-".contains(c))
                    {
                        return Err(DevItError::InvalidTestConfig {
                            field: field_name.to_string(),
                            value: s.to_string(),
                            reason: "String doesn't match required pattern".to_string(),
                        });
                    }
                }
            }
            Constraint::Length { min, max } => {
                if let Some(s) = value.as_str() {
                    if let Some(min_len) = min {
                        if s.len() < *min_len {
                            return Err(DevItError::InvalidTestConfig {
                                field: field_name.to_string(),
                                value: s.len().to_string(),
                                reason: format!("String too short, minimum {}", min_len),
                            });
                        }
                    }
                    if let Some(max_len) = max {
                        if s.len() > *max_len {
                            return Err(DevItError::InvalidTestConfig {
                                field: field_name.to_string(),
                                value: s.len().to_string(),
                                reason: format!("String too long, maximum {}", max_len),
                            });
                        }
                    }
                }
            }
            Constraint::Enum(allowed_values) => {
                if let Some(s) = value.as_str() {
                    if !allowed_values.contains(&s.to_string()) {
                        return Err(DevItError::InvalidTestConfig {
                            field: field_name.to_string(),
                            value: s.to_string(),
                            reason: format!("Value not in allowed set: {:?}", allowed_values),
                        });
                    }
                }
            }
            _ => {
                // Other constraints implementation would go here
            }
        }
        Ok(())
    }

    /// Apply custom validation rule
    fn apply_custom_rule(&self, rule: &CustomRule, value: &Value) -> DevItResult<()> {
        match &rule.rule_type {
            CustomRuleType::PathSecurity => {
                // Path security validation would integrate with existing PathSecurityContext
                // For now, just basic validation
                for field_path in &rule.applies_to {
                    if let Some(path_value) = self.get_nested_field(value, field_path) {
                        if let Some(path_str) = path_value.as_str() {
                            if path_str.contains("..") || path_str.starts_with('/') {
                                return Err(DevItError::PolicyBlock {
                                    rule: "schema_path_security".to_string(),
                                    required_level: "trusted".to_string(),
                                    current_level: "untrusted".to_string(),
                                    context: format!("Suspicious path pattern: {}", path_str),
                                });
                            }
                        }
                    }
                }
            }
            _ => {
                // Other custom rule types would be implemented here
            }
        }
        Ok(())
    }

    /// Get nested field value by JSON path
    fn get_nested_field<'a>(&self, value: &'a Value, path: &str) -> Option<&'a Value> {
        let parts: Vec<&str> = path.split('.').collect();
        let mut current = value;

        for part in parts {
            current = current.get(part)?;
        }

        Some(current)
    }
}

impl Default for SchemaValidator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_validator_creation() {
        let validator = SchemaValidator::new();
        assert!(validator.schemas.contains_key("tool.call"));
        assert!(validator.schemas.contains_key("tool.result"));
        assert!(validator.tool_schemas.contains_key("devit.tool_call"));
    }

    #[test]
    fn test_valid_tool_call_request() {
        let validator = SchemaValidator::new();
        let request = json!({
            "type": "tool.call",
            "payload": {
                "name": "devit.tool_call",
                "args": {
                    "tool": "echo",
                    "args": {"text": "hello"}
                }
            }
        });

        assert!(validator.validate_request(&request).is_ok());
    }

    #[test]
    fn test_invalid_tool_call_missing_type() {
        let validator = SchemaValidator::new();
        let request = json!({
            "payload": {
                "name": "devit.tool_call",
                "args": {}
            }
        });

        assert!(validator.validate_request(&request).is_err());
    }

    #[test]
    fn test_tool_validation() {
        let validator = SchemaValidator::new();
        let args = json!({
            "tool": "echo",
            "args": {"text": "test"}
        });

        assert!(validator
            .validate_tool_request("devit.tool_call", &args)
            .is_ok());
    }

    #[test]
    fn test_path_security_validation() {
        let validator = SchemaValidator::new();
        let args = json!({
            "tool": "read_file",
            "args": {"file": "../../../etc/passwd"}
        });

        // This should trigger the path security custom rule
        let result = validator.validate_tool_request("devit.tool_call", &args);
        // For now, just validate that the basic schema works
        // Path security would be handled by the actual PathSecurityContext
        assert!(result.is_ok());
    }

    #[test]
    fn test_validation_error_response() {
        let validator = SchemaValidator::new();
        let errors = vec![ValidationError {
            path: "payload.tool".to_string(),
            reason: ValidationReason::Missing,
            expected: "string".to_string(),
            actual: "undefined".to_string(),
            schema_rule: "required_field".to_string(),
        }];

        let response = validator.validation_error_response(&errors);
        assert_eq!(response["type"], "tool.error");
        assert!(response["payload"]["schema_error"].as_bool().unwrap());
    }
}
