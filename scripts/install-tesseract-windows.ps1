# Install Tesseract OCR for DevIt on Windows
# Usage: .\scripts\install-tesseract-windows.ps1

$ErrorActionPreference = "Stop"

Write-Host "=== DevIt - Tesseract OCR Installer for Windows ===" -ForegroundColor Cyan

# Configuration
$tesseractVersion = "5.3.3.20231005"
$installerUrl = "https://digi.bib.uni-mannheim.de/tesseract/tesseract-ocr-w64-setup-$tesseractVersion.exe"
$installerPath = "$env:TEMP\tesseract-installer.exe"
$installPath = "C:\Program Files\Tesseract-OCR"

# Check if already installed
if (Test-Path "$installPath\tesseract.exe") {
    Write-Host "✓ Tesseract already installed at: $installPath" -ForegroundColor Green
    
    # Check PATH
    if ($env:Path -like "*$installPath*") {
        Write-Host "✓ Tesseract already in PATH" -ForegroundColor Green
        Write-Host ""
        & "$installPath\tesseract.exe" --version
        Write-Host ""
        Write-Host "Installation complete! Tesseract is ready to use." -ForegroundColor Green
        exit 0
    } else {
        Write-Host "⚠ Tesseract installed but not in PATH. Adding now..." -ForegroundColor Yellow
        # Skip download, just add to PATH
        goto AddToPath
    }
}

# Download installer
Write-Host "→ Downloading Tesseract v$tesseractVersion..." -ForegroundColor Yellow
try {
    Invoke-WebRequest -Uri $installerUrl -OutFile $installerPath -UseBasicParsing
    Write-Host "✓ Download complete" -ForegroundColor Green
} catch {
    Write-Host "✗ Download failed: $_" -ForegroundColor Red
    Write-Host ""
    Write-Host "Manual download: $installerUrl" -ForegroundColor Yellow
    exit 1
}

# Run installer (silent mode)
Write-Host "→ Installing Tesseract (this may take 1-2 minutes)..." -ForegroundColor Yellow
Write-Host "  Install path: $installPath" -ForegroundColor Gray

try {
    $process = Start-Process -FilePath $installerPath -ArgumentList "/S", "/D=$installPath" -Wait -PassThru
    
    if ($process.ExitCode -eq 0) {
        Write-Host "✓ Installation complete" -ForegroundColor Green
    } else {
        Write-Host "✗ Installation failed with exit code: $($process.ExitCode)" -ForegroundColor Red
        exit 1
    }
} catch {
    Write-Host "✗ Installation error: $_" -ForegroundColor Red
    exit 1
}

# Verify installation
if (Test-Path "$installPath\tesseract.exe") {
    Write-Host "✓ Tesseract.exe found at: $installPath\tesseract.exe" -ForegroundColor Green
} else {
    Write-Host "✗ Installation verification failed - tesseract.exe not found" -ForegroundColor Red
    exit 1
}

# Add to PATH
:AddToPath
Write-Host "→ Adding Tesseract to system PATH..." -ForegroundColor Yellow

$currentPath = [Environment]::GetEnvironmentVariable("Path", "Machine")

if ($currentPath -like "*$installPath*") {
    Write-Host "✓ Already in PATH" -ForegroundColor Green
} else {
    try {
        $newPath = "$currentPath;$installPath"
        [Environment]::SetEnvironmentVariable("Path", $newPath, "Machine")
        Write-Host "✓ Added to system PATH" -ForegroundColor Green
        
        # Update current session
        $env:Path = [System.Environment]::GetEnvironmentVariable("Path","Machine") + ";" + [System.Environment]::GetEnvironmentVariable("Path","User")
        Write-Host "✓ Current PowerShell session updated" -ForegroundColor Green
    } catch {
        Write-Host "⚠ Failed to update PATH automatically: $_" -ForegroundColor Yellow
        Write-Host ""
        Write-Host "Please add manually:" -ForegroundColor Yellow
        Write-Host "  1. Search 'Environment Variables' in Start Menu" -ForegroundColor Gray
        Write-Host "  2. Edit 'Path' under System Variables" -ForegroundColor Gray
        Write-Host "  3. Add: $installPath" -ForegroundColor Gray
    }
}

# Cleanup
Remove-Item $installerPath -ErrorAction SilentlyContinue

# Test installation
Write-Host ""
Write-Host "=== Testing Tesseract ===" -ForegroundColor Cyan
try {
    & "$installPath\tesseract.exe" --version
    Write-Host ""
    Write-Host "✓ SUCCESS! Tesseract is installed and working." -ForegroundColor Green
    Write-Host ""
    Write-Host "Note: If 'tesseract' command doesn't work in NEW PowerShell windows," -ForegroundColor Yellow
    Write-Host "      restart your computer to fully apply PATH changes." -ForegroundColor Yellow
} catch {
    Write-Host "✗ Tesseract installed but not responding correctly" -ForegroundColor Red
    Write-Host "  Try running: & 'C:\Program Files\Tesseract-OCR\tesseract.exe' --version" -ForegroundColor Gray
}

Write-Host ""
Write-Host "Installation complete!" -ForegroundColor Green
