use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Error;
use async_trait::async_trait;
use devit_common::orchestration::{
    format_status, DelegatedTask, OrchestrationContext, StatusFormat, TaskNotification, TaskStatus,
};
use mcp_core::{McpError, McpResult, McpTool};
use serde_json::{json, Value};
use tokio::time::sleep;

use crate::errors::{internal_error, io_error, validation_error};
use crate::file_read::FileSystemContext;
use crate::worker::WorkerBridge;

pub struct DelegateTool {
    context: Arc<OrchestrationContext>,
    fs: Arc<FileSystemContext>,
}

impl DelegateTool {
    pub fn new(context: Arc<OrchestrationContext>, fs: Arc<FileSystemContext>) -> Self {
        Self { context, fs }
    }
}

#[async_trait]
impl McpTool for DelegateTool {
    fn name(&self) -> &str {
        "devit_delegate"
    }

    fn description(&self) -> &str {
        "Déléguer une tâche à une IA externe avec monitoring optionnel"
    }

    async fn execute(&self, params: Value) -> McpResult<Value> {
        let goal = params
            .get("goal")
            .and_then(Value::as_str)
            .filter(|goal| !goal.trim().is_empty())
            .ok_or_else(|| {
                validation_error("Le paramètre 'goal' est requis et ne peut pas être vide")
            })?;

        let delegated_to = params
            .get("delegated_to")
            .and_then(Value::as_str)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "claude_code".to_string());

        let timeout = params
            .get("timeout")
            .and_then(Value::as_u64)
            .map(Duration::from_secs);

        let watch_patterns = params
            .get("watch_patterns")
            .and_then(Value::as_array)
            .map(|array| {
                let mut patterns = Vec::new();
                for value in array {
                    let Some(pattern) = value.as_str() else {
                        return Err(validation_error(
                            "'watch_patterns' doit être un tableau de chaînes",
                        ));
                    };
                    patterns.push(pattern.to_string());
                }
                Ok(patterns)
            })
            .transpose()?;

        let context_value = params.get("context").cloned();

        let model = params
            .get("model")
            .and_then(Value::as_str)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        let working_dir = parse_working_dir(params.get("working_dir"), &self.fs)?;

        let response_format = match params.get("format").and_then(Value::as_str) {
            Some("compact") => Some(String::from("compact")),
            Some("default") => None,
            Some(other) => {
                return Err(validation_error(&format!(
                    "Format invalide '{}'. Formats supportés: compact, default",
                    other
                )))
            }
            None => None,
        };

        let result = self
            .context
            .delegate(
                goal.to_string(),
                delegated_to.clone(),
                model.clone(),
                timeout,
                watch_patterns,
                context_value,
                working_dir.clone(),
                response_format.clone(),
            )
            .await
            .map_err(map_error)?;

        let working_dir_display = working_dir
            .as_ref()
            .map(|dir| normalize_path(dir))
            .unwrap_or_else(|| ".".to_string());

        let mode_label = response_format.unwrap_or_else(|| "default".to_string());
        let model_label = model.clone().unwrap_or_else(|| "<default>".to_string());

        Ok(json!({
            "content": [{
                "type": "text",
                "text": format!(
                    "🎯 **Task Delegated Successfully**\n\n**Task ID**: {}\n**Goal**: {}\n**Delegated to**: {}\n**Model**: {}\n**Timeout**: {}s\n**Working dir**: {}\n**Format**: {}\n\n✅ Watchdog monitoring initialisé\n📱 Vous serez notifié à la complétion",
                    result.task_id,
                    goal,
                    delegated_to,
                    model_label,
                    result.timeout_secs,
                    working_dir_display,
                    mode_label
                )
            }]
        }))
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "goal": {"type": "string"},
                "delegated_to": {
                    "type": "string",
                    "description": "Identifiant du worker (ex: claude_code, codex, ...)",
                    "default": "claude_code"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout en secondes"
                },
                "model": {
                    "type": "string",
                    "description": "Nom du modèle à utiliser (override optionnel)"
                },
                "watch_patterns": {
                    "type": "array",
                    "items": {"type": "string"}
                },
                "context": {
                    "type": "object"
                },
                "working_dir": {
                    "type": "string",
                    "description": "Répertoire de travail relatif au sandbox (ex: project-a/tests)"
                },
                "format": {
                    "type": "string",
                    "enum": ["default", "compact"],
                    "default": "default"
                }
            },
            "required": ["goal"],
            "additionalProperties": true
        })
    }
}

fn parse_working_dir(raw: Option<&Value>, fs: &FileSystemContext) -> McpResult<Option<PathBuf>> {
    let Some(Value::String(dir)) = raw else {
        return Ok(None);
    };

    if dir.trim().is_empty() {
        return Err(validation_error(
            "Le paramètre 'working_dir' ne peut pas être vide",
        ));
    }

    let canonical = fs.resolve_path(dir)?;
    let metadata = std::fs::metadata(&canonical).map_err(|err| {
        io_error(
            "stat working directory",
            Some(&canonical),
            format!("Impossible d'accéder au répertoire: {err}"),
        )
    })?;

    if !metadata.is_dir() {
        return Err(validation_error(
            "Le paramètre 'working_dir' doit référencer un répertoire",
        ));
    }

    let relative = canonical
        .strip_prefix(fs.root())
        .map_err(|_| internal_error("Chemin en dehors du sandbox"))?;

    if relative.components().next().is_none() {
        Ok(None)
    } else {
        Ok(Some(relative.to_path_buf()))
    }
}

