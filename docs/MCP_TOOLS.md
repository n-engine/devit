# MCP Tools Reference

## devit_patch_apply

Apply unified diff patches with sandbox validation, dry-run preview, and detailed reporting.

### Parameters
- `diff` *(string, required)* — unified diff payload (for example output of `git diff`)
- `dry_run` *(boolean, optional, default=false)* — if true, validates and previews without modifying files

### Format Requirements
- Git-style diffs (`diff --git a/path b/path`) are fully supported.
- Simple unified diffs (without `diff --git`) are also accepted as long as they contain `--- path`, `+++ path`, and `@@` hunk headers.
- Paths must stay within the workspace sandbox (no absolute paths or `..` traversal).
- Maximum diff size: **1 MB**.
- Context/hunk headers must follow the standard unified diff format (`@@ -X,Y +A,B @@`).

### Successful Response
```json
{
  "content": [
    {
      "type": "text",
      "text": "✅ Patch applied successfully — 1 file(s), 1 hunks, +1 / -0 lines\n\n- modified hello.txt (hunks: 1, +1 / -0)"
    }
  ],
  "structuredContent": {
    "patch": {
      "success": true,
      "dryRun": false,
      "timestamp": "2025-10-11T13:45:00.123Z",
      "summary": {
        "files": 1,
        "files_modified": 1,
        "files_created": 0,
        "files_deleted": 0,
        "hunks": 1,
        "lines_added": 1,
        "lines_removed": 0
      },
      "files": [
        {
          "path": "hello.txt",
          "action": "modified",
          "hunks": 1,
          "lines_added": 1,
          "lines_removed": 0
        }
      ]
    }
  }
}
```

### Dry-Run Preview
```
content[0].text:
🔍 Patch preview successfully — 2 file(s), 3 hunks, +5 / -3 lines

- modified src/lib.rs (hunks: 2, +4 / -3)
- created README.md (hunks: 1, +1 / -0)
```

Structured content mirrors the fields above with `"dryRun": true`.

### Example Input
```diff
diff --git a/src/main.rs b/src/main.rs
--- a/src/main.rs
+++ b/src/main.rs
@@ -10,6 +10,7 @@
 fn main() {
     println!("Hello");
+    println!("World");
 }
```

Generate this structure with:
```bash
git diff HEAD~1 src/main.rs > patch.diff
```
Then provide the contents of `patch.diff` as the `diff` argument.

### Common Errors & Fixes
| Error message | Cause | Resolution |
| ------------- | ----- | ---------- |
| `❌ Patch failed: empty diff provided` | Empty `diff` parameter | Ensure you pass the complete unified diff |
| `❌ Patch failed: unsupported diff format detected (...)` | Context diff or another unsupported format | Regenerate the diff with `git diff` or ensure unified diff headers are present |
| `❌ Patch failed: file 'foo.txt' not found in workspace` | Attempting to modify/delete a file that does not exist | Verify the file path and regenerate the patch |
| `❌ Patch failed: context mismatch in foo.txt line 12 (expected '...', found '...')` | The patch no longer matches workspace contents | Rebase/regenerate the patch against the current workspace |
| `❌ Patch failed: security violation (Path traversal attempt detected)` | Path escapes sandbox (`../`, absolute path, symlink) | Only patch files inside the workspace root |
| `❌ Patch failed: invalid unified diff at line ...` | Malformed hunk header or missing `@@` block | Check the diff syntax – the unified diff header must be intact |

### Tips
- Use `git diff` or `git format-patch` to produce valid unified diffs (do **not** hand-edit away the `diff --git` lines).
- Run with `dry_run=true` first to inspect the impact without touching disk.
- Combine with `devit_snapshot` before applying large patches to keep rollback points.

## devit_delegate

Delegate work to a registered worker (CLI, MCP bridge, or subprocess) and track the orchestration metadata.

