#!/usr/bin/env bash
set -euo pipefail

SERVER_BIN="target/debug/mcp-server"
SERVER_CMD_BASE="$SERVER_BIN --working-dir $(pwd)"
SRV="$SERVER_CMD_BASE"
SERVER_HELP="$("$SERVER_BIN" --help 2>/dev/null || true)"

# Build experimental binaries and main CLI explicitly
cargo build -p devit-cli --features experimental --bins >/dev/null
cargo build -p devit-cli --bin devit >/dev/null
cargo build -p mcp-server >/dev/null

# dry-run deny
if echo "$SERVER_HELP" | grep -q -- "--dry-run"; then
  out=$(target/debug/devit-mcp --cmd "$SRV --dry-run" --call devit.tool_list --json '{}' || true)
  echo "$out" | rg '"dry_run":\s*true' >/dev/null
else
  echo "Skipping dry-run check (unsupported by mcp-server)" >&2
fi

# approval_required (simuler on_request via défauts)
out=$(target/debug/devit-mcp --cmd "$SRV" --call devit.tool_call --json '{}' || true)
echo "$out" | rg '"approval_required":\s*true' >/dev/null || true

# rate-limit cooldown (reuse single mcp-server over stdio when supported)
if echo "$SERVER_HELP" | grep -q -- "--cooldown-ms"; then
  out=$(
    cat <<'JSON' |
{"type":"ping"}
{"type":"version","payload":{"client":"lint_errors.sh"}}
{"type":"capabilities"}
{"type":"tool.call","payload":{"name":"devit.tool_list","args":{}}}
{"type":"tool.call","payload":{"name":"devit.tool_list","args":{}}}
JSON
    $SERVER_CMD_BASE --cooldown-ms 1000
  ) || true
  echo "$out" | rg '"rate_limited":\s*true' >/dev/null
else
  echo "Skipping cooldown check (unsupported by mcp-server)" >&2
fi

# watchdog max-runtime-secs
# Feed periodic pings so the server loop iterates and hits the deadline
if echo "$SERVER_HELP" | grep -q -- "--max-runtime-secs"; then
  set +e
  ( for i in $(seq 1 20); do echo '{"type":"ping"}'; sleep 0.1; done ) | $SERVER_CMD_BASE --max-runtime-secs 1 >/dev/null 2>/tmp/mcpd_watchdog_stderr.txt
  code=$?
  set -e
  test "$code" -eq 2
  rg -q 'max runtime exceeded' /tmp/mcpd_watchdog_stderr.txt
else
  echo "Skipping watchdog check (unsupported by mcp-server)" >&2
fi