fn normalize_path(path: &Path) -> String {
    if path.components().next().is_none() {
        ".".to_string()
    } else {
        path.to_string_lossy().replace('\\', "/")
    }
}

pub struct NotifyTool {
    context: Option<Arc<OrchestrationContext>>,
    worker: Option<Arc<WorkerBridge>>,
}

impl NotifyTool {
    pub fn new(context: Arc<OrchestrationContext>) -> Self {
        Self {
            context: Some(context),
            worker: None,
        }
    }

    pub fn with_worker(context: Arc<OrchestrationContext>, worker: Arc<WorkerBridge>) -> Self {
        Self {
            context: Some(context),
            worker: Some(worker),
        }
    }

    pub fn for_worker(worker: Arc<WorkerBridge>) -> Self {
        Self {
            context: None,
            worker: Some(worker),
        }
    }
}

#[async_trait]
impl McpTool for NotifyTool {
    fn name(&self) -> &str {
        "devit_notify"
    }

    fn description(&self) -> &str {
        "Notifier l'orchestrateur de l'avancement d'une tâche déléguée"
    }

    async fn execute(&self, params: Value) -> McpResult<Value> {
        let task_id = params
            .get("task_id")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| validation_error("Le paramètre 'task_id' est requis"))?;

        let status = params
            .get("status")
            .and_then(Value::as_str)
            .ok_or_else(|| validation_error("Le paramètre 'status' est requis"))?;

        match status {
            "completed" | "failed" | "progress" | "blocked" | "cancelled" | "ack" => {}
            _ => {
                return Err(validation_error(&format!(
                    "Valeur 'status' invalide: {}",
                    status
                )))
            }
        }

        let summary = params
            .get("summary")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| validation_error("Le paramètre 'summary' est requis"))?;

        let details = match params.get("details") {
            Some(Value::Object(_)) => params.get("details").cloned(),
            Some(Value::Null) | None => None,
            Some(_) => {
                return Err(validation_error(
                    "Le paramètre 'details' doit être un objet JSON",
                ))
            }
        };

        let evidence = match params.get("evidence") {
            Some(Value::Object(_)) => params.get("evidence").cloned(),
            Some(Value::Null) | None => None,
            Some(_) => {
                return Err(validation_error(
                    "Le paramètre 'evidence' doit être un objet JSON",
                ))
            }
        };

        let route_via_context = status.eq_ignore_ascii_case("ack");

        if route_via_context {
            let context = self
                .context
                .as_ref()
                .ok_or_else(|| internal_error("Aucun backend d'orchestration disponible"))?;
            context
                .notify(task_id, status, summary, details.clone(), evidence.clone())
                .await
                .map_err(map_error)?;
        } else if let Some(worker) = &self.worker {
            worker
                .notify(task_id, status, summary, details.clone(), evidence.clone())
                .await?;
        } else {
            let context = self
                .context
                .as_ref()
                .ok_or_else(|| internal_error("Aucun backend d'orchestration disponible"))?;
            context
                .notify(task_id, status, summary, details.clone(), evidence.clone())
                .await
                .map_err(map_error)?;
        }

        Ok(json!({
            "content": [{
                "type": "text",
                "text": format!(
                    "📱 **Notification Received**\n\n**Task**: {}\n**Status**: {}\n**Summary**: {}\n\n✅ Orchestrator updated",
                    task_id,
                    status,
                    summary
                )
            }]
        }))
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {"type": "string"},
                "status": {
                    "type": "string",
                    "enum": ["completed", "failed", "progress", "blocked", "cancelled", "ack"]
                },
                "summary": {"type": "string"},
                "details": {"type": ["object", "null"]},
                "evidence": {"type": ["object", "null"]}
            },
            "required": ["task_id", "status", "summary"]
        })
    }
}

pub struct OrchestrationStatusTool {
    context: Arc<OrchestrationContext>,
}

impl OrchestrationStatusTool {
    pub fn new(context: Arc<OrchestrationContext>) -> Self {
        Self { context }
    }
}

#[async_trait]
impl McpTool for OrchestrationStatusTool {
    fn name(&self) -> &str {
        "devit_orchestration_status"
    }

    fn description(&self) -> &str {
        "Obtenir l'état des tâches déléguées"
    }

