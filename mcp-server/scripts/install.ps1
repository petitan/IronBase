# IronBase MCP Server Windows Installer
# Supports both Admin and non-Admin installation
#
# Usage:
#   irm https://github.com/petitan/IronBase/releases/latest/download/install.ps1 | iex
#   .\install.ps1              # Install (auto-downloads if needed)
#   .\install.ps1 -Update      # Update running service to latest version
#   .\install.ps1 -Uninstall   # Uninstall
#   .\install.ps1 -NoDownload  # Install without auto-download
#
# One-liner update (via environment variable):
#   $env:IRONBASE_ACTION='update'; irm https://github.com/petitan/IronBase/releases/latest/download/install.ps1 | iex
#
# Auto-update: If already installed, running without parameters will update automatically.

param(
    [switch]$Update,
    [switch]$Uninstall,
    [switch]$NoDownload
)

# Support environment variable for iex one-liners
if ($env:IRONBASE_ACTION) {
    switch ($env:IRONBASE_ACTION.ToLower()) {
        "update" { $Update = $true }
        "uninstall" { $Uninstall = $true }
    }
    # Clear to avoid affecting subsequent runs
    $env:IRONBASE_ACTION = $null
}

$ErrorActionPreference = "Stop"

# Configuration
$ServiceName = "IronBaseService"
$ExeName = "mcp-ironbase-server.exe"

# GitHub Release Configuration
$GitHubRepo = "petitan/IronBase"
$GitHubApiUrl = "https://api.github.com/repos/$GitHubRepo/releases/latest"

# Check for admin privileges
function Test-Administrator {
    try {
        $currentUser = [Security.Principal.WindowsIdentity]::GetCurrent()
        $principal = New-Object Security.Principal.WindowsPrincipal($currentUser)
        return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    } catch {
        return $false
    }
}

$IsAdmin = Test-Administrator

# Set paths based on admin status
if ($IsAdmin) {
    $InstallDir = "$env:ProgramFiles\IronBase"
    $DataDir = "$env:ProgramData\IronBase"
} else {
    $InstallDir = "$env:LOCALAPPDATA\IronBase"
    $DataDir = "$env:LOCALAPPDATA\IronBase\data"
}

# Download latest release from GitHub
function Get-LatestRelease {
    param([string]$DestPath)

    Write-Host "Checking for latest release from GitHub..." -ForegroundColor Cyan

    try {
        $releaseInfo = Invoke-RestMethod -Uri $GitHubApiUrl -Headers @{
            "Accept" = "application/vnd.github.v3+json"
            "User-Agent" = "IronBase-Installer"
        }

        $version = $releaseInfo.tag_name
        Write-Host "Latest version: $version" -ForegroundColor Green

        # Find the MCP server Windows exe asset (not backup)
        $asset = $releaseInfo.assets | Where-Object {
            $_.name -eq "mcp-ironbase-server-windows.exe"
        } | Select-Object -First 1

        if (-not $asset) {
            # Fallback: search for mcp-server in name
            $asset = $releaseInfo.assets | Where-Object {
                $_.name -like "*mcp*server*windows*.exe"
            } | Select-Object -First 1
        }

        if (-not $asset) {
            Write-Host "No Windows executable found in release." -ForegroundColor Yellow
            return $false
        }

        Write-Host "Downloading $($asset.name) ($([math]::Round($asset.size / 1MB, 2)) MB)..." -ForegroundColor White

        Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $DestPath -UseBasicParsing

        if (Test-Path $DestPath) {
            Write-Host "Downloaded successfully!" -ForegroundColor Green
            return $true
        }
    }
    catch {
        Write-Host "Failed to download: $_" -ForegroundColor Red
    }

    return $false
}