### Parameters
- `goal` *(string, required)* — description of the objective to accomplish.
- `delegated_to` *(string, optional, default=`claude_code`)* — worker identifier (matches `[workers.<id>]` in `devit.core.toml`).
- `timeout` *(integer, optional)* — lease duration in seconds; the daemon enforces the lower of worker and task timeouts.
- `watch_patterns` *(array[string], optional)* — file globs to monitor for changes while the task is active.
- `model` *(string, optional)* — explicit model override. Falls back to `context.model`, then the worker’s `default_model`. Rejected if not in `allowed_models`.
- `context` *(object, optional)* — arbitrary JSON context forwarded to the worker.
- `working_dir` *(string, optional)* — sandbox-relative path (e.g., `project-a/tests`).
- `format` *(string, optional, default=`default`)* — `default` keeps the worker output unchanged, `compact` triggers daemon-side post-processing that emits structured summaries (`structured_data`) instead of 15 KB prose.

### Response
- Chat text summarising the delegation (task id, worker, timeout, working dir, format).
- JSON payload containing the same fields plus the orchestration `mode` and the resolved `metadata` (`time_*`, `model_requested`, `model_used`, etc.).

### Example
```json
{
  "name": "devit_delegate",
  "arguments": {
    "goal": "Refactor renderer.cxx",
    "delegated_to": "claude_code",
    "model": "claude-opus-4",
    "timeout": 1200,
    "format": "compact"
  }
}
```

The daemon stores the chosen format with the task metadata so that `devit_task_result` can return the compact payload automatically.

## Git Investigation Tools

### devit_git_log
- **Purpose:** quick history view (equivalent to `git log --oneline`)
- **Arguments:** `max_count` *(int, optional)*, `path` *(string, optional)*
- **Example:**
  ```json
  {
    "name": "devit_git_log",
    "arguments": {
      "max_count": 5,
      "path": "crates/mcp-tools/src"
    }
  }
  ```

### devit_git_blame
- **Purpose:** inspect authorship for a file or line range (`git blame`)
- **Arguments:** `path` *(string, required)*, `line_start` *(int, optional)*, `line_end` *(int, optional)*
- **Example:**
  ```json
  {
    "name": "devit_git_blame",
    "arguments": {
      "path": "src/lib.rs",
      "line_start": 40,
      "line_end": 60
    }
  }
  ```

### devit_git_show
- **Purpose:** display commit contents or a specific file snapshot (`git show`)
- **Arguments:** `commit` *(string, required)*, `path` *(string, optional)*
- **Example:**
  ```json
  {
    "name": "devit_git_show",
    "arguments": {
      "commit": "HEAD~1",
      "path": "Cargo.toml"
    }
  }
  ```

### devit_git_diff
- **Purpose:** generate a unified diff (`git diff`) for a range or working tree
- **Arguments:** `range` *(string, optional)*, `path` *(string, optional)*
- **Example:**
  ```json
  {
    "name": "devit_git_diff",
    "arguments": {
      "range": "HEAD~3..HEAD"
    }
  }
  ```

### devit_git_search
- **Purpose:** search through history or the tree (`git grep` / `git log -S`)
- **Arguments:**
  - `pattern` *(string, required)*
  - `type` *(string, optional; "grep" or "log", default "grep")*
  - `path` *(string, optional)*
  - `max_count` *(int, optional for log mode)*
- **Example:**
  ```json
  {
    "name": "devit_git_search",
    "arguments": {
      "pattern": "TODO",
      "type": "grep",
      "path": "src"
    }
  }
  ```

**Common error hints:**
- `git log failed …` → ensure the repository has commits in range or the supplied commit exists.
- `git blame failed …` → verify the path is tracked and within the sandbox.
- `(no matches)` → command executed correctly but returned no results (e.g., `git grep`).

## Worker-Mode Tools

### devit_poll_tasks

- **Purpose:** Retrieve the next task assigned by `devitd` when the MCP server runs in `--worker-mode`.
- **Availability:** Only registered when `mcp-server` is launched with `--worker-mode` (not exposed in regular CLI sessions).

#### Parameters
- `wait` *(boolean, optional, default=false)* — block for a few seconds waiting for a task before returning `"status": "idle"`.

#### Response
Returns a combined message with a human summary and a JSON payload. The JSON always includes a `status` field:

