# Security Model (Preview)

This document describes DevIt's security architecture as designed and the current implementation status.

> **⚠️ WARNING**: Version 0.1.0 is a preview release with incomplete security implementation. Many features described here are partially implemented or planned. DO NOT use in production or with sensitive code.

## Executive Summary

DevIt's security model is designed around defense-in-depth, but **current implementation is incomplete**:

### Designed Architecture
1. Shared secret + HMAC authentication
2. Five-tier approval system  
3. Sandbox isolation
4. Path canonicalization (C4-compliant)
5. Cryptographic audit trail
6. Bearer token authentication

### Current Implementation (0.1.0)
- ✅ Basic path validation
- ⚠️ HMAC fields present but not validated
- ⚠️ Approval system designed but not enforced
- ⚠️ Journal uses simple hashing (not cryptographic)
- ⚠️ Token fields present but optional
- ❌ Replay protection not implemented
- ❌ Sandbox isolation incomplete

---

## Authentication & Request Signing

### Design Goal

All CLI-daemon communication should be authenticated using HMAC-SHA256 with replay protection.

### Current Status (0.1.0)

**Partially Implemented:**
```rust
// Wire format includes authentication fields:
struct Msg {
    msg_id: String,
    nonce: String,      // Generated but not validated
    hmac: String,       // Present but not enforced
    timestamp: u64,     // Included but not checked
    payload: Value,
}
```

**Issues:**
- HMAC field exists but validation is not enforced
- Nonces are generated but never checked for reuse
- Timestamps present but no expiration logic
- Secret hardcoded as `b"test-secret"` in code

**Planned Fix (0.2.0):**
- Implement actual HMAC-SHA256 validation
- Add nonce deduplication cache
- Enforce 5-minute timestamp window
- Load secret from environment variable

---

## Approval System

### Design 

Five-tier system to control operation permissions:
1. **Untrusted** - Everything requires confirmation
2. **Ask** - Simple ops auto-approved
3. **Moderate** - Balanced automation (default)
4. **Trusted** - Most ops auto-approved  
5. **Privileged** - Full control with path restrictions

### Current Status (0.1.0)

**Partially Implemented:**
- ✅ PolicyEngine struct exists with approval levels
- ✅ Approval evaluation logic written
- ⚠️ Not consistently enforced across all operations
- ⚠️ Downgrades designed but not fully tested
- ❌ Protected paths hardcoded, not configurable

**Known Gaps:**
- Some operations bypass approval checks
- Binary file detection incomplete
- .gitmodules/.env blocking not enforced
- User confirmation prompts not implemented

---

## Audit Trail

### Design

Tamper-evident journal with HMAC-SHA256 signatures for each operation.

### Current Status (0.1.0)

**Basic Implementation:**
```rust
// Current journal implementation
pub struct Journal {
    path: PathBuf,           // Hardcoded to .devit/journal.log
    secret: Vec<u8>,         // Hardcoded b"test-secret"
    entries: VecDeque<Value>, // In-memory only
}

// Uses DefaultHasher, not HMAC-SHA256
fn compute_hmac(&self, entry: &Value) -> String {
    let mut hasher = DefaultHasher::new();
    serde_json::to_vec(entry).hash(&mut hasher);
    self.secret.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}
```

**Issues:**
- Uses Rust's DefaultHasher (not cryptographic)
- Journal not persisted to disk
- No signature verification on read
- Secret hardcoded in source
- No rotation mechanism

**Planned Fix (0.2.0):**
- Implement proper HMAC-SHA256
- Persist journal to disk atomically
- Add verification command
- Load secret from environment

---

## Path Security

### Design

C4-compliant canonicalization to prevent directory traversal.

### Current Status (0.1.0)

**Mostly Implemented:**
- ✅ Basic canonicalization works
- ✅ Symlink detection functional
- ✅ Parent directory traversal blocked
- ⚠️ Some edge cases not handled
- ⚠️ Windows path handling incomplete

