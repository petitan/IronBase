# IronBase MCP Server - Build from Source & Install as Windows Service
# Builds the MCP server from Rust source code and installs it as a Windows Service.
#
# Usage:
#   .\build-and-install.ps1 -SourceDir "C:\Users\X\MongoLite"
#   .\build-and-install.ps1 -GitRepo "https://github.com/petitan/IronBase.git"
#   .\build-and-install.ps1 -SourceDir "C:\src\MongoLite" -Port 9090 -AdminKey "secret"
#   .\build-and-install.ps1 -Uninstall
#   .\build-and-install.ps1 -SourceDir "C:\src\MongoLite" -SkipService
#   .\build-and-install.ps1 -SkipBuild -InstallDir "C:\Program Files\IronBase"

param(
    [string]$SourceDir,
    [string]$GitRepo = "https://github.com/petitan/IronBase.git",
    [string]$Branch = "master",
    [string]$InstallDir,
    [string]$DataDir,
    [int]$Port = 8080,
    [string]$Host = "0.0.0.0",
    [string]$AdminKey,
    [string]$RustTarget = "x86_64-pc-windows-msvc",
    [switch]$SkipBuild,
    [switch]$SkipService,
    [switch]$Uninstall
)

$ErrorActionPreference = "Stop"

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------
$ServiceName = "IronBaseService"
$ExeName = "mcp-ironbase-server.exe"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

function Write-Step {
    param([string]$Message)
    Write-Host ""
    Write-Host "--- $Message ---" -ForegroundColor Cyan
}

function Write-Ok {
    param([string]$Message)
    Write-Host "  [OK] $Message" -ForegroundColor Green
}

function Write-Info {
    param([string]$Message)
    Write-Host "  $Message" -ForegroundColor White
}

function Write-Warn {
    param([string]$Message)
    Write-Host "  [WARN] $Message" -ForegroundColor Yellow
}

function Write-Err {
    param([string]$Message)
    Write-Host "  [ERROR] $Message" -ForegroundColor Red
}

function Test-Administrator {
    try {
        $currentUser = [Security.Principal.WindowsIdentity]::GetCurrent()
        $principal = New-Object Security.Principal.WindowsPrincipal($currentUser)
        return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    } catch {
        return $false
    }
}

function Test-CommandExists {
    param([string]$Command)
    $null -ne (Get-Command $Command -ErrorAction SilentlyContinue)
}

function Stop-ExistingService {
    param([int]$Timeout = 30)

    $service = Get-Service -Name $ServiceName -ErrorAction SilentlyContinue
    if (-not $service) { return }

    if ($service.Status -eq "Running") {
        Write-Info "Stopping service '$ServiceName' gracefully..."
        Stop-Service -Name $ServiceName -ErrorAction SilentlyContinue

        $waited = 0
        while ($waited -lt $Timeout) {
            Start-Sleep -Seconds 1
            $waited++
            $service = Get-Service -Name $ServiceName -ErrorAction SilentlyContinue
            if ($service.Status -eq "Stopped") {
                Write-Ok "Service stopped after ${waited}s."
                return
            }
            if ($waited % 10 -eq 0) {
                Write-Info "  Still waiting... (${waited}s)"
            }
        }

        Write-Warn "Service did not stop within ${Timeout}s. Killing process..."
        Get-Process -Name "mcp-ironbase-server" -ErrorAction SilentlyContinue | Stop-Process -Force
        Start-Sleep -Seconds 2
    }
}

# ---------------------------------------------------------------------------
# 0. Admin check
# ---------------------------------------------------------------------------

function Assert-Administrator {
    if (-not (Test-Administrator)) {
        Write-Host ""
        Write-Err "This script must be run as Administrator."
        Write-Info "Right-click PowerShell -> 'Run as Administrator', then retry."
        exit 1
    }
}

# ---------------------------------------------------------------------------
# 1. Rust toolchain check
# ---------------------------------------------------------------------------

