# ERP Backend one-key restart script
# 1) kill old erp_server process (if running)
# 2) cargo build --release
# 3) start new erp_server.exe in background
# 4) tail the log (Ctrl+C to detach, service keeps running)
#
# Usage (from server-rust directory):
#   .\restart_backend.ps1                 release mode (default)
#   .\restart_backend.ps1 -Mode debug    build & start debug version
#   .\restart_backend.ps1 -NoBuild       skip build, just kill + start
#   .\restart_backend.ps1 -NoStart       build only, do not start
#   .\restart_backend.ps1 -NoTail        do not tail the log
#   .\restart_backend.ps1 -StopOnly      kill process only
#
# Pair with restart_backend.bat (double-click entry)

[CmdletBinding()]
param(
    [ValidateSet('release', 'debug')] [string]$Mode = 'release',
    [switch]$NoBuild,
    [switch]$NoStart,
    [switch]$NoTail,
    [switch]$StopOnly
)

# Force UTF-8 output so Chinese console messages do not get mis-parsed
# as a command name on legacy code pages (GBK / 936).
$OutputEncoding = [System.Text.Encoding]::UTF8
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$ErrorActionPreference = 'Stop'

$scriptDir = $PSScriptRoot
if (-not $scriptDir) { $scriptDir = (Get-Location).Path }
Set-Location $scriptDir

$exeRel = if ($Mode -eq 'debug') { 'target\debug\erp_server.exe' } else { 'target\release\erp_server.exe' }
$exeAbs = Join-Path $scriptDir $exeRel
$logAbs = Join-Path $scriptDir 'erp_server.log'

$host.UI.RawUI.WindowTitle = "ERP Backend ($Mode) - $scriptDir"

function Write-Step($n, $total, $msg) {
    Write-Host ''
    Write-Host "[$n/$total] $msg" -ForegroundColor Yellow
}
function Write-Ok($msg)   { Write-Host ("      " + $msg) -ForegroundColor Green }
function Write-Info($msg) { Write-Host ("      " + $msg) -ForegroundColor Gray }
function Write-Err($msg)  { Write-Host ("      " + $msg) -ForegroundColor Red }

Write-Host ''
Write-Host '==========================================================' -ForegroundColor Cyan
Write-Host ("  ERP Backend Restart - " + $Mode) -ForegroundColor Cyan
Write-Host ('  ' + (Get-Date -Format 'yyyy-MM-dd HH:mm:ss')) -ForegroundColor Cyan
Write-Host '==========================================================' -ForegroundColor Cyan

$total = if ($StopOnly) { 1 } elseif ($NoBuild) { 2 } else { 3 }

# 1) kill old process
Write-Step 1 $total 'Killing old erp_server.exe ...'
$procs = Get-Process -Name 'erp_server' -ErrorAction SilentlyContinue
if ($procs) {
    foreach ($p in $procs) {
        Write-Info ("Stopping PID " + $p.Id + " (started " + $p.StartTime.ToString('HH:mm:ss') + ")")
        Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    }
    $waited = 0
    while ((Get-Process -Name 'erp_server' -ErrorAction SilentlyContinue) -and $waited -lt 10) {
        Start-Sleep -Seconds 1
        $waited++
    }
    if (Get-Process -Name 'erp_server' -ErrorAction SilentlyContinue) {
        Write-Err 'Process still alive after 10s, abort.'
        exit 1
    }
    Write-Ok 'Killed.'
} else {
    Write-Info 'No running process.'
}

if ($StopOnly) {
    Write-Host ''
    Write-Host 'StopOnly: done.' -ForegroundColor Green
    exit 0
}

# 2) build
if (-not $NoBuild) {
    Write-Step 2 $total ("Building (cargo build --" + $Mode + ") ...")
    # 临时切换 ErrorActionPreference：
    #   脚本开头是 'Stop'，但 cargo 把 warning/进度输出到 stderr，
    #   PowerShell 会把 stderr 行包装成 NativeCommandError 异常，
    #   导致 Stop 模式下脚本在编译阶段就被异常终止（即使 cargo 实际成功）。
    #   这里临时切到 'Continue'，让 warning 正常显示，仅按 $LASTEXITCODE 判断成败。
    $prevEap = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    & cargo build "--$Mode" 2>&1 | ForEach-Object {
        # NativeCommandError 类型的记录用红色显示，普通文本原样输出
        if ($_ -is [System.Management.Automation.ErrorRecord]) {
            Write-Host $_.Exception.Message -ForegroundColor Gray
        } else {
            Write-Host $_
        }
    }
    $buildExit = $LASTEXITCODE
    $ErrorActionPreference = $prevEap
    if ($buildExit -ne 0) {
        Write-Host ''
        Write-Err '!!! BUILD FAILED !!!'
        exit 1
    }
    Write-Ok ("Build OK -> " + $exeRel)
}

# 3) start
if (-not $NoStart) {
    $stepNo = if ($NoBuild) { 2 } else { 3 }
    Write-Step $stepNo $total ("Starting " + $exeRel + " ...")
    if (-not (Test-Path $exeAbs)) {
        Write-Err ("Executable not found: " + $exeAbs)
        Write-Err 'Remove -NoBuild or run cargo build --release manually.'
        exit 1
    }

    $proc = Start-Process -FilePath $exeAbs -WorkingDirectory $scriptDir `
        -RedirectStandardOutput $logAbs -RedirectStandardError ($logAbs + '.err') `
        -PassThru -WindowStyle Hidden
    Write-Ok ("Started PID " + $proc.Id + " (log: " + $logAbs + ")")

    Write-Info 'Waiting for port 8080 ...'
    $ready = $false
    for ($i = 0; $i -lt 30; $i++) {
        Start-Sleep -Seconds 1
        $listen = Get-NetTCPConnection -LocalPort 8080 -State Listen -ErrorAction SilentlyContinue
        if ($listen) { $ready = $true; break }
    }
    if ($ready) {
        Write-Ok 'Service is up on http://0.0.0.0:8080'
    } else {
        Write-Err 'Port 8080 not listening after 30s. Recent log:'
        if (Test-Path $logAbs) {
            Get-Content $logAbs -Tail 30 | ForEach-Object { Write-Host ('    ' + $_) }
        }
        exit 1
    }
}

Write-Host ''
Write-Host '==========================================================' -ForegroundColor Green
Write-Host '  Done.' -ForegroundColor Green
Write-Host '==========================================================' -ForegroundColor Green

# tail log
if (-not $NoStart -and -not $NoTail -and (Test-Path $logAbs)) {
    Write-Host ''
    Write-Host 'Tailing log (Ctrl+C to detach, service keeps running) ...' -ForegroundColor Cyan
    Write-Host ''
    Get-Content $logAbs -Tail 50 -Wait
}
