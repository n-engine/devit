#!/usr/bin/env bash
set -euo pipefail

SOCKET_PATH="${DEVIT_SOCK:-/tmp/devitd.sock}"
SECRET="${DEVIT_SECRET:-change-me-in-production}"

if command -v pkill >/dev/null 2>&1; then
  pkill devitd >/dev/null 2>&1 || true
fi

if [ -e "$SOCKET_PATH" ]; then
  rm -f "$SOCKET_PATH"
fi

cargo build --bin devitd >/dev/null 2>&1

./target/debug/devitd --socket "$SOCKET_PATH" --secret "$SECRET" &
PID=$!

echo "devitd daemon started (PID: $PID, socket: $SOCKET_PATH)"
