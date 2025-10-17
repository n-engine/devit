use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result as AnyResult;
use async_trait::async_trait;
use devit_common::orchestration::OrchestrationContext;
use mcp_core::{McpResult, McpTool};
use regex::RegexBuilder;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::process::Command;

use crate::file_read::FileSystemContext;
use crate::{internal_error, validation_error};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RuleSpec {
    name: String,
    pattern: String,
    #[serde(default)]
    zone: Option<String>,
    #[serde(default)]
    severity: Option<String>,
    #[serde(default)]
    action: Option<String>, // "notify" | "none"
}

pub struct OcrAlertsTool {
    fs: Arc<FileSystemContext>,
    ctx: Arc<OrchestrationContext>,
}

impl OcrAlertsTool {
    pub fn new(fs: Arc<FileSystemContext>, ctx: Arc<OrchestrationContext>) -> Self {
        Self { fs, ctx }
    }

    fn default_screenshots_dir(&self) -> PathBuf {
        self.fs.root().join(".devit").join("screenshots")
    }

    fn resolve_image_path(&self, input: Option<&str>) -> Result<PathBuf, String> {
        match input {
            Some(p) if !p.trim().is_empty() => {
                let p = Path::new(p);
                let abs = if p.is_absolute() {
                    PathBuf::from(p)
                } else {
                    self.fs.root().join(p)
                };
                if abs.exists() {
                    Ok(abs)
                } else {
                    Err(format!("Image not found: {}", abs.display()))
                }
            }
            _ => {
                let dir = self.default_screenshots_dir();
                let rd = std::fs::read_dir(&dir).map_err(|e| format!("{}", e))?;
                let mut latest: Option<(std::time::SystemTime, PathBuf)> = None;
                for entry in rd.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        if let Ok(meta) = std::fs::metadata(&path) {
                            if let Ok(mtime) = meta.modified() {
                                if latest.as_ref().map(|(t, _)| *t < mtime).unwrap_or(true) {
                                    latest = Some((mtime, path));
                                }
                            }
                        }
                    }
                }
                match latest {
                    Some((_t, p)) => Ok(p),
                    None => Err(format!("No screenshots found in {}", dir.display())),
                }
            }
        }
    }

    async fn ocr_text(
        &self,
        img_path: &Path,
        _zone: Option<&str>,
        psm: Option<u32>,
        lang: &str,
    ) -> AnyResult<String> {
        // For now, we simply rely on tesseract with optional PSM; zone cropping can be approximated by psm but we'll leave it to OCR tool's zone as a later enhancement; here we reuse the same image and PSM.
        let mut cmd = Command::new("tesseract");
        cmd.arg(img_path).arg("stdout").arg("-l").arg(lang);
        if let Some(psm_val) = psm {
            cmd.arg("--psm").arg(psm_val.to_string());
        }
        // In a later iteration, we could reuse the preprocess logic to crop zones. For now, defer to full image and rely on rule pattern selectivity.
        let out = cmd.output().await?;
        if !out.status.success() {
            return Err(anyhow::anyhow!(
                String::from_utf8_lossy(&out.stderr).to_string()
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }
}

#[async_trait]
impl McpTool for OcrAlertsTool {
    fn name(&self) -> &str {
        "devit_ocr_alerts"
    }
    fn description(&self) -> &str {
        "Déclenche des alertes OCR (regex) sur une image (dernier screenshot par défaut), avec action optionnelle de notification."
    }

    async fn execute(&self, params: Value) -> McpResult<Value> {
        let rules_val = params
            .get("rules")
            .ok_or_else(|| validation_error("Paramètre 'rules' requis"))?;
        let rules: Vec<RuleSpec> = serde_json::from_value(rules_val.clone())
            .map_err(|e| validation_error(&format!("Paramètre 'rules' invalide: {}", e)))?;

        if rules.is_empty() {
            return Err(validation_error("Paramètre 'rules' ne peut pas être vide"));
        }

        let path_param = params.get("path").and_then(Value::as_str);
        let lang = params.get("lang").and_then(Value::as_str).unwrap_or("eng");
        let psm = params.get("psm").and_then(Value::as_u64).map(|v| v as u32);
        let inline = params
            .get("inline")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let task_id = params.get("task_id").and_then(Value::as_str);

        let img_path = self
            .resolve_image_path(path_param)
            .map_err(|e| validation_error(&e))?;

        // OCR once (full image) for now. Optional: per-rule zone OCR later for precision.
        let text = self
            .ocr_text(&img_path, None, psm, lang)
            .await
            .map_err(|e| internal_error(format!("OCR failed: {}", e)))?;

        let mut alerts = Vec::new();
        let mut any_notified = false;
        for rule in rules.iter() {
            let re = match RegexBuilder::new(&rule.pattern)
                .case_insensitive(true)
                .build()
            {
                Ok(r) => r,
                Err(e) => {
                    return Err(validation_error(&format!(
                        "Regex invalide pour rule '{}': {}",
                        rule.name, e
                    )))
                }
            };
            let mut hits = 0usize;
            let mut samples: Vec<String> = Vec::new();
            for line in text.lines() {
                if re.is_match(line) {
                    hits += 1;
                    if samples.len() < 5 {
                        samples.push(line.to_string());
                    }
                }
            }
            if hits > 0 {
                alerts.push(json!({
                    "name": rule.name,
                    "pattern": rule.pattern,
                    "zone": rule.zone,
                    "severity": rule.severity.as_deref().unwrap_or("info"),
                    "hits": hits,
                    "samples": samples,
                }));

                if rule.action.as_deref() == Some("notify") {
                    if let Some(tid) = task_id {
                        let sev = rule.severity.as_deref().unwrap_or("info");
                        let summary = format!("ALERT [{}] {} ({} hits)", sev, rule.name, hits);
                        let details = json!({"rule": rule.name, "pattern": rule.pattern, "zone": rule.zone, "severity": sev, "hits": hits, "samples": samples});
                        self.ctx
                            .notify(tid, "progress", &summary, Some(details), None)
                            .await
                            .map_err(|e| internal_error(e.to_string()))?;
                        any_notified = true;
                    }
                }
            }
        }

        let img_rel = img_path
            .strip_prefix(self.fs.root())
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| img_path.display().to_string());

        let summary = if alerts.is_empty() {
            format!("🔍 OCR Alerts — 0 match (path: {})", img_rel)
        } else {
            format!(
                "🚨 OCR Alerts — {} rule(s) matched (path: {}){}",
                alerts.len(),
                img_rel,
                if any_notified { " — notified" } else { "" }
            )
        };

        let mut content = vec![json!({"type":"text","text": summary})];
        if inline && !alerts.is_empty() {
            content.push(json!({"type":"json","json": {"alerts": alerts}}));
        }

        Ok(json!({
            "content": content,
            "structuredContent": {
                "ocrAlerts": {
                    "path": img_rel,
                    "lang": lang,
                    "psm": psm,
                    "alert_count": alerts.len(),
                    "notified": any_notified,
                    "alerts": alerts,
                }
            }
        }))
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Chemin image (défaut: dernier screenshot)"},
                "lang": {"type": "string", "default": "eng"},
                "psm": {"type": "integer"},
                "inline": {"type": "boolean", "default": true},
                "task_id": {"type": "string", "description": "Optionnel: task_id pour déclencher devit_notify(status=progress)"},
                "rules": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": {"type": "string"},
                            "pattern": {"type": "string"},
                            "zone": {"type": "string", "enum": ["terminal_bottom", "error_zone"]},
                            "severity": {"type": "string", "default": "info"},
                            "action": {"type": "string", "enum": ["notify", "none"], "default": "none"}
                        },
                        "required": ["name", "pattern"]
                    }
                }
            },
            "required": ["rules"]
        })
    }
}
