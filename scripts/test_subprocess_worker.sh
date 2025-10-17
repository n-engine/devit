#!/usr/bin/env bash
set -euo pipefail

if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required for this test" >&2
  exit 1
fi

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
DEVIT_BIN_DEFAULT="$ROOT_DIR/target/debug/devit"
DEVITD_BIN_DEFAULT="$ROOT_DIR/target/debug/devitd"

declare -r DEVIT_BIN=${DEVIT_BIN:-$DEVIT_BIN_DEFAULT}
declare -r DEVITD_BIN=${DEVITD_BIN:-$DEVITD_BIN_DEFAULT}

cargo build --quiet --bin devit --bin devitd >/dev/null

sanitize_json() {
  sed -E 's/\x1B\[[0-9;]*[A-Za-z]//g' | awk 'BEGIN{capture=0} {
    if (capture==0 && match($0, /^[[:space:]]*[{[]/)) capture=1;
    if (capture) print;
  }'
}

TMP_DIR=$(mktemp -d "$ROOT_DIR/target/devitd-test.XXXXXX")
SOCKET="$TMP_DIR/devitd.sock"
CONFIG="$TMP_DIR/devit.core.toml"
WORKER_SCRIPT="$TMP_DIR/worker.sh"
SECRET="test-subprocess-secret"

cat <<'WORKER' >"$WORKER_SCRIPT"
#!/usr/bin/env bash
set -euo pipefail
# Goal is passed as the first argument; we ignore it for this smoke test.
shift || true
echo '{"result":"subprocess-ok","details":{"worker":"test_worker"}}'
WORKER
chmod +x "$WORKER_SCRIPT"

cat >"$CONFIG" <<CFG
[workspace]
sandbox_root = "${ROOT_DIR}"

[orchestration]
mode = "daemon"
auto_start_daemon = false
daemon_socket = "${SOCKET}"
default_timeout_secs = 30

[workers.test_worker]
type = "cli"
binary = "${WORKER_SCRIPT}"
args = ["{goal}"]
timeout_secs = 30
parse_mode = "json"
CFG

cleanup() {
  if [[ -n "${DAEMON_PID:-}" ]]; then
    kill "$DAEMON_PID" 2>/dev/null || true
    wait "$DAEMON_PID" 2>/dev/null || true
  fi
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

DEVIT_SECRET="$SECRET" "$DEVITD_BIN" --socket "$SOCKET" --config "$CONFIG" --secret "$SECRET" &
DAEMON_PID=$!

for _ in {1..25}; do
  [[ -S "$SOCKET" ]] && break
  sleep 0.2
done

if [[ ! -S "$SOCKET" ]]; then
  echo "daemon socket not ready" >&2
  exit 1
fi

DELEGATE_RAW=$(DEVIT_SECRET="$SECRET" DEVIT_CORE_CONFIG="$CONFIG" "$DEVIT_BIN" delegate \
  --goal "Smoke test subprocess worker" \
  --delegated-to test_worker \
  --timeout 30)
DELEGATE_OUTPUT=$(printf '%s\n' "$DELEGATE_RAW" | sanitize_json)
TASK_ID=$(echo "$DELEGATE_OUTPUT" | jq -r '.task_id')

if [[ -z "$TASK_ID" || "$TASK_ID" == "null" ]]; then
  echo "Failed to parse task_id from delegate output" >&2
  echo "$DELEGATE_OUTPUT" >&2
  exit 1
fi

STATUS=""
for _ in {1..20}; do
  STATUS_RAW=$(DEVIT_SECRET="$SECRET" DEVIT_CORE_CONFIG="$CONFIG" "$DEVIT_BIN" status --format json)
  STATUS_JSON=$(printf '%s\n' "$STATUS_RAW" | sanitize_json)
  STATUS=$(echo "$STATUS_JSON" | jq -r --arg id "$TASK_ID" '(.completed_tasks[]? | select(.id == $id) | .status) // (.active_tasks[]? | select(.id == $id) | .status) // empty')
  if [[ "$STATUS" == "completed" ]]; then
    SUMMARY=$(echo "$STATUS_JSON" | jq -r --arg id "$TASK_ID" '.completed_tasks[]? | select(.id == $id) | .notifications[]?.summary // empty')
    if [[ "$SUMMARY" != *"subprocess-ok"* ]]; then
      echo "Task completed but summary mismatch" >&2
      echo "$STATUS_JSON" | jq '.completed_tasks[]? | select(.id == $id)' >&2
      exit 1
    fi
    echo "Subprocess worker smoke test passed (task $TASK_ID)."
    exit 0
  fi
  sleep 0.5
done

echo "Task $TASK_ID did not reach completed state" >&2
DEVIT_SECRET="$SECRET" DEVIT_CORE_CONFIG="$CONFIG" "$DEVIT_BIN" status --format json >&2
exit 1
