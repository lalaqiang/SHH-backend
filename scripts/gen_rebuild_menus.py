#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""重建 tSys_Menus 表为新结构，与前端 router/index.js 和 app.js menuData 对齐"""

import uuid

# 菜单数据结构：(label, perm_code, path, icon, children)
# perm_code 格式：模块.资源（如 base.goods）
# path 是前端路由路径
# children 是子菜单列表

ZERO_UUID = "00000000-0000-0000-0000-000000000000"

# 生成固定的 UUID（基于菜单 ID 的确定性 UUID）
def gen_uuid(name):
    """基于名称生成确定性 UUID"""
    return str(uuid.uuid5(uuid.NAMESPACE_DNS, f"shenhua-erp-{name}"))

# 菜单定义：9 个顶级模块 + 子菜单
MENUS = [
    {
        "id": "base",
        "label": "基础资料",
        "perm_code": "base",
        "path": "/base/product",
        "icon": "Box",
        "sort": "001",
        "children": [
            ("product", "商品管理", "base.goods", "/base/product", "Goods", "0010"),
            ("gds-type", "商品分类", "base.gdsType", "/base/gds-type", "Grid", "0020"),
            ("brand", "品牌管理", "base.brand", "/base/brand", "Stamp", "0030"),
            ("warehouse", "仓库管理", "base.warehouse", "/base/warehouse", "OfficeBuilding", "0040"),
            ("unit", "单位管理", "base.unit", "/base/unit", "Odometer", "0050"),
            ("supplier", "供应商管理", "base.supplier", "/base/supplier", "Van", "0060"),
            ("customer", "客户管理", "base.customer", "/base/customer", "UserFilled", "0070"),
            ("employee", "员工管理", "base.employee", "/base/employee", "User", "0080"),
            ("pricing-template", "客户定价", "base.pricingTemplate", "/base/pricing-template", "PriceTag", "0090"),
            ("commission-template", "提成模板", "base.commissionTemplate", "/base/commission-template", "TrendCharts", "0100"),
        ],
    },
    {
        "id": "document",
        "label": "单据管理",
        "perm_code": "document",
        "path": "/purchase/order",
        "icon": "Document",
        "sort": "002",
        "children": [
            ("purchase-order", "采购订单", "purchase.order", "/purchase/order", "ShoppingCart", "0010"),
            ("purchase-receipt", "采购收货", "purchase.receipt", "/purchase/receipt", "Box", "0020"),
            ("purchase-return", "采购退货", "purchase.return", "/purchase/return", "RefreshLeft", "0030"),
            ("purchase-quote", "采购报价", "purchase.quote", "/purchase/quote", "Document", "0040"),
            ("purchase-adjprice", "采购调价", "purchase.adjPrice", "/purchase/adjprice", "EditPen", "0050"),
            ("sales-order", "销售订单", "sales.order", "/sales/order", "Sell", "0060"),
            ("sales-outbound", "销售出库", "sales.outbound", "/sales/outbound", "Box", "0070"),
            ("sales-return", "销售退货", "sales.return", "/sales/return", "RefreshRight", "0080"),
            ("sales-quote", "销售报价", "sales.quote", "/sales/quote", "Document", "0090"),
            ("sales-vip", "会员管理", "sales.vip", "/sales/vip", "User", "0100"),
            ("zp-delivery", "门店直配", "inventory.zpDelivery", "/inventory/zp-delivery", "Van", "0110"),
            ("store-return", "门店退仓", "inventory.storeReturn", "/inventory/store-return", "RefreshLeft", "0120"),
            ("inventory-move", "调拨单", "inventory.move", "/inventory/move", "Sort", "0130"),
            ("retail-sale", "门店销售", "retail.sale", "/retail/sale", "Shop", "0140"),
            ("retail-cashier", "收银台", "retail.cashier", "/retail/cashier", "CreditCard", "0150"),
            ("inventory-io", "入出库单", "inventory.io", "/inventory/io", "Switch", "0160"),
            ("oti-inbound", "零散入库", "inventory.otiInbound", "/inventory/oti-inbound", "Download", "0170"),
            ("oto-outbound", "零散出库", "inventory.otoOutbound", "/inventory/oto-outbound", "Upload", "0180"),
            ("requisition", "领用单", "inventory.requisition", "/inventory/requisition", "TakeawayBox", "0190"),
            ("inventory-check", "盘点单", "inventory.check", "/inventory/check", "DocumentChecked", "0200"),
            ("doc-relation-graph", "单据关系图", "document.relationGraph", "/document/relation-graph", "Share", "0210"),
        ],
    },
    {
        "id": "wholesale",
        "label": "批发业务",
        "perm_code": "wholesale",
        "path": "/wholesale/order",
        "icon": "Goods",
        "sort": "003",
        "children": [
            ("wholesale-order", "批发订单", "wholesale.order", "/wholesale/order", "Goods", "0010"),
            ("wholesale-quote", "批发报价", "wholesale.quote", "/wholesale/quote", "Document", "0020"),
            ("wholesale-outbound", "批发出库", "wholesale.outbound", "/wholesale/outbound", "Box", "0030"),
            ("wholesale-return", "批发退货", "wholesale.return", "/wholesale/return", "RefreshRight", "0040"),
            ("wholesale-adjprice", "批发调价", "wholesale.adjPrice", "/wholesale/adjprice", "EditPen", "0050"),
        ],
    },
    {
        "id": "inventory",
        "label": "库存管理",
        "perm_code": "inventory",
        "path": "/inventory/stock",
        "icon": "Goods",
        "sort": "004",
        "children": [
            ("inventory-stock", "库存查询", "inventory.stock", "/inventory/stock", "List", "0010"),
            ("inventory-flows", "库存流水", "inventory.flows", "/inventory/flows", "Operation", "0020"),
            ("inventory-alerts", "库存预警", "inventory.alerts", "/inventory/alerts", "WarningFilled", "0030"),
            ("inventory-shortages", "缺货记录", "inventory.shortages", "/inventory/shortages", "CircleCloseFilled", "0040"),
            ("inventory-adjust", "库存调整", "inventory.adjust", "/inventory/adjust", "EditPen", "0050"),
            ("inventory-replenish", "补货申请", "inventory.replenish", "/inventory/replenish", "Plus", "0060"),
            ("inventory-stock-cycle", "周期盘点", "inventory.stockCycle", "/inventory/stock-cycle", "Calendar", "0070"),
        ],
    },
    {
        "id": "finance",
        "label": "财务管理",
        "perm_code": "finance",
        "path": "/finance/receipts",
        "icon": "Money",
        "sort": "005",
        "children": [
            ("finance-receipts", "收款单", "finance.receipts", "/finance/receipts", "Wallet", "0010"),
            ("finance-payments", "付款单", "finance.payments", "/finance/payments", "CreditCard", "0020"),
        ],
    },
    {
        "id": "report",
        "label": "报表中心",
        "perm_code": "report",
        "path": "/report/sales-report",
        "icon": "DataAnalysis",
        "sort": "006",
        "children": [
            ("sales-report", "销售报表", "report.salesReport", "/report/sales-report", "TrendCharts", "0010"),
            ("purchase-report", "采购报表", "report.purchaseReport", "/report/purchase-report", "DataLine", "0020"),
            ("inventory-report", "库存报表", "report.inventoryReport", "/report/inventory-report", "DataBoard", "0030"),
            ("profit-analysis", "利润报表", "report.profitAnalysis", "/decision/profit-analysis", "Coin", "0040"),
            ("commission-report", "提成报表", "report.commissionReport", "/report/commission-report", "Money", "0050"),
            ("commission-details", "提成明细", "report.commissionDetails", "/report/commission-details", "List", "0060"),
            ("finance-report", "财务报表", "report.finance", "/report/finance", "Coin", "0070"),
            ("sales-task-report", "销售任务报表", "report.salesTask", "/report/sales-task", "Aim", "0080"),
            ("sales-analysis", "销售分析", "report.salesAnalysis", "/decision/sales-analysis", "TrendCharts", "0090"),
            ("purchase-analysis", "采购分析", "report.purchaseAnalysis", "/decision/purchase-analysis", "DataLine", "0100"),
            ("business-report", "业务报表", "report.businessReport", "/report/business-report", "DataAnalysis", "0110"),
            ("custom-report", "自定义报表", "report.custom", "/report/custom", "DocumentCopy", "0120"),
        ],
    },
    {
        "id": "mobile-data",
        "label": "手机数据",
        "perm_code": "mobile",
        "path": "/mobile-data",
        "icon": "Iphone",
        "sort": "007",
        "children": [
            ("mobile-data-replenishment", "补货申请", "mobile.replenishment", "/mobile-data/replenishment", "ShoppingCart", "0010"),
            ("mobile-data-gift", "赠品赠送", "mobile.giftGiving", "/mobile-data/gift-giving", "Present", "0020"),
            ("mobile-data-stock-check", "手机盘点", "mobile.stockCheck", "/mobile-data/stock-check", "DocumentChecked", "0030"),
            ("mobile-data-special-price", "特价申请", "mobile.specialPrice", "/mobile-data/special-price", "Discount", "0040"),
            ("mobile-data-reward", "奖励产品", "mobile.rewardProduct", "/mobile-data/reward-product", "Present", "0050"),
            ("sales-task", "销售任务", "mobile.salesTask", "/sales-task", "Aim", "0060"),
            ("sales-input", "员工销量录入", "mobile.salesInput", "/sales/input", "Edit", "0070"),
        ],
    },
    {
        "id": "online",
        "label": "线上商城",
        "perm_code": "online",
        "path": "/online/shop",
        "icon": "Platform",
        "sort": "008",
        "children": [
            ("online-shop", "线上下单", "online.shop", "/online/shop", "ShoppingCart", "0010"),
            ("online-my-orders", "我的订单", "online.myOrders", "/online/my-orders", "Document", "0020"),
            ("online-address-book", "地址库", "online.addressBook", "/online/address-book", "List", "0030"),
            ("online-manage", "商城管理", "online.manage", "/online/manage", "Setting", "0040"),
            ("online-payment-configs", "支付配置", "online.paymentConfigs", "/online/payment-configs", "CreditCard", "0050"),
        ],
    },
    {
        "id": "system",
        "label": "系统设置",
        "perm_code": "system",
        "path": "/system/overview",
        "icon": "Setting",
        "sort": "009",
        "children": [
            ("system-overview", "系统总览", "system.overview", "/system/overview", "Monitor", "0010"),
            ("system-config", "系统配置", "system.config", "/system/config", "Tools", "0020"),
            ("user-management", "用户管理", "system.user", "/system/user-management", "User", "0030"),
            ("role-management", "角色权限", "system.role", "/system/role-management", "Avatar", "0040"),
            ("menu-management", "菜单管理", "system.menu", "/system/menu-management", "Menu", "0050"),
            ("oper-log", "操作日志", "system.operLog", "/system/oper-log", "Notebook", "0060"),
            ("print-template", "打印模板", "system.printTemplate", "/system/print-template", "Printer", "0070"),
            ("print-log", "打印日志", "system.printLog", "/system/print-log", "List", "0080"),
            ("import-export", "数据导入导出", "system.importExport", "/system/import-export", "Upload", "0090"),
            ("notification", "通知中心", "system.notification", "/system/notification", "Bell", "0100"),
            ("dictionary", "数据字典", "system.dictionary", "/system/dictionary", "Document", "0110"),
            ("params", "系统参数", "system.params", "/system/params", "Setting", "0120"),
            ("backup", "数据备份", "system.backup", "/system/backup", "FolderChecked", "0130"),
            ("change-password", "修改密码", "system.changePassword", "/user/change-password", "Key", "0140"),
            ("oa-workflow", "流程审批", "oa.workflow", "/oa/workflow", "Tickets", "0150"),
            ("oa-notice", "公告通知", "oa.notice", "/oa/notice", "Bell", "0160"),
            ("oa-email", "OA邮件", "oa.email", "/oa/email", "Message", "0170"),
            ("it-report", "IT报表", "it.report", "/it/report", "Monitor", "0180"),
            ("query-index", "综合查询", "query.index", "/query/index", "ZoomIn", "0190"),
        ],
    },
]

