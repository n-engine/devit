use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use crate::journal_best_effort as jbe;
use async_trait::async_trait;
use chrono::Utc;
use devit_common::cache::cache_key;
use devit_common::limits::{resolve_search_limits, EffectiveLimits, LimitSources};
use mcp_core::{McpError, McpResult, McpTool};
use regex::Regex;
use reqwest::redirect::Policy as RedirectPolicy;
use reqwest::Client;
use serde_json::{json, Value};
use tracing::info;
use url::Url;
use uuid::Uuid;

/// MCP tool: devit_search_web — DDG-backed SERP (HTML) with minimal parsing
pub struct SearchWebTool {
    engine: String,
}

impl SearchWebTool {
    pub fn new_default() -> Arc<Self> {
        let engine = std::env::var("DEVIT_ENGINE").unwrap_or_else(|_| "ddg".to_string());
        Arc::new(Self { engine })
    }

    fn query_firewall_blocked(query: &str) -> Option<&'static str> {
        let lowered = query.to_lowercase();
        if lowered.contains("file://") || lowered.contains("s3://") {
            return Some("BLOCKED_QUERY");
        }
        // Private IPv4 ranges
        if Regex::new(r"\b(10\.|192\.168\.|172\.(1[6-9]|2\d|3[0-1])\.)")
            .ok()
            .as_ref()
            .map(|re| re.is_match(&lowered))
            == Some(true)
        {
            return Some("BLOCKED_QUERY");
        }
        // JWT-like
        if Regex::new(r"eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}")
            .ok()
            .as_ref()
            .map(|re| re.is_match(&lowered))
            == Some(true)
        {
            return Some("BLOCKED_QUERY");
        }
        None
    }

    fn build_client(timeout_ms: u64, redirects: usize, agent: &str) -> McpResult<Client> {
        let client = reqwest::Client::builder()
            .user_agent(agent)
            .redirect(RedirectPolicy::limited(redirects))
            .timeout(std::time::Duration::from_millis(timeout_ms))
            .gzip(true)
            .brotli(true)
            .deflate(true)
            .build()
            .map_err(|e| McpError::Internal(format!("http client error: {e}")))?;
        Ok(client)
    }

    fn ddg_base() -> String {
        std::env::var("DEVIT_DDG_BASE")
            .unwrap_or_else(|_| "https://duckduckgo.com/html".to_string())
    }

    fn user_agent() -> String {
        std::env::var("DEVIT_HTTP_USER_AGENT").unwrap_or_else(|_| "DevItBot/1.0".to_string())
    }

    fn max_per_domain(safe_mode: &str) -> usize {
        match safe_mode {
            "off" => usize::MAX,
            "moderate" => 3,
            _ => 2, // strict (default)
        }
    }

    fn parse_ddg_results(html: &str, limit: usize, max_per_domain: usize) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut per_domain: HashMap<String, usize> = HashMap::new();

        // Capture the DDG redirect link and title
        let re =
            Regex::new(r#"<a[^>]+href=\"(https://duckduckgo\.com/l/[^\"]+)\"[^>]*>(.*?)</a>"#).ok();

        if let Some(re) = re {
            for caps in re.captures_iter(html) {
                let href = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                let title_raw = caps.get(2).map(|m| m.as_str()).unwrap_or("");
                let Ok(ddg_url) = Url::parse(href) else {
                    continue;
                };
                let mut target: Option<String> = None;
                for (k, v) in ddg_url.query_pairs() {
                    if k == "uddg" {
                        target = Some(v.into_owned()); // percent-decoded by url crate
                        break;
                    }
                }
                let Some(decoded) = target else { continue };
                if seen.contains(&decoded) {
                    continue;
                }
                let domain = domain_of(&decoded).unwrap_or_else(|| "".to_string());
                let count = per_domain.get(&domain).cloned().unwrap_or(0);
                if count >= max_per_domain {
                    continue;
                }
                let title = html_unescape(title_raw);
                out.push((decoded.clone(), title));
                seen.insert(decoded);
                per_domain.insert(domain, count + 1);
                if out.len() >= limit {
                    break;
                }
            }
        }

        out
    }
}

#[async_trait]
impl McpTool for SearchWebTool {
    fn name(&self) -> &str {
        "devit_search_web"
    }

    fn description(&self) -> &str {
        "Search the web (SERP) via DuckDuckGo HTML with safety guards"
    }

    async fn execute(&self, params: Value) -> McpResult<Value> {
        let query = params
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| McpError::InvalidRequest("'query' is required".into()))?;

        if let Some(code) = Self::query_firewall_blocked(query) {
            return Err(McpError::rpc(
                -32600,
                "Query blocked by safety policy",
                Some(json!({
                    "code": code,
                    "message": "The provided query is not allowed",
                    "hint": "Remove internal URLs, private IPs or secret-like tokens"
                })),
            ));
        }

