<#!
.SYNOPSIS
  Notification hook example for Windows mirroring the Linux bash version.

.DESCRIPTION
  - Logs incoming notification metadata.
  - Attempts to bring the "Claude" window to foreground and send a short summary via SendKeys.
  - Retries on failure (default 3 attempts, 15s delay) and waits for daemon ACK via DEVIT_ACK_PIPE or DEVIT_ACK_MARKER.
  - Requires PowerShell 5.1+ running in STA mode for SendKeys to work reliably.

.NOTES
  Set DEVIT_NOTIFY_HOOK to point at this script:
    set DEVIT_NOTIFY_HOOK=C:\path\to\devit_notify_example.ps1

  Optional overrides via environment variables:
    DEVIT_NOTIFY_MAX_RETRIES   (int, default 3)
    DEVIT_NOTIFY_RETRY_DELAY   (seconds, default 15)
    DEVIT_NOTIFY_ACK_TIMEOUT   (seconds, default 30)
#>

chcp 65001 | Out-Null
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
if ($PSStyle -and $PSStyle.OutputRendering) {
    $PSStyle.OutputRendering = 'PlainText'
}

Set-StrictMode -Version Latest

$ErrorActionPreference = 'Stop'

function Write-Log {
    param([string]$Message)
    Add-Content -Path $script:LogFile -Value $Message -Encoding UTF8
}

function Format-Value {
    param([string]$Value)
    if ([string]::IsNullOrEmpty($Value)) { return "<unset>" }
    return $Value
}

function Get-EnvValue {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [string]$Default = ''
    )
    $val = [Environment]::GetEnvironmentVariable($Name)
    if ([string]::IsNullOrEmpty($val)) { return $Default }
    return $val
}

# --- Initialise logging ----------------------------------------------------
$logRoot = Join-Path -Path ([IO.Path]::GetTempPath()) -ChildPath 'devit-notify'
New-Item -Path $logRoot -ItemType Directory -Force | Out-Null
$LogFile = Join-Path -Path $logRoot -ChildPath 'notify.log'

Write-Log ''
Write-Log ('[{0}] Notification received' -f (Get-Date).ToString('s'))

# --- Gather environment ----------------------------------------------------
$TaskId   = Get-EnvValue 'DEVIT_NOTIFY_TASK_ID'
$Status   = Get-EnvValue 'DEVIT_NOTIFY_STATUS'
$Summary  = Get-EnvValue 'DEVIT_NOTIFY_SUMMARY'
$Worker   = Get-EnvValue 'DEVIT_NOTIFY_WORKER'
$Timestamp= Get-EnvValue 'DEVIT_NOTIFY_TIMESTAMP'
$WorkDir  = Get-EnvValue 'DEVIT_NOTIFY_WORKDIR'
$Details  = Get-EnvValue 'DEVIT_NOTIFY_DETAILS'
$Evidence = Get-EnvValue 'DEVIT_NOTIFY_EVIDENCE'
$Payload  = Get-EnvValue 'DEVIT_NOTIFY_PAYLOAD'
$AckPipe  = Get-EnvValue 'DEVIT_ACK_PIPE'
$AckMarker= Get-EnvValue 'DEVIT_ACK_MARKER'

Write-Log ('  Task id     : {0}' -f (Format-Value $TaskId))
Write-Log ('  Status      : {0}' -f (Format-Value $Status))
Write-Log ('  Worker      : {0}' -f (Format-Value $Worker))
Write-Log ('  Summary     : {0}' -f (Format-Value $Summary))
Write-Log ('  Timestamp   : {0}' -f (Format-Value $Timestamp))
if ($WorkDir)  { Write-Log ('  Working dir : {0}' -f $WorkDir) }
if ($Details)  { Write-Log ('  Details     : {0}' -f $Details) }
if ($Evidence) { Write-Log ('  Evidence    : {0}' -f $Evidence) }
Write-Log ('  Payload JSON: {0}' -f (Format-Value $Payload))

