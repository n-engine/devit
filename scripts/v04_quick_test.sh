#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
DEVIT="$ROOT_DIR/target/debug/devit"
MCP_SERVER="$ROOT_DIR/target/debug/mcp-server"
MCP="$ROOT_DIR/target/debug/devit-mcp"
SERVER_CMD_BASE="'$MCP_SERVER' --working-dir '$ROOT_DIR'"
SERVER_HELP="$("$MCP_SERVER" --help 2>/dev/null || true)"
REPORT=".devit/reports"
mkdir -p "$REPORT"

if [[ ! -x "$DEVIT" || ! -x "$MCP_SERVER" || ! -x "$MCP" ]]; then
  echo "[build] building devit/devit-mcp/mcp-server…" >&2
  cargo build -q -p devit-cli --features experimental
  cargo build -q -p mcp-server
fi

TOK="ghp_$(printf 'a%.0s' $(seq 1 36))"
PLACE="REDACTED"

pass(){ printf '\033[32mPASS\033[0m %s\n' "$*"; }
fail(){ printf '\033[31mFAIL\033[0m %s\n' "$*"; exit 1; }

echo "[1/3] MCP redaction via echo" >&2
PAY_ECHO=$(jq -cn --arg t "$TOK" '{tool:"echo",args:{msg:("token " + $t)}}')
FLAGS_REDACT=""
if echo "$SERVER_HELP" | grep -q -- "--secrets-scan"; then FLAGS_REDACT+=" --secrets-scan"; fi
if echo "$SERVER_HELP" | grep -q -- "--redact-placeholder"; then FLAGS_REDACT+=" --redact-placeholder '$PLACE'"; fi

OUT1=$("$MCP" --cmd "$SERVER_CMD_BASE$FLAGS_REDACT" --call devit.tool_call --json "$PAY_ECHO" || true)
echo "$OUT1" > "$REPORT/v04_echo.json"
echo "$OUT1" | rg -q "$PLACE|\"redacted\"\s*:\s*true" && pass "echo redacted" || fail "echo non redacted"

echo "[2/3] MCP redaction via shell_exec (echo token)" >&2
PAY_SH=$(jq -cn --arg t "$TOK" '{tool:"shell_exec",args:{cmd:("echo token " + $t)}}')
child_dump_arg=""
if echo "$SERVER_HELP" | grep -q -- "--child-dump-dir"; then
  child_dump_arg=" --child-dump-dir .devit/reports"
fi
OUT2=$("$MCP" --cmd "$SERVER_CMD_BASE$FLAGS_REDACT$child_dump_arg" --call devit.tool_call --json "$PAY_SH" || true)
echo "$OUT2" > "$REPORT/v04_shell.json"
echo "$OUT2" | rg -q "$PLACE|\"redacted\"\s*:\s*true" && pass "shell_exec redacted" || {
  echo "-- child stdout/stderr (last) --" >&2
  ls -t .devit/reports/child_* 2>/dev/null | head -n2 | xargs -r -I {} sh -c 'echo ========== {}; sed -n "1,80p" {}'
  fail "shell_exec non redacted"
}

echo "[3/3] DevIt CLI direct (json-only)" >&2
OUT3=$(jq -cn --arg t "$TOK" '{name:"shell_exec",args:{cmd:("echo token " + $t)}}' | "$DEVIT" tool call - --json-only || true)
echo "$OUT3" > "$REPORT/v04_cli.json"
echo "$OUT3" | rg -q "token" && pass "cli json-only ok" || fail "cli json-only vide"

echo "OK"
