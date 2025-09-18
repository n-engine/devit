# DevIt VS Code Extension

Minimal MCP bridge for DevIt: timeline panel, approvals, and recipe launcher.

## Prerequisites

- Node.js 20+
- `npm` (bundled with Node)
- Local DevIt workspace (extension shells out to `target/debug/devit` by default)

## Build and package locally

```bash
npm ci
npm run build
npx vsce package --no-dependencies
```

The VSIX artifact (`devit-vscode.vsix`) lands in the current directory. Install it via the VS Code command palette (`Extensions: Install from VSIX...`).

## Commands exposed

- **DevIt: Show Panel** — opens the timeline webview (last 10 `.devit/journal.jsonl` events).
- **DevIt: Approve Last Request** — reads the journal for the latest `approval_required` entry and sends `server.approve` through `devit-mcpd`.
- **DevIt: Run Recipe…** — lists recipes (via `devit recipe list`) and runs the chosen id with `--dry-run`.

Quick tips:
- The panel refreshes automatically when `.devit/journal.jsonl` changes.
- Set `DEVIT_TUI_DEVIT_BIN`/`DEVIT_BIN` environment variables to point at a custom `devit` binary if needed.
- `npm run watch` keeps `out/` updated during development; launch the VS Code Extension Host (`Run Extension`) to test commands interactively.
