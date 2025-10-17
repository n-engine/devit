use std::env;

use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use devit_cli::core::config::ScreenshotToolConfig;
use devit_common::orchestration::types::{OrchestrationConfig, DEFAULT_DAEMON_SOCKET};
use devitd_client::{DevitClient, ScreenshotResponse};
use image::codecs::png::PngEncoder;
use image::ImageEncoder;
use image::{self, imageops::FilterType, ColorType, DynamicImage, GenericImageView};
use mcp_core::{McpResult, McpTool};
use serde_json::{json, Value};
use tokio::fs;
use uuid::Uuid;

use crate::{internal_error, validation_error};

pub struct ScreenshotTool {
    socket: String,
    secret: String,
}

impl ScreenshotTool {
    pub fn from_config(
        tool_cfg: &ScreenshotToolConfig,
        orchestration_cfg: &OrchestrationConfig,
    ) -> Result<Option<Self>, String> {
        if !(tool_cfg.enabled && orchestration_cfg.capabilities.screenshot.enabled) {
            return Ok(None);
        }

        let socket = orchestration_cfg
            .daemon_socket
            .clone()
            .unwrap_or_else(|| DEFAULT_DAEMON_SOCKET.to_string());
        let secret =
            env::var("DEVIT_SECRET").unwrap_or_else(|_| "change-me-in-production".to_string());
        Ok(Some(Self { socket, secret }))
    }

    async fn capture(&self) -> Result<ScreenshotResponse, String> {
        let ident = format!("mcp-screenshot-{}", Uuid::new_v4());
        let client = DevitClient::connect_with_capabilities(
            &self.socket,
            &ident,
            &self.secret,
            Some("mcp-screenshot".to_string()),
            vec!["screenshot".to_string()],
        )
        .await
        .map_err(|err| format!("connection to devitd failed: {err}"))?;

        client
            .capture_screenshot()
            .await
            .map_err(|err| format!("screenshot capture failed: {err}"))
    }
}

// Build a PNG thumbnail embed for MCP content with size budget and width constraint.
// Returns (image_block, thumbnail_meta) when within budget.
fn build_thumbnail_embed(
    img: &DynamicImage,
    thumb_width: u32,
    max_inline_kb: u64,
) -> Option<(serde_json::Value, serde_json::Value)> {
    let (w, h) = img.dimensions();
    let target_w = if w > thumb_width { thumb_width } else { w };
    let target_h = ((h as f32) * (target_w as f32 / w as f32)).round() as u32;
    let resized: DynamicImage = if target_w < w {
        img.resize(target_w, target_h, FilterType::Triangle)
    } else {
        img.clone()
    };
    let mut buf = Vec::new();
    let (tw, th) = resized.dimensions();
    let rgba = resized.to_rgba8();
    if PngEncoder::new(&mut buf)
        .write_image(&rgba, tw, th, ColorType::Rgba8.into())
        .is_err()
    {
        return None;
    }
    if buf.len() as u64 <= (max_inline_kb * 1024) {
        let b64 = BASE64.encode(&buf);
        let img_block = json!({ "type": "image", "data": b64, "mimeType": "image/png" });
        let meta = json!({
            "format": "png",
            "width": target_w,
            "height": target_h,
            "bytes": buf.len(),
        });
        Some((img_block, meta))
    } else {
        None
    }
}

#[async_trait]
impl McpTool for ScreenshotTool {
    fn name(&self) -> &str {
        "devit_screenshot"
    }

    fn description(&self) -> &str {
        "Capture un screenshot du bureau actuel via devitd (supporte plusieurs backends)."
    }

