<#
.SYNOPSIS
  End-to-end smoke test for devit_mouse, devit_keyboard and devit_screenshot via MCP HTTP (PowerShell version).

.DESCRIPTION
  - Calls initialize → tools/list → devit_mouse → devit_keyboard → devit_screenshot.
  - Expects the MCP server to expose the HTTP transport `/message`.
  - Outputs the textual response blocks for each tool and surfaces raw errors when present.

.PARAMETER Url
  MCP HTTP endpoint (default: http://127.0.0.1:3001/message).

.PARAMETER Bearer
  Optional bearer token for Authorization header.

.PARAMETER MoveX
  Absolute X coordinate for the mouse move (default: 360).

.PARAMETER MoveY
  Absolute Y coordinate for the mouse move (default: 240).

.PARAMETER Text
  Text payload to type via devit_keyboard (default: "Hello from devit_keyboard!").

.PARAMETER SkipScreenshot
  Switch to skip the devit_screenshot call.
#>
[CmdletBinding()]
param(
    [string]$Url = "http://127.0.0.1:3001/message",
    [string]$Bearer = "",
    [int]$MoveX = 360,
    [int]$MoveY = 240,
    [string]$Text = "Hello from devit_keyboard!",
    [switch]$SkipScreenshot
)

function Write-Step {
    param([string]$Message)
    Write-Host $Message -ForegroundColor Cyan
}

function Invoke-McpRequest {
    param(
        [string]$Label,
        [hashtable]$Payload
    )

    $headers = @{ "Content-Type" = "application/json" }
    if ($Bearer) {
        $headers["Authorization"] = "Bearer $Bearer"
    }

    try {
        $jsonBody = ($Payload | ConvertTo-Json -Depth 10)
        $response = Invoke-RestMethod -Uri $Url -Method Post -Headers $headers -Body $jsonBody
    } catch {
        Write-Host "[$Label] HTTP error: $($_.Exception.Message)" -ForegroundColor Red
        throw
    }

    if ($response.error) {
        Write-Host "[$Label] MCP error:" -ForegroundColor Red
        $response.error | ConvertTo-Json -Depth 10 | Write-Host
        throw "MCP call failed for $Label"
    }

    return $response
}

function Show-TextContent {
    param(
        [string]$Label,
        $Response
    )

    if (-not $Response.result) {
        Write-Host "[$Label] unexpected response:" -ForegroundColor Yellow
        $Response | ConvertTo-Json -Depth 10 | Write-Host
        return
    }

    $texts = @()
    if ($Response.result.content) {
        foreach ($item in $Response.result.content) {
            if ($item.type -eq "text" -and $item.text) {
                $texts += $item.text
            }
        }
    }

    if ($texts.Count -gt 0) {
        foreach ($line in $texts) {
            Write-Host "  $line"
        }
    } else {
        Write-Host "[$Label] no textual content" -ForegroundColor Yellow
    }
}

Write-Step "[1/5] initialize"
$initPayload = @{
    jsonrpc = "2.0"
    id      = 0
    method  = "initialize"
    params  = @{}
}
Invoke-McpRequest -Label "initialize" -Payload $initPayload | Out-Null

Write-Step "[2/5] tools/list (checking devit_mouse/devit_keyboard)"
$toolsPayload = @{
    jsonrpc = "2.0"
    id      = 1
    method  = "tools/list"
    params  = @{}
}
$toolsResponse = Invoke-McpRequest -Label "tools/list" -Payload $toolsPayload
$toolNames = @()
if ($toolsResponse.result -and $toolsResponse.result.tools) {
    $toolNames = $toolsResponse.result.tools | ForEach-Object { $_.name }
}
if ($toolNames -notcontains "devit_mouse" -or $toolNames -notcontains "devit_keyboard") {
    throw "Required tools not reported by the server"
}
Write-Host "  -> tools detected"

Write-Step "[3/5] devit_mouse"
$mousePayload = @{
    jsonrpc = "2.0"
    id      = 2
    method  = "tools/call"
    params  = @{
        name       = "devit_mouse"
        arguments  = @{
            actions = @(
                @{ type = "move"; x = $MoveX; y = $MoveY; sync = $true },
                @{ type = "sleep"; millis = 250 },
                @{ type = "click"; button = 1; count = 1 },
                @{ type = "sleep"; millis = 250 }
            )
        }
    }
}
$mouseResponse = Invoke-McpRequest -Label "devit_mouse" -Payload $mousePayload
Show-TextContent -Label "devit_mouse" -Response $mouseResponse

Write-Step "[4/5] devit_keyboard"
$keyboardPayload = @{
    jsonrpc = "2.0"
    id      = 3
    method  = "tools/call"
    params  = @{
        name       = "devit_keyboard"
        arguments  = @{
            actions = @(
                @{ type = "text"; text = $Text; delay_ms = 20 },
                @{ type = "sleep"; millis = 300 }
            )
        }
    }
}
$keyboardResponse = Invoke-McpRequest -Label "devit_keyboard" -Payload $keyboardPayload
Show-TextContent -Label "devit_keyboard" -Response $keyboardResponse

if (-not $SkipScreenshot.IsPresent) {
    Write-Step "[5/5] devit_screenshot"
    $screenshotPayload = @{
        jsonrpc = "2.0"
        id      = 4
        method  = "tools/call"
        params  = @{
            name       = "devit_screenshot"
            arguments  = @{}
        }
    }
    $shotResponse = Invoke-McpRequest -Label "devit_screenshot" -Payload $screenshotPayload
    Show-TextContent -Label "devit_screenshot" -Response $shotResponse
    $shotPath = $shotResponse.result.structuredContent.image.path
    if ($shotPath) {
        Write-Host "  Screenshot saved at: $shotPath"
    }
} else {
    Write-Step "[5/5] devit_screenshot skipped (per flag)"
}

Write-Host ""
Write-Host "✅ Desktop input demo completed."