| Status | Meaning |
|--------|---------|
| `assigned` | New task assigned to this worker. Payload includes goal, working directories (relative + absolute), watch patterns, context, and the raw task map. |
| `cancelled` | Task was revoked by the orchestrator. `reason` contains the summary if provided. |
| `timeout` | Lease expired before completion; stop work and await a new task. |
| `idle` | No task currently available (only returned when `wait=false` or timeout reached). |

#### Examples

**Assignment**
```json
{
  "name": "devit_poll_tasks",
  "arguments": {}
}
```
Response snippet:
```json
{
  "content": [
    {"type": "text", "text": "✅ Nouvelle tâche assignée par le daemon"},
    {
      "type": "json",
      "json": {
        "status": "assigned",
        "task_id": "a77df92b-a34b-4199-8ec9-a6f98a9503c2",
        "goal": "E2E test task",
        "working_dir": "project-a",
        "absolute_working_dir": "/home/user/workspace/devIt/project-a",
        "timeout_secs": 1800,
        "watch_patterns": [],
        "context": null
      }
    }
  ]
}
```

**No task (idle)**
```json
{
  "name": "devit_poll_tasks",
  "arguments": {"wait": false}
}
```
Response contains `"metadata": {"status": "idle"}` and a text message `📭 Aucune tâche disponible`.

#### Tips
- Call `devit_poll_tasks` before doing any work to ensure a task is actively leased to this worker.
- When you receive `cancelled` or `timeout`, stop immediately and poll again; the lease has been released by `devitd`.
- Combine with `devit_notify` to report `completed`, `failed`, or `progress` updates back to the daemon.

#### Version handshake

Lorsque `devitd` est configuré avec `expected_worker_version`, un REGISTER sans champ `version` (ou avec une valeur différente) est refusé (`ERR` + `E_VERSION_MISMATCH`). Le binaire `mcp-server` annonce automatiquement `mcp-server/<version>` et peut, si besoin, forcer la version du daemon via `DEVIT_EXPECTED_DAEMON_VERSION`. Consultez `docs/CONFIGURATION.md` pour la syntaxe détaillée.

### Notification hook côté daemon

Si `devitd` est lancé avec `DEVIT_NOTIFY_HOOK`, toute notification enregistrée via `devit_notify` déclenche la commande indiquée. Le hook est exécuté de façon asynchrone (via `bash -lc`) avec les variables suivantes :

| Env var | Description |
|---------|-------------|
| `DEVIT_NOTIFY_TASK_ID` | UUID de la tâche |
| `DEVIT_NOTIFY_STATUS` | Statut (`completed`, `failed`, `cancelled`, …) |
| `DEVIT_NOTIFY_WORKER` | Worker ayant rendu le résultat |
| `DEVIT_NOTIFY_RETURN_TO` | Destinataire prévu pour la notification |
| `DEVIT_NOTIFY_SUMMARY` | Résumé textuel |
| `DEVIT_NOTIFY_TIMESTAMP` | Horodatage RFC3339 |
| `DEVIT_NOTIFY_WORKDIR` | (optionnel) Répertoire de travail |
| `DEVIT_NOTIFY_DETAILS` | (optionnel) JSON sérialisé du champ `details` |
| `DEVIT_NOTIFY_EVIDENCE` | (optionnel) JSON sérialisé du champ `evidence` |
| `DEVIT_NOTIFY_PAYLOAD` | JSON complet regroupant tout le contexte |
| `DEVIT_ACK_MARKER` | (optionnel) Chemin de fichier marker que le daemon créera lors d'un ACK |
| `DEVIT_ACK_SOCKET` | (optionnel, Unix) Chemin de socket Unix pour attendre un signal ACK |
| `DEVIT_ACK_PIPE` | (optionnel, Windows) Nom de Named Pipe `\\.\pipe\devit-ack-<task>-<pid>` |

Des exemples de hooks sont fournis dans :
- `scripts/devit_notify_example.sh` (bash, écrit dans `/tmp/devit-notify/notify.log`, support `notify-send`)
- `scripts/devit_notify_example.ps1` (PowerShell ≥5.1, log dans `%TEMP%\devit-notify`, attente via `NamedPipeClientStream`)

#### ACK (sémantique & usage)

