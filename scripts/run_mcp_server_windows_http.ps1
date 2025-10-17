<#
.SYNOPSIS
  Helper script to launch devit-mcp-server on Windows with HTTP/SSE transport.

.DESCRIPTION
  - Ensures the binary `devit-mcp-server.exe` exists (defaults to release build for x86_64-pc-windows-msvc).
  - Configures logging (`RUST_LOG`) and optional notification hook.
  - Starts the MCP server with HTTP transport, host/port overrides, auth token, CORS, etc.

.PARAMETER RepoRoot
  Repository root (defaults to parent of scripts folder).

.PARAMETER Target
  Cargo target triple (default: x86_64-pc-windows-msvc).

.PARAMETER Profile
  Cargo profile to use (debug | release).

.PARAMETER ListenHost
  Host binding for HTTP transport (default: 0.0.0.0).

.PARAMETER Port
  Port binding for HTTP transport (default: 3001).

.PARAMETER AuthToken
  Optional bearer token required from clients.

.PARAMETER CorsOrigin
  Optional CORS allowed origins (array).

.PARAMETER DisableSse
  Disable SSE endpoint (enabled by default).

.PARAMETER WorkingDir
  Override MCP working directory (defaults to repository root/config resolution).

.PARAMETER Config
  Optional path to `devit.core.toml`.

.PARAMETER NoNotifyHook
  Skip wiring `DEVIT_NOTIFY_HOOK`.

.PARAMETER ExtraArgs
  Additional arguments appended verbatim.

.EXAMPLE
  .\scripts\run_mcp_server_windows_http.ps1

.EXAMPLE
  .\scripts\run_mcp_server_windows_http.ps1 `
      -ListenHost 127.0.0.1 `
      -Port 3001 `
      -AuthToken dv_test123 `
      -CorsOrigin http://localhost:5173
#>

[CmdletBinding()]
param(
    [string]$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")),
    [string]$Target = "x86_64-pc-windows-msvc",
    [ValidateSet("debug", "release")]
    [string]$Profile = "release",
    [string]$ListenHost = "0.0.0.0",
    [int]$Port = 3001,
    [string]$AuthToken = "",
    [string[]]$CorsOrigin = @(),
    [switch]$DisableSse,
    [string]$WorkingDir = "",
    [string]$Config = "",
    [switch]$NoNotifyHook,
    [ValidateRange(0, 86400)]
    [int]$DevitAutoShutdownSeconds = 30,
    [string[]]$ExtraArgs = @()
)

chcp 65001 | Out-Null
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
if ($PSStyle -and $PSStyle.OutputRendering) {
    $PSStyle.OutputRendering = 'PlainText'
}

$binaryPath = Join-Path $RepoRoot "target\$Target\$Profile\mcp-server.exe"
if (-not (Test-Path $binaryPath)) {
    Write-Error "mcp-server.exe not found at '$binaryPath'. Build it first (cargo build -p mcp-server --target $Target --$Profile)."
    exit 1
}

Push-Location $RepoRoot

try {
    if (-not $env:RUST_LOG) {
        $env:RUST_LOG = "devit_mcp_server=debug,devit_mcp_tools=info,axum=info"
    }

    $env:DEVIT_AUTO_SHUTDOWN_AFTER = [string]$DevitAutoShutdownSeconds

    if (-not $NoNotifyHook.IsPresent) {
        $hookPath = Join-Path $RepoRoot "scripts\devit_notify_example.ps1"
        if (Test-Path $hookPath) {
            $env:DEVIT_NOTIFY_HOOK = $hookPath
        } else {
            Write-Warning "Notification hook example not found at $hookPath. Continuing without DEVIT_NOTIFY_HOOK."
        }
    }

    $args = @("--transport", "http", "--host", $ListenHost, "--port", $Port)

    if ($DisableSse) {
        $args += "--disable-sse"
    } else {
        $args += "--enable-sse"
    }

    if ($AuthToken) {
        $args += @("--auth-token", $AuthToken)
    }

    if ($CorsOrigin) {
        foreach ($origin in $CorsOrigin) {
            $args += @("--cors-origin", $origin)
        }
    }

    if ($WorkingDir) {
        $args += @("--working-dir", (Resolve-Path $WorkingDir))
    }

    if ($Config) {
        $args += @("--config", (Resolve-Path $Config))
    }

    if ($ExtraArgs.Count -gt 0) {
        $args += $ExtraArgs
    }

    Write-Host "Launching devit-mcp-server (HTTP) from $binaryPath" -ForegroundColor Cyan
    Write-Host "  Host    : $ListenHost"
    Write-Host "  Port    : $Port"
    if ($AuthToken) { Write-Host "  Auth    : Bearer $AuthToken" }
    if ($CorsOrigin) { Write-Host "  CORS    : $($CorsOrigin -join ', ')" }
    Write-Host "  SSE     : $([bool](-not $DisableSse))"
    Write-Host "  DEVIT_AUTO_SHUTDOWN_AFTER = $DevitAutoShutdownSeconds"
    if ($WorkingDir) { Write-Host "  Working : $WorkingDir" }
    if ($Config) { Write-Host "  Config  : $Config" }
    Write-Host "  RUST_LOG= $($env:RUST_LOG)"
    if ($env:DEVIT_NOTIFY_HOOK) {
        Write-Host "  DEVIT_NOTIFY_HOOK= $($env:DEVIT_NOTIFY_HOOK)"
    }

    & $binaryPath @args
}
finally {
    Pop-Location
}
