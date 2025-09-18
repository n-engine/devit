# Recipes YAML Reference

DevIt recipes live under `.devit/recipes/` and power the TUI (`R` panel), VS Code commands, and the `devit recipe` CLI. This page documents the accepted fields and bundled starters.

## Minimal schema

```yaml
id: my-recipe          # required, unique identifier
name: "Display name"   # required

description: "Optional short blurb"
steps:                 # optional (defaults to empty list)
  - kind: shell        # shell | git | devit
    name: "Optional label"
    run: "echo 'hi'"   # shell step: command executed via bash -lc
  - kind: git
    args: ["status", "--short"]
  - kind: devit
    args: ["tool", "list"]
```

Key points:
- `id` and `name` are mandatory and must be non-empty.
- `description` is optional but recommended for UI listings.
- `steps` accepts zero or more items; each item must specify `kind` and the fields required by that kind:
  - `shell`: requires `run` (string, executed via `bash -lc`).
  - `git`: requires `args` (array of arguments passed to `git`).
  - `devit`: requires `args` (array passed to the local `devit` binary).
- Unknown keys are ignored; keep YAML concise for maintainability.

Validate recipes locally:

```bash
yamllint .devit/recipes
cargo run -p devit-cli -- recipe list
```

## Bundled starters

| Recipe id | File | Purpose |
|-----------|------|---------|
| `add-ci` | `.devit/recipes/add-ci.yaml` | Reminder to add a GitHub Actions workflow |
| `migrate-jest-vitest` | `.devit/recipes/migrate-jest-vitest.yaml` | Checklist for moving from Jest to Vitest |
| `rust-upgrade-1.81` | `.devit/recipes/rust-upgrade-1.81.yaml` | Guidance for upgrading the Rust toolchain |

List available recipes:

```bash
devit recipe list | jq
```

Run a recipe without applying changes (dry run):

```bash
devit recipe run add-ci --dry-run
```

The same actions are exposed in the TUI (press `R`) and the VS Code extension (“DevIt: Run Recipe…” command).