- Pour accuser réception côté MCP, appelez `devit_notify` avec `status="ack"` et le `task_id` concerné:
  ```json
  {
    "name": "devit_notify",
    "arguments": { "task_id": "<id>", "status": "ack", "summary": "ack" }
  }
  ```
- Le daemon ne modifie pas l’état de la tâche pour `ack`. Il se contente de créer le fichier `DEVIT_ACK_MARKER` préalablement communiqué au hook, ce qui débloque son attente.
- Le routage `ack` passe toujours via l’`OrchestrationContext` (jamais via `WorkerBridge`).

##### V2 (IPC socket / pipe)
- Sur Unix, le daemon peut fournir `DEVIT_ACK_SOCKET` (socket éphémère par notification). Lors d’un `ack`, il écrit 1 octet, puis supprime la socket.
- Sur Windows, le daemon expose `DEVIT_ACK_PIPE` (Named Pipe restreinte au compte courant). Lors d’un `ack`, il écrit 1 octet sur la connexion active.
- Les hooks doivent tenter d’abord l’attente sur le canal IPC (`socat`/`nc -U` sous Unix, `NamedPipeClientStream` côté Windows), puis retomber sur le marker si indisponible.

## devit_screenshot

Prend une capture d'écran complète via le daemon `devitd`. Selon la plateforme, le backend peut être `native` (capture en mémoire, défaut sur Windows), `scrot` ou `imagemagick` (Linux). Toutes les captures respectent le quota défini dans `orchestration.capabilities.screenshot`.

### Parameters
- *(object vide)* — aucun paramètre requis. Envoyez `{}` ou omettez complètement `arguments`.

### Response
- **Texte** — résumé humain, par exemple `📸 Capture enregistrée (1.12 MB) — .devit/screenshots/screenshot-....png`.
- **Image** — bloc image MCP avec base64 + `mimeType` (pas de Data URI) :

  ```json
  {
    "type": "image",
    "data": "iVBORw0KGgoAAA...",
    "mimeType": "image/png"
  }
  ```
- **structuredContent** — métadonnées sous `structuredContent.image` (chemin disque, tailles, thumbnail) :

  ```json
  {
    "content": [
      {"type":"text","text":"📸 Capture enregistrée (1.12 MB) — .devit/screenshots/screenshot-....png"},
      {"type":"image","data":"iVBORw0KGgoAAA...","mimeType":"image/png"}
    ],
    "structuredContent": {
      "image": {
        "embedded": true,
        "format": "png",
        "inline": true,
        "path": ".devit/screenshots/screenshot-20251007T205456.713Z.png",
        "size": {"bytes": 1180424, "human": "1.13 MB"},
        "thumbnail": {"bytes": 31738, "format": "png", "width": 480, "height": 135}
      }
    }
  }
  ```

### Notes
- Ne fonctionne que si `tools.screenshot.enabled = true` **et** `orchestration.capabilities.screenshot.enabled = true`.
- Le daemon applique le `rate_limit` (10/minute par défaut) ; au-delà, le tool renvoie l'erreur du daemon.
- Linux : `scrot` (par défaut) ou `imagemagick` (`tools.screenshot.backend = "imagemagick"`).
- Windows : backend `native` (`tools.screenshot.backend = "native"`, activé par défaut) stockant les captures sous `%LOCALAPPDATA%\DevIt\screenshots`.
- Le tool peut embarquer un thumbnail PNG et laisse l'original sur disque. L’inline est contrôlé par les paramètres ci‑dessous.

### Parameters (optionnels)
- `inline` *(bool, défaut=true)* — embarquer un thumbnail en base64 si la taille le permet.
- `max_inline_kb` *(int, défaut=512)* — taille max du thumbnail encodé (KB) pour l’inline.
- `thumb_width` *(int, défaut=480)* — largeur cible du thumbnail (pas d’upscale).
- `thumb_quality` *(int, défaut=80)* — réservé (non utilisé pour PNG actuellement).

### Comportement d’embed
- Si `inline=true` et le thumbnail ≤ `max_inline_kb`, un bloc `{type:"image", data:"<base64>", mimeType:"image/png"}` est ajouté.
- Sinon, aucun bloc image n’est inclus; utilisez `structuredContent.image.path` pour le fichier.

