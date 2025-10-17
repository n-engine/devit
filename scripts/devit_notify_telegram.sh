#!/usr/bin/env bash
set -euo pipefail

# Telegram notification hook for devitd notifications.
# Requires:
#   - DEVIT_TELEGRAM_BOT_TOKEN
#   - DEVIT_TELEGRAM_CHAT_ID
#
# This script is intended to be used via DEVIT_NOTIFY_HOOK.
# It sends a concise message for status in {completed, failed, cancelled, progress}.

BOT_TOKEN="${DEVIT_TELEGRAM_BOT_TOKEN:-}"
CHAT_ID="${DEVIT_TELEGRAM_CHAT_ID:-}"
API_URL="https://api.telegram.org/bot${BOT_TOKEN}/sendMessage"

STATUS="${DEVIT_NOTIFY_STATUS:-}"
TASK_ID="${DEVIT_NOTIFY_TASK_ID:-}"
WORKER="${DEVIT_NOTIFY_WORKER:-}"
RETURN_TO="${DEVIT_NOTIFY_RETURN_TO:-}"
SUMMARY="${DEVIT_NOTIFY_SUMMARY:-}"
TIMESTAMP="${DEVIT_NOTIFY_TIMESTAMP:-}"
WORKDIR="${DEVIT_NOTIFY_WORKDIR:-}"

LOG_DIR="/tmp/devit-notify"
mkdir -p "$LOG_DIR"
LOG_FILE="$LOG_DIR/telegram.log"

if [[ -z "$BOT_TOKEN" || -z "$CHAT_ID" ]]; then
  echo "[$(date --iso-8601=seconds)] Telegram: missing DEVIT_TELEGRAM_BOT_TOKEN or DEVIT_TELEGRAM_CHAT_ID" >>"$LOG_FILE"
  exit 0
fi

if ! command -v curl >/dev/null 2>&1; then
  echo "[$(date --iso-8601=seconds)] Telegram: curl not found" >>"$LOG_FILE"
  exit 0
fi

# Build message text (URL-encoded by curl)
TITLE="DevIt Notification"
if [[ "$STATUS" == "completed" ]]; then TITLE="✅ DevIt Completed"; fi
if [[ "$STATUS" == "failed" ]]; then TITLE="❌ DevIt Failed"; fi
if [[ "$STATUS" == "cancelled" ]]; then TITLE="🚫 DevIt Cancelled"; fi
if [[ "$STATUS" == "progress" ]]; then TITLE="📣 DevIt Update"; fi

TEXT="${TITLE}
Task: ${TASK_ID}
Status: ${STATUS}
Worker: ${WORKER}
ReturnTo: ${RETURN_TO}
When: ${TIMESTAMP}
Workdir: ${WORKDIR}
Summary: ${SUMMARY}"

# Optionally, include compact details/evidence sizes if present
DETAILS="${DEVIT_NOTIFY_DETAILS:-}"
EVIDENCE="${DEVIT_NOTIFY_EVIDENCE:-}"
if [[ -n "$DETAILS" ]]; then
  # Try to shorten to avoid huge payloads
  LEN=${#DETAILS}
  if (( LEN > 200 )); then
    TEXT+=$'\nDetails: (truncated)'
  else
    TEXT+=$'\nDetails: present'
  fi
fi
if [[ -n "$EVIDENCE" ]]; then
  TEXT+=$'\nEvidence: present'
fi

# Send message
if ! curl -sS \
  --data-urlencode "chat_id=${CHAT_ID}" \
  --data-urlencode "text=${TEXT}" \
  --data-urlencode "disable_web_page_preview=true" \
  --data-urlencode "parse_mode=HTML" \
  "$API_URL" >>"$LOG_FILE" 2>&1; then
  echo "[$(date --iso-8601=seconds)] Telegram: send failed" >>"$LOG_FILE"
  exit 0
fi

echo "[$(date --iso-8601=seconds)] Telegram: sent status=${STATUS} task=${TASK_ID}" >>"$LOG_FILE"
exit 0

