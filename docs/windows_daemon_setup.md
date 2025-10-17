# Windows DevIt Daemon Quickstart

This note captures the steps we followed to get the Windows VM fully operational
with the DevIt daemon, MCP HTTP transport, Claude worker, and notification hook.
Avoiding UNC paths and stale binaries were the two big pain points.

## 1. Workspace layout

- **Clone/Sync on a local drive.** Put the repo under something like
  `C:\Users\naskel\devIt`. Running from a VirtualBox shared folder
  (`\\?\UNC\VBoxSvr\devIt`) breaks Node watchers and the PowerShell hook.
- If you must use the share, map it to a drive letter (`net use Z: \\VBoxSvr\devIt`)
  and refer to `Z:\…` everywhere; avoid the `\\?\` prefix.

## 2. Build & process hygiene

```powershell
taskkill /F /IM devitd.exe   # stop lingering daemons
cargo build --target x86_64-pc-windows-msvc --release
taskkill /F /IM devitd.exe   # clean up any process spawned during build
```

Always kill `devitd.exe` (and `mcp-server.exe` if running) before launching new
instances; Windows keeps “ghost” binaries alive otherwise.

## 3. Environment variables

Set these once per PowerShell session (or persist with `setx`):

```powershell
$env:DEVIT_DAEMON_SOCKET = '\\.\pipe\devitd'
$env:DEVIT_NOTIFY_HOOK   = 'C:\Users\naskel\devIt\scripts\devit_notify_example.ps1'
# Optional: force PowerShell path if auto-detection fails
$env:DEVIT_POWERSHELL    = 'C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe'
# Optional: auto-shutdown devitd after 30s with no clients (0 disables)
$env:DEVIT_AUTO_SHUTDOWN_AFTER = '30'
```

For MCP HTTP runs, also set the auth token and base URL overrides as needed.

## 4. Launch helpers

From `PowerShell`, run:

```powershell
PS C:\Users\naskel\devIt> scripts\run_devitd_windows.ps1 `
    -Socket \\.\pipe\devitd `
    -Secret <shared-secret> `
    -Config .\win_devit.core.toml `
    -AutoShutdownSeconds 30
```

Key expectations in the log:

- `Resolved PowerShell executable for notify hook…` shows the resolved path.
- `Notify hook script path inspection… exists: true` confirms the hook is found.
- `Notification hook completed successfully` with a matching ACK marker path.

For the HTTP transport:

```powershell
PS C:\Users\naskel\devIt> scripts\run_mcp_server_windows_http.ps1 `
    -ListenHost 0.0.0.0 `
    -Port 3001 `
    -AuthToken dv_test123 `
    -CorsOrigin http://localhost:5173
```

Be sure `/message` and `/sse` stay on the same host as the manifest. When tunneling
through ngrok, append `?ngrok-skip-browser-warning=1` to every URL.

> Tip: the HTTP helper also exports `DEVIT_AUTO_SHUTDOWN_AFTER` so that any daemon
> spawned by the MCP server inherits the same idle timeout.

## 5. Notification hook checklist

- `scripts/devit_notify_example.ps1` requires STA; the daemon now launches
  PowerShell with `-STA -File`.
- The hook logs to `%TEMP%\devit-notify\notify.log` and expects `zk` to be
  focused via SendKeys/clipboard. Claude window title must contain “Claude”.
- ACK flow: the daemon exports `DEVIT_ACK_PIPE` (named pipe) and
  `DEVIT_ACK_MARKER` (file). Success shows `ACK received; created marker file`
  in the daemon log and `ACK received via pipe.` inside the hook log.

## 6. Claude worker configuration

`win_devit.core.toml` now launches the CLI through PowerShell:

```toml
[workers.claude_code]
type = "cli"
binary = 'C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe'
args = [
    '-NoProfile',
    '-NonInteractive',
    '-ExecutionPolicy', 'Bypass',
    '-File', 'C:\Users\naskel\AppData\Roaming\npm\claude.ps1',
    '--print', '--output-format', 'json', '--dangerously-skip-permissions'
]
```

This keeps the worker out of the UNC path and lets the CLI emit proper JSON for
DevIt to consume.

## 7. Sanity checks

1. `scripts/desktop_input_demo.ps1` → mouse, keyboard, and screenshot work.
2. Delegate a task via MCP (`devit_delegate`) → notification hook fires, ACK marker appears.
3. `devit_orchestration_status` from MCP client returns immediately with the latest task state.
4. HTTP endpoints: `Invoke-RestMethod https://<host>/.well-known/mcp.json?…`,
   `curl.exe -N https://<host>/sse?…` remain responsive.

Keep this checklist handy whenever the VM or shared folder setup changes; most
failures boiled down to a mismatched executable path or the repo living on the
VirtualBox share.
