# Orchestration Examples

## Running the Examples

```bash
# Basic delegation workflow
cargo run --example basic_delegation

# Monitor tasks via daemon backend
cargo run --example daemon_monitoring
```

## Prerequisites

- Build the DevIt workspace: `cargo build --bin devit --bin devitd`
- Ensure the daemon binary is available (`./scripts/devitd-start.sh` is handy)
- Set `DEVIT_SECRET` if you customised the daemon configuration

## Notes

- The examples auto-launch the daemon when running in Auto/Daemon modes.
- On CI or environments without Unix sockets, run the local mode examples only.
- Inspect `docs/ORCHESTRATION.md` for architecture details.
