# MyKAI Node — One-click build script for Windows
# Right-click this file → "Run with PowerShell"
# Or open PowerShell and run: .\build.ps1

$ErrorActionPreference = "Stop"
Write-Host "`n=== MyKAI Node Builder ===" -ForegroundColor Cyan

# 1. Check/install Rust
if (-not (Get-Command rustc -ErrorAction SilentlyContinue)) {
    Write-Host "`nInstalling Rust..." -ForegroundColor Yellow
    Invoke-WebRequest -Uri "https://win.rustup.rs/x86_64" -OutFile "$env:TEMP\rustup-init.exe"
    & "$env:TEMP\rustup-init.exe" -y --default-toolchain stable
    $env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
    Write-Host "Rust installed." -ForegroundColor Green
} else {
    Write-Host "Rust: OK" -ForegroundColor Green
}

# 2. Check/install Node.js
if (-not (Get-Command node -ErrorAction SilentlyContinue)) {
    Write-Host "`nNode.js is required. Please install from https://nodejs.org/" -ForegroundColor Red
    Write-Host "Then re-run this script." -ForegroundColor Red
    pause
    exit 1
} else {
    Write-Host "Node.js: OK ($(node --version))" -ForegroundColor Green
}

# 3. Install Tauri CLI
Write-Host "`nInstalling Tauri CLI..." -ForegroundColor Yellow
cargo install tauri-cli --version "^2" 2>&1 | Out-Null
Write-Host "Tauri CLI: OK" -ForegroundColor Green

# 4. Install npm dependencies
Write-Host "`nInstalling npm dependencies..." -ForegroundColor Yellow
npm install 2>&1 | Out-Null
Write-Host "npm: OK" -ForegroundColor Green

# 5. Generate placeholder icons (if missing)
$iconDir = "src-tauri\icons"
if (-not (Test-Path "$iconDir\icon.png")) {
    Write-Host "`nGenerating placeholder icons..." -ForegroundColor Yellow
    if (-not (Test-Path $iconDir)) { New-Item -ItemType Directory -Path $iconDir | Out-Null }

    # Create a minimal 32x32 PNG (1x1 teal pixel scaled — placeholder)
    # You should replace these with proper MyKAI branding later
    Add-Type -AssemblyName System.Drawing
    foreach ($size in @(32, 128, 256)) {
        $bmp = New-Object System.Drawing.Bitmap($size, $size)
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        $g.Clear([System.Drawing.Color]::FromArgb(73, 234, 203))

        # Draw a simple "K" letter
        $font = New-Object System.Drawing.Font("Segoe UI", [math]::Floor($size * 0.5), [System.Drawing.FontStyle]::Bold)
        $brush = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(13, 17, 23))
        $sf = New-Object System.Drawing.StringFormat
        $sf.Alignment = [System.Drawing.StringAlignment]::Center
        $sf.LineAlignment = [System.Drawing.StringAlignment]::Center
        $rect = New-Object System.Drawing.RectangleF(0, 0, $size, $size)
        $g.DrawString("K", $font, $brush, $rect, $sf)
        $g.Dispose()

        if ($size -eq 32) { $bmp.Save("$iconDir\32x32.png", [System.Drawing.Imaging.ImageFormat]::Png) }
        if ($size -eq 128) {
            $bmp.Save("$iconDir\128x128.png", [System.Drawing.Imaging.ImageFormat]::Png)
            $bmp.Save("$iconDir\icon.png", [System.Drawing.Imaging.ImageFormat]::Png)
        }
        if ($size -eq 256) { $bmp.Save("$iconDir\128x128@2x.png", [System.Drawing.Imaging.ImageFormat]::Png) }

        # Also create .ico for Windows
        if ($size -eq 256) {
            $icon = [System.Drawing.Icon]::FromHandle($bmp.GetHicon())
            $fs = New-Object System.IO.FileStream("$iconDir\icon.ico", [System.IO.FileMode]::Create)
            $icon.Save($fs)
            $fs.Close()
        }

        $bmp.Dispose()
    }
    Write-Host "Icons: OK (placeholder — replace with MyKAI branding later)" -ForegroundColor Green
}

# 6. Build!
Write-Host "`n=== Building MyKAI Node ===" -ForegroundColor Cyan
Write-Host "This will take a few minutes on first build...`n" -ForegroundColor Yellow
cargo tauri build

# 7. Done
$installer = Get-ChildItem -Path "src-tauri\target\release\bundle\nsis\*.exe" -ErrorAction SilentlyContinue | Select-Object -First 1
if ($installer) {
    Write-Host "`n=== BUILD COMPLETE ===" -ForegroundColor Green
    Write-Host "Installer: $($installer.FullName)" -ForegroundColor Cyan
    Write-Host "Size: $([math]::Round($installer.Length / 1MB, 1)) MB" -ForegroundColor Cyan
    Write-Host "`nThis .exe is what you distribute to users." -ForegroundColor White

    # Open the folder
    explorer.exe $installer.DirectoryName
} else {
    Write-Host "`nBuild finished but installer not found. Check output above for errors." -ForegroundColor Red
}

pause
