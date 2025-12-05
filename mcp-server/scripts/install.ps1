# IronBase MCP Server Windows Installer
# Supports both Admin and non-Admin installation
#
# Usage:
#   irm https://github.com/petitan/IronBase/releases/latest/download/install.ps1 | iex
#   .\install.ps1              # Install (auto-downloads if needed)
#   .\install.ps1 -Uninstall   # Uninstall
#   .\install.ps1 -NoDownload  # Install without auto-download

param(
    [switch]$Uninstall,
    [switch]$NoDownload
)

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

        # Find the Windows exe asset
        $asset = $releaseInfo.assets | Where-Object {
            $_.name -like "*windows*.exe" -or $_.name -eq $ExeName
        } | Select-Object -First 1

        if (-not $asset) {
            $asset = $releaseInfo.assets | Where-Object { $_.name -like "*.exe" } | Select-Object -First 1
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
if ($Uninstall) {
    Uninstall-IronBase
} else {
    Install-IronBase
}