function Assert-RustToolchain {
    Write-Step "Checking Rust toolchain"

    # rustup
    if (-not (Test-CommandExists "rustup")) {
        Write-Err "rustup not found."
        Write-Info "Install Rust from https://rustup.rs"
        exit 1
    }
    $rustupVer = & rustup --version 2>&1 | Select-Object -First 1
    Write-Ok "rustup: $rustupVer"

    # rustc
    if (-not (Test-CommandExists "rustc")) {
        Write-Err "rustc not found. Run: rustup install stable"
        exit 1
    }
    $rustcVer = & rustc --version 2>&1 | Select-Object -First 1
    Write-Ok "rustc:  $rustcVer"

    # cargo
    if (-not (Test-CommandExists "cargo")) {
        Write-Err "cargo not found. Run: rustup install stable"
        exit 1
    }
    $cargoVer = & cargo --version 2>&1 | Select-Object -First 1
    Write-Ok "cargo:  $cargoVer"

    # target triple
    $installedTargets = & rustup target list --installed 2>&1
    if ($installedTargets -notcontains $RustTarget) {
        Write-Err "Rust target '$RustTarget' is not installed."
        Write-Info "Run: rustup target add $RustTarget"
        exit 1
    }
    Write-Ok "target: $RustTarget"
}

# ---------------------------------------------------------------------------
# 2. Visual Studio Build Tools check
# ---------------------------------------------------------------------------

function Assert-VsBuildTools {
    Write-Step "Checking Visual Studio Build Tools"

    $vswhereLocations = @(
        "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe",
        "${env:ProgramFiles}\Microsoft Visual Studio\Installer\vswhere.exe"
    )

    $vswhere = $null
    foreach ($loc in $vswhereLocations) {
        if (Test-Path $loc) {
            $vswhere = $loc
            break
        }
    }

    if ($vswhere) {
        $vsInstalls = & $vswhere -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property displayName 2>$null
        if ($vsInstalls) {
            Write-Ok "MSVC found: $($vsInstalls | Select-Object -First 1)"
            return
        }
    }

    # Fallback: check for cl.exe in PATH
    if (Test-CommandExists "cl") {
        Write-Ok "MSVC linker (cl.exe) found in PATH."
        return
    }

    # Fallback: check registry
    $regPath = "HKLM:\SOFTWARE\Microsoft\VisualStudio\SxS\VS7"
    if (Test-Path $regPath) {
        Write-Ok "Visual Studio detected via registry."
        return
    }

    Write-Err "Visual Studio Build Tools with C++ not found."
    Write-Host ""
    Write-Info "Install Visual Studio Build Tools:"
    Write-Info "  1. Download from https://visualstudio.microsoft.com/visual-cpp-build-tools/"
    Write-Info "  2. Select 'Desktop development with C++'"
    Write-Info "  3. Ensure 'MSVC v14x' and 'Windows SDK' are checked"
    Write-Info "  4. Restart your terminal after installation"
    exit 1
}

# ---------------------------------------------------------------------------
# 3. Source acquisition
# ---------------------------------------------------------------------------

function Get-SourceDirectory {
    Write-Step "Preparing source code"

    if ($script:SourceDir) {
        $resolved = Resolve-Path $script:SourceDir -ErrorAction SilentlyContinue
        if (-not $resolved) {
            Write-Err "Source directory not found: $($script:SourceDir)"
            exit 1
        }
        $srcPath = $resolved.Path

        # Verify it looks like the right repo
        $cargoToml = Join-Path $srcPath "mcp-server\Cargo.toml"
        if (-not (Test-Path $cargoToml)) {
            Write-Err "Directory does not appear to be the IronBase repo."
            Write-Info "Expected: $cargoToml"
            exit 1
        }

        Write-Ok "Using local source: $srcPath"
        return $srcPath
    }

    # Git clone / pull
    if (-not (Test-CommandExists "git")) {
        Write-Err "git not found and -SourceDir not specified."
        Write-Info "Either install git or pass -SourceDir pointing to the repo."
        exit 1
    }

    $cloneDir = Join-Path $env:TEMP "IronBase-Build"

    if (Test-Path (Join-Path $cloneDir ".git")) {
        Write-Info "Updating existing clone at $cloneDir ..."
        Push-Location $cloneDir
        try {
            & git fetch origin 2>&1 | Out-Null
            & git checkout $script:Branch 2>&1 | Out-Null
            & git pull origin $script:Branch 2>&1 | Out-Null
            Write-Ok "Source updated (branch: $($script:Branch))."
        } finally {
            Pop-Location
        }
    } else {
        Write-Info "Cloning $($script:GitRepo) (branch: $($script:Branch)) ..."
        & git clone --branch $script:Branch --single-branch $script:GitRepo $cloneDir 2>&1 | Out-Null
        if ($LASTEXITCODE -ne 0) {
            Write-Err "git clone failed."
            exit 1
        }
        Write-Ok "Repository cloned to $cloneDir"
    }

    return $cloneDir
}

