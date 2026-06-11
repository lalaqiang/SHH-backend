# 库存预警定时检查 - PowerShell 脚本
# 用法：手动运行  /  配合 Windows 任务计划程序（每天 8:00 跑一次）
#
# 调度方式（管理员权限运行 PowerShell）：
#   $action = New-ScheduledTaskAction -Execute "powershell.exe" `
#     -Argument "-NoProfile -File C:\path\to\low_stock_alert.ps1"
#   $trigger = New-ScheduledTaskTrigger -Daily -At "08:00"
#   Register-ScheduledTask -TaskName "ERP_LowStockAlert" `
#     -Action $action -Trigger $trigger -Description "ERP 库存预警每日检查"

param(
    [string]$ApiBase = "http://localhost:8080",
    [string]$Token    = "",
    [int]$AlertThreshold = 0   # 紧急项超过此值时打印告警
)

$ErrorActionPreference = "Stop"

# ===== 1) 调预警查询接口 =====
$alertUrl = "$ApiBase/api/inventory/low_stock_alert"
$headers = @{}
if ($Token) { $headers["Authorization"] = "Bearer $Token" }

Write-Host "[$(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')] 正在检查库存预警..." -ForegroundColor Cyan

try {
    $resp = Invoke-RestMethod -Uri $alertUrl -Method Post `
        -Headers $headers -ContentType "application/json" -Body "{}" `
        -TimeoutSec 30
}
catch {
    Write-Host "[ERROR] 调用接口失败: $_" -ForegroundColor Red
    exit 1
}

if ($resp.code -ne 0) {
    Write-Host "[ERROR] 接口返回错误: $($resp.message)" -ForegroundColor Red
    exit 1
}

$alert = $resp.data
Write-Host "  紧急项: $($alert.critical)  警告项: $($alert.warning)  提醒项: $($alert.total - $alert.critical - $alert.warning)" -ForegroundColor Yellow

# ===== 2) 紧急项超阈值 → 自动转补货申请 =====
if ($alert.critical -gt $AlertThreshold) {
    Write-Host "[WARN] 紧急项 = $($alert.critical) 超过阈值 = $AlertThreshold，自动转补货申请..." -ForegroundColor Magenta
    $createUrl = "$ApiBase/api/inventory/replenish_from_alert"
    try {
        $createResp = Invoke-RestMethod -Uri $createUrl -Method Post `
            -Headers $headers -ContentType "application/json" -Body '{}' `
            -TimeoutSec 60
        if ($createResp.code -eq 0) {
            $docs = $createResp.data.Documents
            Write-Host "  已生成补货申请 $($createResp.data.CreatedCount) 张：" -ForegroundColor Green
            foreach ($d in $docs) {
                Write-Host "    - $($d.ReplenishApplyNo) [$($d.StkID)] 明细 $($d.DetailCount) 项"
            }
        } else {
            Write-Host "[ERROR] 生成补货申请失败: $($createResp.message)" -ForegroundColor Red
        }
    } catch {
        Write-Host "[ERROR] 调用补货申请接口失败: $_" -ForegroundColor Red
    }
}

# ===== 3) 写出当日清单 =====
$logDir = "C:\ERP\logs"
if (!(Test-Path $logDir)) { New-Item -ItemType Directory -Path $logDir -Force | Out-Null }
$logFile = Join-Path $logDir "low_stock_$(Get-Date -Format 'yyyyMMdd').log"

$lines = @()
$lines += "=== 库存预警报告 $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss') ==="
$lines += "紧急: $($alert.critical)  警告: $($alert.warning)  提醒: $($alert.total - $alert.critical - $alert.warning)"
$lines += ""
foreach ($item in $alert.items) {
    $lines += "[$($item.AlertLevel)] $($item.GDSNO) - $($item.GDSDesc)"
    $lines += "    仓库: $($item.StkName)  当前QQty: $($item.QQty)  下限: $($item.BttomStkQty)  建议补: $($item.SuggestQty)"
}
$lines | Out-File -FilePath $logFile -Encoding UTF8
Write-Host "  报告已写入: $logFile" -ForegroundColor Cyan
Write-Host "[DONE] 库存预警检查完成" -ForegroundColor Green