    async fn execute(&self, params: Value) -> McpResult<Value> {
        if !params.is_null() && !params.is_object() {
            return Err(validation_error(
                "Les paramètres doivent être un objet JSON (ou omis).",
            ));
        }

        let response = self
            .capture()
            .await
            .map_err(|err| internal_error(err.to_string()))?;

        let human_size = response
            .human_size
            .clone()
            .unwrap_or_else(|| format!("{:.2} MB", response.bytes as f64 / (1024.0 * 1024.0)));
        let path_str = response.path.clone();
        let format_str = response.format.clone();
        let total_bytes = response.bytes;

        // Options d'inline
        let inline = params
            .get("inline")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let max_inline_kb = params
            .get("max_inline_kb")
            .and_then(Value::as_u64)
            .unwrap_or(512);
        let thumb_width = params
            .get("thumb_width")
            .and_then(Value::as_u64)
            .unwrap_or(480) as u32;
        let _thumb_quality = params
            .get("thumb_quality")
            .and_then(Value::as_u64)
            .unwrap_or(80) as u8;
        // Nous embarquons un thumbnail si inline=true et que le thumbnail tient dans le budget.
        // Le fichier original reste sur disque et est référencé dans le bloc JSON.
        let should_try_thumbnail = inline;

        // Lire, réduire et encoder l'image en base64 (thumbnail JPEG) si demandé
        let (image_block, thumb_meta) = if should_try_thumbnail {
            let bytes = fs::read(&response.path)
                .await
                .map_err(|err| internal_error(format!("Failed to read screenshot file: {err}")))?;
            match image::load_from_memory(&bytes) {
                Ok(img) => match build_thumbnail_embed(&img, thumb_width, max_inline_kb) {
                    Some((img_block, meta)) => (Some(img_block), Some(meta)),
                    None => (None, None),
                },
                Err(_e) => (None, None),
            }
        } else {
            (None, None)
        };

        let mut content = vec![json!({
            "type": "text",
            "text": format!("📸 Capture enregistrée ({human_size}) — {path}", path = path_str)
        })];
        if let Some(ref img) = image_block {
            content.push(img.clone());
        }

        let meta = json!({
            "embedded": image_block.is_some(),
            "format": format_str,
            "inline": inline,
            "path": path_str,
            "size": { "bytes": total_bytes, "human": human_size },
            "thumbnail": thumb_meta
        });

        Ok(json!({
            "content": content,
            "structuredContent": { "image": meta }
        }))
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "inline": { "type": "boolean", "description": "Inclure un thumbnail en base64 si taille compatible", "default": true },
                "max_inline_kb": { "type": "integer", "description": "Seuil max (KB) pour l'inline base64", "default": 512 },
                "thumb_width": { "type": "integer", "description": "Largeur max du thumbnail (px)", "default": 480 },
                "thumb_quality": { "type": "integer", "description": "Qualité JPEG du thumbnail (1-100)", "default": 80 }
            },
            "additionalProperties": false
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnail_embeds_under_threshold() {
        // Create a small synthetic image
        let img = DynamicImage::new_rgba8(64, 32);
        let res = build_thumbnail_embed(&img, 480, 64); // 64KB budget
        assert!(res.is_some(), "expected thumbnail to embed under threshold");
        let (_img_block, meta) = res.unwrap();
        assert_eq!(meta.get("width").and_then(|v| v.as_u64()), Some(64));
        assert_eq!(meta.get("height").and_then(|v| v.as_u64()), Some(32));
    }

    #[test]
    fn no_upscale_when_thumb_wider_than_image() {
        let img = DynamicImage::new_rgba8(120, 60);
        let res = build_thumbnail_embed(&img, 480, 64);
        let (_img_block, meta) = res.expect("embed expected");
        // Width should remain the original (no upscale)
        assert_eq!(meta.get("width").and_then(|v| v.as_u64()), Some(120));
        assert_eq!(meta.get("height").and_then(|v| v.as_u64()), Some(60));
    }

    #[test]
    fn thumbnail_skipped_if_budget_too_small() {
        let img = DynamicImage::new_rgba8(256, 256);
        // Budget zero enforces no inline, regardless of image size
        let res = build_thumbnail_embed(&img, 128, 0);
        assert!(
            res.is_none(),
            "thumbnail should be skipped when budget too small"
        );
    }
}
