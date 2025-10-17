<#
.SYNOPSIS
  Helper script to launch devitd on Windows with debug logging and pipe transport.

.DESCRIPTION
  - Ensures the compiled binary exists (defaults to release build for x86_64-pc-windows-msvc).
  - Sets useful environment variables (RUST_LOG, optional DEVIT_NOTIFY_HOOK).
  - Starts devitd with --debug, named pipe socket, optional config path and secret override.

.EXAMPLE
  .\scripts\run_devitd_windows.ps1

.EXAMPLE
  .\scripts\run_devitd_windows.ps1 -Socket "\\.\pipe\devitd-test" -Secret dv_secret -Config ".\devit.core.toml"
#>

[CmdletBinding()]
param(
    [string]$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")),
    [string]$Target = "x86_64-pc-windows-msvc",
    [ValidateSet("debug", "release")]
    [string]$Profile = "release",
    [string]$Socket = "\\.\pipe\devitd",
    [string]$Secret = "",
    [string]$Config = "",
    [switch]$NoNotifyHook,
    [ValidateRange(0, 86400)]
    [int]$AutoShutdownSeconds = 30,
    [string[]]$ExtraArgs = @()
)

taskkill /F /IM devitd.exe 2>$null
taskkill /F /IM mcp-server.exe 2>$null
chcp 65001 | Out-Null
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
if ($PSStyle -and $PSStyle.OutputRendering) {
    $PSStyle.OutputRendering = 'PlainText'
}

$devitdPath = Join-Path $RepoRoot "target\$Target\$Profile\devitd.exe"
if (-not (Test-Path $devitdPath)) {
    Write-Error "devitd binary not found at '$devitdPath'. Please build it first (e.g. cargo build --target $Target --release)."
    exit 1
}

Push-Location $RepoRoot

try {
    if (-not $env:RUST_LOG) {
        $env:RUST_LOG = "devitd=debug,devit_mcp_tools=info"
    }

    $args = @("--socket", $Socket, "--debug")
    if ($AutoShutdownSeconds -gt 0) {
        $args += @("--auto-shutdown-after", $AutoShutdownSeconds)
    }
    if ($Secret) {
        $args += @("--secret", $Secret)
    }
    if ($Config) {
        $args += @("--config", (Resolve-Path $Config))
    }
    if ($ExtraArgs.Count -gt 0) {
        $args += $ExtraArgs
    }

    Write-Host "Launching devitd from $devitdPath"
    Write-Host "  Socket : $Socket"
    if ($Secret) { Write-Host "  Secret : $Secret" }
    if ($Config) { Write-Host "  Config : $Config" }
    if ($AutoShutdownSeconds -gt 0) {
        Write-Host "  Auto   : $AutoShutdownSeconds s idle timeout"
    } else {
        Write-Host "  Auto   : disabled"
    }
    Write-Host "  RUST_LOG=$($env:RUST_LOG)"
    if ($env:DEVIT_NOTIFY_HOOK) {
        Write-Host "  DEVIT_NOTIFY_HOOK=$($env:DEVIT_NOTIFY_HOOK)"
    }

    & $devitdPath @args
}
finally {
    Pop-Location
}
