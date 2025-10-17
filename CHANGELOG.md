# Changelog

All notable changes to DevIt are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [0.1.0] — 2025-10-17

### About This Release

**Initial preview release** of DevIt's architectural foundation.

This is an early development release showcasing the core architecture and tooling approach. The system demonstrates the feasibility of secure LLM-codebase interaction but is not yet production-ready. Many security features described in our roadmap are partially implemented or planned.

**Status**: Pre-alpha preview — APIs will change, security features are incomplete, use for experimentation only.

### Added

#### Core Architecture
- **Basic daemon architecture** — Background devitd process handles tool orchestration
- **MCP protocol support** — Initial implementation of 30+ tools exposed via Model Context Protocol
- **Task orchestration** — Basic multi-agent task delegation with message passing
- **IPC communication** — Unix sockets (Linux/macOS) and named pipes (Windows) for CLI-daemon communication

#### Security Foundation (Partial)
- **Five-tier approval system design** — Untrusted, Ask, Moderate, Trusted, Privileged (policy engine implemented, enforcement partial)
- **Basic path validation** — Directory traversal protection and symlink checks
- **Simple journaling** — Operations logged to `.devit/journal.log` with basic hashing (not cryptographic)
- **Message authentication** — HMAC field present in wire protocol (validation not yet enforced)

#### File Operations
- **Patch parsing** — Unified diff parser with basic application support
- **Snapshot system** — Create project snapshots for comparison
- **File read/write** — Basic file operations with path validation
- **Directory listing** — Explore project structure safely

#### Git Integration
- **git log** — View commit history
- **git blame** — Track code authorship
- **git show** — Display commit details
- **git diff** — Compare changes
- **git search** — Find patterns in history

#### Process Management
- **devit_exec** — Execute commands with basic isolation
- **Process registry** — Track running processes
- **Process termination** — Kill processes by PID

#### Desktop Automation (Linux only)
- **Screenshot tool** — Capture desktop via backend drivers
- **Mouse control** — Click, move, scroll with xdotool
- **Keyboard control** — Type text, send key combinations
- **OCR** — Extract text from screenshots via Tesseract
- **OCR Alerts** — Pattern matching on screenshot text

#### HTTP Transport
- **HTTP server for MCP** — Basic HTTP endpoint for MCP protocol
- **Bearer token support** — Token field present (validation optional)
- **CORS headers** — Basic CORS configuration

#### Developer Tools
- **Configuration files** — `devit.toml` and `devitd.core.toml` for settings
- **Test runner** — Execute test suites
- **Help system** — Command documentation
- **Example patches** — Sample files for testing

### Known Limitations

#### Security (Not Yet Implemented)
- **Journal signing** — Currently uses DefaultHasher, not HMAC-SHA256
- **Replay protection** — Nonces generated but not validated
- **Timestamp validation** — Timestamps present but not checked
- **Secret management** — Hardcoded test secret in code
- **Token validation** — Bearer tokens accepted but not required
- **Approval enforcement** — Policy engine exists but not fully integrated

#### Platform Support
- **Linux** — Primary development target, most features work
- **Windows** — ~40% complete (named pipes work, many tools missing)
- **macOS** — Untested, should partially work via Unix sockets

#### Missing Features
- **Persistent journal** — Currently in-memory only
- **Cryptographic audit trail** — Planned but not implemented
- **Replay attack protection** — Design complete, implementation pending
- **HTTPS/TLS** — Not implemented
- **Production error handling** — Many unwrap() calls remain
- **Comprehensive testing** — Test coverage incomplete

### Architecture Notes

The codebase consists of 13 crates:
- Core functionality split across `crates/cli`, `crates/common`, `crates/mcp-*`
- Daemon in `devitd/`
- Experimental features in various states of completion

### Security Warning

**⚠️ This is experimental software not suitable for production use:**
- Security features are partially implemented
- No formal security audit has been performed  
- Authentication/authorization is incomplete
- Error handling needs hardening
- Use only in isolated development environments

### Future Roadmap

#### Next Release (0.2.0)
- [ ] Implement proper HMAC-SHA256 journal signing
- [ ] Add nonce/timestamp validation
- [ ] Replace hardcoded secrets with environment variables
- [ ] Complete Windows process management
- [ ] Add integration tests

#### Version 0.3.0
- [ ] Full approval system enforcement
- [ ] Persistent journal to disk
- [ ] Replay attack protection
- [ ] Bearer token validation
- [ ] Basic CI/CD pipeline

#### Version 1.0.0 (Target)
- [ ] Production-ready security model
- [ ] Complete platform support
- [ ] Performance optimization
- [ ] Formal security audit
- [ ] Comprehensive documentation

---

## Installation

### Development Setup Only

```bash
# Clone and build from source
git clone https://github.com/n-engine/devit.git
cd devit
cargo build --workspace

# Run with test configuration
export DEVIT_SECRET="test-secret"  # Currently hardcoded, env var for future use
./target/debug/devitd --socket /tmp/devitd.sock
./target/debug/devit snapshot --pretty
```

### Not Recommended For
- Production environments
- Handling sensitive code
- Automated CI/CD pipelines
- Multi-user systems

---

## Getting Started

### Basic Testing

```bash
# Start daemon (development mode)
devitd --socket /tmp/devitd.sock

# Create snapshot
devit snapshot --pretty

# Try a simple patch
devit patch-preview samples/ok_add_fn.diff --pretty
```

### MCP Exploration

Basic MCP server available for experimentation:
```bash
mcp-server --transport http --host 127.0.0.1 --port 3001
```

Note: Authentication not enforced, use only on localhost.

---

## Contributors

Initial development by the DevIt team. Community contributions welcome for:
- Security implementation completion
- Platform support expansion
- Test coverage improvement
- Documentation

---

## License

DevIt is released under the [MIT License](LICENSE).

---

## Disclaimer

**Pre-Alpha Software**: For experimental use only.

- **Security**: Core mechanisms designed but not fully implemented
- **Stability**: Frequent breaking changes expected
- **Reliability**: Contains known bugs and incomplete error handling
- **Performance**: Not optimized, no benchmarks available

This release is intended for developers interested in the architecture and approach. Production use strongly discouraged until version 1.0.0.