if (-not $TaskId -or -not $Status) {
    Write-Log '  ❌ Missing task/status, skipping notification.'
    exit 0
}

# Provide a fallback ACK marker if daemon did not supply one
if (-not $AckMarker) {
    $AckMarker = Join-Path $logRoot ('ack-{0}-{1}.marker' -f $TaskId, $PID)
    Write-Log ('  ⚠️  DEVIT_ACK_MARKER not provided, using fallback: {0}' -f $AckMarker)
}
Write-Log ('  ACK pipe    : {0}' -f (Format-Value $AckPipe))
Write-Log ('  ACK marker  : {0}' -f (Format-Value $AckMarker))

# --- Config ----------------------------------------------------------------
function Get-IntOrDefault {
    param([string]$Name, [int]$Default)
    $val = Get-EnvValue $Name
    if ($val -and $val -match '^-?\d+$') { return [int]$val }
    return $Default
}

$MaxRetries = Get-IntOrDefault 'DEVIT_NOTIFY_MAX_RETRIES' 3
if ($MaxRetries -lt 1) { $MaxRetries = 1 }
$RetryDelay = Get-IntOrDefault 'DEVIT_NOTIFY_RETRY_DELAY' 15
if ($RetryDelay -lt 1) { $RetryDelay = 1 }
$AckTimeout = Get-IntOrDefault 'DEVIT_NOTIFY_ACK_TIMEOUT' 30
if ($AckTimeout -lt 5) { $AckTimeout = 5 }

Write-Log ('  Retries     : {0} (delay {1}s, ACK timeout {2}s)' -f $MaxRetries, $RetryDelay, $AckTimeout)

# --- Helper: ensure STA for SendKeys ---------------------------------------
if ([Threading.Thread]::CurrentThread.ApartmentState -ne 'STA') {
    Write-Log '  ⚠️  Not running in STA mode; SendKeys may fail. Launch PowerShell with -STA.'
}

# Load required assemblies/functions once
Add-Type -AssemblyName System.Windows.Forms | Out-Null
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class DevitNative {
    [DllImport("user32.dll", SetLastError=true)]
    public static extern bool SetForegroundWindow(IntPtr hWnd);
}
"@

# --- SendKeys helper -------------------------------------------------------
function Send-ToClaude {
    param([string]$Message)

    try {
        $process = Get-Process | Where-Object { $_.MainWindowTitle -match 'Claude' } | Select-Object -First 1
    } catch {
        Write-Log ('  ⚠️  Failed to enumerate processes: {0}' -f $_.Exception.Message)
        return $false
    }

    if (-not $process -or $process.MainWindowHandle -eq 0) {
        Write-Log '  ⚠️  Claude window not found.'
        return $false
    }

    $handle = $process.MainWindowHandle
    if (-not [DevitNative]::SetForegroundWindow($handle)) {
        Write-Log '  ⚠️  Unable to focus Claude window.'
        return $false
    }

    Start-Sleep -Milliseconds 400

    $originalClipboard = $null
    $clipboardHadText = $false
    try {
        if ([System.Windows.Forms.Clipboard]::ContainsText()) {
            $originalClipboard = [System.Windows.Forms.Clipboard]::GetText()
            $clipboardHadText = $true
        }
        [System.Windows.Forms.Clipboard]::SetText($Message)

        [System.Windows.Forms.SendKeys]::SendWait("^v")
        Start-Sleep -Milliseconds 300
        [System.Windows.Forms.SendKeys]::SendWait("{ENTER}")
        Write-Log '  ✅ Message dispatched via clipboard paste.'
        return $true
    } catch {
        Write-Log ('  ❌ SendKeys/clipboard failed: {0}' -f $_.Exception.Message)
        return $false
    } finally {
        try {
            if ($clipboardHadText -and $null -ne $originalClipboard) {
                [System.Windows.Forms.Clipboard]::SetText($originalClipboard)
            } else {
                [System.Windows.Forms.Clipboard]::Clear()
            }
        } catch {
            Write-Log ('  ⚠️  Unable to restore clipboard: {0}' -f $_.Exception.Message)
        }
    }
}