# 生成 SQL
sql_lines = []
sql_lines.append("-- 重建 tSys_Menus 表为新结构")
sql_lines.append("-- 与前端 router/index.js 和 stores/app.js menuData 完全对齐")
sql_lines.append("-- 生成时间: 2026-07-05")
sql_lines.append("")
sql_lines.append("-- 备份旧菜单数据（按 Used=Y 过滤，只备份启用的）")
sql_lines.append("IF NOT EXISTS (SELECT * FROM sys.tables WHERE name = 'tSys_Menus_BAK')")
sql_lines.append("BEGIN")
sql_lines.append("    SELECT * INTO tSys_Menus_BAK FROM tSys_Menus;")
sql_lines.append("    PRINT '已备份旧菜单到 tSys_Menus_BAK';")
sql_lines.append("END")
sql_lines.append("GO")
sql_lines.append("")
sql_lines.append("-- 清空旧菜单数据")
sql_lines.append("DELETE FROM tSys_Menus;")
sql_lines.append("PRINT '已清空旧菜单数据';")
sql_lines.append("GO")
sql_lines.append("")
sql_lines.append("-- 插入新结构菜单数据（使用事务，无 GO 分隔以保持事务上下文）")
sql_lines.append("BEGIN TRANSACTION;")
sql_lines.append("")