# ---------------------------------------------------------------------------
# 4. Build
# ---------------------------------------------------------------------------

function Invoke-CargoBuild {
    param([string]$SrcPath)

    Write-Step "Building MCP server (release)"

    $mcsDir = Join-Path $SrcPath "mcp-server"
    if (-not (Test-Path $mcsDir)) {
        Write-Err "mcp-server directory not found at: $mcsDir"
        exit 1
    }

    Write-Info "Building in: $mcsDir"
    Write-Info "This may take several minutes on first build..."

    $sw = [System.Diagnostics.Stopwatch]::StartNew()

    Push-Location $SrcPath
    try {
        & cargo build --release -p mcp-ironbase-server 2>&1 | ForEach-Object {
            if ($_ -match "error") {
                Write-Host "  $_" -ForegroundColor Red
            } elseif ($_ -match "warning") {
                Write-Host "  $_" -ForegroundColor Yellow
            } else {
                Write-Host "  $_" -ForegroundColor DarkGray
            }
        }

        if ($LASTEXITCODE -ne 0) {
            Write-Err "cargo build failed with exit code $LASTEXITCODE"
            exit 1
        }
    } finally {
        Pop-Location
    }

    $sw.Stop()
    $elapsed = $sw.Elapsed

    # Verify binary exists
    $builtExe = Join-Path $SrcPath "target\release\$ExeName"
    if (-not (Test-Path $builtExe)) {
        Write-Err "Build succeeded but executable not found at: $builtExe"
        exit 1
    }

    $exeSize = (Get-Item $builtExe).Length / 1MB
    Write-Ok ("Build complete in {0:mm\:ss} ({1:F1} MB)" -f $elapsed, $exeSize)

    # Try to get version
    try {
        $version = & $builtExe --version 2>$null
        if ($version) {
            Write-Ok "Version: $version"
        }
    } catch {}

    return $builtExe
}

# ---------------------------------------------------------------------------
# 5. Existing service handling
# ---------------------------------------------------------------------------

function Remove-ExistingInstallation {
    param([string]$TargetInstallDir)

    Write-Step "Checking existing installation"

    $existingExe = Join-Path $TargetInstallDir $ExeName

    # Stop service if running
    Stop-ExistingService -Timeout 30

    # Uninstall service if registered
    $service = Get-Service -Name $ServiceName -ErrorAction SilentlyContinue
    if ($service) {
        Write-Info "Uninstalling existing service..."
        if (Test-Path $existingExe) {
            & $existingExe uninstall 2>$null
            Start-Sleep -Seconds 2
        }
        # Fallback: sc.exe delete
        $service = Get-Service -Name $ServiceName -ErrorAction SilentlyContinue
        if ($service) {
            & sc.exe delete $ServiceName 2>$null
            Start-Sleep -Seconds 1
        }
        Write-Ok "Old service removed."
    } else {
        Write-Info "No existing service found."
    }

    # Backup old exe
    if (Test-Path $existingExe) {
        $backupPath = "${existingExe}.backup"
        Write-Info "Backing up old executable to: $backupPath"
        Copy-Item -Path $existingExe -Destination $backupPath -Force
    }
}

# ---------------------------------------------------------------------------
# 6. Installation
# ---------------------------------------------------------------------------