function Install-IronBase {
    Write-Host ""
    Write-Host "========================================" -ForegroundColor Cyan
    Write-Host "  IronBase MCP Server Installation" -ForegroundColor Cyan
    Write-Host "========================================" -ForegroundColor Cyan
    Write-Host ""

    if ($IsAdmin) {
        Write-Host "Mode: Administrator (with Windows Service)" -ForegroundColor Green
    } else {
        Write-Host "Mode: User (without Windows Service)" -ForegroundColor Yellow
    }
    Write-Host ""

    # Find the executable
    $ScriptDir = if ($MyInvocation.PSCommandPath) {
        Split-Path -Parent $MyInvocation.PSCommandPath
    } else {
        Get-Location
    }

    $SearchPaths = @(
        (Join-Path $ScriptDir $ExeName),
        (Join-Path (Get-Location) $ExeName),
        (Join-Path $ScriptDir "..\target\release\$ExeName"),
        ".\target\release\$ExeName"
    )

    $SourceExe = $null
    foreach ($path in $SearchPaths) {
        if (Test-Path $path) {
            $SourceExe = (Resolve-Path $path).Path
            break
        }
    }

    if (-not $SourceExe) {
        if ($NoDownload) {
            Write-Host "ERROR: Cannot find $ExeName" -ForegroundColor Red
            exit 1
        }

        Write-Host "Downloading from GitHub..." -ForegroundColor Yellow

        $TempDir = Join-Path $env:TEMP "IronBase-Install"
        New-Item -ItemType Directory -Force -Path $TempDir | Out-Null
        $TempExe = Join-Path $TempDir $ExeName

        if (Get-LatestRelease -DestPath $TempExe) {
            $SourceExe = $TempExe
        }
        else {
            Write-Host "ERROR: Could not download $ExeName" -ForegroundColor Red
            Write-Host "Download manually: https://github.com/$GitHubRepo/releases" -ForegroundColor Yellow
            exit 1
        }
    }

    # Create directories
    Write-Host "Creating directories..." -ForegroundColor White
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    New-Item -ItemType Directory -Force -Path $DataDir | Out-Null

    # Copy executable
    $ExePath = "$InstallDir\$ExeName"
    Write-Host "Installing to: $InstallDir" -ForegroundColor White
    Copy-Item -Path $SourceExe -Destination $ExePath -Force

    # Create config
    $ConfigPath = "$DataDir\config.toml"
    if (-not (Test-Path $ConfigPath)) {
        Write-Host "Creating configuration..." -ForegroundColor White
        $DataDirForward = $DataDir -replace '\\', '/'
        @"
# IronBase MCP Server Configuration

[server]
host = "0.0.0.0"
port = 8080
# Max request body size (supports: B, KB, MB, GB)
# Default: 1GB - suitable for batch operations with large attachments
max_body_size = "1GB"

[database]
path = "$DataDirForward/data.mlite"

[logging]
level = "info"
"@ | Set-Content -Path $ConfigPath -Encoding UTF8
    }

    # Install service only if admin
    $ServiceInstalled = $false
    if ($IsAdmin) {
        Write-Host "Installing Windows service..." -ForegroundColor White
        $env:IRONBASE_PATH = "$DataDir\data.mlite"
        $env:MCP_CONFIG = $ConfigPath

        & $ExePath install 2>$null
        if ($LASTEXITCODE -eq 0) {
            $ServiceInstalled = $true
        }
    }

    # Output
    Write-Host ""
    Write-Host "========================================" -ForegroundColor Green
    Write-Host "  Installation Complete!" -ForegroundColor Green
    Write-Host "========================================" -ForegroundColor Green
    Write-Host ""
    Write-Host "Install Dir:  $InstallDir" -ForegroundColor White
    Write-Host "Data Dir:     $DataDir" -ForegroundColor White
    Write-Host "Config:       $ConfigPath" -ForegroundColor White
    Write-Host ""

    if ($ServiceInstalled) {
        Write-Host "Windows Service: INSTALLED" -ForegroundColor Green
        Write-Host ""
        Write-Host "Start service:  sc start $ServiceName" -ForegroundColor Cyan
        Write-Host "Stop service:   sc stop $ServiceName" -ForegroundColor Cyan
        Write-Host ""
    } else {
        Write-Host "Windows Service: NOT INSTALLED" -ForegroundColor Yellow
        if (-not $IsAdmin) {
            Write-Host "(Run as Administrator to install service)" -ForegroundColor DarkGray
        }
        Write-Host ""
    }

    Write-Host "Run manually (HTTP mode):" -ForegroundColor Cyan
    Write-Host "  $ExePath" -ForegroundColor White
    Write-Host ""
    Write-Host "For Claude Desktop (stdio mode):" -ForegroundColor Cyan
    Write-Host "  $ExePath --stdio" -ForegroundColor White
    Write-Host ""

    # Claude Desktop config hint
    Write-Host "Claude Desktop config (~\AppData\Roaming\Claude\claude_desktop_config.json):" -ForegroundColor DarkGray
    $EscapedPath = $ExePath -replace '\\', '\\\\'
    Write-Host @"
{
  "mcpServers": {
    "ironbase": {
      "command": "$EscapedPath",
      "args": ["--stdio"]
    }
  }
}
"@ -ForegroundColor DarkGray
}

