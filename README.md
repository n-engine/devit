# DevIt

DevIt is a Rust toolchain that lets Large Language Models (and humans) work safely on your codebase.  
It combines a CLI (`devit`), a background daemon (`devitd`) and an MCP/HTTP server so that agents such as Claude Desktop can apply patches, run tests and inspect files without escaping the sandbox.

> **Status**: actively developed (daily use on Linux & Windows VMs). APIs may evolve before 1.0.

---

## ✨ What you get

- **Secure orchestration** – approval levels, protected paths and per-command sandbox profiles.
- **Atomic patching** – unified diff parsing, idempotent application, rollback command generation.
- **Audit trail** – every operation is journaled with HMAC signatures for later verification.
- **MCP compatible** – expose 30+ DevIt tools to Claude Desktop or any MCP compliant client.
- **Cross‑platform** – daemon and tools run on Linux, macOS and Windows (via PowerShell helper scripts).
- **Auto housekeeping** – idle daemon auto-shutdown, secret based authentication, named pipes on Windows.

---

## 🧱 Architecture at a glance

```
┌──────────┐   JSON/HMAC   ┌────────────┐   sandboxed processes   ┌─────────────┐
│ devit CLI│ ─────────────▶│  devitd    │ ───────────────────────▶│ your system │
└──────────┘               │ (daemon)   │                         └─────────────┘
        ▲                  └────┬───────┘
        │                       │
        │   WebSocket/SSE       │
        │                       ▼
┌─────────────┐  ─────────────▶ ┌──────────────┐
│ MCP client  │                 │  mcp-server  │
│ (Claude, …) │ ◀────────────── │  (HTTP/SSE)  │
└─────────────┘                 └──────────────┘
```

---

## ✅ Requirements

| Platform | Minimum | Notes |
|----------|---------|-------|
| Linux / macOS | Rust 1.79+, Git, OpenSSL | main development targets |
| Windows 10/11 | Visual Studio Build Tools, Rust MSVC toolchain | use provided PowerShell scripts |
| CPU/RAM | any modern 4+ cores / 8 GB | full test suite benefits from 16 GB+ |

If you run Claude Desktop over HTTP/SSE, plan for a reverse proxy (Caddy/ngrok) with HTTPS.

---

## 🚀 Installation

### Linux / macOS

```bash
# 1. Install binaries
cargo install devit devitd mcp-server

# 2. Pick a shared secret used by devit (CLI), devitd and mcp-server
export DEVIT_SECRET="$(openssl rand -hex 32)"

# 3. Start the daemon (Unix socket by default)
devitd --socket /tmp/devitd.sock --secret "$DEVIT_SECRET"

# 4. In another terminal, point the CLI to the daemon
export DEVIT_DAEMON_SOCKET=/tmp/devitd.sock
devit snapshot --pretty
```

> Tip: add the `export` lines to your shell profile for convenience.

### Windows (PowerShell)

```powershell
# From the repository
Set-Location C:\Users\you\devIt

# Build release binaries once
cargo build --release --target x86_64-pc-windows-msvc

# Launch the daemon with helper script
.\scripts\run_devitd_windows.ps1 `
    -Socket \\.\pipe\devitd `
    -Secret 0143c321920e55bd9b17bb0d5ac8543c6fa0200961803c3ff01598e4e6f4007b `
    -Config .\win_devit.core.toml

# Point the CLI to the named pipe
$env:DEVIT_SECRET="0143c321920e55bd9b17bb0d5ac8543c6fa0200961803c3ff01598e4e6f4007b"
$env:DEVIT_DAEMON_SOCKET="\\.\pipe\devitd"
target\x86_64-pc-windows-msvc\release\devit.exe snapshot --pretty
```

### From source

```bash
git clone https://github.com/n-engine/devit.git
cd devit
cargo build --workspace
```

---

## 🔁 Quick start (CLI)

```bash
# Ensure the daemon is running and DEVIT_SECRET matches
devit snapshot --pretty               # create a baseline snapshot
devit file-read README.md --pretty    # safe read with policy enforcement
devit patch-preview samples/ok_add_fn.diff --pretty
devit patch-apply samples/ok_add_fn.diff --dry-run --pretty
```

Each command:
- checks the approval level you requested (`--approval moderate` by default),
- enforces sandbox rules (protected paths, symlink checks, approval downgrade),
- records a signed journal entry under `.devit/journal.jsonl`.

---

## 🤝 MCP / Claude Desktop setup

1. **Expose the HTTP server**
   ```bash
   export DEVIT_SECRET=...              # same secret as devitd
   export DEVIT_DAEMON_SOCKET=/tmp/devitd.sock
   mcp-server --transport http --host 127.0.0.1 --port 3001 \
       --working-dir /path/to/workspace --enable-sse
   ```

2. **Publish the MCP manifest**
   Serve a file at `https://yourdomain/.well-known/mcp.json` containing:
   ```json
   {
     "protocolVersion": "2025-06-18",
     "transport": {
       "type": "http",
       "url": "https://yourdomain/message",
       "sseUrl": "https://yourdomain/sse"
     },
     "capabilities": {
       "tools": {},
       "resources": {},
       "prompts": {}
     },
     "serverInfo": {
       "name": "DevIt MCP Server",
       "version": "0.1.0",
       "description": "Expose DevIt tools over MCP HTTP"
     }
   }
   ```

