# 使用 .NET SqlClient 直接执行菜单重建
$connectionString = "Server=DESKTOP-QKTHTQP\SQLEXPRESS;Database=TestERP;User Id=sa;Password=sa123456;TrustServerCertificate=True;Encrypt=False"

# 加载 System.Data
Add-Type -AssemblyName "System.Data"

function New-Guid([string]$name) {
    $bytes = [System.Text.Encoding]::UTF8.GetBytes("shenhua-erp-$name")
    $sha1 = [System.Security.Cryptography.SHA1]::Create()
    $hash = $sha1.ComputeHash($bytes)
    # UUID v5: 版本 5，变体 2
    $hash[6] = ($hash[6] -band 0x0F) -bor 0x50
    $hash[8] = ($hash[8] -band 0x3F) -bor 0x80
    return [Guid]::new($hash).ToString()
}

$ZERO_UUID = "00000000-0000-0000-0000-000000000000"

# 菜单定义
$MENUS = @(
    @{ id="base"; label="基础资料"; perm_code="base"; path="/base/product"; sort="001"; children=@(
        @("product","商品管理","base.goods","/base/product","0010"),
        @("gds-type","商品分类","base.gdsType","/base/gds-type","0020"),
        @("brand","品牌管理","base.brand","/base/brand","0030"),
        @("warehouse","仓库管理","base.warehouse","/base/warehouse","0040"),
        @("unit","单位管理","base.unit","/base/unit","0050"),
        @("supplier","供应商管理","base.supplier","/base/supplier","0060"),
        @("customer","客户管理","base.customer","/base/customer","0070"),
        @("employee","员工管理","base.employee","/base/employee","0080"),
        @("pricing-template","客户定价","base.pricingTemplate","/base/pricing-template","0090"),
        @("commission-template","提成模板","base.commissionTemplate","/base/commission-template","0100")
    )},
    @{ id="document"; label="单据管理"; perm_code="document"; path="/purchase/order"; sort="002"; children=@(
        @("purchase-order","采购订单","purchase.order","/purchase/order","0010"),
        @("purchase-receipt","采购收货","purchase.receipt","/purchase/receipt","0020"),
        @("purchase-return","采购退货","purchase.return","/purchase/return","0030"),
        @("purchase-quote","采购报价","purchase.quote","/purchase/quote","0040"),
        @("purchase-adjprice","采购调价","purchase.adjPrice","/purchase/adjprice","0050"),
        @("sales-order","销售订单","sales.order","/sales/order","0060"),
        @("sales-outbound","销售出库","sales.outbound","/sales/outbound","0070"),
        @("sales-return","销售退货","sales.return","/sales/return","0080"),
        @("sales-quote","销售报价","sales.quote","/sales/quote","0090"),
        @("sales-vip","会员管理","sales.vip","/sales/vip","0100"),
        @("zp-delivery","门店直配","inventory.zpDelivery","/inventory/zp-delivery","0110"),
        @("store-return","门店退仓","inventory.storeReturn","/inventory/store-return","0120"),
        @("inventory-move","调拨单","inventory.move","/inventory/move","0130"),
        @("retail-sale","门店销售","retail.sale","/retail/sale","0140"),
        @("retail-cashier","收银台","retail.cashier","/retail/cashier","0150"),
        @("inventory-io","入出库单","inventory.io","/inventory/io","0160"),
        @("oti-inbound","零散入库","inventory.otiInbound","/inventory/oti-inbound","0170"),
        @("oto-outbound","零散出库","inventory.otoOutbound","/inventory/oto-outbound","0180"),
        @("requisition","领用单","inventory.requisition","/inventory/requisition","0190"),
        @("inventory-check","盘点单","inventory.check","/inventory/check","0200"),
        @("doc-relation-graph","单据关系图","document.relationGraph","/document/relation-graph","0210")
    )},
    @{ id="wholesale"; label="批发业务"; perm_code="wholesale"; path="/wholesale/order"; sort="003"; children=@(
        @("wholesale-order","批发订单","wholesale.order","/wholesale/order","0010"),
        @("wholesale-quote","批发报价","wholesale.quote","/wholesale/quote","0020"),
        @("wholesale-outbound","批发出库","wholesale.outbound","/wholesale/outbound","0030"),
        @("wholesale-return","批发退货","wholesale.return","/wholesale/return","0040"),
        @("wholesale-adjprice","批发调价","wholesale.adjPrice","/wholesale/adjprice","0050")
    )},
    @{ id="inventory"; label="库存管理"; perm_code="inventory"; path="/inventory/stock"; sort="004"; children=@(
        @("inventory-stock","库存查询","inventory.stock","/inventory/stock","0010"),
        @("inventory-flows","库存流水","inventory.flows","/inventory/flows","0020"),
        @("inventory-alerts","库存预警","inventory.alerts","/inventory/alerts","0030"),
        @("inventory-shortages","缺货记录","inventory.shortages","/inventory/shortages","0040"),
        @("inventory-adjust","库存调整","inventory.adjust","/inventory/adjust","0050"),
        @("inventory-replenish","补货申请","inventory.replenish","/inventory/replenish","0060"),
        @("inventory-stock-cycle","周期盘点","inventory.stockCycle","/inventory/stock-cycle","0070")
    )},
    @{ id="finance"; label="财务管理"; perm_code="finance"; path="/finance/receipts"; sort="005"; children=@(
        @("finance-receipts","收款单","finance.receipts","/finance/receipts","0010"),
        @("finance-payments","付款单","finance.payments","/finance/payments","0020")
    )},
    @{ id="report"; label="报表中心"; perm_code="report"; path="/report/sales-report"; sort="006"; children=@(
        @("sales-report","销售报表","report.salesReport","/report/sales-report","0010"),
        @("purchase-report","采购报表","report.purchaseReport","/report/purchase-report","0020"),
        @("inventory-report","库存报表","report.inventoryReport","/report/inventory-report","0030"),
        @("profit-analysis","利润报表","report.profitAnalysis","/decision/profit-analysis","0040"),
        @("commission-report","提成报表","report.commissionReport","/report/commission-report","0050"),
        @("commission-details","提成明细","report.commissionDetails","/report/commission-details","0060"),
        @("finance-report","财务报表","report.finance","/report/finance","0070"),
        @("sales-task-report","销售任务报表","report.salesTask","/report/sales-task","0080"),
        @("sales-analysis","销售分析","report.salesAnalysis","/decision/sales-analysis","0090"),
        @("purchase-analysis","采购分析","report.purchaseAnalysis","/decision/purchase-analysis","0100"),
        @("business-report","业务报表","report.businessReport","/report/business-report","0110"),
        @("custom-report","自定义报表","report.custom","/report/custom","0120")
    )},
    @{ id="mobile-data"; label="手机数据"; perm_code="mobile"; path="/mobile-data"; sort="007"; children=@(
        @("mobile-data-replenishment","补货申请","mobile.replenishment","/mobile-data/replenishment","0010"),
        @("mobile-data-gift","赠品赠送","mobile.giftGiving","/mobile-data/gift-giving","0020"),
        @("mobile-data-stock-check","手机盘点","mobile.stockCheck","/mobile-data/stock-check","0030"),
        @("mobile-data-special-price","特价申请","mobile.specialPrice","/mobile-data/special-price","0040"),
        @("mobile-data-reward","奖励产品","mobile.rewardProduct","/mobile-data/reward-product","0050"),
        @("sales-task","销售任务","mobile.salesTask","/sales-task","0060"),
        @("sales-input","员工销量录入","mobile.salesInput","/sales/input","0070")
    )},
    @{ id="online"; label="线上商城"; perm_code="online"; path="/online/shop"; sort="008"; children=@(
        @("online-shop","线上下单","online.shop","/online/shop","0010"),
        @("online-my-orders","我的订单","online.myOrders","/online/my-orders","0020"),
        @("online-address-book","地址库","online.addressBook","/online/address-book","0030"),
        @("online-manage","商城管理","online.manage","/online/manage","0040"),
        @("online-payment-configs","支付配置","online.paymentConfigs","/online/payment-configs","0050")
    )},
    @{ id="system"; label="系统设置"; perm_code="system"; path="/system/overview"; sort="009"; children=@(
        @("system-overview","系统总览","system.overview","/system/overview","0010"),
        @("system-config","系统配置","system.config","/system/config","0020"),
        @("user-management","用户管理","system.user","/system/user-management","0030"),
        @("role-management","角色权限","system.role","/system/role-management","0040"),
        @("menu-management","菜单管理","system.menu","/system/menu-management","0050"),
        @("oper-log","操作日志","system.operLog","/system/oper-log","0060"),
        @("print-template","打印模板","system.printTemplate","/system/print-template","0070"),
        @("print-log","打印日志","system.printLog","/system/print-log","0080"),
        @("import-export","数据导入导出","system.importExport","/system/import-export","0090"),
        @("notification","通知中心","system.notification","/system/notification","0100"),
        @("dictionary","数据字典","system.dictionary","/system/dictionary","0110"),
        @("params","系统参数","system.params","/system/params","0120"),
        @("backup","数据备份","system.backup","/system/backup","0130"),
        @("change-password","修改密码","system.changePassword","/user/change-password","0140"),
        @("oa-workflow","流程审批","oa.workflow","/oa/workflow","0150"),
        @("oa-notice","公告通知","oa.notice","/oa/notice","0160"),
        @("oa-email","OA邮件","oa.email","/oa/email","0170"),
        @("it-report","IT报表","it.report","/it/report","0180"),
        @("query-index","综合查询","query.index","/query/index","0190")
    )}
)

