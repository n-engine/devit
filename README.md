# DevIt

**The problem:** LLMs can code. But giving them direct access to your filesystem, shell, and git history is suicide.

**The solution:** DevIt is a Rust-based security sandbox that lets AI agents work on your codebase without shooting themselves (or you) in the foot. Every operation goes through an approval engine, gets HMAC-signed, and lands in an immutable audit trail.

**The killer feature:** Multi-LLM orchestration with visual debugging. Claude Desktop can delegate heavy refactoring to GPT-5, monitor progress via screenshots, and get OCR alerts when builds fail. All sandboxed. All audited.

> **Status:** Linux stable; macOS validated; Windows support active (PowerShell scripts provided). APIs stabilizing for 1.0.

---

## 🎯 What makes DevIt different

Most "AI coding assistants" are wrappers around `subprocess.run()` with fingers crossed. DevIt is paranoid by design:

**Security theater → Actual security:**
- HMAC signatures on every request (nonce + timestamp included; replay window enforcement planned)
- Approval levels: `Untrusted → Low → Moderate → High → Privileged`
- Protected paths: `.git/`, `~/.ssh/`, `/etc/` = instant rejection
- Audit journal: Every action logged with truncated HMAC for verification

**Single LLM → Multi-LLM orchestration:**
```bash
# Claude Desktop delegates to GPT-5 for analysis
devit delegate "refactor auth module" --worker codex

# Monitor execution with visual feedback
devit screenshot  # Gets embedded thumbnail in response

# Auto-detect build failures via OCR
devit ocr-alerts --rules build_failures,port_conflicts
```

**Blind execution → Visual debugging:**
- **Screenshot tool**: Capture desktop, get 480px thumbnail (~30KB) embedded in MCP response
- **OCR tool**: Extract text from screenshots (Tesseract), detect errors/success patterns
- **OCR Alerts**: Regex rules on OCR output → auto-trigger notifications when builds fail

---

## ⚡ Quick example: Before/After

**Without DevIt:**
```python
# LLM executes this
subprocess.run(['rm', '-rf', user_input])  # 🔥 YOLO
```

**With DevIt:**
```bash
# 1. Claude sends patch via MCP
devit_patch_apply samples/refactor.diff

# 2. DevIt policy engine validates:
#    - Approval level sufficient? ✅
#    - Protected paths touched? ❌ (rejected)
#    - Symlinks outside workspace? ❌ (blocked)
#    - Binary executable changes? ⚠️ (downgraded approval)

# 3. Atomic application or rollback
#    - Applied cleanly → journal entry signed
#    - Conflict detected → rollback command generated

# 4. Audit trail
cat .devit/journal.jsonl
{"op":"patch_apply","approval":"moderate","hmac":"a3f2...","timestamp":...}
```

---

## 🧱 Architecture

```
┌──────────────┐
│ Claude       │  MCP tools (devit_file_read, devit_patch_apply, 
│ Desktop      │             devit_delegate, devit_screenshot...)
└──────┬───────┘
       │ HTTP/SSE or stdio
       ▼
┌──────────────┐
│ mcp-server   │  Stateless MCP → daemon bridge
│ (Rust)       │  HMAC verification, tool dispatch
└──────┬───────┘
       │ Unix socket / named pipe
       ▼
┌──────────────┐
│ devitd       │  Persistent daemon
│ (daemon)     │  - Task registry (multi-LLM orchestration)
│              │  - Process manager (background workers)
│              │  - Screenshot/OCR capabilities
└──────┬───────┘
       │ sandboxed spawns
       ▼
┌──────────────┐
│ Workers      │  Claude Code, GPT-5, Ollama, custom tools
│              │  Each runs in isolated profile (strict/permissive)
└──────────────┘
```

**Key insight:** The daemon is stateful. MCP server is stateless. This lets multiple AI agents (Claude Desktop, Cursor, CLI) coordinate through the daemon without stepping on each other.

