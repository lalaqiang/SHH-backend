# ============================================================================
# init_database.ps1 — 一键初始化 ERP 数据库
# ============================================================================
# 用途：新部署 ERP 系统时，按顺序执行所有数据库初始化脚本
# 使用：.\init_database.ps1 -SqlServer "localhost" -DbUser "sa" -DbPassword "your_sa_password"
#
# 执行顺序：
#   1. 00_init_schema.sql    —— 创建数据库 + tSys_Migration 表
#   2. DB-01 至 DB-16        —— 业务表触发器、索引、约束、存储过程
#   3. init_menu_permcode.sql —— 菜单权限码初始化
#   4. seed_admin_user.sql   —— 创建 admin 用户
#   5. seed_default_menus_roles.sql —— 创建默认角色与权限
# ============================================================================

param(
    [Parameter(Mandatory=$true)]
    [string]$SqlServer,

    [Parameter(Mandatory=$true)]
    [string]$DbUser,

    [Parameter(Mandatory=$true)]
    [string]$DbPassword,

    [string]$Database = "TestERP"
)

$ErrorActionPreference = "Stop"

# 切换到脚本所在目录
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
Set-Location $scriptDir

Write-Host ""
Write-Host "============================================================" -ForegroundColor Cyan
Write-Host "  ERP 数据库初始化脚本" -ForegroundColor Cyan
Write-Host "  SQL Server: $SqlServer" -ForegroundColor Cyan
Write-Host "  Database:   $Database" -ForegroundColor Cyan
Write-Host "============================================================" -ForegroundColor Cyan
Write-Host ""

# 执行 SQL 文件的辅助函数
function Invoke-SqlFile {
    param([string]$file, [string]$description)

    if (-not (Test-Path $file)) {
        Write-Host "[SKIP] 文件不存在: $file" -ForegroundColor Yellow
        return
    }

    Write-Host "[RUN] $description" -ForegroundColor White
    Write-Host "      文件: $file" -ForegroundColor Gray

    $result = & sqlcmd -S $SqlServer -U $DbUser -P $DbPassword -d $Database -i $file -b 2>&1

    if ($LASTEXITCODE -ne 0) {
        Write-Host "[FAIL] 执行失败 (exit=$LASTEXITCODE)" -ForegroundColor Red
        Write-Host $result -ForegroundColor Red
        throw "执行失败: $file"
    }

    # 只显示最后几行（包含 PRINT 输出）
    $result | Select-Object -Last 5 | ForEach-Object { Write-Host "      $_" -ForegroundColor DarkGray }
    Write-Host "[OK]   完成" -ForegroundColor Green
    Write-Host ""
}

try {
    # 1. 入口脚本（创建数据库 + tSys_Migration 表）
    Invoke-SqlFile -file ".\db\00_init_schema.sql" -description "1/21 创建数据库与迁移版本表"

    # 2-18. DB-01 至 DB-16 系列
    $dbScripts = @(
        @{ file = ".\db\DB-01-触发器与约束.sql"; desc = "2/21 触发器与约束" },
        @{ file = ".\db\DB-02-非聚集索引.sql"; desc = "3/21 非聚集索引" },
        @{ file = ".\db\DB-02-cleanup-duplicate-indexes.sql"; desc = "4/21 清理重复索引" },
        @{ file = ".\db\DB-03-外键约束.sql"; desc = "5/21 外键约束" },
        @{ file = ".\db\DB-04-单据号生成.sql"; desc = "6/21 单据号序列表" },
        @{ file = ".\db\DB-05-库存分页查询.sql"; desc = "7/21 库存分页查询" },
        @{ file = ".\db\DB-06-单据分页查询.sql"; desc = "8/21 单据分页查询" },
        @{ file = ".\db\DB-07-月结.sql"; desc = "9/21 月结存储过程" },
        @{ file = ".\db\DB-07-rollback-月结回滚.sql"; desc = "10/21 月结回滚" },
        @{ file = ".\db\DB-08-软删除与恢复.sql"; desc = "11/21 软删除与恢复" },
        @{ file = ".\db\DB-09-数据字典.sql"; desc = "12/21 数据字典" },
        @{ file = ".\db\DB-11-备份还原.sql"; desc = "13/21 备份还原" },
        @{ file = ".\db\DB-12-用户权限.sql"; desc = "14/21 应用账号与权限" },
        @{ file = ".\db\DB-13-14-运维.sql"; desc = "15/21 运维脚本" },
        @{ file = ".\db\DB-15-CHECK-约束.sql"; desc = "16/21 CHECK 约束" },
        @{ file = ".\db\DB-16-按月归档表.sql"; desc = "17/21 按月归档表" }
    )
    foreach ($s in $dbScripts) {
        Invoke-SqlFile -file $s.file -description $s.desc
    }

    # 19. 菜单权限码
    Invoke-SqlFile -file ".\init_menu_permcode.sql" -description "18/21 菜单 PermCode 初始化"

    # 20. admin 用户
    Invoke-SqlFile -file ".\seed_admin_user.sql" -description "19/21 创建 admin 用户"

    # 21. 默认角色
    Invoke-SqlFile -file ".\seed_default_menus_roles.sql" -description "20/21 创建默认角色与权限"

    Write-Host "============================================================" -ForegroundColor Green
    Write-Host "  数据库初始化完成！" -ForegroundColor Green
    Write-Host "  默认账号：admin / 123456" -ForegroundColor Yellow
    Write-Host "  请立即修改 admin 密码" -ForegroundColor Yellow
    Write-Host "============================================================" -ForegroundColor Green
}
catch {
    Write-Host ""
    Write-Host "============================================================" -ForegroundColor Red
    Write-Host "  初始化失败：$_" -ForegroundColor Red
    Write-Host "============================================================" -ForegroundColor Red
    exit 1
}
