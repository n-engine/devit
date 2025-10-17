use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use image::{imageops::FilterType, DynamicImage, GenericImageView};
use mcp_core::{McpResult, McpTool};
use serde_json::{json, Value};
use tokio::process::Command;

use crate::file_read::FileSystemContext;
use crate::{internal_error, validation_error};

pub struct OcrTool {
    fs: Arc<FileSystemContext>,
}

impl OcrTool {
    pub fn new(fs: Arc<FileSystemContext>) -> Self {
        Self { fs }
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
                // Pick most recent file from .devit/screenshots
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
                    None => Err(format!(
                        "No screenshots found in {}. Provide a path.",
                        dir.display()
                    )),
                }
            }
        }
    }
}

#[async_trait]
impl McpTool for OcrTool {
    fn name(&self) -> &str {
        "devit_ocr"
    }

    fn description(&self) -> &str {
        "Extrait du texte d'une image (OCR via tesseract). Par défaut, lit le dernier screenshot."
    }

    async fn execute(&self, params: Value) -> McpResult<Value> {
        if !params.is_null() && !params.is_object() {
            return Err(validation_error(
                "Les paramètres doivent être un objet JSON (ou omis).",
            ));
        }

        let path_param = params.get("path").and_then(Value::as_str);
        let lang = params.get("lang").and_then(Value::as_str).unwrap_or("eng");
        let psm = params.get("psm").and_then(Value::as_u64);
        let oem = params.get("oem").and_then(Value::as_u64);
        let max_chars = params
            .get("max_chars")
            .and_then(Value::as_u64)
            .unwrap_or(2000) as usize;
        let format = params
            .get("format")
            .and_then(Value::as_str)
            .unwrap_or("text");
        let output_path = params.get("output_path").and_then(Value::as_str);
        let explicit_inline = params.get("inline").and_then(Value::as_bool);
        let silent = params
            .get("silent")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let inline = explicit_inline.unwrap_or(true) && !silent;

        if !matches!(format, "text" | "tsv" | "hocr") {
            return Err(validation_error(
                "Paramètre 'format' invalide (attendu: text|tsv|hocr)",
            ));
        }

        let img_path = self
            .resolve_image_path(path_param)
            .map_err(|e| validation_error(&e))?;

        // Optional preprocessing pipeline
        let mut tesseract_input_path = img_path.clone();
        let mut temp_path: Option<PathBuf> = None;
        let preprocess_cfg = params.get("preprocess");
        let zone = params.get("zone").and_then(Value::as_str);
        let do_preprocess = match preprocess_cfg {
            Some(Value::Bool(b)) => *b,
            Some(Value::Object(_)) => true,
            _ => false,
        };
        let do_preprocess = do_preprocess || zone.is_some();
        if do_preprocess {
            // Extract options
            let pp = preprocess_cfg.and_then(|v| v.as_object());
            let grayscale = pp
                .and_then(|m| m.get("grayscale").and_then(Value::as_bool))
                .unwrap_or(true);
            let threshold = pp
                .and_then(|m| m.get("threshold").and_then(Value::as_u64))
                .map(|v| v.min(255) as u8);
            let resize_width = pp
                .and_then(|m| m.get("resize_width").and_then(Value::as_u64))
                .map(|v| v as u32);
            let crop = pp.and_then(|m| m.get("crop").and_then(Value::as_object));
            let mut crop_params = crop.map(|m| {
                (
                    m.get("x").and_then(Value::as_u64).unwrap_or(0) as u32,
                    m.get("y").and_then(Value::as_u64).unwrap_or(0) as u32,
                    m.get("width").and_then(Value::as_u64),
                    m.get("height").and_then(Value::as_u64),
                )
            });

            match image::open(&img_path) {
                Ok(mut img) => {
                    // If no explicit crop and a zone is specified, derive crop from template
                    if crop_params.is_none() {
                        if let Some(z) = zone {
                            let (iw, ih) = img.dimensions();
                            match z {
                                // Bottom 35% of the screen (full width)
                                "terminal_bottom" => {
                                    let y = ((ih as f32) * 0.65) as u32;
                                    crop_params =
                                        Some((0, y, Some(iw as u64), Some((ih - y) as u64)));
                                }
                                // Central band: 50% width, 40% height, centered
                                "error_zone" => {
                                    let w = ((iw as f32) * 0.5) as u32;
                                    let h = ((ih as f32) * 0.4) as u32;
                                    let x = (iw - w) / 2;
                                    let y = ((ih as i64) * 2 / 10).max(0) as u32; // ~20% from top
                                    crop_params = Some((x, y, Some(w as u64), Some(h as u64)));
                                }
                                _ => {}
                            }
                        }
                    }

                    // Crop first (if provided or resolved via zone)
                    if let Some((x, y, w_opt, h_opt)) = crop_params {
                        let (iw, ih) = img.dimensions();
                        let w = w_opt.map(|v| v as u32).unwrap_or(iw.saturating_sub(x));
                        let h = h_opt.map(|v| v as u32).unwrap_or(ih.saturating_sub(y));
                        let cx = x.min(iw);
                        let cy = y.min(ih);
                        let cw = w.min(iw.saturating_sub(cx));
                        let ch = h.min(ih.saturating_sub(cy));
                        let sub = image::imageops::crop_imm(&img, cx, cy, cw, ch).to_image();
                        img = DynamicImage::ImageRgba8(sub);
                    }

                    // Convert to grayscale if requested
                    if grayscale {
                        img = DynamicImage::ImageLuma8(img.to_luma8());
                    }

                    // Threshold if requested
                    if let Some(th) = threshold {
                        let mut gray = img.to_luma8();
                        for p in gray.pixels_mut() {
                            let v = p[0];
                            p[0] = if v >= th { 255 } else { 0 };
                        }
                        img = DynamicImage::ImageLuma8(gray);
                    }

                    // Resize if requested
                    if let Some(tw) = resize_width {
                        let (w, h) = img.dimensions();
                        let nw = tw.min(w.max(1));
                        let nh = ((h as f32) * (nw as f32 / w.max(1) as f32))
                            .round()
                            .max(1.0) as u32;
                        img = img.resize(nw, nh, FilterType::CatmullRom);
                    }

                    // Persist to temp file for tesseract input
                    let ts = Utc::now().format("%Y%m%dT%H%M%S");
                    let rel = PathBuf::from(".devit")
                        .join("ocr")
                        .join(format!("preproc-{}.png", ts));
                    let abs = self.fs.root().join(&rel);
                    if let Some(parent) = abs.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    if let Err(e) = img.save(&abs) {
                        // If save fails, fallback to original
                        tracing::warn!(
                            "Failed to save preprocessed image: {} (fallback to original)",
                            e
                        );
                    } else {
                        tesseract_input_path = abs.clone();
                        temp_path = Some(abs);
                    }
                }
                Err(e) => {
                    tracing::warn!("Preprocess load failed: {} (fallback to original)", e);
                }
            }
        }

        // Build tesseract command
        let mut cmd = Command::new("tesseract");
        cmd.arg(&tesseract_input_path)
            .arg("stdout")
            .arg("-l")
            .arg(lang);
        if let Some(psm_val) = psm {
            cmd.arg("--psm").arg(psm_val.to_string());
        }
        if let Some(oem_val) = oem {
            cmd.arg("--oem").arg(oem_val.to_string());
        }
        if format != "text" {
            cmd.arg(format);
        }

        let output = cmd
            .output()
            .await
            .map_err(|e| internal_error(e.to_string()))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(internal_error(format!(
                "tesseract failed (code {:?}): {}",
                output.status.code(),
                stderr
            )));
        }
        let mut text = String::from_utf8_lossy(&output.stdout).to_string();
        // Decide saved_to path: user-provided, or auto when inline=false (silent mode)
        let mut saved_to: Option<PathBuf> = None;
        let desired_ext = match format {
            "text" => "txt",
            "tsv" => "tsv",
            _ => "html",
        };
        if let Some(path_str) = output_path {
            let pb = Path::new(path_str);
            let abs = if pb.is_absolute() {
                pb.to_path_buf()
            } else {
                self.fs.root().join(pb)
            };
            if !abs.starts_with(self.fs.root()) {
                return Err(validation_error(
                    "'output_path' doit être à l'intérieur du workspace",
                ));
            }
            if let Some(parent) = abs.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::write(&abs, &text)
                .map_err(|e| internal_error(format!("Écriture output_path échouée: {}", e)))?;
            saved_to = Some(abs);
        } else if !inline {
            // Silent mode without explicit path: auto-generate under .devit/ocr/
            let base = img_path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "ocr".into());
            let ts = Utc::now().format("%Y%m%dT%H%M%S");
            let rel = PathBuf::from(".devit")
                .join("ocr")
                .join(format!("{}-{}.{}", base, ts, desired_ext));
            let abs = self.fs.root().join(&rel);
            if let Some(parent) = abs.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::write(&abs, &text)
                .map_err(|e| internal_error(format!("Écriture output_path échouée: {}", e)))?;
            saved_to = Some(abs);
        }
        let full_len = text.len();
        let truncated = if text.len() > max_chars {
            text.truncate(max_chars);
            true
        } else {
            false
        };

        let img_rel = img_path
            .strip_prefix(self.fs.root())
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| img_path.display().to_string());
        let saved_rel = saved_to.as_ref().map(|p| {
            p.strip_prefix(self.fs.root())
                .map(|q| q.display().to_string())
                .unwrap_or_else(|_| p.display().to_string())
        });
        let summary = if let Some(ref s) = saved_rel {
            format!(
                "📝 OCR extrait {} caractères — {} (lang: {}, format: {}) → sauvegardé: {}{}",
                full_len,
                img_rel,
                lang,
                format,
                s,
                if !inline && truncated {
                    ""
                } else if truncated {
                    " (tronqué)"
                } else {
                    ""
                }
            )
        } else {
            format!(
                "📝 OCR extrait {} caractères — {} (lang: {}, format: {}){}",
                full_len,
                img_rel,
                lang,
                format,
                if truncated { " (tronqué)" } else { "" }
            )
        };

        let mut content = vec![json!({"type":"text","text": summary})];
        if inline {
            content.push(json!({"type":"text","text": text}));
        }

        let result = json!({
            "content": content,
            "structuredContent": {
                "ocr": {
                    "path": img_path
                        .strip_prefix(self.fs.root())
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|_| img_path.display().to_string()),
                    "engine": "tesseract",
                    "lang": lang,
                    "psm": psm,
                    "oem": oem,
                    "format": format,
                    "chars": full_len,
                    "truncated": truncated,
                    "inline": inline,
                    "saved_to": saved_rel
                }
            }
        });

        // Cleanup temp preprocessed file
        if let Some(p) = temp_path {
            let _ = std::fs::remove_file(p);
        }

        Ok(result)
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Chemin image (par défaut: dernier screenshot)"},
                "lang": {"type": "string", "description": "Langue tesseract (ex: eng, fra)", "default": "eng"},
                "psm": {"type": "integer", "description": "Page segmentation mode (tesseract --psm)"},
                "oem": {"type": "integer", "description": "OCR Engine mode (tesseract --oem)"},
                "max_chars": {"type": "integer", "description": "Taille max du texte renvoyé dans la réponse (si inline=true)", "default": 2000},
                "format": {"type": "string", "enum": ["text", "tsv", "hocr"], "default": "text"},
                "inline": {"type": "boolean", "description": "Inclure un extrait texte dans la réponse", "default": true},
                "silent": {"type": "boolean", "description": "Alias de inline=false (force le mode silencieux)", "default": false},
                "output_path": {"type": "string", "description": "Chemin où enregistrer la sortie complète (txt/tsv/html)"},
                "zone": {"type": "string", "description": "Zone template: terminal_bottom | error_zone", "enum": ["terminal_bottom", "error_zone"]},
                "preprocess": {
                    "type": ["boolean", "object"],
                    "description": "Activer le prétraitement (grayscale/threshold/resize/crop)",
                    "properties": {
                        "grayscale": {"type": "boolean", "default": true},
                        "threshold": {"type": "integer", "minimum": 0, "maximum": 255},
                        "resize_width": {"type": "integer", "minimum": 1},
                        "crop": {
                            "type": "object",
                            "properties": {
                                "x": {"type": "integer", "minimum": 0},
                                "y": {"type": "integer", "minimum": 0},
                                "width": {"type": "integer", "minimum": 1},
                                "height": {"type": "integer", "minimum": 1}
                            }
                        }
                    }
                }
            },
            "additionalProperties": false
        })
    }
}
