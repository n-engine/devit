use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct EffectiveLimits {
    pub timeout_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u64>,
    pub max_redirects: u32,
    pub follow_redirects: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct LimitSources {
    pub timeout_ms: &'static str, // "param" | "env" | "default"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<&'static str>, // "param" | "env" | "default"
    pub max_redirects: &'static str, // always "default" for S1
    pub follow_redirects: &'static str, // "param" | "env" | "default"
}

fn parse_env_u64(name: &str) -> Option<u64> {
    std::env::var(name).ok()?.parse::<u64>().ok()
}

fn parse_env_bool(name: &str) -> Option<bool> {
    let v = std::env::var(name).ok()?;
    match v.to_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn clamp(v: u64, min: u64, max: u64) -> u64 {
    if v < min {
        return min;
    }
    if v > max {
        return max;
    }
    v
}

/// Resolve limits for the Search tool.
/// - timeout_ms: from param → env(DEVIT_SEARCH_TIMEOUT_MS) → default(8000), clamped [100..10000]
/// - max_redirects: default 2
/// - follow_redirects: default true
pub fn resolve_search_limits(param_timeout_ms: Option<u64>) -> (EffectiveLimits, LimitSources) {
    let (timeout_ms, timeout_src) = if let Some(p) = param_timeout_ms {
        (clamp(p, 100, 10_000), "param")
    } else if let Some(envv) = parse_env_u64("DEVIT_SEARCH_TIMEOUT_MS") {
        (clamp(envv, 100, 10_000), "env")
    } else {
        (8000, "default")
    };

    let max_redirects = 2u32;
    let follow_redirects = true;

    let effective = EffectiveLimits {
        timeout_ms,
        max_bytes: None,
        max_redirects,
        follow_redirects,
    };
    let sources = LimitSources {
        timeout_ms: timeout_src,
        max_bytes: None,
        max_redirects: "default",
        follow_redirects: "default",
    };
    (effective, sources)
}

/// Resolve limits for the Fetch tool.
/// - timeout_ms: from param → env(DEVIT_FETCH_TIMEOUT_MS) → default(8000), clamped [100..10000]
/// - max_bytes: from param → env(DEVIT_FETCH_MAX_BYTES) → default(500000), clamped [1..1000000]
/// - follow_redirects: from param → env(DEVIT_FOLLOW_REDIRECTS) → default(true)
/// - max_redirects: default 2
pub fn resolve_fetch_limits(
    param_timeout_ms: Option<u64>,
    param_max_bytes: Option<u64>,
    param_follow_redirects: Option<bool>,
) -> (EffectiveLimits, LimitSources) {
    let (timeout_ms, timeout_src) = if let Some(p) = param_timeout_ms {
        (clamp(p, 100, 10_000), "param")
    } else if let Some(envv) = parse_env_u64("DEVIT_FETCH_TIMEOUT_MS") {
        (clamp(envv, 100, 10_000), "env")
    } else {
        (8000, "default")
    };

    let (max_bytes, max_bytes_src) = if let Some(p) = param_max_bytes {
        (Some(clamp(p, 1, 1_000_000)), Some("param"))
    } else if let Some(envv) = parse_env_u64("DEVIT_FETCH_MAX_BYTES") {
        (Some(clamp(envv, 1, 1_000_000)), Some("env"))
    } else {
        (Some(500_000), Some("default"))
    };

    let (follow_redirects, fr_src) = if let Some(p) = param_follow_redirects {
        (p, "param")
    } else if let Some(envv) = parse_env_bool("DEVIT_FOLLOW_REDIRECTS") {
        (envv, "env")
    } else {
        (true, "default")
    };

    let max_redirects = 2u32;

    let effective = EffectiveLimits {
        timeout_ms,
        max_bytes,
        max_redirects,
        follow_redirects,
    };
    let sources = LimitSources {
        timeout_ms: timeout_src,
        max_bytes: max_bytes_src,
        max_redirects: "default",
        follow_redirects: fr_src,
    };
    (effective, sources)
}
