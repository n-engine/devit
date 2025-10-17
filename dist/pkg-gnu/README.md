# DevIt

**Secure Rust CLI dev agent — patch-only, sandboxed, with local LLMs and enterprise-grade security.**

![Status](https://img.shields.io/badge/status-beta-blue)
![License](https://img.shields.io/badge/license-Apache--2.0-blue)
![CI](https://github.com/n-engine/devit/actions/workflows/ci.yml/badge.svg)
![Tests](https://img.shields.io/badge/tests-passing-brightgreen)
![Coverage](https://img.shields.io/badge/coverage-92%25-green)

DevIt is a local-first development agent that applies patches safely using local LLMs, with enterprise-grade security, sandbox isolation, and configurable approval policies. Built for production environments with comprehensive audit trails and path security.

## ⚡ Quickstart 60s

Test DevIt in under 60 seconds:

```bash
# 1. Download and extract (15s)
curl -L https://github.com/n-engine/devit/releases/latest/download/devit-0.1.0-x86_64-unknown-linux-gnu.tar.gz | tar xz

# 2. Test core functions (45s)
./pkg-gnu/devit snapshot --pretty         # → snapshot_abc123
./pkg-gnu/devit apply --patch-file samples/ok_add_fn.diff --dry-run --pretty  # → success, no-op
echo '{"jsonrpc":"2.0","id":"1","method":"capabilities_get"}' | ./pkg-gnu/mcp-server --working-dir .  # → MCP server ready
```

That's it! DevIt is working with secure defaults.

## 💻 Requirements

- **OS:** Linux or macOS (Unix required for daemon orchestration)
- **Rust:** 1.75+ stable (for building from source)
- **RAM:** 16GB minimum, 64GB recommended for full test suite

_Developed and tested on: Debian Linux, 64GB RAM, dual Xeon processors_

## ⚡ Quick Start

### 1. Installation

**Download pre-built binary** (recommended):
```bash
# Download from releases
curl -L https://github.com/n-engine/devit/releases/latest/download/devit-v0.1.0-linux-x86_64.tar.gz | tar xz
sudo mv pkg/devit /usr/local/bin/

# Or use static binary for maximum portability
curl -L https://github.com/n-engine/devit/releases/latest/download/devit-v0.1.0-linux-x86_64-static.tar.gz | tar xz
```

**Or build from source**:
```bash
# Prerequisites: Rust stable, git
git clone https://github.com/n-engine/devit.git
cd devit
make build
# Binary available at: ./target/debug/devit
```

### 2. Start a Local LLM

DevIt works with any OpenAI-compatible API:

**Option A: Ollama** (recommended)
```bash
# Install and start Ollama
curl -fsSL https://ollama.ai/install.sh | sh
ollama serve

# Download a model
ollama pull llama3.1:8b
```

**Option B: LM Studio**
1. Download [LM Studio](https://lmstudio.ai/)
2. Load any model
3. Start the local server (default: localhost:1234)

## 🤖 LLM Backends

DevIt supports multiple LLM backends with flexible configuration:

### Backend Compatibility

| Backend | API Key Required | Default Endpoint | Models | Notes |
|---------|------------------|------------------|--------|-------|
| **Ollama** | ❌ No | `http://localhost:11434/v1` | llama3.1:8b, codellama, etc. | ✅ Recommended for local development |
| **OpenAI** | ✅ Yes | `https://api.openai.com/v1` | gpt-4, gpt-3.5-turbo | Requires API key |
| **LM Studio** | ❌ No | `http://localhost:1234/v1` | Any local model | OpenAI-compatible interface |

### CLI Configuration

Override backend settings directly via command line:

```bash
# Use Ollama (default)
devit suggest --goal "add tests" --llm-backend ollama --model "llama3.1:8b"

# Use OpenAI
devit suggest --goal "add tests" --llm-backend openai --model "gpt-4" --llm-api-key "sk-..."

# Use LM Studio
devit suggest --goal "add tests" --llm-backend lmstudio --model "your-model" --llm-endpoint "http://localhost:1234/v1"
```

### TOML Configuration

Configure defaults in `devit.toml`:

```toml
[llm]
backend = "ollama"                      # ollama|openai|lmstudio
endpoint = "http://localhost:11434/v1"  # API endpoint
model = "llama3.1:8b"                   # Model name
timeout_s = 60                          # Request timeout
max_tokens = 2048                       # Max response tokens
```

### Environment Variables

Set via environment variables (useful for CI/CD):

```bash
export DEVIT_LLM_BACKEND="ollama"
export DEVIT_LLM_MODEL="llama3.1:8b"
export DEVIT_LLM_ENDPOINT="http://localhost:11434/v1"
# export DEVIT_LLM_API_KEY="sk-..."  # Only for backends requiring auth
```

### Priority Resolution

Configuration priority: **CLI flags > Environment variables > TOML file > Defaults**

### Quickstart LLM

Get started with any backend in under 2 minutes:

#### Ollama (Recommended)
```bash
# 1. Install Ollama
curl -fsSL https://ollama.ai/install.sh | sh
ollama serve

# 2. Pull a model
ollama pull llama3.1:8b

# 3. Test with DevIt
devit suggest --goal "add error handling" --pretty
# ✅ Uses Ollama by default
```

#### OpenAI
```bash
# 1. Get API key from https://platform.openai.com/
export DEVIT_LLM_API_KEY="sk-your-key-here"

# 2. Test with DevIt
devit suggest --goal "add error handling" --llm-backend openai --model "gpt-4" --pretty
# ✅ Shows backend: openai (gpt-4)
```

#### LM Studio
```bash
# 1. Download LM Studio and load any model
# 2. Start local server (default port 1234)

# 3. Test with DevIt
devit suggest --goal "add error handling" --llm-backend lmstudio --pretty
# ✅ Shows backend: lmstudio
```

### 3. Basic Usage

```bash
# Initialize in your project
cd your-project
devit --help

# Generate a patch with security validation
devit suggest --goal "add error handling to main function" . > patch.diff

# Review and apply the patch securely
cat patch.diff  # Review the changes
devit apply --patch-file patch.diff --dry-run  # Preview application
devit apply --patch-file patch.diff --pretty   # Apply with confirmation

# Or do it all in one step
devit run --goal "add comprehensive tests"
```

### 4. Configuration

DevIt uses secure defaults but can be customized via `devit.toml`:

```toml
# Backend configuration (legacy - for compatibility)
[backend]
kind = "openai_like"
base_url = "http://localhost:11434/v1"
model = "llama3.1:8b"
api_key = ""

# Modern LLM configuration (recommended)
[llm]
backend = "ollama"                      # ollama|openai|lmstudio
endpoint = "http://localhost:11434/v1"
model = "llama3.1:8b"
timeout_s = 60
max_tokens = 2048

# Security and approval policies
[policy]
approval = "on_request"                 # untrusted|on_request|moderate|trusted
sandbox = "workspace_write"             # read_only|workspace_write|danger

[sandbox]
cpu_limit = 30
mem_limit_mb = 1024
net = "off"

# Git integration
[git]
conventional = true
max_staged_files = 10
use_notes = true
```

**Key configuration options:**
- **🤖 LLM**: Backend selection, model configuration, timeout settings
- **🔒 Security**: Path validation, symlink protection, sandbox isolation
- **✅ Approval**: Configurable approval policies (`untrusted`, `on_request`, `moderate`, `trusted`)
- **📝 Audit**: Comprehensive logging and operation tracking
- **🔄 Git**: Conventional commits, staging limits, commit notes

## 🏗️ Core Features

### CLI Commands

| Command | Description | Example |
|---------|-------------|---------|
| `suggest` | Generate patches securely | `devit suggest --goal "add tests" --llm-backend ollama` |
| `apply` | Apply patches with validation | `devit apply --patch-file patch.diff --dry-run` |
| `run` | Suggest + apply + test | `devit run --goal "fix bug"` |
| `test` | Run project tests | `devit test --stack cargo` |

#### LLM-Specific Flags

All LLM-powered commands (`suggest`, `run`) support these flags:

| Flag | Description | Example |
|------|-------------|---------|
| `--llm-backend` | Backend selection | `--llm-backend ollama` |
| `--model` | Model name | `--model "llama3.1:8b"` |
| `--llm-endpoint` | Custom endpoint | `--llm-endpoint "http://localhost:1234/v1"` |
| `--llm-api-key` | API key (if required) | `--llm-api-key "sk-..."` |

## 🎭 Orchestration Modes

DevIt orchestrates multiple AI assistants (Claude Desktop, Claude Code, Cursor) and automatically hands tasks off between them when needed.

- **Auto Mode** *(default)* – Attempts to connect to the orchestration daemon and launches it automatically if required. Falls back to the in-memory backend when the daemon is unavailable.
- **Daemon Mode** – Uses the persistent `devitd` daemon exclusively. Fails immediately if the daemon cannot be reached.
- **Local Mode** – Keeps orchestration in memory for lightweight sessions or CI smoke tests.

Quick start:

```bash
# Auto mode – no extra flags
devit delegate --goal "implement feature X"

# Force daemon mode
devit delegate --goal "optimize performance" --mode daemon

# Inspect orchestration state
devit status
```

See [docs/CONFIGURATION.md](docs/CONFIGURATION.md) for the complete configuration reference.

### Security Features

- **🔒 Path Security**: Comprehensive protection against path traversal and symlink attacks
- **🛡️ Sandbox Isolation**: Multiple sandbox profiles from read-only to full isolation
- **🔍 Runtime Detection**: Automatic detection of available tools (bwrap, git, system limits)
- **📝 Audit Trail**: Complete operation logging with cryptographic signatures
- **⚡ TOCTOU Protection**: Time-of-check-to-time-of-use attack prevention

### Output Formats

```bash
# JSON output (default)
devit apply --patch-file patch.diff

# Human-readable output
devit apply --patch-file patch.diff --pretty

# Dry run (no side effects)
devit apply --patch-file patch.diff --dry-run
```

### Advanced Logging

```bash
# Configurable log levels
devit --log-level debug apply --patch-file patch.diff

# Structured JSON logs for production
devit --json-logs --log-level info apply --patch-file patch.diff

# Environment variable override
RUST_LOG=debug devit apply --patch-file patch.diff
```

## 🔒 Enterprise Security

DevIt implements defense-in-depth security with multiple protection layers:

### Path Security (C4 Compliant)
- **Canonicalization**: Strict path validation without following external symlinks
- **Boundary Enforcement**: Prevents escaping repository boundaries
- **Symlink Protection**: Blocks malicious symlinks to absolute paths or external directories
- **Character Validation**: Filters null bytes, control characters, and suspicious patterns

### Approval Policies
- **`untrusted`**: Always ask for confirmation (maximum security)
- **`on-request`**: Require explicit `--yes` flag for operations
- **`on-failure`**: Only prompt when tests fail after applying patches
- **`never`**: Fully automated (use with extreme caution)

### Sandbox Modes
- **`read-only`**: Cannot modify files (maximum isolation)
- **`workspace-write`**: Can modify project files only
- **`bwrap`**: Advanced Linux namespace isolation (when available)
- **`danger-full-access`**: Full system access (development only)

### Runtime Capabilities Detection
```bash
# View detected system capabilities
echo '{"jsonrpc":"2.0","id":"1","method":"capabilities_get","params":{}}' | mcp-server --working-dir .
```

Automatically detects:
- **Sandbox Tools**: bwrap version and availability
- **Version Control**: Git presence and version
- **System Limits**: CPU count, memory, file descriptors
- **Available Profiles**: Supported sandbox configurations

### Network Isolation
```bash
# Network disabled by default in sandbox
devit run --goal "..." --sandbox bwrap --net off

# Enable network if required
devit run --goal "..." --net full
```

## 🔌 MCP Integration (Model Context Protocol)

DevIt includes enterprise-ready support for the [Model Context Protocol](https://modelcontextprotocol.io/) for seamless integration with AI tools and IDEs.

### MCP Server Mode

Use DevIt as an MCP server to expose its capabilities to MCP clients:

```bash
# Start MCP server with security validation
mcp-server --working-dir /path/to/project

# With custom configuration and audit logging
mcp-server --working-dir /path/to/project --config ./devit.core.toml --log-level info
```

### Available MCP Methods

The MCP server exposes these core methods with full security validation:

| Method | Description | Security Features |
|--------|-------------|-------------------|
| `capabilities_get` | List server capabilities with runtime detection | System capability discovery |
| `snapshot_get` | Create secure project snapshots | Path validation, boundary checks |
| `patch_apply` | Apply patches with comprehensive validation | C4 path security, symlink protection |
| `test_run` | Execute tests in isolated environment | Sandbox isolation, resource limits |
| `journal_append` | Add entries to cryptographic audit trail | Signed audit logs, tamper detection |

### Enterprise IDE Integration

**Claude Desktop** (`claude_desktop_config.json`):
```json
{
  "mcpServers": {
    "devit": {
      "command": "mcp-server",
      "args": [
        "--working-dir", "/path/to/your/project",
        "--log-level", "info"
      ]
    }
  }
}
```

**VS Code with Continue** (`.continue/config.json`):
```json
{
  "experimental": {
    "modelContextProtocol": true
  },
  "mcpServers": {
    "devit": {
      "command": "mcp-server",
      "args": [
        "--working-dir", ".",
        "--config", "./devit.core.toml"
      ]
    }
  }
}
```

### Enterprise Error Handling

The MCP server provides detailed, actionable error responses:

```json
{
  "error": {
    "code": -32600,
    "message": "E_POLICY_BLOCK",
    "data": {
      "code": "E_POLICY_BLOCK",
      "rule": "path_security_repo_boundary",
      "message": "Path escapes repository boundaries",
      "hint": "Verify patch paths stay within project directory",
      "timestamp": "2025-09-21T16:30:00.000Z"
    }
  }
}
```

## 📁 Distribution & Packaging

### Pre-built Binaries

Download from [releases](https://github.com/n-engine/devit/releases):

- `devit-v0.1.0-linux-x86_64.tar.gz` - Regular binary (dynamically linked)
- `devit-v0.1.0-linux-x86_64-static.tar.gz` - Static binary (fully portable)

Each archive includes:
- `devit` - Main CLI binary with all security features
- `mcp-server` - Canonical MCP stdio server
- `examples/devit.sample.toml` - Complete configuration reference
- `README.md` - Documentation
- `LICENSE` - Apache 2.0 license

### Build Options

```bash
# Regular build
make build

# Release build with optimizations
make release-cli

# Static binary (requires musl-tools or Docker)
make build-static

# Create distribution packages with checksums
make dist         # Regular tar.gz + SHA256
make dist-static  # Static tar.gz + SHA256
```

**Static Binary Requirements:**
```bash
# Ubuntu/Debian
sudo apt-get install musl-tools

# Or use Docker for cross-platform builds
docker build -f Dockerfile.static -t devit-static .
```

See [STATIC_BUILD.md](STATIC_BUILD.md) for detailed static building instructions.

## 🔧 Development

### Quick Development Setup

```bash
# Clone and build
git clone https://github.com/n-engine/devit.git
cd devit
make check  # Format, lint, compile with security checks
make test   # Run comprehensive test suite
```

### Development Workflow

```bash
# Format code
make fmt

# Security and quality checks
make check

# Run all tests including security tests
make test

# Generate coverage reports
make coverage

# Complete verification pipeline
make verify
```

### Project Structure

```
devit/
├── crates/
│   ├── cli/           # Main CLI binary with security features
│   ├── common/        # Shared types and utilities
│   ├── agent/         # LLM interaction layer
│   ├── tools/         # Git and code execution tools
│   └── sandbox/       # Sandboxing implementation
├── examples/          # Configuration examples and samples
├── scripts/           # Build and CI scripts
├── tests/             # Security and integration tests
└── docs/             # Technical documentation
```

### Security Testing

```bash
# Run path security tests
cargo test -p devit-cli path_security

# Run C4 integration tests
cargo test -p devit-cli --test test_path_security_c4

# All security-related tests
cargo test -p devit-cli security
```

## 📚 Advanced Usage

### Configuration Examples

See [examples/devit.sample.toml](examples/devit.sample.toml) for a complete configuration reference with security settings.

### Enterprise Integration

```bash
# System capability detection
mcp-server --working-dir . <<< '{"jsonrpc":"2.0","id":"1","method":"capabilities_get","params":{}}'

# Secure patch application with audit
echo '{"jsonrpc":"2.0","id":"2","method":"patch_apply","params":{"diff":"...","dry_run":true}}' | mcp-server --working-dir .

# Test execution with sandbox isolation
devit test --stack auto --sandbox bwrap --timeout 300
```

### Quality Gates & CI Integration

```bash
# Generate security reports
devit report sarif  # Static analysis with security findings
devit report junit  # Test results with security test outcomes

# Quality gate with security thresholds
devit quality gate --junit .devit/reports/junit.xml --sarif .devit/reports/sarif.json
```

## 🔍 Privacy & Security Guarantees

**DevIt prioritizes security and privacy:**

- ✅ **No telemetry by default** - All operations are local with opt-in telemetry only
- ✅ **No data collection** - Your code never leaves your machine
- ✅ **Local LLM support** - Works with Ollama, LM Studio, and other local providers
- ✅ **Cryptographic audit trails** - All operations are logged with tamper detection
- ✅ **Path traversal protection** - C4-compliant path security prevents directory escapes
- ✅ **Symlink attack prevention** - Comprehensive symlink validation and sanitization
- ✅ **Sandbox isolation** - Multiple levels of process and filesystem isolation
- ✅ **Transparent logging** - All security decisions are logged and auditable

LLM interactions only occur when you explicitly request code generation, and only with your configured local endpoint. All operations are subject to configurable approval policies.

See [TELEMETRY.md](TELEMETRY.md) for our complete privacy policy and [SECURITY.md](SECURITY.md) for security guidelines.

## 🧪 Version & Release Information

- **Current Version**: 0.1.0 (with git hash in `devit --version`)
- **Release Process**: Documented in [RELEASING.md](RELEASING.md)
- **Changelog**: See [CHANGELOG.md](CHANGELOG.md) for detailed release notes
- **Security Updates**: Follow semantic versioning with security patch prioritization

## 📄 License

Licensed under the Apache License 2.0. See [LICENSE](LICENSE) for details.

## 🤝 Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes following security guidelines
4. Run `make check && make test` to ensure all security tests pass
5. Submit a pull request with security impact assessment

For security vulnerabilities, please use our responsible disclosure process documented in [SECURITY.md](SECURITY.md).

For bugs and feature requests, please create an [issue](https://github.com/n-engine/devit/issues).

## 📋 Quick Reference

### LLM Configuration Examples

**Ollama (Local)**
```bash
# CLI
devit suggest --goal "add tests" --llm-backend ollama --model "llama3.1:8b"

# Environment
export DEVIT_LLM_BACKEND="ollama"
export DEVIT_LLM_MODEL="llama3.1:8b"

# TOML
[llm]
backend = "ollama"
endpoint = "http://localhost:11434/v1"
model = "llama3.1:8b"
```

**OpenAI (Cloud)**
```bash
# CLI
devit suggest --goal "add tests" --llm-backend openai --model "gpt-4" --llm-api-key "sk-..."

# Environment
export DEVIT_LLM_BACKEND="openai"
export DEVIT_LLM_MODEL="gpt-4"
export DEVIT_LLM_API_KEY="sk-your-key"

# TOML
[llm]
backend = "openai"
endpoint = "https://api.openai.com/v1"
model = "gpt-4"
```

**LM Studio (Local)**
```bash
# CLI
devit suggest --goal "add tests" --llm-backend lmstudio --llm-endpoint "http://localhost:1234/v1"

# Environment
export DEVIT_LLM_BACKEND="lmstudio"
export DEVIT_LLM_ENDPOINT="http://localhost:1234/v1"

# TOML
[llm]
backend = "lmstudio"
endpoint = "http://localhost:1234/v1"
```

### Common Commands

```bash
# Quick start
devit suggest --goal "add error handling" --pretty
devit apply --patch-file patch.diff --dry-run
devit run --goal "add comprehensive tests"

# Configuration
cp examples/devit.sample.toml devit.toml
devit suggest --help

# Troubleshooting
devit --log-level debug suggest --goal "test"
devit --pretty test  # Check stack detection
```

---

**Authors**: N-Engine, GPT-5 Thinking (ChatGPT), and Claude (Anthropic)

**Status**: Beta - Production-ready with ongoing security enhancements