## devit_mouse

Pilote la souris du poste (Linux/X11) via `xdotool`. Permet d’enchaîner des mouvements, clics et scrolls pour automatiser l’UI.

### Parameters
- `actions` *(array, requis)* — séquence d’actions. Chaque élément possède `type` et les champs associés :
  - `{"type":"move","x":1280,"y":720,"sync":true}` — déplacement absolu.
  - `{"type":"move_relative","dx":-200,"dy":50}` — déplacement relatif.
  - `{"type":"click","button":1,"count":2}` — clic (1=Gauche, 2=Milieu, 3=Droite).
  - `{"type":"scroll","vertical":-3}` — scroll (valeur >0 = bas, <0 = haut ; utiliser `horizontal` pour gauche/droite).
  - `{"type":"sleep","millis":250}` — pause explicite entre actions.
- `delay_ms` *(integer, défaut=40)* — délai (ms) injecté entre deux actions si non spécifié via `sleep`.

### Response
- **Texte** — résumé `🖱️ Mouse actions executed (N)`.
- **structuredContent.desktop** — payload JSON listant les actions exécutées.

### Prérequis
- Linux + session X11 avec `xdotool` installé (`sudo apt install xdotool`).
- Le focus de fenêtre doit correspondre à la cible (pas de sélection automatique).
- Variables d’environnement (optionnelles) :
  - `DEVIT_XDOTOOL_PATH` — chemin alternatif vers `xdotool`.
  - `DEVIT_MOUSE_DEFAULT_DELAY_MS` — délai par défaut entre actions.

## devit_keyboard

Envoie des frappes clavier via `xdotool`. Supporte la saisie de texte et les combinaisons de touches.

### Parameters
- `actions` *(array, requis)* — séquence d’actions :
  - `{"type":"text","text":"cargo test","delay_ms":20}` — saisie de texte (delay appliqué entre caractères).
  - `{"type":"key","keys":["ctrl","shift","t"]}` — combinaison `Ctrl+Shift+T`.
  - `{"type":"sleep","millis":500}` — pause explicite.
  - Champs optionnels : `repeat` (nombre de répétitions), `clear_modifiers` (défaut=true), `delay_ms` (précise un délai par action).
- `delay_ms` *(integer, défaut=35)* — délai par défaut entre actions et dans les répétitions si non précisé.

### Response
- **Texte** — résumé `⌨️ Keyboard actions executed (N)`.
- **structuredContent.desktop** — historique JSON des actions exécutées.

### Prérequis
- Linux + `xdotool` dans le `PATH`.
- `DEVIT_XDOTOOL_PATH` — permet de pointer vers un binaire custom.
- `DEVIT_KEYBOARD_DEFAULT_DELAY_MS` — override global du délai par défaut.

### Validation rapide
- Linux/macOS: `scripts/desktop_input_demo.sh --url http://127.0.0.1:3001/message`
  - Vérifie la présence de `devit_mouse`/`devit_keyboard`, enchaîne clic + saisie, puis `devit_screenshot`.
  - Supporte `--bearer <token>` si l’endpoint requiert un header `Authorization`.
  - Dépendances: `curl`, `jq`, `python3` côté client.
- Windows PowerShell: `scripts/desktop_input_demo.ps1 -Url http://127.0.0.1:3001/message`
  - Nécessite PowerShell 5.1+ (ou PS7), `Invoke-RestMethod` intégré; mêmes options (`-Bearer`, `-MoveX`, `-MoveY`, `-Text`, `-SkipScreenshot`).

## devit_ocr

Extrait le texte d’une image à l’aide de Tesseract. Par défaut, utilise la capture la plus récente dans `.devit/screenshots/`.