total_top = 0
total_child = 0

for mod in MENUS:
    mod_id = gen_uuid(mod["id"])
    sql_lines.append(f"-- 模块: {mod['label']} ({mod['id']})")
    # MDCallName 存储前端路由路径（varchar(100)），SYM_PPT 默认 0（图标由前端根据 PermCode 映射）
    sql_lines.append(f"INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)")
    sql_lines.append(f"VALUES ('{mod_id}', '{ZERO_UUID}', N'{mod['label']}', N'{mod['sort']}', N'{mod['path']}', 0, N'Y', N'{mod['perm_code']}', N'0');")
    total_top += 1

    for child in mod["children"]:
        child_id, child_label, child_perm, child_path, child_icon, child_sort = child
        child_uuid = gen_uuid(f"{mod['id']}-{child_id}")
        sql_lines.append(f"INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)")
        sql_lines.append(f"VALUES ('{child_uuid}', '{mod_id}', N'{child_label}', N'{child_sort}', N'{child_path}', 0, N'Y', N'{child_perm}', N'0');")
        total_child += 1

    sql_lines.append("")
    mod_label = mod["label"]
    child_count = len(mod["children"])
    sql_lines.append(f"PRINT N'已插入模块: {mod_label} ({child_count} 个子菜单)';")
    sql_lines.append("")

sql_lines.append("COMMIT TRANSACTION;")
sql_lines.append("")
sql_lines.append(f"PRINT N'菜单重建完成: {total_top} 个顶级模块, {total_child} 个子菜单, 共 {total_top + total_child} 条记录';")

# 写入文件
output_file = r"c:\Users\Administrator\Desktop\ERP\server-rust\scripts\rebuild_menus.sql"
with open(output_file, "w", encoding="utf-8") as f:
    f.write("\n".join(sql_lines))

print(f"SQL 文件已生成: {output_file}")
print(f"统计: {total_top} 个顶级模块, {total_child} 个子菜单, 共 {total_top + total_child} 条记录")