**Working Examples:**
```
../../../etc/passwd     → BLOCKED
./src/../../../secret   → BLOCKED  
safe_link -> ../lib/ok  → ALLOWED (if in workspace)
```

---

## HTTP/MCP Transport

### Design

Authenticated HTTP transport with bearer tokens and TLS.

### Current Status (0.1.0)

**Basic Implementation:**
- ✅ HTTP server runs
- ✅ MCP protocol works over HTTP
- ⚠️ Bearer token field present but not required
- ⚠️ CORS headers configurable but permissive by default
- ❌ No HTTPS/TLS support
- ❌ No rate limiting

**Current Behavior:**
```bash
# Works without authentication (dev mode)
curl http://localhost:3001/message -d '{...}'

# Token accepted but not validated
curl -H "Authorization: Bearer anything" http://localhost:3001/message
```

---

## Sandbox Profiles

### Design

Strict and Permissive profiles for operation isolation.

### Current Status (0.1.0)

**Minimal Implementation:**
- ✅ Profile enums defined
- ⚠️ Basic workspace boundary checks
- ❌ No actual sandboxing (bwrap/Job Objects)
- ❌ Network isolation not implemented
- ❌ Resource limits not enforced

---

## Known Vulnerabilities (0.1.0)

### Critical
1. **No Authentication Enforcement** - HMAC validation skipped
2. **Hardcoded Secrets** - `b"test-secret"` in source code
3. **No Replay Protection** - Nonces not validated

### High
4. **Journal Not Cryptographic** - Uses non-crypto hash
5. **No Sandbox Isolation** - Processes run unrestricted
6. **Bearer Tokens Optional** - HTTP accepts any request

### Medium
7. **Approval Bypass** - Some operations skip policy engine
8. **No Rate Limiting** - DoS possible on HTTP endpoint
9. **CORS Too Permissive** - Accepts all origins by default

### Low
10. **Incomplete Error Handling** - Many unwrap() calls
11. **No Audit Log Backup** - Journal only in memory
12. **Windows Paths** - Some validation gaps

---

## Security Checklist

### DO NOT Use DevIt 0.1.0 For:
- ❌ Production environments
- ❌ Sensitive codebases
- ❌ Multi-user systems
- ❌ Internet-exposed services
- ❌ CI/CD pipelines
- ❌ Customer data processing

### Safe For Experimentation:
- ✅ Local development (isolated)
- ✅ Architecture evaluation
- ✅ Feature exploration
- ✅ Contributing to development

---

## Roadmap to Security

### Version 0.2.0 (Next)
- [ ] Implement HMAC-SHA256 validation
- [ ] Add replay protection
- [ ] Persist journal to disk
- [ ] Environment-based secrets
- [ ] Fix approval enforcement

### Version 0.3.0
- [ ] Add bwrap/Job Objects sandboxing
- [ ] Require bearer token validation
- [ ] Implement rate limiting
- [ ] Add timestamp checks
- [ ] Complete Windows support

### Version 1.0.0 (Production Target)
- [ ] Full security audit
- [ ] Penetration testing
- [ ] TLS/HTTPS support
- [ ] Key rotation mechanism
- [ ] Compliance documentation

---

## Reporting Security Issues

For this preview release, report issues publicly on GitHub.

Once we reach 0.3.0, security issues should be reported to: contact@getdevit.com

---

## Current Mitigations

Until security is complete, use these practices:

1. **Run only on localhost** - Never expose ports
2. **Use test data only** - No sensitive code
3. **Isolated environment** - VM or container recommended
4. **Monitor processes** - Check for unexpected behavior
5. **Regular updates** - Security fixes coming rapidly

---

## References

- **Design Inspiration**: OWASP guidelines, C4 model
- **Future Implementation**: HMAC (RFC 2104), seccomp/bwrap (Linux), Job Objects (Windows)
- **Current State**: Basic architecture demonstration only