### Parameters
- `path` *(string, optionnel)* — chemin de l’image. Par défaut: dernier screenshot.
- `lang` *(string, défaut="eng")* — langue Tesseract (ex. `eng`, `fra`).
- `psm` *(integer, optionnel)* — page segmentation mode (Tesseract `--psm`).
- `oem` *(integer, optionnel)* — OCR engine mode (Tesseract `--oem`).
- `format` *(string, défaut="text")* — `text` | `tsv` | `hocr`.
- `inline` *(boolean, défaut=true)* — inclure un extrait texte dans la réponse; mettre à `false` pour le mode silencieux.
- `silent` *(boolean, défaut=false)* — alias de `inline=false`.
- `output_path` *(string, optionnel)* — écrit la sortie complète (txt/tsv/html) dans un fichier du workspace. Si `inline=false` et non fourni, un chemin par défaut `.devit/ocr/<basename>-<timestamp>.<ext>` est utilisé.
- `max_chars` *(integer, défaut=2000)* — limite de caractères renvoyés en texte (si `inline=true`).
- `preprocess` *(bool|object, optionnel)* — active un prétraitement simple de l’image avant OCR. Lorsque c’est un objet, les clés suivantes sont supportées:
  - `grayscale` *(bool, défaut=true)* — conversion en niveaux de gris.
  - `threshold` *(int 0..255)* — seuillage binaire (>= seuil = blanc, sinon noir).
  - `resize_width` *(int)* — redimensionnement en largeur (hauteur proportionnelle).
  - `crop` *(object)* — découpe une zone `{x,y,width,height}` (px).
 - `zone` *(string, optionnel)* — modèle de zone prédéfini (si `crop` absent) :
   - `terminal_bottom` — 35% bas de l’écran (pleine largeur)
   - `error_zone` — bande centrale (50% largeur, 40% hauteur)

### Response
- **Texte** — résumé humain + texte OCR (tronqué selon `max_chars`).
- **structuredContent.ocr** — métadonnées sur l’extraction :

  ```json
  {
    "content": [
      {"type": "text", "text": "📝 OCR extrait 5234 caractères — .devit/screenshots/... (lang: eng) (tronqué)"},
      {"type": "text", "text": "<texte OCR tronqué>"}
    ],
    "structuredContent": {
      "ocr": {
        "path": ".devit/screenshots/screenshot-2025....png",
        "engine": "tesseract",
        "lang": "eng",
        "psm": 6,
        "oem": 1,
        "format": "text",
        "chars": 5234,
        "truncated": true,
        "saved_to": ".devit/ocr/last.txt"
      }
    }
  }
  ```

### Prérequis
- Binaire `tesseract` installé et accessible dans le `PATH` (`sudo apt install tesseract-ocr` sur Debian/Ubuntu).
- Jeux de langues optionnels (`tesseract-ocr-fra`, etc.) selon vos besoins.

### Notes
- Pour OCR d’un screenshot récent: appelez `devit_screenshot` puis `devit_ocr` sans `path`.
- Pour un retour “silencieux” (pas d’énorme bloc texte dans la réponse), utilisez `inline=false` (ou `silent=true`): la sortie complète sera écrite sur disque et `saved_to` la référencera.
- Réglez `max_chars` si vous souhaitez un aperçu plus long en mode `inline=true`.
- Le prétraitement écrit un fichier temporaire `.devit/ocr/preproc-<timestamp>.png` utilisé par Tesseract puis supprimé automatiquement.

## devit_ocr_alerts

Déclenche des alertes OCR (regex) sur une image (dernier screenshot par défaut), avec action optionnelle de notification dans l’orchestration.

### Parameters
- `rules` (array[object], requis):
  - `name` (string)
  - `pattern` (string, regex, insensible à la casse)
  - `zone` (string, optionnel) — `terminal_bottom` | `error_zone`
  - `severity` (string, défaut="info") — libre (ex: critical, warning)
  - `action` (string, défaut="none") — `notify` | `none`
- `path` (string, optionnel) — image; sinon dernier screenshot
- `lang` (string, défaut="eng") — langue tesseract
- `psm` (integer, optionnel) — page segmentation mode
- `inline` (boolean, défaut=true) — inclure les détails en JSON (alerts) dans la réponse
- `task_id` (string, optionnel) — si présent et `action=notify`, envoie `devit_notify(status="progress")` avec un résumé et des détails

### Response
- **Texte** — résumé: nombre de règles matchées et chemin de l’image
- **structuredContent.ocrAlerts** — détails:
  - `path`, `lang`, `psm`, `alert_count`, `notified`, `alerts[]` (name, pattern, zone, severity, hits, samples)