try {
    $conn = New-Object System.Data.SqlClient.SqlConnection($connectionString)
    $conn.Open()
    Write-Host "数据库连接成功"

    $cmd = $conn.CreateCommand()
    $cmd.Transaction = $conn.BeginTransaction()

    try {
        # 备份旧数据
        $cmd.CommandText = "IF NOT EXISTS (SELECT * FROM sys.tables WHERE name = 'tSys_Menus_BAK') SELECT * INTO tSys_Menus_BAK FROM tSys_Menus"
        $cmd.ExecuteNonQuery() | Out-Null
        Write-Host "已备份旧菜单数据"

        # 清空旧数据
        $cmd.CommandText = "DELETE FROM tSys_Menus"
        $rowsDeleted = $cmd.ExecuteNonQuery()
        Write-Host "已清空旧菜单数据（$rowsDeleted 行）"

        $insertSql = "INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg) VALUES (@id, @pid, @caption, @no, @path, 0, 'Y', @perm, '0')"
        $total = 0

        foreach ($mod in $MENUS) {
            $modId = New-Guid $mod.id
            $cmd.CommandText = $insertSql
            $cmd.Parameters.Clear()
            $cmd.Parameters.AddWithValue("@id", $modId) | Out-Null
            $cmd.Parameters.AddWithValue("@pid", $ZERO_UUID) | Out-Null
            $cmd.Parameters.AddWithValue("@caption", $mod.label) | Out-Null
            $cmd.Parameters.AddWithValue("@no", $mod.sort) | Out-Null
            $cmd.Parameters.AddWithValue("@path", $mod.path) | Out-Null
            $cmd.Parameters.AddWithValue("@perm", $mod.perm_code) | Out-Null
            $cmd.ExecuteNonQuery() | Out-Null
            $total++

            foreach ($child in $mod.children) {
                $childId = New-Guid "$($mod.id)-$($child[0])"
                $cmd.CommandText = $insertSql
                $cmd.Parameters.Clear()
                $cmd.Parameters.AddWithValue("@id", $childId) | Out-Null
                $cmd.Parameters.AddWithValue("@pid", $modId) | Out-Null
                $cmd.Parameters.AddWithValue("@caption", $child[1]) | Out-Null
                $cmd.Parameters.AddWithValue("@no", $child[4]) | Out-Null
                $cmd.Parameters.AddWithValue("@path", $child[3]) | Out-Null
                $cmd.Parameters.AddWithValue("@perm", $child[2]) | Out-Null
                $cmd.ExecuteNonQuery() | Out-Null
                $total++
            }

            Write-Host "  已插入模块: $($mod.label) ($($mod.children.Count) 个子菜单)"
        }

        $cmd.Transaction.Commit()
        Write-Host ""
        Write-Host "菜单重建完成: 共 $total 条记录"

        # 验证
        $cmd.CommandText = "SELECT COUNT(*) FROM tSys_Menus"
        $count = $cmd.ExecuteScalar()
        Write-Host "验证: tSys_Menus 现有 $count 条记录"

        $cmd.CommandText = "SELECT SYM_CAPTION, SYM_NO, PermCode, MDCallName FROM tSys_Menus WHERE SYM_PID = '$ZERO_UUID' ORDER BY SYM_NO"
        $reader = $cmd.ExecuteReader()
        Write-Host ""
        Write-Host "顶级模块列表:"
        while ($reader.Read()) {
            Write-Host ("  {0} | {1} | {2} | {3}" -f $reader["SYM_NO"], $reader["SYM_CAPTION"], $reader["PermCode"], $reader["MDCallName"])
        }
        $reader.Close()
    } catch {
        $cmd.Transaction.Rollback()
        Write-Host "错误: $_"
        Write-Host "事务已回滚"
    }
} catch {
    Write-Host "连接错误: $_"
} finally {
    if ($conn -and $conn.State -eq 'Open') { $conn.Close() }
}
