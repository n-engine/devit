# DevIt Daemon Protocol

## Overview

The `devitd` daemon exposes a lightweight JSON protocol over Unix sockets. Clients authenticate using a shared secret and exchange request/response messages that resemble WebSocket semantics.

## Message Format

```json
{
  "msg_type": "request | response | notification",
  "msg_id": "uuid",
  "method": "delegate | notify | status | task",
  "params": {},
  "timestamp": "2024-01-01T00:00:00Z"
}
```

## Authentication

1. Client connects to the Unix socket.
2. Sends an authentication message containing the shared secret (`DEVIT_SECRET`).
3. Daemon validates and returns a session token.
4. All subsequent messages include the session token in the payload.

## Registration & Version Check

Avant d'échanger des tâches, chaque client envoie un message `REGISTER` signé (HMAC) contenant :

```json
{
  "caps": ["delegate", "notify", "screenshot"],
  "pid": 12345,
  "version": "mcp-server/0.1.0"
}
```

- `version` est optionnel, mais si `devitd` est configuré avec `expected_worker_version`, l'absence ou un mismatch entraîne une réponse `ERR` (`code = E_VERSION_MISMATCH`).
- Sur succès, le daemon renvoie `ACK` avec `daemon_version` (ex. `devitd/0.1.0`) et rappelle la version attendue.
- Les clients peuvent, côté MCP, forcer la version minimale du daemon avec `DEVIT_EXPECTED_DAEMON_VERSION`.

Les binaires officiels `mcp-server` et `devit-cli` annoncent automatiquement leur version lors du `REGISTER`.

## Methods

### `delegate`
Submit a new task to an AI assistant.

Parameters:
- `goal`: textual objective
- `delegated_to`: worker identifier (e.g., `claude_code`)
- `model`: optional model override (falls back to `context.model`, then worker default)
- `timeout_s`: optional timeout in seconds
- `context`: optional structured metadata
- `watch_patterns`: optional file patterns for monitoring
- `working_dir`: optional sandbox-relative path (e.g., `project-a/tests`)
- `format`: optional response format (`default` or `compact`). `compact` instructs the daemon to post-process long prose into structured summaries.

Returns: `{ "task_id": "task_xxxxx" }`

Policy & approval outcomes:

- Si la policy renvoie `Deny`, le daemon répond immédiatement avec `ERR` (`code = E_POLICY_DENIED`) et envoie un `NOTIFY` `status="failed"` au `return_to` (ou au client d'origine) contenant `metadata.failure.reason = policy_denied`.
- Si la policy exige une approbation (`NeedApproval`), la requête est envoyée à `[daemon.approvals].default_target`. Un refus (`status="denied"`) génère également un `NOTIFY` `failed` + `metadata.failure.reason = approval_denied` afin que le client MCP cesse d'attendre et journalise l'échec.
- Les notifications ainsi générées sont journalisées dans `Journal::append("POLICY", …)` et apparaissent dans `status`/`task` avec un historique normal.

### `notify`
Update the status of an existing task.

Parameters:
- `task_id`
- `status`: `pending | in_progress | completed | failed | cancelled | ack`
- `summary`: human-readable summary
- `evidence`: optional structured data (logs, metrics, etc.)

Returns: `{ "ok": true }`

Notification ACK semantics (V1):
- `ack` is a special status used to acknowledge reception/processing by a UI/agent.
- When `status = "ack"`, the daemon DOES NOT change the task state nor append a notification.
- Instead, it signals the daemon-side notification hook by creating a filesystem marker at the path provided via the hook env `DEVIT_ACK_MARKER`.
- This allows external scripts (e.g., xdotool-based notifiers) to block until the UI has acknowledged the message, then proceed or exit.
- Backward compatible: clients that don’t use `ack` maintain current behavior.

ACK (V2 – IPC sockets / pipes):
- On Unix, the daemon may also provide a per-notification Unix socket via `DEVIT_ACK_SOCKET` to the hook environment.
- On Windows, the daemon provides a per-notification Named Pipe via `DEVIT_ACK_PIPE` (e.g., `\\.\pipe\devit-ack-<task>-<pid>`), secured so only the current user can connect.
- Upon receiving `status="ack"`, the daemon writes a single byte to the socket/pipe, then cleans up the endpoint.
- Hooks should prefer waiting on the IPC endpoint when present (blocking read of 1 byte/EOF), and fall back to `DEVIT_ACK_MARKER` otherwise.

### `status`
Query global orchestration state.

Parameters:
- `filter`: `all | active | completed | failed`

Returns: `OrchestrationStatus` (active tasks, completed tasks, summary counts).

### `task`
Fetch detailed information about a specific task.

Parameters:
- `task_id`

Returns: `DelegatedTask` (goal, delegated_to, status, notifications, timestamps).

`DelegatedTask` now includes:

```json
{
  "working_dir": "project-a/tests",
  "response_format": "compact"
}
```

The value is always relative to the configured sandbox root. The daemon rejects attempts to escape the jail and logs any violation attempts.

When the worker output exceeds `max_response_chars`, the daemon adds `truncated=true` and `original_size=<bytes>` to the stored task artifacts so clients can display a warning.

Each notification `artifacts` payload now includes a structured `metadata` block capturing the scheduling timestamps (`time_queued`, `time_started`, `time_completed`), the total/execution durations, the worker type (`cli` or `mcp`) and the exit status (`exit_code`, `exit_reason`). When available, the daemon also exposes the worker version, the underlying LLM model plus token/cost counters.

```json
{
  "summary": "Analyse complète",
  "details": {"stdout": "..."},
  "metadata": {
    "time_queued": "2025-10-06T21:45:12.420Z",
    "time_started": "2025-10-06T21:45:12.612Z",
    "time_completed": "2025-10-06T21:45:15.001Z",
    "duration_total_ms": 2581,
    "duration_execution_ms": 2389,
    "worker_type": "cli",
    "model_requested": "llama3:8b",
    "model_used": "llama3:8b",
    "exit_code": 0,
    "exit_reason": "success"
  }
}
```

Legacy clients remain compatible: `metadata` is optional and safely ignored when not consumed.

### `screenshot`
Capture une capture d'écran contrôlée par le daemon (Linux uniquement).

Parameters: none (payload vide).

Returns (ACK):
```json
{
  "path": ".devit/screenshots/screenshot-20251007T205456.713Z.png",
  "format": "png",
  "size": {
    "bytes": 1180424,
    "human": "1.13 MB"
  }
}
```

Le daemon refuse la capture avec `ERR` si :
- la capacité `orchestration.capabilities.screenshot` est désactivée (`code = E_SCREENSHOT_DENIED`),
- le taux dépasse `rate_limit` (`code = E_SCREENSHOT_DENIED`),
- le backend (`scrot` ou `import`) n'est pas disponible (`code = E_SCREENSHOT_FAILED`).

Les fichiers générés sont forcés dans le sandbox (`<workspace>/.devit/screenshots`) ou `/tmp/devit-screenshots/`.

## Error Codes

| Code | Description |
|------|-------------|
| 1001 | Authentication failed |
| 1002 | Task not found |
| 1003 | Invalid request payload |
| 1004 | Timeout waiting for worker |
| 1005 | Internal daemon error |

Clients receive errors as JSON-RPC error objects with these codes in the `data.code` field.

## References

- [docs/ORCHESTRATION.md](ORCHESTRATION.md) – high-level architecture
- [docs/CONFIGURATION.md](CONFIGURATION.md) – configuration and environment variables
