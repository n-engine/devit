Param(
    [string]$Socket = "127.0.0.1:60459",
    [string]$Secret = "change-me",
    [switch]$Release
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Write-Host "[smoke] Building devitd..."
if ($Release) {
    cargo build -p devitd --release | Out-Null
    $devitdPath = Join-Path (Resolve-Path "target/release").Path "devitd.exe"
} else {
    cargo build -p devitd | Out-Null
    $devitdPath = Join-Path (Resolve-Path "target/debug").Path "devitd.exe"
}

if (-not (Test-Path $devitdPath)) {
    Write-Error "devitd binary not found at $devitdPath"
    exit 1
}

Write-Host "[smoke] Starting devitd at $Socket"
$args = @("--socket", $Socket, "--secret", $Secret, "--debug")
$p = Start-Process -FilePath $devitdPath -ArgumentList $args -NoNewWindow -PassThru

try {
    # Wait for port to be open (max 10s)
    $ok = $false
    for ($i = 0; $i -lt 20; $i++) {
        try {
            $host,$port = $Socket.Split(":",2)
            $tcp = New-Object Net.Sockets.TcpClient
            $async = $tcp.BeginConnect($host, [int]$port, $null, $null)
            $ready = $async.AsyncWaitHandle.WaitOne(500)
            if ($ready -and $tcp.Connected) {
                $tcp.Close()
                $ok = $true
                break
            }
            $tcp.Close()
        } catch {
            Start-Sleep -Milliseconds 500
        }
    }

    if (-not $ok) {
        Write-Error "[smoke] devitd did not open $Socket"
        exit 1
    }

    Write-Host "[smoke] OK: devitd listening on $Socket"
    exit 0
}
finally {
    if ($p -and !$p.HasExited) {
        Stop-Process -Id $p.Id -Force
        Start-Sleep -Milliseconds 200
    }
}