### Exemple
```json
{
  "name": "devit_ocr_alerts",
  "arguments": {
    "rules": [
      {"name": "detect_errors", "zone": "terminal_bottom", "pattern": "(ERROR|FATAL|CRASH)", "action": "notify", "severity": "critical"}
    ],
    "task_id": "<optional-task-id>",
    "inline": true
  }
}
```

### Preset prêt à l'emploi
- Fichier: `docs/examples/ocr_alerts_rules_default.json`
- Contenu: règles générales élargies pour erreurs build, conflits de port, succès, panic/crash.
- Exemple d’appel:

```json
{
  "name": "devit_ocr_alerts",
  "arguments": {
    "rules": (CONTENU DU FICHIER ocr_alerts_rules_default.json),
    "inline": true
  }
}
```

### Notes
- Sans `task_id`, aucune notification n’est envoyée au daemon; les alertes sont uniquement retournées au client MCP.
- `action=notify` utilise `status="progress"` pour ne pas altérer l’état des tâches.

## devit_task_result

Retrieve the latest outcome recorded for a delegated task.

### Parameters
- `task_id` *(string, required)* — identifier returned by `devit_delegate` or the orchestration status tool.

### Response
Returns a chat-style message containing:

- Human summary highlighting the task identifier, current status, and latest summary text.
- JSON payload with:
  - `status` *(string)* — normalized task status (`pending`, `in_progress`, `completed`, `failed`, `cancelled`).
  - `goal` *(string)* — task goal supplied during delegation.
  - `delegated_to` *(string)* — worker identifier.
  - `timeout_secs` *(integer)* — timeout configured for the task.
  - `working_dir` *(string|null)* — relative working directory inside the sandbox, when available.
  - `format` *(string)* — `default` or `compact` depending on the delegation request.
  - `result.summary` *(string|null)* — latest summary recorded via `devit_notify` (typically the worker response).
  - `result.details` *(object|null)* — raw JSON details captured from the worker (stdout/stderr payloads, etc.).
  - `result.evidence` *(object|null)* — optional evidence block if the worker supplied one.
  - `result.metadata` *(object|null)* — timings (`time_queued`, `time_started`, `time_completed`), durations, worker type, `exit_code` and (when available) LLM usage metrics.
  - When the daemon truncated an oversized output, `result.details` also contains `truncated: true` and `original_size`.

### Example
```json
{
  "name": "devit_task_result",
  "arguments": {
    "task_id": "896b7205-1a9f-4656-9cbc-85b441652806"
  }
}
```
Response (abridged):
```json
{
  "content": [
    { "type": "text", "text": "📬 **Task Result**\n\nTask: 896b7205-…\nStatus: completed\nSummary: 你好！(Nǐ hǎo!)" },
    {
      "type": "json",
      "json": {
        "task_id": "896b7205-1a9f-4656-9cbc-85b441652806",
        "status": "completed",
        "goal": "Say hello in Chinese and tell me a fun fact about pandas.",
        "delegated_to": "claude_code",
        "format": "compact",
        "result": {
          "summary": "你好！(Nǐ hǎo!)…",
          "details": {
            "stdout": {
              "type": "text",
              "text": "你好！(Nǐ hǎo!) …"
            }
          },
          "evidence": null,
          "metadata": {
            "time_queued": "2025-10-06T21:45:12.420Z",
            "time_started": "2025-10-06T21:45:12.612Z",
            "time_completed": "2025-10-06T21:45:15.001Z",
            "duration_total_ms": 2581,
            "duration_execution_ms": 2389,
            "worker_type": "mcp",
            "exit_code": 0,
            "exit_reason": "success"
          }
        }
      }
    }
  ]
}
```

### Tips
- Call immediately after `devit_delegate` to fetch the worker response without scanning the full status table.
- The tool aggregates notifications recorded for the task; if multiple updates exist, it returns the entry matching the task’s current status (falling back to the latest notification).
- Combine with `devit_orchestration_status` for an overview of all tasks when debugging multiple assignments.
- Oversized outputs are clipped according to `max_response_chars`; check `result.details.truncated` and `result.metadata` for the full size and exit diagnostics.
