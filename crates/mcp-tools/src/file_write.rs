use std::sync::Arc;

use anyhow::Error;
use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};
use devit_common::fs::{SafeFileWriter, WriteMode as SafeWriteMode};
use mcp_core::{McpError, McpResult, McpTool};
use serde_json::{json, Value};

use crate::errors::{internal_error, io_error, validation_error};
use crate::file_read::FileSystemContext;

const MAX_WRITE_SIZE: usize = 10 * 1024 * 1024; // 10 MB

pub struct FileWriteTool {
    context: Arc<FileSystemContext>,
    writer: SafeFileWriter,
}

impl FileWriteTool {
    pub fn new(context: Arc<FileSystemContext>) -> McpResult<Self> {
        let writer = SafeFileWriter::new()
            .map(|writer| {
                writer.with_allowed_dirs(vec![context.root().to_path_buf(), std::env::temp_dir()])
            })
            .map(|writer| writer.with_max_size(Some(MAX_WRITE_SIZE)))
            .map_err(|err| {
                internal_error(format!("Initialisation SafeFileWriter échouée: {err}"))
            })?;

        Ok(Self { context, writer })
    }

    fn parse_mode(&self, raw_mode: Option<&str>) -> McpResult<SafeWriteMode> {
        match raw_mode.unwrap_or("overwrite") {
            "overwrite" => Ok(SafeWriteMode::Overwrite),
            "append" => Ok(SafeWriteMode::Append),
            "create_new" => Ok(SafeWriteMode::CreateNew),
            other => Err(validation_error(&format!(
                "Mode invalide: '{}'. Modes supportés: overwrite, append, create_new",
                other
            ))),
        }
    }

    fn ensure_size(&self, size: usize) -> McpResult<()> {
        if size > MAX_WRITE_SIZE {
            return Err(validation_error(&format!(
                "Le contenu ({size} octets) dépasse la limite de {} octets",
                MAX_WRITE_SIZE
            )));
        }
        Ok(())
    }

    fn map_write_error(&self, path: &std::path::Path, err: Error) -> McpError {
        if let Some(io_err) = err.downcast_ref::<std::io::Error>() {
            io_error("write file", Some(path), io_err.to_string())
        } else {
            validation_error(&err.to_string())
        }
    }
}

#[async_trait]
impl McpTool for FileWriteTool {
    fn name(&self) -> &str {
        "devit_file_write"
    }

    fn description(&self) -> &str {
        "Write file content with security checks, multiple modes and encoding support"
    }

    async fn execute(&self, params: Value) -> McpResult<Value> {
        let path_str = params
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| validation_error("Le paramètre 'path' est requis"))?;

        let content = params
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| validation_error("Le paramètre 'content' est requis"))?;

        let encoding = params
            .get("encoding")
            .and_then(Value::as_str)
            .unwrap_or("utf8");

        let mode = self.parse_mode(params.get("mode").and_then(Value::as_str))?;

        let target_path = self.context.resolve_path(path_str)?;

        if target_path.is_dir() {
            return Err(validation_error("Impossible d'écrire dans un répertoire"));
        }

        let (buffer, byte_len) = match encoding {
            "utf8" => (content.as_bytes().to_vec(), content.len()),
            "binary" => match general_purpose::STANDARD.decode(content) {
                Ok(bytes) => {
                    let len = bytes.len();
                    (bytes, len)
                }
                Err(err) => {
                    return Err(validation_error(&format!(
                        "Encodage base64 invalide: {err}"
                    )))
                }
            },
            other => {
                return Err(validation_error(&format!(
                    "Encoding invalide: '{other}'. Encodings supportés: utf8, binary"
                )));
            }
        };

        self.ensure_size(byte_len)?;

        if matches!(mode, SafeWriteMode::CreateNew) && target_path.exists() {
            return Err(validation_error(&format!(
                "Le fichier existe déjà: {}",
                target_path.display()
            )));
        }

        self.writer
            .write(&target_path, &buffer, mode)
            .map_err(|err| self.map_write_error(&target_path, err))?;

        Ok(json!({
            "content": [{
                "type": "text",
                "text": format!(
                    "✅ Fichier écrit avec succès : {}\nMode: {}\nTaille: {} octets",
                    path_str,
                    params
                        .get("mode")
                        .and_then(Value::as_str)
                        .unwrap_or("overwrite"),
                    byte_len
                )
            }],
            "metadata": {
                "path": target_path.to_string_lossy(),
                "bytes_written": byte_len,
                "encoding": encoding
            }
        }))
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "content": {"type": "string"},
                "mode": {"type": "string", "enum": ["overwrite", "append", "create_new"]},
                "encoding": {"type": "string", "enum": ["utf8", "binary"]}
            },
            "required": ["path", "content"]
        })
    }
}
