#!/usr/bin/env bash
# Example notification hook for devitd.
# Requires devitd started with DEVIT_NOTIFY_HOOK="/path/to/devit_notify_example.sh".

set -euo pipefail

LOG_DIR="/tmp/devit-notify"
mkdir -p "$LOG_DIR"
LOG_FILE="$LOG_DIR/notify.log"

{
  echo "[$(date --iso-8601=seconds)] Notification received"
  echo "  Task id     : ${DEVIT_NOTIFY_TASK_ID:-<unset>}"
  echo "  Status      : ${DEVIT_NOTIFY_STATUS:-<unset>}"
  echo "  Worker      : ${DEVIT_NOTIFY_WORKER:-<unset>}"
  echo "  Summary     : ${DEVIT_NOTIFY_SUMMARY:-<unset>}"
  echo "  Timestamp   : ${DEVIT_NOTIFY_TIMESTAMP:-<unset>}"
  if [[ -n "${DEVIT_NOTIFY_WORKDIR:-}" ]]; then
    echo "  Working dir : ${DEVIT_NOTIFY_WORKDIR}"
  fi
  if [[ -n "${DEVIT_NOTIFY_DETAILS:-}" ]]; then
    echo "  Details     : ${DEVIT_NOTIFY_DETAILS}"
  fi
  if [[ -n "${DEVIT_NOTIFY_EVIDENCE:-}" ]]; then
    echo "  Evidence    : ${DEVIT_NOTIFY_EVIDENCE}"
  fi
  echo "  Payload JSON: ${DEVIT_NOTIFY_PAYLOAD:-<unset>}"
  echo "---"
} >>"$LOG_FILE"

# Optional desktop notification integration
if command -v notify-send >/dev/null 2>&1; then
  notify-send "DevIt Task ${DEVIT_NOTIFY_STATUS:-?}" "${DEVIT_NOTIFY_SUMMARY:-Notification received}" >/dev/null 2>&1 || true
fi

# Wait for ACK (V2: socket) then fallback (V1: marker)
ACK_TIMEOUT="${ACK_TIMEOUT:-10}"
if [[ -n "${DEVIT_ACK_SOCKET:-}" ]]; then
  if command -v socat >/dev/null 2>&1; then
    echo "  Waiting for ACK via socket: ${DEVIT_ACK_SOCKET} (socat, ${ACK_TIMEOUT}s)" >>"$LOG_FILE"
    timeout "${ACK_TIMEOUT}"s socat - UNIX-CONNECT:"${DEVIT_ACK_SOCKET}" -u >/dev/null 2>&1 && {
      echo "  ✅ ACK reçu via socket" >>"$LOG_FILE"; exit 0; }
  elif command -v nc >/dev/null 2>&1; then
    echo "  Waiting for ACK via socket: ${DEVIT_ACK_SOCKET} (nc, ${ACK_TIMEOUT}s)" >>"$LOG_FILE"
    # Some nc support -U (Unix); redirect /dev/null for single read
    timeout "${ACK_TIMEOUT}"s nc -U "${DEVIT_ACK_SOCKET}" >/dev/null 2>&1 && {
      echo "  ✅ ACK reçu via socket" >>"$LOG_FILE"; exit 0; }
  else
    echo "  ⚠️  No socket client (socat/nc) available; falling back to marker" >>"$LOG_FILE"
  fi
fi

if [[ -n "${DEVIT_ACK_MARKER:-}" ]]; then
  {
    echo "  Waiting for ACK marker: ${DEVIT_ACK_MARKER} (timeout=${ACK_TIMEOUT}s)"
  } >>"$LOG_FILE"
  for _i in $(seq 1 "$ACK_TIMEOUT"); do
    if [[ -f "${DEVIT_ACK_MARKER}" ]]; then
      echo "  ✅ ACK received (marker present)" >>"$LOG_FILE"
      rm -f "${DEVIT_ACK_MARKER}" || true
      exit 0
    fi
    sleep 1
  done
  echo "  ⚠️  ACK timeout (no marker within ${ACK_TIMEOUT}s)" >>"$LOG_FILE"
fi