---

## 🚀 Installation

### Linux / macOS (5 minutes)

```bash
# 1. Install Rust toolchain (if not already)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 2. Build from source
git clone https://github.com/n-engine/devit.git
cd devit
cargo build --release --workspace

# 3. Generate shared secret
export DEVIT_SECRET="$(openssl rand -hex 32)"

# 4. Start daemon
./target/release/devitd --socket /tmp/devitd.sock --secret "$DEVIT_SECRET" &

# 5. Verify with CLI
export DEVIT_DAEMON_SOCKET=/tmp/devitd.sock
./target/release/devit snapshot --pretty

# 6. (Optional) Start MCP server for Claude Desktop
./target/release/mcp-server \
    --transport http \
    --host 127.0.0.1 \
    --port 3001 \
    --working-dir $(pwd) \
    --enable-sse
```

**Production tip:** Put the exports in your `.bashrc`/`.zshrc` and create systemd units for `devitd` and `mcp-server`.

### Windows (PowerShell)

```powershell
# 1. Build release binaries
cargo build --release --target x86_64-pc-windows-msvc

# 2. Launch daemon via helper script
.\scripts\run_devitd_windows.ps1 `
    -Socket \\.\pipe\devitd `
    -Secret $(openssl rand -hex 32) `
    -Config .\win_devit.core.toml

# 3. Point CLI to named pipe
$env:DEVIT_SECRET = "<your-secret>"
$env:DEVIT_DAEMON_SOCKET = "\\.\pipe\devitd"
.\target\x86_64-pc-windows-msvc\release\devit.exe snapshot --pretty
```

**Windows notes:**
- Named pipes instead of Unix sockets
- PowerShell scripts handle process lifecycle
- Tesseract OCR: `.\scripts\install-tesseract-windows.ps1`

---

## 🎮 Claude Desktop setup (MCP over HTTP)

**Goal:** Expose DevIt tools to Claude Desktop via HTTP + SSE transport.

### 1. Expose MCP server

```bash
# Start with SSE enabled (required for streaming)
export DEVIT_SECRET="<your-secret>"
export DEVIT_DAEMON_SOCKET=/tmp/devitd.sock

mcp-server --transport http \
    --host 0.0.0.0 \
    --port 3001 \
    --working-dir /path/to/your/project \
    --enable-sse
```

### 2. Tunnel to HTTPS (if remote)

Claude Desktop requires HTTPS. Options:
- **ngrok:** `ngrok http 3001` (add `?ngrok-skip-browser-warning=1` to URLs)
- **Caddy reverse proxy:** Auto HTTPS with Let's Encrypt
- **Cloudflare Tunnel:** Zero-config HTTPS

### 3. Create MCP manifest

Serve this at `https://yourdomain.com/.well-known/mcp.json`:

```json
{
  "protocolVersion": "2025-06-18",
  "transport": {
    "type": "http",
    "url": "https://yourdomain.com/message",
    "sseUrl": "https://yourdomain.com/sse"
  },
  "capabilities": {"tools": {}, "resources": {}, "prompts": {}},
  "serverInfo": {
    "name": "DevIt MCP Server",
    "version": "0.1.0"
  }
}
```

### 4. Add to Claude Desktop

Settings → Developer → MCP Servers → Add Server → Paste manifest URL

**Verify:** Open Claude Desktop chat. Type "what tools do you have?" → Should list `devit_file_read`, `devit_patch_apply`, `devit_delegate`, `devit_screenshot`, etc.

### SSE requirements (important)

- Emit an initial `event: ready` with `data: {}` as soon as the SSE connection opens.
- Send periodic heartbeats (e.g., every 10–15s) and flush each write.
- Disable compression on `/sse` (gzip/zstd breaks SSE framing).
- Use HTTP/1.1 between reverse proxy and backend to preserve chunked flush behavior.

---

## 🎮 Claude Desktop setup (STDIO)