        let max_results = params
            .get("max_results")
            .and_then(Value::as_u64)
            .map(|n| n.min(20).max(1) as usize)
            .unwrap_or(5);
        let include_content = params
            .get("include_content")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let (effective_limits, limit_sources): (EffectiveLimits, LimitSources) =
            resolve_search_limits(params.get("timeout_ms").and_then(Value::as_u64));
        let timeout_ms = effective_limits.timeout_ms;
        let safe_mode = params
            .get("safe_mode")
            .and_then(Value::as_str)
            .unwrap_or("strict");

        let trace_id = Uuid::new_v4().to_string();
        let start = Instant::now();
        let ddg_endpoint = Self::ddg_base();
        let user_agent = Self::user_agent();
        let client = Self::build_client(
            timeout_ms,
            effective_limits.max_redirects as usize,
            &user_agent,
        )?;

        // Build URL
        let mut url = Url::parse(&ddg_endpoint)
            .map_err(|e| McpError::InvalidRequest(format!("invalid DDG base: {e}")))?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("q", query);
        }
        // Precompute cache key for logging/metadata
        let accept = "text/html";
        let cache_key_val = cache_key(query, accept, &user_agent, safe_mode, include_content);

        let resp = client.get(url.clone()).send().await;
        let (results_json, partial, elapsed_ms) = match resp {
            Ok(r) => {
                let status = r.status();
                let body = r.text().await.unwrap_or_default();
                if !status.is_success() {
                    info!(target: "mcp.search", %trace_id, op="search", engine=%self.engine, code=%status.as_u16(), elapsed_ms=%start.elapsed().as_millis() as u64, effective_limits=?effective_limits, limit_sources=?limit_sources, delegation_context=?None::<()> , cache_key=%cache_key_val, "search http error");
                    (Vec::new(), true, start.elapsed().as_millis() as u64)
                } else {
                    let pairs = Self::parse_ddg_results(
                        &body,
                        max_results,
                        Self::max_per_domain(safe_mode),
                    );
                    let results_json: Vec<Value> = pairs
                        .into_iter()
                        .enumerate()
                        .map(|(i, (url, title))| {
                            json!({
                                "rank": i + 1,
                                "title": title,
                                "url": url,
                                "domain": domain_of(&url).unwrap_or_default()
                            })
                        })
                        .collect();

                    (results_json, false, start.elapsed().as_millis() as u64)
                }
            }
            Err(e) => {
                info!(target: "mcp.search", %trace_id, op="search", engine=%self.engine, err=%e.to_string(), elapsed_ms=%start.elapsed().as_millis() as u64, effective_limits=?effective_limits, limit_sources=?limit_sources, delegation_context=?None::<()> , cache_key=%cache_key_val, "search request failed");
                (Vec::new(), true, start.elapsed().as_millis() as u64)
            }
        };

        let retrieved_at = Utc::now().to_rfc3339();
        let meta = json!({
            "engine": self.engine,
            "trace_id": trace_id,
            "partial": partial,
            "cache": "miss",
            "elapsed_ms": elapsed_ms,
            "effective_limits": effective_limits,
            "limit_sources": limit_sources,
            "delegation_context": serde_json::Value::Null,
            "cache_key": cache_key_val
        });
        // Journal best-effort
        jbe::append(
            "search",
            &json!({
                "query": query,
                "retrieved_at": retrieved_at,
                "results_count": results_json.len(),
                "meta": meta
            }),
        );

        let out = json!({
            "content": [
                {
                    "type": "text",
                    "text": format!("Query: '{}'\nmax_results: {} (include_content: {}, safe_mode: {})", query, max_results, include_content, safe_mode)
                }
            ],
            "metadata": {
                "query": query,
                "retrieved_at": retrieved_at,
                "results": results_json,
                "meta": meta
            }
        });

        Ok(out)
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "minLength": 1},
                "max_results": {"type": "integer", "minimum": 1, "maximum": 20},
                "include_content": {"type": "boolean"},
                "timeout_ms": {"type": "integer", "minimum": 100, "maximum": 10000},
                "safe_mode": {"type": "string", "enum": ["strict", "moderate", "off"]}
            },
            "required": ["query"]
        })
    }
}

// --- helpers ---

fn html_unescape(s: &str) -> String {
    let mut out = s
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">");
    out = out.replace("&quot;", "\"").replace("&#39;", "'");
    if let Ok(re) = Regex::new(r"<[^>]+>") {
        out = re.replace_all(&out, "").to_string();
    }
    out
}

fn domain_of(u: &str) -> Option<String> {
    Url::parse(u)
        .ok()
        .and_then(|p| p.domain().map(|d| d.to_string()))
}