# --- ACK wait helpers ------------------------------------------------------
function Wait-AckPipe {
    param([string]$PipeName, [int]$TimeoutSeconds)
    if (-not $PipeName) { return $false }

    $shortName = $PipeName
    if ($shortName -like '\\\\.\\pipe\\*') {
        $shortName = $shortName.Substring(9)
    }

    try {
        $pipe = New-Object System.IO.Pipes.NamedPipeClientStream('.', $shortName, [System.IO.Pipes.PipeDirection]::In)
        $pipe.Connect($TimeoutSeconds * 1000)
        $buffer = New-Object byte[] 1
        $read = $pipe.Read($buffer, 0, 1)
        $pipe.Dispose()
        if ($read -gt 0) {
            Write-Log '  ✅ ACK received via pipe.'
            return $true
        }
        Write-Log '  ⚠️  ACK pipe closed without data.'
    } catch {
        Write-Log ('  ⚠️  ACK pipe error: {0}' -f $_.Exception.Message)
    } finally {
        if ($pipe) { $pipe.Dispose() }
    }
    return $false
}

function Wait-AckMarker {
    param([string]$MarkerPath, [int]$TimeoutSeconds)
    if (-not $MarkerPath) { return $false }

    for ($i = 1; $i -le $TimeoutSeconds; $i++) {
        if (Test-Path -LiteralPath $MarkerPath) {
            Write-Log ('  ✅ ACK received via marker at t={0}s' -f $i)
            try { Remove-Item -LiteralPath $MarkerPath -ErrorAction SilentlyContinue } catch {}
            return $true
        }
        Start-Sleep -Seconds 1
    }

    Write-Log ('  ⚠️  ACK marker timeout after {0}s' -f $TimeoutSeconds)
    return $false
}

function Wait-ForAck {
    param([int]$TimeoutSeconds)

    if (Wait-AckPipe -PipeName $AckPipe -TimeoutSeconds $TimeoutSeconds) {
        return $true
    }

    return (Wait-AckMarker -MarkerPath $AckMarker -TimeoutSeconds $TimeoutSeconds)
}

# --- Message composition ---------------------------------------------------
$message = "task: $TaskId status: $Status"
if ($Summary) {
    if ($Summary.Length -le 20) {
        $message += " | $Summary"
    } else {
        Write-Log ('  ℹ️  Summary too long for inline display ({0} chars).' -f $Summary.Length)
    }
}

# --- Retry loop ------------------------------------------------------------
for ($attempt = 1; $attempt -le $MaxRetries; $attempt++) {
    Write-Log ''
    Write-Log ('[{0}] Attempt {1}/{2}' -f (Get-Date).ToString('s'), $attempt, $MaxRetries)

    if (Send-ToClaude -Message $message) {
        if (Wait-ForAck -TimeoutSeconds $AckTimeout) {
            Write-Log '  🎉 Notification delivered with ACK.'
            exit 0
        }
    } else {
        Write-Log '  ❌ Failed to deliver message to Claude.'
    }

    if ($attempt -lt $MaxRetries) {
        Write-Log ('  ⏳ Retry in {0}s...' -f $RetryDelay)
        Start-Sleep -Seconds $RetryDelay
    }
}

# --- Final failure ---------------------------------------------------------
Write-Log ''
Write-Log ('[{0}] ❌ FINAL FAILURE' -f (Get-Date).ToString('s'))
Write-Log ('  Claude window unreachable or no ACK received.')
Write-Log ('  Task: {0} | Status: {1}' -f $TaskId, $Status)

exit 1
