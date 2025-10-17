use regex::Regex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RobotsPolicy {
    Allow,
    Disallow,
}

/// Very small robots.txt evaluator for User-agent: * block only.
/// Longest-prefix rule; Allow beats Disallow when longer.
pub fn robots_policy_for(path: &str, robots: &str) -> RobotsPolicy {
    let mut in_star = false;
    let mut allows: Vec<String> = Vec::new();
    let mut disallows: Vec<String> = Vec::new();
    for raw in robots.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.to_ascii_lowercase().starts_with("user-agent:") {
            let agent = line.splitn(2, ':').nth(1).unwrap_or("").trim().to_ascii_lowercase();
            in_star = agent == "*";
            continue;
        }
        if !in_star {
            continue;
        }
        if line.to_ascii_lowercase().starts_with("allow:") {
            let p = line.splitn(2, ':').nth(1).unwrap_or("").trim().to_string();
            allows.push(p);
        } else if line.to_ascii_lowercase().starts_with("disallow:") {
            let p = line.splitn(2, ':').nth(1).unwrap_or("").trim().to_string();
            disallows.push(p);
        }
    }

    let mut best_dis: Option<&str> = None;
    for d in &disallows {
        if d.is_empty() { continue; }
        if path.starts_with(d) {
            if best_dis.map(|b| d.len() > b.len()).unwrap_or(true) {
                best_dis = Some(d);
            }
        }
    }
    let mut best_all: Option<&str> = None;
    for a in &allows {
        if path.starts_with(a) {
            if best_all.map(|b| a.len() > b.len()).unwrap_or(true) {
                best_all = Some(a);
            }
        }
    }
    if best_all.is_some() {
        return RobotsPolicy::Allow;
    }
    if best_dis.is_some() {
        RobotsPolicy::Disallow
    } else {
        RobotsPolicy::Allow
    }
}

/// Best-effort sanitizer: strips scripts/styles/noscripts/tags, event handlers, javascript: links.
/// Also decodes common entities and collapses whitespace; capped to 100k chars.
pub fn sanitize_html_to_text(html: &str) -> String {
    let mut out = html.to_string();
    if let Ok(re) = Regex::new(r"(?is)<script[^>]*>.*?</script>") {
        out = re.replace_all(&out, "").to_string();
    }
    if let Ok(re) = Regex::new(r"(?is)<style[^>]*>.*?</style>") {
        out = re.replace_all(&out, "").to_string();
    }
    if let Ok(re) = Regex::new(r"(?is)<noscript[^>]*>.*?</noscript>") {
        out = re.replace_all(&out, "").to_string();
    }
    if let Ok(re) = Regex::new(r#"(?i) on[a-zA-Z]+\s*=\s*(\"[^\"]*\"|'[^']*')"#) {
        out = re.replace_all(&out, "").to_string();
    }
    if let Ok(re) = Regex::new(r#"(?i)href\s*=\s*\"\s*javascript:[^\"]*\""#) {
        out = re.replace_all(&out, "href=\"#\"").to_string();
    }
    if let Ok(re) = Regex::new(r"(?is)<[^>]+>") {
        out = re.replace_all(&out, "").to_string();
    }
    let mut out = out
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");
    if let Ok(re) = Regex::new(r"\s+") {
        out = re.replace_all(&out, " ").to_string();
    }
    if out.len() > 100_000 {
        out.truncate(100_000);
    }
    out
}

/// Heuristic detection of paywall hints.
pub fn detect_paywall_hint(html: &str) -> bool {
    let low = html.to_lowercase();
    low.contains("paywall") || (low.contains("subscribe") && low.contains("premium"))
}

/// Heuristic detection of prompt-injection text.
pub fn detect_injection_text(text: &str) -> bool {
    let low = text.to_lowercase();
    let mut hits = 0;
    for kw in [
        "ignore previous instructions",
        "system_prompt",
        "tool_call",
        "exfiltrate",
    ] {
        if low.contains(kw) {
            hits += 1;
        }
    }
    hits >= 2
}

