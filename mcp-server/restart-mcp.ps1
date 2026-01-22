# restart-mcp.ps1 - MCP Server Start/Restart
# Supports both Service mode (graceful) and Console mode (fallback)

param(
    [string]$DbPath = "D:\data\ironbase_data.mlite",
    [int]$Port = 8080,
    [int]$GracefulTimeout = 30
)

$serviceName = "IronBaseService"
$processName = "mcp-ironbase-server"
$exePath = "\\wsl$\Ubuntu\home\petitan\MongoLite\mcp-server\target\release\mcp-ironbase-server.exe"

# Check if running as Windows Service
$service = Get-Service -Name $serviceName -ErrorAction SilentlyContinue

if ($service -and $service.Status -eq "Running") {
    # === SERVICE MODE (Graceful Shutdown) ===
    Write-Host "=== MCP Server Restart (Service Mode) ===" -ForegroundColor Cyan
    Write-Host "Stopping service gracefully..." -ForegroundColor Yellow

    Stop-Service -Name $serviceName -ErrorAction SilentlyContinue

    # Wait for stop
    try {
        $service.WaitForStatus("Stopped", [TimeSpan]::FromSeconds($GracefulTimeout))
        Write-Host "Service stopped gracefully." -ForegroundColor Green
    } catch {
        Write-Host "Timeout waiting for service stop." -ForegroundColor Red
    }

    # Start service
    Write-Host "Starting service..." -ForegroundColor Cyan
    Start-Service -Name $serviceName
    Write-Host "Service started." -ForegroundColor Green
    Write-Host "Health check: curl -k https://localhost:$Port/health" -ForegroundColor Gray

} elseif ($service -and $service.Status -eq "Stopped") {
    # Service exists but not running - check for console process first
    $procs = @(Get-Process -Name $processName -ErrorAction SilentlyContinue)
    if ($procs.Count -gt 0) {
        # Console process running - stop it and start as service
        Write-Host "=== MCP Server Restart (Console -> Service) ===" -ForegroundColor Cyan
        Write-Host "Found console process, stopping..." -ForegroundColor Yellow
        $procs | Stop-Process -ErrorAction SilentlyContinue
        Start-Sleep -Seconds 2
    } else {
        Write-Host "=== MCP Server Start (Service Mode) ===" -ForegroundColor Cyan
    }
    Start-Service -Name $serviceName
    Write-Host "Service started." -ForegroundColor Green
    Write-Host "Health check: curl -k https://localhost:$Port/health" -ForegroundColor Gray

} else {
    # === CONSOLE MODE (Fallback - NOT graceful!) ===
    $procs = @(Get-Process -Name $processName -ErrorAction SilentlyContinue)

    if ($procs.Count -gt 0) {
        Write-Host "=== MCP Server Restart (Console Mode) ===" -ForegroundColor Cyan
        Write-Host "WARNING: Console mode uses Stop-Process (NOT graceful!)" -ForegroundColor Red
        Write-Host "TIP: Install as service for graceful shutdown: mcp-ironbase-server.exe install" -ForegroundColor Yellow
        Write-Host "Found $($procs.Count) running instance(s)" -ForegroundColor Yellow

        foreach ($proc in $procs) {
            Write-Host "Stopping PID: $($proc.Id)..." -ForegroundColor Yellow
            Stop-Process -Id $proc.Id -ErrorAction SilentlyContinue
        }

        # Wait for all to exit
        $waited = 0
        while (($waited -lt $GracefulTimeout) -and (@(Get-Process -Name $processName -ErrorAction SilentlyContinue).Count -gt 0)) {
            Start-Sleep -Seconds 1
            $waited++
            Write-Host "." -NoNewline
        }
        Write-Host ""

        # Force kill any remaining
        $remaining = @(Get-Process -Name $processName -ErrorAction SilentlyContinue)
        if ($remaining.Count -gt 0) {
            Write-Host "Timeout - forcing kill..." -ForegroundColor Red
            $remaining | Stop-Process -Force
            Start-Sleep -Seconds 2
        }

        Write-Host "All instances stopped." -ForegroundColor Green
    } else {
        Write-Host "=== MCP Server Start (Console Mode) ===" -ForegroundColor Cyan
        Write-Host "TIP: Install as service for graceful shutdown: mcp-ironbase-server.exe install" -ForegroundColor Yellow
    }

    # Start console process
    Write-Host "Starting server..." -ForegroundColor Cyan

    $env:IRONBASE_PATH = $DbPath
    $env:IRONBASE_PORT = $Port

    Start-Process -FilePath $exePath -NoNewWindow

    Write-Host "Server running on port $Port with DB: $DbPath" -ForegroundColor Green
    Write-Host "Health check: curl -k https://localhost:$Port/health" -ForegroundColor Gray
}