Claude can also run DevIt via STDIO (no HTTP, no reverse proxy).

1) Build the server

```bash
cargo build -p mcp-server --release
```

2) Launch in STDIO mode

```bash
RUST_LOG=info ./target/release/mcp-server --transport stdio --working-dir /path/to/your/project
```

3) Add to Claude Desktop

- Settings → Developer → MCP Servers → Add Local
- Command: absolute path to `./target/release/mcp-server`
- Args: `["--transport","stdio","--working-dir","/path/to/your/project"]`
- Env (recommended): `RUST_LOG=info`
- Optional network safety envs: `DEVIT_RESPECT_ROBOTS=1 DEVIT_FOLLOW_REDIRECTS=1 DEVIT_BLOCK_PRIVATE_CIDRS=1`

---

## 🔥 Killer use cases

### 1. Multi-LLM refactoring pipeline

```bash
# Claude Desktop delegates heavy lifting to GPT-5
devit delegate "migrate Express to Fastify" --worker codex --model gpt-5

# Monitor task status
devit status --pretty

# GPT-5 completes → notifies Claude → Claude reviews diff
```

**Config** (in `win_devit.core.toml` or `devit.core.toml`):
```toml
[workers.codex]
type = "mcp"
binary = "codex"
args = ["--model", "{model}", "mcp-server"]
default_model = "gpt-5"
allowed_models = ["gpt-5", "gpt-5-codex"]
```

### 2. Visual debugging loop

```bash
# 1. Claude runs tests
devit exec cargo test

# 2. Captures screenshot on error
devit screenshot

# 3. OCR extracts error message
devit ocr --zone terminal_bottom

# 4. Regex alerts detect failure
devit ocr-alerts --rules build_failures --action notify

# 5. Auto-notifies orchestrator → retry with fix
```

**Rules** (built-in):
- `build_failures`: `(BUILD FAILED|compilation failed)`
- `port_conflicts`: `(EADDRINUSE|port.*already.*use)`
- `panic_crash`: `(panic|segfault|core dumped)`
- `success_confirmations`: `(✓.*PASS|build.*success)`

### 3. Paranoid patch application

```bash
# Claude proposes patch
devit patch-apply refactor.diff --dry-run --pretty

# Output shows:
# ✅ Hunks: 3 valid, 0 invalid
# ⚠️  Protected path: src/.git/config → REJECTED
# ✅ Sandbox check: All paths within workspace
# ✅ Symlink check: No symlinks outside workspace
# 📝 Rollback command: devit patch-apply rollback_<hash>.diff

# Apply if green
devit patch-apply refactor.diff --approval moderate
```

---

## 🔐 Security model (simplified)

### Threat model
DevIt defends against:
1. **Malicious LLM prompts** (jailbreak attempts, path traversal)
2. **Accidental chaos** (Claude removes `.git/` by mistake)
3. **Supply chain attacks** (injected code in patches)

### Defense layers

**Layer 1: HMAC signatures**
- Every request signed with `DEVIT_SECRET` (nonce + timestamp included)
- Replay window enforcement planned (anti-replay cache with skew window)
- No signature = instant 401

**Layer 2: Approval engine**
- Operations start at `Moderate` approval (default)
- Policy engine downgrades/rejects based on risk:
  - Protected paths → `Privileged` (rejected if insufficient)
  - Binary changes → downgrade to `Low`
  - Exec bit toggles → downgrade to `Low`
  - Submodule edits → downgrade to `Moderate`

**Layer 3: Sandbox profiles**
- `strict`: Filesystem access limited to workspace (default for patches)
- `permissive`: Broader access for builds (still no `/etc/`, `~/.ssh/`)
- Process isolation via platform-specific backends (Unix: sandbox, Windows: Job Objects)

**Layer 4: Audit trail**
- `.devit/journal.jsonl` logs every operation
- HMAC truncated to 8 chars (verifiable with `devit journal-verify`)
- Immutable append-only log (tamper detection)

