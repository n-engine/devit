#!/usr/bin/env bash
set -euo pipefail

TMP_ERR_FILE="/tmp/desktop_input_demo_jq.$$"
trap 'rm -f "$TMP_ERR_FILE"' EXIT

MESSAGE_URL="http://127.0.0.1:3001/message"
AUTH_TOKEN=""
MOVE_X=360
MOVE_Y=240
TYPE_TEXT="Hello from devit_keyboard!"
SKIP_SCREENSHOT=false
EXTRA_ARGS=()

usage() {
  cat <<'EOF'
desktop_input_demo.sh - sanity-check devit_mouse/devit_keyboard/devit_screenshot over MCP HTTP.

Usage:
  desktop_input_demo.sh [--url http(s)://host:port/message] [--bearer TOKEN]
                        [--move-x px] [--move-y px] [--text "string"] [--skip-screenshot]

Environment prerequisites:
  - MCP server running (HTTP transport) exposing devit_mouse/devit_keyboard/devit_screenshot.
  - DISPLAY and xdotool available on the host where the server executes.
  - curl + jq installed locally.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --url)
      MESSAGE_URL="$2"; shift 2;;
    --bearer)
      AUTH_TOKEN="$2"; shift 2;;
    --move-x)
      MOVE_X="$2"; shift 2;;
    --move-y)
      MOVE_Y="$2"; shift 2;;
    --text)
      TYPE_TEXT="$2"; shift 2;;
    --skip-screenshot)
      SKIP_SCREENSHOT=true; shift;;
    --help|-h)
      usage; exit 0;;
    --)
      shift; EXTRA_ARGS+=("$@"); break;;
    *)
      echo "Unknown argument: $1" >&2; usage; exit 1;;
  esac
done

command -v curl >/dev/null 2>&1 || { echo "curl is required" >&2; exit 1; }
command -v jq >/dev/null 2>&1 || { echo "jq is required" >&2; exit 1; }
command -v python3 >/dev/null 2>&1 || { echo "python3 is required" >&2; exit 1; }

if [[ -z "${DISPLAY:-}" ]]; then
  echo "Warning: DISPLAY is not set. devit_mouse/devit_keyboard will likely fail unless the server sets it." >&2
fi

if ! [[ "$MOVE_X" =~ ^-?[0-9]+$ && "$MOVE_Y" =~ ^-?[0-9]+$ ]]; then
  echo "move-x / move-y must be integers" >&2; exit 1;
fi

AUTH_HEADER=()
if [[ -n "$AUTH_TOKEN" ]]; then
  AUTH_HEADER+=("-H" "Authorization: Bearer $AUTH_TOKEN")
fi

call_rpc() {
  local payload="$1"
  local label="$2"
  local response
  response=$(curl -sS -X POST -H "Content-Type: application/json" "${AUTH_HEADER[@]}" --data "$payload" "$MESSAGE_URL")
  if [[ -z "$response" ]]; then
    echo "Empty response while calling $label" >&2
    exit 1
  fi
  if jq -e '.error' >/dev/null 2>&1 <<<"$response"; then
    echo "RPC error during $label:" >&2
    jq '.' <<<"$response" >&2
    exit 1
  fi
  echo "$response"
}

print_text_content() {
  local response="$1"
  local label="$2"
  set +e
  local extracted
  extracted=$(jq -r '.result.content[]? | select(.type=="text") | .text' <<<"$response" 2>"$TMP_ERR_FILE")
  local status=$?
  set -e
  if [[ $status -ne 0 ]]; then
    echo "  [$label] raw response (unable to parse JSON)"
    echo "$response"
  elif [[ -z "$extracted" ]]; then
    echo "  [$label] no text content in response"
  else
    while IFS= read -r line; do
      echo "  $line"
    done <<<"$extracted"
  fi
}

echo "[1/5] initialize"
INIT_PAYLOAD='{"jsonrpc":"2.0","id":0,"method":"initialize"}'
call_rpc "$INIT_PAYLOAD" "initialize" >/dev/null

echo "[2/5] tools/list (checking devit_mouse/devit_keyboard)"
tools_resp=$(call_rpc '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' "tools/list")
if ! jq -e '.result.tools[]?.name | select(. == "devit_mouse")' >/dev/null <<<"$tools_resp"; then
  echo "devit_mouse not reported by server" >&2
  exit 1
fi
if ! jq -e '.result.tools[]?.name | select(. == "devit_keyboard")' >/dev/null <<<"$tools_resp"; then
  echo "devit_keyboard not reported by server" >&2
  exit 1
fi
echo "  -> tools detected"

echo "[3/5] devit_mouse"
mouse_payload=$(python3 - "$MOVE_X" "$MOVE_Y" <<'PY'
import json, sys
move_x = int(sys.argv[1])
move_y = int(sys.argv[2])
payload = {
    "jsonrpc": "2.0",
    "id": 2,
    "method": "tools/call",
    "params": {
        "name": "devit_mouse",
        "arguments": {
            "actions": [
                {"type": "move", "x": move_x, "y": move_y, "sync": True},
                {"type": "sleep", "millis": 250},
                {"type": "click", "button": 1, "count": 1},
                {"type": "sleep", "millis": 250},
            ]
        }
    }
}
print(json.dumps(payload))
PY
)
mouse_resp=$(call_rpc "$mouse_payload" "devit_mouse")
print_text_content "$mouse_resp" "devit_mouse"

echo "[4/5] devit_keyboard"
keyboard_payload=$(python3 - "$TYPE_TEXT" <<'PY'
import json, sys
text = sys.argv[1]
payload = {
    "jsonrpc": "2.0",
    "id": 3,
    "method": "tools/call",
    "params": {
        "name": "devit_keyboard",
        "arguments": {
            "actions": [
                {"type": "text", "text": text, "delay_ms": 20},
                {"type": "sleep", "millis": 300},
            ]
        }
    }
}
print(json.dumps(payload))
PY
)
keyboard_resp=$(call_rpc "$keyboard_payload" "devit_keyboard")
print_text_content "$keyboard_resp" "devit_keyboard"

if [[ "$SKIP_SCREENSHOT" == false ]]; then
  echo "[5/5] devit_screenshot"
  shot_resp=$(call_rpc '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"devit_screenshot","arguments":{}}}' "devit_screenshot")
  print_text_content "$shot_resp" "devit_screenshot"
  shot_path=$(jq -r '.result.structuredContent.image.path // empty' <<<"$shot_resp")
  if [[ -n "$shot_path" ]]; then
    echo "  Screenshot saved at: $shot_path"
  fi
else
  echo "[5/5] devit_screenshot skipped (per flag)"
fi

echo
echo "✅ Desktop input demo completed."
