# IronBase MCP Server Windows Installer
# Run as Administrator: powershell -ExecutionPolicy Bypass -File install.ps1
#
# Usage:
#   .\install.ps1              # Install service
#   .\install.ps1 -Uninstall   # Uninstall service

param(
    [switch]$Uninstall
)

$ErrorActionPreference = "Stop"

# Configuration
$ServiceName = "IronBase"
$DisplayName = "IronBase MCP Server"
$Description = "IronBase Document Database MCP Server - provides document storage via Model Context Protocol"
$InstallDir = "$env:ProgramFiles\IronBase"
$DataDir = "$env:ProgramData\IronBase"
$ExeName = "mcp-ironbase-server.exe"

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

function Install-IronBase {
    Write-Host "=== IronBase MCP Server Installation ===" -ForegroundColor Cyan
    Write-Host ""

    # Check if service already exists
    $existingService = Get-Service -Name $ServiceName -ErrorAction SilentlyContinue
    if ($existingService) {
        Write-Host "Service '$ServiceName' already exists." -ForegroundColor Yellow
        Write-Host "Use -Uninstall to remove it first, or stop the service and update the executable." -ForegroundColor Yellow
        exit 1
    }

    # Find the executable - check multiple locations
    $ScriptDir = Split-Path -Parent $MyInvocation.PSCommandPath
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
        Write-Host "ERROR: Cannot find $ExeName" -ForegroundColor Red
        Write-Host "Searched locations:" -ForegroundColor Yellow
        foreach ($path in $SearchPaths) {
            Write-Host "  - $path" -ForegroundColor Yellow
        }
        Write-Host ""
        Write-Host "Please place $ExeName in the same folder as this script." -ForegroundColor Yellow
        exit 1
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

    # Create default config
    $ConfigPath = "$DataDir\config.toml"
    if (-not (Test-Path $ConfigPath)) {
        Write-Host "Creating default configuration..." -ForegroundColor White
        @"
# IronBase MCP Server Configuration

[server]
host = "0.0.0.0"
port = 8080

[database]
path = "$($DataDir.Replace('\', '\\'))\\data.mlite"

[logging]
level = "info"
"@ | Set-Content -Path $ConfigPath
    }

    # Set environment variables for the service
    $ExePath = "$InstallDir\$ExeName"

    # Create the Windows service
    Write-Host "Creating Windows service..." -ForegroundColor White
    $binPath = "`"$ExePath`""

    sc.exe create $ServiceName binPath= $binPath start= auto DisplayName= $DisplayName
    sc.exe description $ServiceName $Description
    sc.exe failure $ServiceName reset= 86400 actions= restart/5000/restart/10000/restart/30000

    # Set service environment
    $regPath = "HKLM:\SYSTEM\CurrentControlSet\Services\$ServiceName"
    $envValue = @(
        "IRONBASE_PATH=$DataDir\data.mlite",
        "MCP_CONFIG=$ConfigPath"
    )
    Set-ItemProperty -Path $regPath -Name "Environment" -Value $envValue -Type MultiString

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
    Write-Host "  View logs:       Get-EventLog -LogName Application -Source $ServiceName" -ForegroundColor White
    Write-Host ""
    Write-Host "Or use the built-in commands:" -ForegroundColor Cyan
    Write-Host "  $ExePath start" -ForegroundColor White
    Write-Host "  $ExePath stop" -ForegroundColor White
    Write-Host "  $ExePath status" -ForegroundColor White
}

function Uninstall-IronBase {
    Write-Host "=== IronBase MCP Server Uninstallation ===" -ForegroundColor Cyan
    Write-Host ""

    # Stop the service if running
    $service = Get-Service -Name $ServiceName -ErrorAction SilentlyContinue
    if ($service) {
        if ($service.Status -eq 'Running') {
            Write-Host "Stopping service..." -ForegroundColor White
            Stop-Service -Name $ServiceName -Force
            Start-Sleep -Seconds 2
        }

        # Delete the service
        Write-Host "Removing service..." -ForegroundColor White
        sc.exe delete $ServiceName
    } else {
        Write-Host "Service '$ServiceName' not found." -ForegroundColor Yellow
    }

    # Remove executable (but keep data)
    if (Test-Path "$InstallDir\$ExeName") {
        Write-Host "Removing executable..." -ForegroundColor White
        Remove-Item -Path "$InstallDir\$ExeName" -Force
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