function Install-IronBase {
    param(
        [string]$BuiltExePath,
        [string]$TargetInstallDir,
        [string]$TargetDataDir
    )

    Write-Step "Installing IronBase"

    # Create directories
    Write-Info "Creating directories..."
    New-Item -ItemType Directory -Force -Path $TargetInstallDir | Out-Null
    New-Item -ItemType Directory -Force -Path $TargetDataDir | Out-Null
    New-Item -ItemType Directory -Force -Path (Join-Path $TargetDataDir "logs") | Out-Null
    Write-Ok "Install: $TargetInstallDir"
    Write-Ok "Data:    $TargetDataDir"

    # Copy executable
    $destExe = Join-Path $TargetInstallDir $ExeName
    Write-Info "Copying executable..."
    Copy-Item -Path $BuiltExePath -Destination $destExe -Force
    Write-Ok "Installed: $destExe"

    # Generate config.toml (only if it does not already exist)
    $configPath = Join-Path $TargetDataDir "config.toml"
    if (-not (Test-Path $configPath)) {
        Write-Info "Generating config.toml..."
        $dataPathForward = ($TargetDataDir -replace '\\', '/') + "/data.mlite"

        $configContent = @"
# IronBase MCP Server Configuration
# Generated by build-and-install.ps1 on $(Get-Date -Format "yyyy-MM-dd HH:mm:ss")

[server]
host = "$($script:Host)"
port = $($script:Port)
# Max request body size (supports: B, KB, MB, GB)
max_body_size = "1GB"

[database]
path = "$dataPathForward"

[logging]
level = "info"
"@

        if ($script:AdminKey) {
            $configContent += @"

[auth]
admin_key = "$($script:AdminKey)"
"@
        }

        $configContent | Set-Content -Path $configPath -Encoding UTF8
        Write-Ok "Config: $configPath"
    } else {
        Write-Warn "Config already exists, not overwriting: $configPath"
    }

    # Install Windows Service
    if (-not $script:SkipService) {
        Write-Info "Installing Windows Service..."
        $env:IRONBASE_PATH = Join-Path $TargetDataDir "data.mlite"
        $env:MCP_CONFIG = $configPath

        & $destExe install 2>&1 | ForEach-Object {
            Write-Host "  $_" -ForegroundColor DarkGray
        }

        if ($LASTEXITCODE -eq 0) {
            Write-Ok "Service '$ServiceName' installed."
        } else {
            Write-Warn "Service installation returned exit code $LASTEXITCODE."
            Write-Info "You can install manually: $destExe install"
        }
    } else {
        Write-Info "Skipping service installation (-SkipService)."
    }

    # Firewall rule
    if ($script:Port -gt 0 -and -not $script:SkipService) {
        $ruleName = "IronBase MCP Server (TCP $($script:Port))"
        $existing = Get-NetFirewallRule -DisplayName $ruleName -ErrorAction SilentlyContinue
        if (-not $existing) {
            Write-Info "Adding firewall rule for port $($script:Port)..."
            try {
                New-NetFirewallRule `
                    -DisplayName $ruleName `
                    -Direction Inbound `
                    -Protocol TCP `
                    -LocalPort $script:Port `
                    -Action Allow `
                    -Profile Domain,Private `
                    -Description "Allow IronBase MCP Server HTTP traffic" | Out-Null
                Write-Ok "Firewall rule added (Domain, Private profiles)."
            } catch {
                Write-Warn "Could not add firewall rule: $_"
                Write-Info "Add manually: New-NetFirewallRule -DisplayName '$ruleName' -Direction Inbound -Protocol TCP -LocalPort $($script:Port) -Action Allow"
            }
        } else {
            Write-Info "Firewall rule already exists."
        }
    }

    return $destExe
}

# ---------------------------------------------------------------------------
# 7. Start & Health check
# ---------------------------------------------------------------------------

function Start-AndVerify {
    param(
        [string]$InstalledExe,
        [int]$HttpPort
    )

    if ($script:SkipService) {
        Write-Step "Service start skipped (-SkipService)"
        Write-Info "Run manually: $InstalledExe"
        return
    }

    Write-Step "Starting service and verifying"

    # Start service
    try {
        Start-Service -Name $ServiceName
    } catch {
        Write-Err "Failed to start service: $_"
        Write-Info "Check logs at: $($script:DataDir)\logs\"
        Write-Info "Try manually: sc.exe start $ServiceName"
        return
    }

    # Wait for Running status (max 30s)
    $waited = 0
    $maxWait = 30
    while ($waited -lt $maxWait) {
        Start-Sleep -Seconds 1
        $waited++
        $svc = Get-Service -Name $ServiceName -ErrorAction SilentlyContinue
        if ($svc.Status -eq "Running") {
            Write-Ok "Service is running (after ${waited}s)."
            break
        }
        if ($svc.Status -eq "Stopped") {
            Write-Err "Service stopped unexpectedly."
            Write-Info "Check Windows Event Viewer or logs."
            return
        }
    }

    $svc = Get-Service -Name $ServiceName -ErrorAction SilentlyContinue
    if ($svc.Status -ne "Running") {
        Write-Warn "Service not yet running after ${maxWait}s."
        return
    }

    # Health check with retry
    Start-Sleep -Seconds 2
    $healthUrl = "http://localhost:${HttpPort}/health"
    $healthOk = $false

    for ($i = 1; $i -le 5; $i++) {
        try {
            $resp = Invoke-WebRequest -Uri $healthUrl -UseBasicParsing -TimeoutSec 5
            if ($resp.StatusCode -eq 200) {
                $healthOk = $true
                break
            }
        } catch {
            if ($i -lt 5) {
                Start-Sleep -Seconds 2
            }
        }
    }

    if ($healthOk) {
        Write-Ok "Health check passed: $healthUrl"
    } else {
        Write-Warn "Health check did not succeed at $healthUrl"
        Write-Info "The service may still be starting up. Check manually."
    }
}

# ---------------------------------------------------------------------------
# 8. Summary
# ---------------------------------------------------------------------------

function Write-Summary {
    param(
        [string]$InstalledExe,
        [string]$TargetInstallDir,
        [string]$TargetDataDir,
        [System.TimeSpan]$BuildTime
    )

    Write-Host ""
    Write-Host "========================================" -ForegroundColor Green
    Write-Host "  IronBase Build & Install Complete" -ForegroundColor Green
    Write-Host "========================================" -ForegroundColor Green
    Write-Host ""

    if ($BuildTime) {
        Write-Host "  Build time:    $("{0:mm\:ss}" -f $BuildTime)" -ForegroundColor White
    }

    # Version
    try {
        $ver = & $InstalledExe --version 2>$null
        if ($ver) {
            Write-Host "  Version:       $ver" -ForegroundColor White
        }
    } catch {}

    Write-Host "  Install dir:   $TargetInstallDir" -ForegroundColor White
    Write-Host "  Data dir:      $TargetDataDir" -ForegroundColor White
    Write-Host "  Config:        $(Join-Path $TargetDataDir 'config.toml')" -ForegroundColor White
    Write-Host "  Port:          $($script:Port)" -ForegroundColor White

    # Service status
    $svc = Get-Service -Name $ServiceName -ErrorAction SilentlyContinue
    if ($svc) {
        $color = if ($svc.Status -eq "Running") { "Green" } else { "Yellow" }
        Write-Host "  Service:       $($svc.Status)" -ForegroundColor $color
    } else {
        Write-Host "  Service:       Not installed" -ForegroundColor DarkGray
    }

    Write-Host ""
    Write-Host "  Useful commands:" -ForegroundColor Cyan
    Write-Host "    Get-Service $ServiceName          # Service status" -ForegroundColor DarkGray
    Write-Host "    Restart-Service $ServiceName       # Restart" -ForegroundColor DarkGray
    Write-Host "    Invoke-WebRequest http://localhost:$($script:Port)/health  # Health check" -ForegroundColor DarkGray
    Write-Host ""
}

# ---------------------------------------------------------------------------
# Uninstall
# ---------------------------------------------------------------------------

function Uninstall-IronBase {
    param(
        [string]$TargetInstallDir,
        [string]$TargetDataDir
    )

    Write-Host ""
    Write-Host "========================================" -ForegroundColor Cyan
    Write-Host "  IronBase Uninstall (Build Install)" -ForegroundColor Cyan
    Write-Host "========================================" -ForegroundColor Cyan
    Write-Host ""

    # Stop service
    Stop-ExistingService -Timeout 30

    # Uninstall service
    $exePath = Join-Path $TargetInstallDir $ExeName
    $service = Get-Service -Name $ServiceName -ErrorAction SilentlyContinue
    if ($service) {
        Write-Info "Uninstalling service..."
        if (Test-Path $exePath) {
            & $exePath uninstall 2>$null
            Start-Sleep -Seconds 2
        }
        $service = Get-Service -Name $ServiceName -ErrorAction SilentlyContinue
        if ($service) {
            & sc.exe delete $ServiceName 2>$null
        }
        Write-Ok "Service removed."
    }

    # Remove executable
    if (Test-Path $exePath) {
        Write-Info "Removing executable..."
        Remove-Item -Path $exePath -Force
        Write-Ok "Executable removed."
    }

    # Remove backup if present
    $backupPath = "${exePath}.backup"
    if (Test-Path $backupPath) {
        Remove-Item -Path $backupPath -Force
    }

    # Remove install dir if empty
    if ((Test-Path $TargetInstallDir) -and ((Get-ChildItem $TargetInstallDir -ErrorAction SilentlyContinue | Measure-Object).Count -eq 0)) {
        Remove-Item -Path $TargetInstallDir -Force -ErrorAction SilentlyContinue
        Write-Ok "Install directory removed."
    }

    # Remove firewall rule
    $ruleName = "IronBase MCP Server (TCP $($script:Port))"
    $fwRule = Get-NetFirewallRule -DisplayName $ruleName -ErrorAction SilentlyContinue
    if ($fwRule) {
        Remove-NetFirewallRule -DisplayName $ruleName -ErrorAction SilentlyContinue
        Write-Ok "Firewall rule removed."
    }

    Write-Host ""
    Write-Host "========================================" -ForegroundColor Green
    Write-Host "  Uninstall Complete" -ForegroundColor Green
    Write-Host "========================================" -ForegroundColor Green
    Write-Host ""
    Write-Warn "Data preserved at: $TargetDataDir"
    Write-Host "  To remove all data: Remove-Item -Recurse '$TargetDataDir'" -ForegroundColor DarkGray
    Write-Host ""
}

# ===========================================================================
# Main
# ===========================================================================

Write-Host ""
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "  IronBase MCP Server" -ForegroundColor Cyan
Write-Host "  Build from Source & Install" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""

# Resolve install/data directories
if (-not $InstallDir) {
    $InstallDir = "$env:ProgramFiles\IronBase"
}
if (-not $DataDir) {
    $DataDir = "$env:ProgramData\IronBase"
}

# --- Uninstall mode ---
if ($Uninstall) {
    Assert-Administrator
    Uninstall-IronBase -TargetInstallDir $InstallDir -TargetDataDir $DataDir
    exit 0
}

# --- Admin check (service install needs it) ---
if (-not $SkipService) {
    Assert-Administrator
}

# --- Toolchain checks (unless skipping build) ---
$builtExePath = $null
$buildElapsed = $null

if (-not $SkipBuild) {
    Assert-RustToolchain
    Assert-VsBuildTools

    # Get source
    $srcPath = Get-SourceDirectory

    # Build
    $buildSw = [System.Diagnostics.Stopwatch]::StartNew()
    $builtExePath = Invoke-CargoBuild -SrcPath $srcPath
    $buildSw.Stop()
    $buildElapsed = $buildSw.Elapsed
} else {
    Write-Step "Skipping build (-SkipBuild)"

    # Look for pre-built binary
    $candidates = @(
        (Join-Path $InstallDir $ExeName)
    )
    if ($SourceDir) {
        $candidates += Join-Path $SourceDir "target\release\$ExeName"
        $candidates += Join-Path $SourceDir "mcp-server\target\release\$ExeName"
    }

    foreach ($c in $candidates) {
        if (Test-Path $c) {
            $builtExePath = (Resolve-Path $c).Path
            Write-Ok "Found existing binary: $builtExePath"
            break
        }
    }

    if (-not $builtExePath) {
        Write-Err "No pre-built binary found. Remove -SkipBuild to build from source."
        exit 1
    }
}

# --- Handle existing installation ---
Remove-ExistingInstallation -TargetInstallDir $InstallDir

# --- Install ---
$installedExe = Install-IronBase -BuiltExePath $builtExePath -TargetInstallDir $InstallDir -TargetDataDir $DataDir

# --- Start & verify ---
Start-AndVerify -InstalledExe $installedExe -HttpPort $Port

# --- Summary ---
Write-Summary -InstalledExe $installedExe -TargetInstallDir $InstallDir -TargetDataDir $DataDir -BuildTime $buildElapsed