3. **Configure Claude Desktop**
   - In *Developer → MCP Servers → Add Server*, paste the manifest URL.
   - If you tunnel via ngrok, append `?ngrok-skip-browser-warning=1` to both manifest and transport URLs.
   - Claude now sees `devit_file_read`, `devit_patch_apply`, `devit_screenshot`, etc.

---

## 🔐 Security model

- **Shared secret + HMAC** – all clients (CLI/MCP) sign their requests with `DEVIT_SECRET`. Nonces and timestamps prevent replays.
- **Approval engine** – operations go through the policy engine (levels: Untrusted → Privileged). Protected paths, binary whitelists, submodule edits and exec-bit toggles are automatically downgraded or denied.
- **Sandbox profiles** – each tool runs with a profile (`strict` / `permissive`) controlling filesystem access. Symlinks outside the workspace are blocked.
- **Audit journal** – `.devit/journal.jsonl` stores every action with a truncated HMAC. The `devit_journal_verify` helper can replay and verify integrity.
- **Auto-shutdown** – `devitd` can terminate after `DEVIT_AUTO_SHUTDOWN_AFTER` seconds of inactivity to reduce exposure.
- **HTTP transport** – the MCP server is stateless; put it behind HTTPS and forward `Authorization: Bearer …` if you need tokens per client.

---

## ⚙️ Configuration cheatsheet

| Variable | Default | Meaning |
|----------|---------|---------|
| `DEVIT_SECRET` | _required_ | shared secret used for HMAC |
| `DEVIT_DAEMON_SOCKET` | `/tmp/devitd.sock` (Unix) | socket/pipename for CLI → daemon |
| `DEVIT_AUTO_SHUTDOWN_AFTER` | `0` (disabled) | idle timeout in seconds |
| `DEVIT_ORCHESTRATION_MODE` | `daemon` | `local` to skip auto-connecting to devitd |
| `DEVIT_SNAPSHOT_DIR` | `.devit/snapshots` | where snapshots are stored |

Configuration files:
- `devit.toml` for CLI defaults (approvals, sandbox, git policies).
- `devitd.core.toml` or `win_devit.core.toml` for daemon worker definitions.

---

## 🛠️ Development & tests

```bash
cargo fmt
cargo check
cargo test -p devit-cli --test contract_test_4
```

Notes:
- Many integration tests spawn `devitd`. In CI-like sandboxes (no process spawning), use `cargo test --workspace --no-run` or set `DEVIT_SKIP_DAEMON_TESTS=1`.
- Windows scripts live under `scripts/`. The PowerShell helpers kill previous daemons before launching new ones to avoid zombies.

---

## 📁 Repository layout

```
crates/
  agent/          # high-level orchestration helper
  cli/            # devit CLI + core engine
  common/         # shared types (ApprovalLevel, SandboxProfile…)
  mcp-*           # MCP server + tool wrappers
  sandbox/        # sandbox utilities
devitd/           # daemon executable
scripts/          # setup helpers (Linux & Windows)
site/             # landing page (React/Tailwind)
.archive/         # legacy binaries (approver, patch-fix…)
```

---

## 🙋 Support & resources

- Issues / feature requests: <https://github.com/n-engine/devit/issues>
- Claude Desktop MCP doc (guide interne) : `docs/mcp_setup.md`
- Windows quickstart : `docs/windows_daemon_setup.md`
- Security policies : `docs/approvals.md`

Contributions are welcome! Open a PR with a short description and run `cargo fmt && cargo check` before pushing. If the pre-commit hook is too opinionated, set `SKIP_HOOK=1` in your environment while we streamline it.
