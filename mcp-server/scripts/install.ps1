# IronBase MCP Server Windows Installer
# Run as Administrator: powershell -ExecutionPolicy Bypass -File install.ps1
#
# Usage:
#   .\install.ps1              # Install service (auto-downloads if needed)
#   .\install.ps1 -Uninstall   # Uninstall service
#   .\install.ps1 -NoDownload  # Install without auto-download

param(
    [switch]$Uninstall,
    [switch]$NoDownload
)

$ErrorActionPreference = "Stop"

# Configuration
$ServiceName = "IronBaseService"
$DisplayName = "IronBase MCP Server"
$InstallDir = "$env:ProgramFiles\IronBase"
$DataDir = "$env:ProgramData\IronBase"
$ExeName = "mcp-ironbase-server.exe"

# GitHub Release Configuration
$GitHubRepo = "petitan/IronBase"
$GitHubApiUrl = "https://api.github.com/repos/$GitHubRepo/releases/latest"
$ArtifactName = "mcp-ironbase-server-windows"

# Check for admin privileges
function Test-Administrator {
    $currentUser = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($currentUser)
    return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

if (-not (Test-Administrator)) {
    Write-Host "ERROR: This script requires Administrator privileges." -ForegroundColor Red
    Write-Host "Please run PowerShell as Administrator and try again." -ForegroundColor Yellow
    exit 1
}

# Download latest release from GitHub
function Get-LatestRelease {
    param([string]$DestPath)

    Write-Host "Checking for latest release from GitHub..." -ForegroundColor Cyan

    try {
        # Get latest release info
        $releaseInfo = Invoke-RestMethod -Uri $GitHubApiUrl -Headers @{
            "Accept" = "application/vnd.github.v3+json"
            "User-Agent" = "IronBase-Installer"
        }

        $version = $releaseInfo.tag_name
        Write-Host "Latest version: $version" -ForegroundColor Green

        # Find the Windows exe asset
        $asset = $releaseInfo.assets | Where-Object {
            $_.name -like "*windows*.exe" -or $_.name -eq $ExeName
        } | Select-Object -First 1

        if (-not $asset) {
            # Try to find any .exe asset
            $asset = $releaseInfo.assets | Where-Object { $_.name -like "*.exe" } | Select-Object -First 1
        }

        if (-not $asset) {
            Write-Host "No Windows executable found in release assets." -ForegroundColor Yellow
            Write-Host "Available assets:" -ForegroundColor Yellow
            $releaseInfo.assets | ForEach-Object { Write-Host "  - $($_.name)" -ForegroundColor Yellow }
            return $false
        }

        Write-Host "Downloading $($asset.name) ($([math]::Round($asset.size / 1MB, 2)) MB)..." -ForegroundColor White

        # Download the asset
        $downloadUrl = $asset.browser_download_url
        Invoke-WebRequest -Uri $downloadUrl -OutFile $DestPath -UseBasicParsing

        if (Test-Path $DestPath) {
            Write-Host "Downloaded successfully to $DestPath" -ForegroundColor Green
            return $true
        }
    }
    catch {
        Write-Host "Failed to download from GitHub: $_" -ForegroundColor Red
    }

    return $false
}

function Install-IronBase {
    Write-Host "=== IronBase MCP Server Installation ===" -ForegroundColor Cyan
    Write-Host ""

    # Find the executable - check multiple locations
    $ScriptDir = if ($MyInvocation.PSCommandPath) {
        Split-Path -Parent $MyInvocation.PSCommandPath
    } else {
        Get-Location
    }
    $SearchPaths = @(
        (Join-Path $ScriptDir $ExeName),                           # Same folder as script
        (Join-Path (Get-Location) $ExeName),                       # Current directory
        (Join-Path $ScriptDir "..\target\release\$ExeName"),       # Dev build path
        ".\target\release\$ExeName"                                # Alt dev path
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
            Write-Host "Searched locations:" -ForegroundColor Yellow
            foreach ($path in $SearchPaths) {
                Write-Host "  - $path" -ForegroundColor Yellow
            }
            Write-Host ""
            Write-Host "Please place $ExeName in the same folder as this script." -ForegroundColor Yellow
            Write-Host "Or run without -NoDownload to auto-download from GitHub." -ForegroundColor Yellow
            exit 1
        }

        # Try to download from GitHub
        Write-Host "Executable not found locally. Attempting to download from GitHub..." -ForegroundColor Yellow
        Write-Host ""

        # Create temp directory for download
        $TempDir = Join-Path $env:TEMP "IronBase-Install"
        New-Item -ItemType Directory -Force -Path $TempDir | Out-Null
        $TempExe = Join-Path $TempDir $ExeName

        if (Get-LatestRelease -DestPath $TempExe) {
            $SourceExe = $TempExe
        }
        else {
            Write-Host ""
            Write-Host "ERROR: Could not download or find $ExeName" -ForegroundColor Red
            Write-Host ""
            Write-Host "Options:" -ForegroundColor Yellow
            Write-Host "  1. Download manually from: https://github.com/$GitHubRepo/releases" -ForegroundColor White
            Write-Host "  2. Build from source: cargo build --release" -ForegroundColor White
            Write-Host "  3. Place $ExeName in the same folder as this script" -ForegroundColor White
            exit 1
        }
    }

    Write-Host "Source executable: $SourceExe" -ForegroundColor Green

    # Create directories
    Write-Host "Creating directories..." -ForegroundColor White
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    New-Item -ItemType Directory -Force -Path $DataDir | Out-Null
    New-Item -ItemType Directory -Force -Path "$DataDir\logs" | Out-Null

    # Copy executable
    Write-Host "Copying executable to $InstallDir..." -ForegroundColor White
    Copy-Item -Path $SourceExe -Destination "$InstallDir\$ExeName" -Force

    # Create default config (use forward slashes for TOML compatibility)
    $ConfigPath = "$DataDir\config.toml"
    if (-not (Test-Path $ConfigPath)) {
        Write-Host "Creating default configuration..." -ForegroundColor White
        $DataDirForward = $DataDir -replace '\\', '/'
        @"
# IronBase MCP Server Configuration

[server]
host = "0.0.0.0"
port = 8080

[database]
path = "$DataDirForward/data.mlite"

[logging]
level = "info"
"@ | Set-Content -Path $ConfigPath
    }

    # Set environment variables for the service
    $ExePath = "$InstallDir\$ExeName"

    # Install the service using the binary's install command
    Write-Host "Installing Windows service..." -ForegroundColor White

    # Set environment variables before install
    $env:IRONBASE_PATH = "$DataDir\data.mlite"
    $env:MCP_CONFIG = $ConfigPath

    & $ExePath install

    if ($LASTEXITCODE -ne 0) {
        Write-Host "Service installation failed. You can still run the server manually." -ForegroundColor Yellow
    }

    Write-Host ""
    Write-Host "=== Installation Complete ===" -ForegroundColor Green
    Write-Host ""
    Write-Host "Service Name:    $ServiceName" -ForegroundColor White
    Write-Host "Install Dir:     $InstallDir" -ForegroundColor White
    Write-Host "Data Dir:        $DataDir" -ForegroundColor White
    Write-Host "Config File:     $ConfigPath" -ForegroundColor White
    Write-Host ""
    Write-Host "Commands:" -ForegroundColor Cyan
    Write-Host "  Start service:   sc start $ServiceName" -ForegroundColor White
    Write-Host "  Stop service:    sc stop $ServiceName" -ForegroundColor White
    Write-Host "  Check status:    sc query $ServiceName" -ForegroundColor White
    Write-Host ""
    Write-Host "Or run manually:" -ForegroundColor Cyan
    Write-Host "  $ExePath" -ForegroundColor White
    Write-Host ""
    Write-Host "For Claude Desktop (stdio mode):" -ForegroundColor Cyan
    Write-Host "  $ExePath --stdio" -ForegroundColor White
}