**Layer 5: Auto-shutdown**
- Daemon terminates after `DEVIT_AUTO_SHUTDOWN_AFTER` seconds idle
- Reduces attack surface when unused

---

## ⚙️ Configuration cheatsheet

| Variable | Default | Meaning |
|----------|---------|---------|
| `DEVIT_SECRET` | **required** | Shared secret for HMAC (32+ hex chars) |
| `DEVIT_DAEMON_SOCKET` | `/tmp/devitd.sock` | Unix socket or Windows named pipe |
| `DEVIT_AUTO_SHUTDOWN_AFTER` | `0` (off) | Idle timeout in seconds |
| `DEVIT_ORCHESTRATION_MODE` | `daemon` | `local` = skip daemon (tests only) |

### Config files

- **`devit.toml`**: CLI defaults (approval levels, sandbox profiles)
- **`devit.core.toml` / `win_devit.core.toml`**: Daemon worker definitions (Claude Code, GPT-5, Ollama)

### Network (Search/Fetch) ENV

- `DEVIT_ENGINE`: search engine (`ddg`).
- `DEVIT_DDG_BASE`: DDG HTML base (default `https://duckduckgo.com/html`).
- `DEVIT_SEARCH_TIMEOUT_MS`: global search timeout (100..10000 ms, default 8000).
- `DEVIT_FETCH_TIMEOUT_MS`: global fetch timeout (100..10000 ms, default 8000).
- `DEVIT_HTTP_USER_AGENT`: HTTP User-Agent (default `DevItBot/1.0`).
- `DEVIT_RESPECT_ROBOTS`: `1/0` respect robots.txt (default 1).
- `DEVIT_FOLLOW_REDIRECTS`: `1/0` follow limited redirects (default 1).
- `DEVIT_BLOCK_PRIVATE_CIDRS`: `1/0` block private/local hosts (default 1).

**Example worker config:**
```toml
[workers.ollama_local]
type = "cli"
binary = "ollama"
args = ["run", "{model}", "--format", "json"]
default_model = "mistral-nemo:12b"
allowed_models = ["llama3:8b", "llama3.3:70b"]
timeout = 256
```

---

## 🛠️ MCP Tools reference

DevIt exposes **30+ tools** via MCP. Highlights:

### Core file operations
- `devit_file_read` – Safe read with approval checks
- `devit_file_write` – Write with overwrite/append/create modes
- `devit_file_list` – Directory listing with metadata
- `devit_file_search` – Regex search with context lines

### Git operations
- `devit_git_log` – History with `--oneline` format
- `devit_git_blame` – Line-by-line authorship
- `devit_git_diff` – Diff between commits/ranges
- `devit_git_search` – `git grep` or `git log -S` pickaxe

### Patching
- `devit_patch_apply` – Atomic unified diff application
- `devit_patch_preview` – Validate before applying

### Orchestration
- `devit_delegate` – Assign task to another LLM worker
- `devit_notify` – Update task status (completed/failed/progress)
- `devit_orchestration_status` – List active/completed tasks
- `devit_task_result` – Fetch detailed task output

### Visual debugging
- `devit_screenshot` – Capture desktop → thumbnail embedded in response
- `devit_ocr` – Tesseract OCR on images (text/tsv/hocr formats)
- `devit_ocr_alerts` – Regex rules on OCR → auto-notify on matches

### Desktop automation (experimental)
- `devit_mouse` – Move cursor, click, scroll
- `devit_keyboard` – Type text, send key combos

Note (Linux): requires an active X11 display (`DISPLAY`). On Wayland, use XWayland; otherwise `xdotool` cannot open the display.

### Process management
- `devit_exec` – Execute binaries (foreground/background)
- `devit_ps` – Query running processes
- `devit_kill` – Terminate background process

### Web access
- `devit_search_web` – DuckDuckGo SERP scraping
- `devit_fetch_url` – HTTP GET with safety guards

