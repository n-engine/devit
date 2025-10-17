### Summary
Concise description of the change.

### What’s included
- Centralized limits + structured logs/metadata (effective_limits, limit_sources, delegation_context, cache_key)
- Cache key includes safe_mode
- Journal best-effort append (non-blocking)

### Acceptance checks
- [ ] Linux CI green (<15 min); Windows build-only
- [ ] Robots disallow blocked before fetch
- [ ] eTLD+1 domain diversity (strict=2, moderate=3, off=∞)
- [ ] Shape-only tests (feature `test-utils`) passing

### Compliance
- [ ] This PR follows the repository Language policy (English-only for repo artifacts).