function Uninstall-IronBase {
    Write-Host "=== IronBase MCP Server Uninstallation ===" -ForegroundColor Cyan
    Write-Host ""

    $ExePath = "$InstallDir\$ExeName"

    # Uninstall the service using the binary's uninstall command
    if (Test-Path $ExePath) {
        Write-Host "Uninstalling service..." -ForegroundColor White
        & $ExePath uninstall
    }

    # Remove executable (but keep data)
    if (Test-Path $ExePath) {
        Write-Host "Removing executable..." -ForegroundColor White
        Remove-Item -Path $ExePath -Force
    }

    # Remove install directory if empty
    if ((Test-Path $InstallDir) -and ((Get-ChildItem $InstallDir | Measure-Object).Count -eq 0)) {
        Remove-Item -Path $InstallDir -Force
    }

    Write-Host ""
    Write-Host "=== Uninstallation Complete ===" -ForegroundColor Green
    Write-Host ""
    Write-Host "Note: Configuration and data files were preserved:" -ForegroundColor Yellow
    Write-Host "  Config: $DataDir\config.toml" -ForegroundColor White
    Write-Host "  Data:   $DataDir\data.mlite" -ForegroundColor White
    Write-Host ""
    Write-Host "To completely remove all data, delete: $DataDir" -ForegroundColor Yellow
}

# Main
if ($Uninstall) {
    Uninstall-IronBase
} else {
    Install-IronBase
}
