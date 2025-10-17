# DevIt Configuration Reference

## File Locations

- User config: `~/.config/devit/config.toml`
- System config: `/etc/devit/config.toml`
- Project config: `.devit/config.toml`

## Complete Configuration

```toml
# Core settings
[core]
log_level = "info"  # trace | debug | info | warn | error
color_output = true

# Orchestration
[orchestration]
mode = "auto"  # auto | daemon | local
auto_start_daemon = true
daemon_socket = "/tmp/devitd.sock"
daemon_start_timeout_ms = 5000
expected_daemon_version = "devitd/0.1.0"      # Daemon version expected by MCP workers (optional)
max_concurrent_tasks = 5
default_timeout_secs = 1800
default_watch_patterns = ["*.rs", "*.toml", "*.md"]

[orchestration.capabilities.screenshot]
enabled = true
rate_limit = "10/minute"

[daemon]
expected_worker_version = "mcp-server/0.1.0"

[daemon.approvals]
default_target = "client:approver"  # Worker/client ident that receives approval requests

# Workspace sandbox
[workspace]
sandbox_root = "~/workspace/devit"
default_project = "project-a"
allowed_projects = ["project-a/**", "project-b", "experiments/*"]
max_size_mb = 1000
max_files = 10000

[tools.screenshot]
enabled = true
backend = "scrot"         # ou "imagemagick"
format = "png"
output_dir = ".devit/screenshots"

> Tip: initialise the sandbox once with `devit init --sandbox ~/workspace/devit --allow project-a/**`.

### Workspace CLI helpers

| Command | Description |
|---------|-------------|
| `devit init --sandbox <path>` | Canonicalises the sandbox root and writes `[workspace]` entries to `devit.core.toml`. |
| `devit cd <relative>` | Validates the target against the sandbox allow-list, updates `default_project`, and prints the canonical absolute path. |
| `devit pwd` | Prints the currently-resolved sandbox working directory (taking `default_project` into account). |

# Delegation defaults
[orchestration.delegation]
default_ai = "claude_code"  # identifiant d'un worker déclaré (ex: claude_code, codex)
default_context_size = "16k"
include_git_diff = true

# Chaos testing (development)
[chaos]
enabled = false
error_rate = 0.1
delay_ms = 100
```

### Capabilities

La section `orchestration.capabilities` active ou non des fonctionnalités sensibles côté daemon. Exemple :

```toml
[orchestration.capabilities.screenshot]
enabled = true
rate_limit = "10/minute"  # format `<quantité>/<fenêtre>` avec minute, second, hour…
```

> ⚠️ Le daemon refuse toute capture d'écran si la capacité est désactivée ou si le quota est dépassé.

### Tools

Les outils MCP peuvent également être configurés via `[tools.<nom>]`. Le screenshot reprend un backend Linux (`scrot` par défaut) et force la sortie dans le sandbox :

```toml
[tools.screenshot]
enabled = true
backend = "scrot"         # ou "imagemagick" (importe)
format = "png"
output_dir = ".devit/screenshots"  # relatif au sandbox, ou chemin absolu dans /tmp/devit-screenshots
```

Le backend crée les captures dans `<sandbox>/.devit/screenshots` (ou `/tmp/devit-screenshots`) et retourne le chemin relatif dans la réponse MCP.

## Environment Variables

Override config file settings:

| Variable | Description | Default | Example |
|----------|-------------|---------|---------|
| `DEVIT_CONFIG` | Config file path | `~/.config/devit/config.toml` | `DEVIT_CONFIG=./devit.toml` |
| `DEVIT_NO_AUTO_START` | Disable daemon auto-launch | unset | `DEVIT_NO_AUTO_START=1` |
| `DEVITD_BINARY` | Custom `devitd` path | auto-detect | `DEVITD_BINARY=/opt/devit/devitd` |
| `DEVIT_EXPECTED_WORKER_VERSION` | Force la version worker acceptée par `devitd` (override config) | depuis config | `DEVIT_EXPECTED_WORKER_VERSION=mcp-server/0.1.0` |
| `DEVIT_EXPECTED_DAEMON_VERSION` | Version minimale acceptée côté MCP | unset | `DEVIT_EXPECTED_DAEMON_VERSION=devitd/0.1.0` |
| `DEVIT_CLIENT_VERSION` | Version explicitement annoncée pendant REGISTER | unset | `DEVIT_CLIENT_VERSION=custom-bot/1.2.3` |
| `DEVIT_FORCE_ROOT` | Force la racine sandbox (désactive l'auto-détection) | unset | `DEVIT_FORCE_ROOT=/home/user/project` |
| `DEVIT_DAEMON_SOCKET` | Socket path | `/tmp/devitd.sock` | `DEVIT_DAEMON_SOCKET=/run/user/1000/devit.sock` |
| `DEVIT_SECRET` | Daemon auth secret | `change-me-in-production` | `DEVIT_SECRET=$(openssl rand -hex 32)` |
| `DEVIT_SANDBOX_ROOT` | Override sandbox root (CLI convenience) | unset | `DEVIT_SANDBOX_ROOT=$PWD` |
| `DEVIT_LOG` | Override log level | from config | `DEVIT_LOG=debug` |
| `CI` | CI environment flag (disables auto-start) | unset | `CI=1` |

> ℹ️ **Version handshake** — lorsque `expected_worker_version` est défini (ou `DEVIT_EXPECTED_WORKER_VERSION`), `devitd` refuse tout REGISTER dont le champ `version` ne correspond pas. Côté MCP, la variable `DEVIT_EXPECTED_DAEMON_VERSION` permet de refuser un daemon trop ancien. Les binaires officiels annoncent automatiquement leur version (`mcp-server/x.y.z`, `devit-cli/x.y.z`).

## Precedence Order

1. CLI arguments (highest)
2. Environment variables
3. Project config (`.devit/config.toml`)
4. User config (`~/.config/devit/config.toml`)
5. System config (`/etc/devit/config.toml`)
6. Built-in defaults (lowest)

## Worker Definitions

Déclare chaque worker sous `[workers.<ident>]` dans `devit.core.toml` :

```toml
[workers.claude_code]
type = "cli"
binary = "claude"
args = [
  "--print",
  "--output-format", "json",
  "--mcp-config", "{workspace}/devit-mcp.json",
  "--dangerously-skip-permissions",
  "{goal}"
]
timeout_secs = 300
parse_mode = "json"
```

Pour un worker MCP (stdio) comme Codex :

```toml
[workers.codex]
type = "mcp"
binary = "/home/naskel/.npm-global/bin/codex"
args = [
  "--cd", "/home/naskel/workspace/devIt",
  "--model", "{model}",
  "mcp-server"
]
timeout_secs = 180
default_model = "o4-mini"
allowed_models = ["o3", "o4-mini", "o4-opus"]
```

Champs supportés :

| Champ | Description |
|-------|-------------|
| `type` | `cli` (exécutable one-shot) ou `mcp` (serveur MCP sur stdin/stdout). |
| `binary` | Commande à lancer (chemin relatif ou absolu, sans `..`). |
| `args` | Arguments passés au worker.<br>Pour `cli`, les placeholders `{goal}`, `{workspace}`, `{task_id}`, `{model}` sont évalués avant exécution.<br>Pour `mcp`, la commande est lancée telle quelle (le goal est envoyé via JSON-RPC et `{model}` est remplacé avant le spawn). |
| `timeout_secs` | Timeout maximum côté daemon (borne supérieure partagée avec le timeout de la tâche). |
| `parse_mode` | `json` (par défaut) ou `text`. JSON ajoute la sortie complète dans `details.stdout`. |
| `working_dir` (optionnel) | Répertoire à utiliser pour le subprocess après interpolation (sinon sandbox/working_dir). |
| `max_response_chars` (optionnel) | Limite en caractères pour la sortie `stdout` du worker. Au-delà, le daemon tronque la réponse, ajoute `truncated=true` et `original_size`. |
| `mcp_tool` (optionnel) | Pour les workers `type="mcp"`, nom de l’outil à invoquer via `tools/call` (par défaut `devit_delegate`). |
| `default_model` (optionnel) | Modèle utilisé par défaut lorsqu’aucune valeur n’est fournie via `devit_delegate`. Recommandé si `args` contient `{model}`. |
| `allowed_models` (optionnel) | Liste blanche des modèles autorisés. Si définie, toute requête hors liste est rejetée avant de lancer le worker. |
| `mcp_arguments` (optionnel) | Objet JSON fusionné dans les arguments envoyés à l’outil MCP (permet d’ajouter `sandbox`, options expérimentales, etc.). |

> ℹ️ **Workers MCP** — le daemon lance le binaire, effectue le handshake JSON-RPC (`initialize`, `tools/list`), puis appelle l’outil spécifié par `mcp_tool` (avec `goal` et `prompt` = ta requête). Le processus est stoppé après chaque tâche. Vérifie que le serveur MCP parle bien sur STDIN/STDOUT (ex: `codex … mcp-server`).

Sans entrée correspondante, le daemon retombe sur le mode polling traditionnel (workers MCP utilisant `devit_poll_tasks`).

> Exemple :
> ```toml
> [workers.claude_code]
> max_response_chars = 10000
> ```
> Tronque les réponses supérieures à ~2 500 tokens tout en signalant la taille initiale.


## Daemon Management

### Manual Start

```bash
# Helper script
./scripts/devitd-start.sh

# Direct launch
DEVIT_SECRET=my-secret ./target/release/devitd --socket /tmp/devitd.sock

# Verbose logs
RUST_LOG=debug ./target/release/devitd
```

### Status Check

```bash
# Via CLI
devit status --mode daemon

# Ping socket
nc -zU /tmp/devitd.sock && echo "Daemon running"
```

### Shutdown

```bash
# Graceful shutdown (TODO: implement RPC)
# devit daemon shutdown

# Force kill
pkill -f "devitd.*sock"
```

## Further Reading

- [docs/ORCHESTRATION.md](ORCHESTRATION.md) – architecture and mode selection
- [docs/DAEMON_PROTOCOL.md](DAEMON_PROTOCOL.md) – daemon protocol details
- [docs/WORKSPACE.md](WORKSPACE.md) – sandbox navigation, `devit cd`, and MCP tooling
### Approvals target

`devitd` route toutes les demandes d'approbation (`PolicyAction::NeedApproval`) vers la cible définie dans `[daemon.approvals]`. Exemple :

```toml
[daemon.approvals]
default_target = "team:reviewers"
```

Si le champ est omis ou vide, le daemon utilise `client:approver`. Les refus d'approbation renvoient désormais un `NOTIFY` avec `status="failed"` et un `ERR` structuré côté client initiateur.