function Update-IronBase {
    Write-Host ""
    Write-Host "========================================" -ForegroundColor Cyan
    Write-Host "  IronBase MCP Server Update" -ForegroundColor Cyan
    Write-Host "========================================" -ForegroundColor Cyan
    Write-Host ""

    if (-not $IsAdmin) {
        Write-Host "ERROR: Update requires Administrator privileges" -ForegroundColor Red
        Write-Host "Run PowerShell as Administrator" -ForegroundColor Yellow
        exit 1
    }

    $ExePath = "$InstallDir\$ExeName"

    # Check if installed
    if (-not (Test-Path $ExePath)) {
        Write-Host "IronBase not installed at $InstallDir" -ForegroundColor Yellow
        Write-Host "Running full installation instead..." -ForegroundColor White
        Install-IronBase
        return
    }

    # Get current version
    $CurrentVersion = "unknown"
    try {
        $CurrentVersion = & $ExePath --version 2>$null
        if (-not $CurrentVersion) { $CurrentVersion = "unknown" }
    } catch {}
    Write-Host "Current version: $CurrentVersion" -ForegroundColor White

    # Check if service is running and stop it
    $ServiceRunning = $false
    $Service = Get-Service -Name $ServiceName -ErrorAction SilentlyContinue
    if ($Service -and $Service.Status -eq "Running") {
        $ServiceRunning = $true
        Write-Host "Stopping service..." -ForegroundColor Yellow
        Stop-Service -Name $ServiceName -Force

        # Wait for service to fully stop (max 30 seconds)
        $timeout = 30
        $waited = 0
        while ($waited -lt $timeout) {
            Start-Sleep -Seconds 1
            $waited++
            $Service = Get-Service -Name $ServiceName -ErrorAction SilentlyContinue
            if ($Service.Status -eq "Stopped") {
                Write-Host "Service stopped." -ForegroundColor Green
                break
            }
            if ($waited % 5 -eq 0) {
                Write-Host "  Waiting for service to stop... ($waited s)" -ForegroundColor DarkGray
            }
        }

        if ($Service.Status -ne "Stopped") {
            Write-Host "WARNING: Service did not stop gracefully. Killing process..." -ForegroundColor Yellow
            Get-Process -Name "mcp-ironbase-server" -ErrorAction SilentlyContinue | Stop-Process -Force
            Start-Sleep -Seconds 2
        }
    }

    # Download latest
    Write-Host ""
    $TempDir = Join-Path $env:TEMP "IronBase-Update"
    New-Item -ItemType Directory -Force -Path $TempDir | Out-Null
    $TempExe = Join-Path $TempDir $ExeName

    if (-not (Get-LatestRelease -DestPath $TempExe)) {
        Write-Host "ERROR: Failed to download latest version" -ForegroundColor Red
        if ($ServiceRunning) {
            Write-Host "Restarting service with old version..." -ForegroundColor Yellow
            Start-Service -Name $ServiceName
        }
        exit 1
    }

    # Backup old exe
    $BackupPath = "$ExePath.backup"
    Write-Host "Backing up current version..." -ForegroundColor White
    Copy-Item -Path $ExePath -Destination $BackupPath -Force

    # Copy new exe
    Write-Host "Installing new version..." -ForegroundColor White
    Copy-Item -Path $TempExe -Destination $ExePath -Force

    # Get new version
    $NewVersion = "unknown"
    try {
        $NewVersion = & $ExePath --version 2>$null
        if (-not $NewVersion) { $NewVersion = "unknown" }
    } catch {}

    # Restart service if it was running
    if ($ServiceRunning) {
        Write-Host "Starting service..." -ForegroundColor Green
        Start-Service -Name $ServiceName
        Start-Sleep -Seconds 1
        $Service = Get-Service -Name $ServiceName
        if ($Service.Status -eq "Running") {
            Write-Host "Service started successfully!" -ForegroundColor Green
        } else {
            Write-Host "WARNING: Service failed to start!" -ForegroundColor Red
            Write-Host "Restoring backup..." -ForegroundColor Yellow
            Copy-Item -Path $BackupPath -Destination $ExePath -Force
            Start-Service -Name $ServiceName
        }
    }

    # Cleanup
    Remove-Item -Path $TempDir -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item -Path $BackupPath -Force -ErrorAction SilentlyContinue

    Write-Host ""
    Write-Host "========================================" -ForegroundColor Green
    Write-Host "  Update Complete!" -ForegroundColor Green
    Write-Host "========================================" -ForegroundColor Green
    Write-Host ""
    Write-Host "Previous: $CurrentVersion" -ForegroundColor White
    Write-Host "Current:  $NewVersion" -ForegroundColor Green
    Write-Host ""
}

