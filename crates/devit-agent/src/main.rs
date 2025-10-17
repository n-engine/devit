use anyhow::Result;
use clap::Parser;
use serde_json::{json, Value};
use std::io::{Read, Write};

#[derive(Parser, Debug)]
#[command(
    name = "devit-cli",
    version,
    about = "Minimal JSON-only CLI for DevIt"
)]
struct Cli
{
    /// Print the embedded JSON schema and exit
    #[arg(long = "schema")]
    schema: bool,
}

fn main() -> Result<()>
{
    let cli = Cli::parse();

    if cli.schema
    {
        let schema = include_str!("../../../devit_api.schema.json");
        let mut out = std::io::stdout().lock();
        out.write_all(schema.as_bytes())?;
        out.write_all(b"\n")?;
        out.flush()?;
        return Ok(());
    }

    let mut buf = String::new();
    std::io::stdin().lock().read_to_string(&mut buf)?;

    let parsed: Value = match serde_json::from_str(&buf)
    {
        Ok(v) => v,
        Err(e) =>
        {
            return emit_json(&json!({
                "status": "error",
                "message": format!("invalid JSON: {e}")
            }));
        }
    };

    let action = match parsed.get("action").and_then(|v| v.as_str())
    {
        Some(a) => a.to_string(),
        None =>
        {
            return emit_json(&json!({
                "status": "error",
                "message": "missing field: action"
            }));
        }
    };

    let params = parsed
        .get("input")
        .cloned()
        .unwrap_or_else(|| json!({}));

    match action.as_str()
    {
        "apply_patch" =>
        {
            let resp = json!({
                "status": "ok",
                "action": "apply_patch",
                "files_changed": 0,
                "rollback_hint": "git apply -R last.diff"
            });
            emit_json(&resp)
        }
        "run_recipe" =>
        {
            let name = params
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let resp = json!({
                "status": "ok",
                "action": "run_recipe",
                "name": name
            });
            emit_json(&resp)
        }
        "open_view" =>
        {
            let target = params
                .get("target")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let resp = json!({
                "status": "ok",
                "action": "open_view",
                "target": target
            });
            emit_json(&resp)
        }
        "journal_append" =>
        {
            let resp = json!({
                "status": "ok",
                "action": "journal_append",
                "hmac": "deadbeef"
            });
            emit_json(&resp)
        }
        other =>
        {
            emit_json(&json!({
                "status": "error",
                "message": format!("unknown action: {other}")
            }))
        }
    }
}

fn emit_json(v: &Value) -> Result<()>
{
    let mut out = std::io::stdout().lock();
    serde_json::to_writer(&mut out, v)?;
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}