    async fn execute(&self, params: Value) -> McpResult<Value> {
        let format =
            StatusFormat::parse(params.get("format").and_then(Value::as_str)).map_err(map_error)?;

        let filter = match params.get("filter").and_then(Value::as_str) {
            Some(value @ ("all" | "active" | "completed" | "failed")) => Some(value.to_string()),
            Some(other) => {
                return Err(validation_error(&format!(
                    "Filtre invalide: {}. Filtres supportés: all, active, completed, failed",
                    other
                )))
            }
            None => None,
        };

        let status = self
            .context
            .status(filter.as_deref())
            .await
            .map_err(map_error)?;
        let text = format_status(&status, format).map_err(map_error)?;

        Ok(json!({
            "content": [{
                "type": "text",
                "text": format!(
                    "🎛️ **Orchestration Status** (format: {})\n\n{}",
                    format.as_str(),
                    text
                )
            }]
        }))
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "format": {
                    "type": "string",
                    "enum": ["json", "compact", "table"],
                    "default": "compact"
                },
                "filter": {
                    "type": "string",
                    "enum": ["all", "active", "completed", "failed"],
                    "default": "all"
                }
            }
        })
    }
}

fn map_error(err: Error) -> McpError {
    match err.downcast::<std::io::Error>() {
        Ok(io_err) => io_error("orchestration", None, io_err.to_string()),
        Err(other) => internal_error(other.to_string()),
    }
}

pub struct TaskResultTool {
    context: Arc<OrchestrationContext>,
}

impl TaskResultTool {
    pub fn new(context: Arc<OrchestrationContext>) -> Self {
        Self { context }
    }
}

#[async_trait]
impl McpTool for TaskResultTool {
    fn name(&self) -> &str {
        "devit_task_result"
    }

    fn description(&self) -> &str {
        "Récupérer le résultat détaillé d'une tâche déléguée"
    }

    async fn execute(&self, params: Value) -> McpResult<Value> {
        let task_id = params
            .get("task_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| validation_error("Le paramètre 'task_id' est requis"))?;

        const MAX_ATTEMPTS: usize = 20;
        const RETRY_DELAY: Duration = Duration::from_millis(400);

        let mut attempts = 0usize;
        let task = loop {
            if let Some(task) = self.context.task(task_id).await.map_err(map_error)? {
                let is_terminal = matches!(
                    task.status,
                    TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled
                );
                let has_notifications = !task.notifications.is_empty();

                if is_terminal || has_notifications {
                    break task;
                }
            }

            attempts += 1;
            if attempts >= MAX_ATTEMPTS {
                return Err(validation_error(&format!(
                    "Tâche '{}' introuvable dans l'orchestration",
                    task_id
                )));
            }

            sleep(RETRY_DELAY).await;
            // Force a daemon refresh so we observe newly completed tasks.
            let _ = self.context.status(None).await.map_err(map_error)?;
        };

        let status_label = task_status_label(&task.status);
        let result_note = find_result_notification(&task);
        let format_label = task.response_format.as_deref().unwrap_or("default");

        let summary_text = result_note.map(|note| note.summary.clone());
        let details_value = result_note.and_then(|note| note.details.clone());
        let evidence_value = result_note.and_then(|note| note.evidence.clone());
        let metadata_value = result_note.and_then(|note| note.metadata.clone());

        let working_dir = task.working_dir.as_ref().map(|path| normalize_path(path));

        let response_json = json!({
            "task_id": task.id,
            "status": status_label,
            "goal": task.goal,
            "delegated_to": task.delegated_to,
            "timeout_secs": task.timeout_secs,
            "working_dir": working_dir,
            "format": format_label,
            "result": {
                "summary": summary_text,
                "details": details_value,
                "evidence": evidence_value,
                "metadata": metadata_value
            }
        });

        let human_summary = if let Some(note) = result_note {
            format!(
                "📬 **Task Result**\n\nTask: {}\nStatus: {}\nFormat: {}\nSummary: {}",
                task.id, status_label, format_label, note.summary
            )
        } else {
            format!(
                "📬 **Task Result**\n\nTask: {}\nStatus: {}\nFormat: {}\nAucun compte rendu n'a encore été enregistré",
                task.id,
                status_label,
                format_label
            )
        };

        let payload_json = serde_json::to_string_pretty(&response_json).unwrap_or_else(|_| {
            "{\n  \"error\": \"Failed to serialize task payload\"\n}".to_string()
        });

        let combined_text = format!(
            "{}\n\nPayload JSON:\n```json\n{}\n```",
            human_summary, payload_json
        );

        Ok(json!({
            "content": [
                {"type": "text", "text": combined_text}
            ]
        }))
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {"type": "string"}
            },
            "required": ["task_id"],
            "additionalProperties": false
        })
    }
}

fn find_result_notification(task: &DelegatedTask) -> Option<&TaskNotification> {
    if task.notifications.is_empty() {
        return None;
    }

    let desired = task_status_label(&task.status);
    task.notifications
        .iter()
        .rev()
        .find(|note| note.status.eq_ignore_ascii_case(desired))
        .or_else(|| task.notifications.last())
}

fn task_status_label(status: &TaskStatus) -> &'static str {
    match status {
        TaskStatus::Pending => "pending",
        TaskStatus::InProgress => "in_progress",
        TaskStatus::Completed => "completed",
        TaskStatus::Failed => "failed",
        TaskStatus::Cancelled => "cancelled",
    }
}