**Full docs:** `docs/MCP_TOOLS.md`

---

## 📁 Repository structure

```
crates/
  agent/           # High-level orchestration helpers
  cli/             # devit CLI + core engine
  common/          # Shared types (ApprovalLevel, SandboxProfile...)
  mcp-server/      # HTTP/SSE MCP server
  mcp-tools/       # MCP tool implementations
  orchestration/   # Multi-LLM coordination (daemon/local backends)
  sandbox/         # Process isolation primitives
devitd/            # Daemon executable
scripts/           # Setup helpers (Linux & Windows)
docs/              # Configuration, MCP tools, approval policies
examples/          # Sample configs, plugins
```

---

## 🧪 Development

```bash
# Format + lint
cargo fmt
cargo clippy --all-targets

# Run specific test suite
cargo test -p devit-cli --test contract_test_4

# Full integration tests (spawns daemon)
cargo test --workspace

# CI sandbox (no daemon spawning)
DEVIT_SKIP_DAEMON_TESTS=1 cargo test --workspace
```

**Windows devs:** Use `.\scripts\run_devitd_windows.ps1` before tests. The script kills zombie daemons automatically.

---

## 🙋 Why DevIt exists

**Backstory:** After watching Claude Desktop accidentally `rm -rf` a `.git/` directory during a refactoring session, we built DevIt. The "are you sure?" prompt came *after* the damage.

**Design philosophy:**
1. **Paranoid by default** – Assume LLMs will try unsafe operations (intentionally or not)
2. **Audit everything** – Trust but verify (and log verification)
3. **Orchestration-first** – Multiple AIs should coordinate, not conflict
4. **Visual feedback** – AI agents need to "see" what's happening (screenshots/OCR)

**Non-goals:**
- Not a code editor (use VSCode/Cursor)
- Not a CI/CD system (use GitHub Actions)
- Not a deployment tool (use Docker/K8s)

DevIt is the **security and coordination layer** between AI agents and your codebase.

---

## 🚫 When NOT to use DevIt

- **Greenfield toy projects** – Overkill for "build a todo app"
- **Single-file scripts** – Just use Claude directly
- **Read-only analysis** – DevIt's value is in safe *writes*
- **Production deployments** – DevIt is for development, not prod servers

**Sweet spot:** Multi-file refactors, migrations, test generation, and any task where an LLM needs git/filesystem/shell access.

---

## 🚀 Roadmap to 1.0

- [x] Core security primitives (HMAC, approval engine, sandbox)
- [x] MCP HTTP/SSE transport
- [x] Multi-LLM orchestration (delegate/notify/status)
- [x] Visual debugging (screenshot, OCR, alerts)
- [x] Git tool suite
- [ ] API stabilization (semver guarantees)
- [ ] Performance benchmarks (latency targets)
- [ ] Windows feature parity (native screenshot backend)
- [ ] macOS testing/validation
- [ ] VSCode extension (inline DevIt commands)

**Timeline:** Q1 2026 for 1.0 candidate

---

## 📞 Support & Contributing

- **Issues/features:** [github.com/n-engine/devit/issues](https://github.com/n-engine/devit/issues)
- **Docs:** `docs/` directory (MCP setup, approval policies, Windows quickstart)
- **Contributing:** PRs welcome! Run `cargo fmt && cargo clippy` before pushing.

**Internal docs** (if you have repo access):
- `PROJECT_TRACKING/ORCHESTRATOR_GUIDE.md` – How to use orchestration
- `PROJECT_TRACKING/FEATURES/` – Feature implementation notes
- `docs/windows_daemon_setup.md` – Windows-specific setup

---

## 📜 License

Apache-2.0 license – See `LICENSE` file.

**TL;DR:** Use it, fork it, ship it. Just don't blame us if your LLM breaks something (though DevIt should prevent that).

---

**Built with Rust 🦀, paranoia 🔐, and too many hours debugging Claude's creative patch formats.**