function Uninstall-IronBase {
    Write-Host ""
    Write-Host "========================================" -ForegroundColor Cyan
    Write-Host "  IronBase MCP Server Uninstallation" -ForegroundColor Cyan
    Write-Host "========================================" -ForegroundColor Cyan
    Write-Host ""

    $ExePath = "$InstallDir\$ExeName"

    if ($IsAdmin -and (Test-Path $ExePath)) {
        Write-Host "Removing Windows service..." -ForegroundColor White
        & $ExePath uninstall 2>$null
    }

    if (Test-Path $ExePath) {
        Write-Host "Removing executable..." -ForegroundColor White
        Remove-Item -Path $ExePath -Force
    }

    if ((Test-Path $InstallDir) -and ((Get-ChildItem $InstallDir -ErrorAction SilentlyContinue | Measure-Object).Count -eq 0)) {
        Remove-Item -Path $InstallDir -Force -ErrorAction SilentlyContinue
    }

    Write-Host ""
    Write-Host "Uninstallation complete!" -ForegroundColor Green
    Write-Host ""
    Write-Host "Data preserved at: $DataDir" -ForegroundColor Yellow
    Write-Host "To remove all data: Remove-Item -Recurse '$DataDir'" -ForegroundColor DarkGray
}

# Main
if ($Update) {
    Update-IronBase
} elseif ($Uninstall) {
    Uninstall-IronBase
} else {
    # Auto-detect: if already installed, switch to update mode
    $ExistingExe = "$InstallDir\$ExeName"
    if (Test-Path $ExistingExe) {
        Write-Host ""
        Write-Host "IronBase is already installed at: $InstallDir" -ForegroundColor Cyan
        Write-Host "Switching to update mode..." -ForegroundColor Cyan
        Update-IronBase
    } else {
        Install-IronBase
    }
}
