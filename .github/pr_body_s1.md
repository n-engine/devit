### Summary
V1.0 S1 — Centralized limits, cache-key hardening, structured logs/metadata, and journal best-effort writer. Draft while CI runs.

### What’s included
- Common limits: `resolve_search_limits/resolve_fetch_limits` with clamping.
- Metadata & logs: `effective_limits`, `limit_sources`, `delegation_context: null`, `cache_key`.
- Cache key includes `safe_mode` (+ `include_content`, `user_agent`).
- Best-effort JSONL journal append on **all** paths (including early returns).
- Shape-only tests behind `--features test-utils` (robots, sanitizer, injection threshold, paywall hints, metadata shape).
- CI: Linux Tier1 (fmt, clippy, test-utils + workspace libs), Windows Tier2 (build-only).

### Acceptance checks
- [ ] Linux CI green (< 15 min)
- [ ] Windows build-only green
- [ ] Robots “disallow” blocked **before** fetch (policy logged)
- [ ] eTLD+1 domain diversity (strict=2, moderate=3, off=∞)
- [ ] Shape-only tests passing (`cargo test -p mcp-tools --features test-utils`)
- [ ] Journal lines written for success + early-returns (non-blocking)

### Compliance
- [ ] Follows repository **Language policy** (English-only for repo artifacts)

