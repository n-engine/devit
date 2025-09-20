use anyhow::Result;
use clap::Parser;
use serde::Serialize;
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

#[derive(Serialize)]
struct ErrorResp<'a>
{
    status: &'a str,
    message: String,
}

fn main() -> Result<()>
{
    let cli = Cli::parse();

    if cli.schema
    {
        let schema = include_str!("../devit_api.schema.json");
        let mut out = std::io::stdout().lock();
        out.write_all(schema.as_bytes())?;
        out.write_all(b"\n")?;
        out.flush()?;
        return Ok(());
    }

    // Read exactly one JSON object from stdin
    let mut buf = String::new();
    std::io::stdin().lock().read_to_string(&mut buf)?;

    let parsed: Value = match serde_json::from_str(&buf)
    {
        Ok(v) => v,
        Err(e) =>
        {
            return print_one_json(&json!(ErrorResp {
                status: "error",
                message: format!("invalid JSON: {e}")
            }));
        }
    };

    // Extract action
    let action = match parsed.get("action").and_then(|v| v.as_str())
    {
        Some(a) => a.to_string(),
        None =>
        {
            return print_one_json(&json!(ErrorResp {
                status: "error",
                message: "missing field: action".to_string()
            }));
        }
    };

    // Parameters can be under "input" (preferred) or "params"
    let params = parsed
        .get("input")
        .cloned()
        .or_else(|| parsed.get("params").cloned())
        .unwrap_or_else(|| json!({}));

    // Dispatch
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
            print_one_json(&resp)
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
            print_one_json(&resp)
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
            print_one_json(&resp)
        }
        "journal_append" =>
        {
            let resp = json!({
                "status": "ok",
                "action": "journal_append",
                "hmac": "deadbeef"
            });
            print_one_json(&resp)
        }
        other =>
        {
            print_one_json(&json!(ErrorResp {
                status: "error",
                message: format!("unknown action: {other}")
            }))
        }
    }
}

fn print_one_json(v: &Value) -> Result<()>
{
    let mut out = std::io::stdout().lock();
    serde_json::to_writer(&mut out, v)?;
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}

