#!/usr/bin/env bash
# Exemple de hook de notification pour devitd.
# Nécessite que devitd soit démarré avec DEVIT_NOTIFY_HOOK="/chemin/vers/devit_notify_example.sh".

set -euo pipefail

LOG_DIR="/tmp/devit-notify"
mkdir -p "$LOG_DIR"
LOG_FILE="$LOG_DIR/notify.log"

{
  echo "[$(date --iso-8601=seconds)] Notification reçue"
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

# Exemple d’intégration IHM : envoyer une notif desktop (optionnel)
if command -v notify-send >/dev/null 2>&1; then
  notify-send "DevIt Task ${DEVIT_NOTIFY_STATUS:-?}" "${DEVIT_NOTIFY_SUMMARY:-Notification reçue}" >/dev/null 2>&1 || true
fi

# Attente ACK (V2: socket) puis fallback (V1: marker)
ACK_TIMEOUT="${ACK_TIMEOUT:-10}"
if [[ -n "${DEVIT_ACK_SOCKET:-}" ]]; then
  if command -v socat >/dev/null 2>&1; then
    echo "  Attente ACK via socket: ${DEVIT_ACK_SOCKET} (socat, ${ACK_TIMEOUT}s)" >>"$LOG_FILE"
    timeout "${ACK_TIMEOUT}"s socat - UNIX-CONNECT:"${DEVIT_ACK_SOCKET}" -u >/dev/null 2>&1 && {
      echo "  ✅ ACK reçu via socket" >>"$LOG_FILE"; exit 0; }
  elif command -v nc >/dev/null 2>&1; then
    echo "  Attente ACK via socket: ${DEVIT_ACK_SOCKET} (nc, ${ACK_TIMEOUT}s)" >>"$LOG_FILE"
    # Certains nc supportent -U (Unix); redirection /dev/null pour lecture unique
    timeout "${ACK_TIMEOUT}"s nc -U "${DEVIT_ACK_SOCKET}" >/dev/null 2>&1 && {
      echo "  ✅ ACK reçu via socket" >>"$LOG_FILE"; exit 0; }
  else
    echo "  ⚠️  Aucun client socket (socat/nc) disponible; fallback marker" >>"$LOG_FILE"
  fi
fi

if [[ -n "${DEVIT_ACK_MARKER:-}" ]]; then
  {
    echo "  ACK marker attendu : ${DEVIT_ACK_MARKER} (timeout=${ACK_TIMEOUT}s)"
  } >>"$LOG_FILE"
  for _i in $(seq 1 "$ACK_TIMEOUT"); do
    if [[ -f "${DEVIT_ACK_MARKER}" ]]; then
      echo "  ✅ ACK reçu (marker présent)" >>"$LOG_FILE"
      rm -f "${DEVIT_ACK_MARKER}" || true
      exit 0
    fi
    sleep 1
  done
  echo "  ⚠️  ACK timeout (no marker within ${ACK_TIMEOUT}s)" >>"$LOG_FILE"
fi
