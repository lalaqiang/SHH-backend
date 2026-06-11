# ERP Backend Rebuild Script
# Steps: kill old process -> cargo build -> done (you start the server yourself)

$ErrorActionPreference = 'Continue'
$scriptDir = $PSScriptRoot
Set-Location $scriptDir

Write-Host ''
Write-Host '==========================================================' -ForegroundColor Cyan
Write-Host '  ERP Backend Rebuild' -ForegroundColor Cyan
Write-Host ('  ' + (Get-Date -Format 'yyyy-MM-dd HH:mm:ss')) -ForegroundColor Cyan
Write-Host '==========================================================' -ForegroundColor Cyan
Write-Host ''

# 1. Kill old process
Write-Host '[1/2] Killing old erp_server.exe ...' -ForegroundColor Yellow
$procs = Get-Process -Name 'erp_server' -ErrorAction SilentlyContinue
if ($procs) {
    foreach ($p in $procs) {
        Write-Host ('      Killing PID ' + $p.Id) -ForegroundColor Gray
        Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    }
    Start-Sleep -Seconds 3
    Write-Host '      Done.' -ForegroundColor Green
} else {
    Write-Host '      No running process.' -ForegroundColor Gray
}

# 2. Rebuild
Write-Host ''
Write-Host '[2/2] Building (cargo build --release) ...' -ForegroundColor Yellow
& cargo build --release 2>&1 | ForEach-Object { Write-Host $_ }
if ($LASTEXITCODE -ne 0) {
    Write-Host ''
    Write-Host '!!! BUILD FAILED !!!' -ForegroundColor Red
} else {
    Write-Host ''
    Write-Host 'Build OK.' -ForegroundColor Green
}

Write-Host ''
pause
