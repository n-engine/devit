#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
BIN_DEVIT="$ROOT_DIR/target/debug/devit"
BIN_MCP_SERVER="$ROOT_DIR/target/debug/mcp-server"
BIN_MCP="$ROOT_DIR/target/debug/devit-mcp"

if [[ ! -x "$BIN_DEVIT" || ! -x "$BIN_MCP_SERVER" || ! -x "$BIN_MCP" ]]; then
  echo "Building binaries…" >&2
  cargo build -q -p devit-cli --features experimental
  cargo build -q -p mcp-server --quiet
fi

PAY='{"tool":"shell_exec","args":{"cmd":"printf ghp_ABCDEF1234567890"}}'

echo "Running MCP redaction smoke (sandbox=none)…" >&2
"$BIN_MCP" --cmd "'$BIN_MCP_SERVER' --working-dir '$ROOT_DIR'" \
  --call devit.tool_call --json "$PAY" | tee /tmp/mcp_redaction.out

echo "Searching for redaction markers…" >&2
grep -E 'REDACTED|redacted' /tmp/mcp_redaction.out && echo "OK" || { echo "Redaction markers not found" >&2; exit 1; }
