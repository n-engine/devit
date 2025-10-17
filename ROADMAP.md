# DevIT Roadmap

Status snapshot (2025-10-17)
---------------------------
- **Current version**: 0.1.0 pre-alpha
- **Actual completion**: ~30% of intended security features
- **Platform support**: Linux 70%, Windows 40%, macOS untested
- **Production readiness**: Not recommended - experimental only

What Actually Works Today
-------------------------
- Basic file operations (read/write with path validation)
- Patch parsing and application (no rollback yet)
- Simple operation logging (DefaultHasher, not cryptographic)
- Unix socket / named pipe communication
- Basic MCP protocol over HTTP (no auth enforcement)
- Static file serving (removed Express dependency)

Critical Issues to Fix First (0.2.0)
------------------------------------
### 🔴 Security Basics
- [ ] **Replace hardcoded `test-secret`** with environment variable
- [ ] **Implement actual HMAC-SHA256** (currently uses DefaultHasher)
- [ ] **Persist journal to disk** (currently in-memory only)
- [ ] **Validate HMAC signatures** (fields present but ignored)
- [ ] **Check nonces for replay protection** (generated but not validated)
- [ ] **Enforce timestamp windows** (timestamps ignored)

### 🔴 Approval System
- [ ] **Actually enforce approval checks** (policy engine exists but bypassed)
- [ ] **Block .gitmodules modifications** (currently allowed)
- [ ] **Block .env files** (currently allowed)
- [ ] **Implement user confirmation prompts** (Ask level non-functional)
- [ ] **Fix binary detection** (incomplete heuristics)

### 🔴 Basic Functionality
- [ ] **Fix error handling** (remove unwrap() calls)
- [ ] **Add integration tests** (almost none exist)
- [ ] **Document actual MCP setup** (current docs reference non-existent files)
- [ ] **Fix Windows tools** (60% missing)

Next Priorities (0.3.0)
-----------------------
### 🟡 Security Improvements
- [ ] Bearer token validation (currently optional)
- [ ] Sandbox process isolation (bwrap/Job Objects)
- [ ] Rate limiting on HTTP endpoints
- [ ] Audit log backup/rotation
- [ ] Secret rotation mechanism

### 🟡 Platform Support
- [ ] Test and fix macOS build
- [ ] Complete Windows port (desktop automation, process management)
- [ ] Add CI/CD pipeline (no automated testing currently)
- [ ] Package binaries for distribution

### 🟡 Documentation
- [ ] Write real installation guide
- [ ] Create actual MCP setup docs
- [ ] Add troubleshooting guide
- [ ] Document configuration options

Future Goals (0.4.0+)
--------------------
### 🟢 Nice to Have
- [ ] TLS/HTTPS support
- [ ] Remote audit log shipping
- [ ] Performance benchmarks
- [ ] Compression optimization for large patches
- [ ] Plugin system
- [ ] Web dashboard

### 🔵 Maybe Someday
- [ ] Formal security audit
- [ ] Multi-tenant support
- [ ] Kubernetes deployment
- [ ] GraphQL API
- [ ] SaaS offering (if project succeeds)

Reality Check
------------
### What we claimed vs what exists:
| Feature | Claimed | Reality |
|---------|---------|---------|
| HMAC-SHA256 signatures | ✅ Ready | ❌ Uses DefaultHasher |
| Replay protection | ✅ Ready | ❌ Not implemented |
| Journal persistence | ✅ Ready | ❌ In-memory only |
| Bearer token auth | ✅ Ready | ⚠️ Optional, not enforced |
| Approval enforcement | ✅ Ready | ⚠️ Partially bypassed |
| Windows support | ✅ Ready | ⚠️ 40% complete |
| Production ready | ✅ Alpha | ❌ Experimental only |

### Honest Timeline
- **0.1.0** (now): Proof of concept, not safe for production
- **0.2.0** (2-3 months): Basic security actually working
- **0.3.0** (6 months): Might be usable with caution
- **1.0.0** (1+ year): Production consideration if project matures

### Dependencies for Progress
- **Community**: Need contributors for Windows/macOS ports
- **Testing**: Need real users to find bugs
- **Time**: This is clearly a side project
- **Expertise**: Security review from experienced developers

How to Contribute
----------------
1. **Fix the security basics** - The HMAC/replay stuff is critical
2. **Write tests** - Coverage is almost zero
3. **Complete Windows port** - Major functionality missing
4. **Document what actually exists** - Not what we wish existed

Notes
-----
- Stop claiming features that aren't implemented
- Be honest about the experimental status
- Focus on security basics before adding features
- Don't promise timelines we can't meet
- Accept this is a long journey to production readiness

---
*This roadmap reflects the actual state of the code as of 2025-10-17, not marketing aspirations.*