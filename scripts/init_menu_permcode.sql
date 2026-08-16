-- 为 tSys_Menus 添加权限码字段并批量初始化
USE TestERP;
GO

-- 1. 添加 PermCode 字段（权限码，如 base.goods / purchase.order）
IF NOT EXISTS (SELECT 1 FROM sys.columns WHERE object_id = OBJECT_ID('tSys_Menus') AND name = 'PermCode')
BEGIN
    ALTER TABLE tSys_Menus ADD PermCode nvarchar(100) NULL;
    PRINT 'PermCode 字段已添加';
END
GO

-- 2. 基于 MDCallName 批量初始化 PermCode
UPDATE tSys_Menus SET PermCode = CASE
    -- 基础资料
    WHEN MDCallName = 'OpenGoods' THEN 'base.goods'
    WHEN MDCallName = 'OpenBillType' THEN 'base.category'
    WHEN MDCallName = 'OpenStock' THEN 'base.warehouse'
    WHEN MDCallName = 'OpenSUPP' THEN 'base.supplier'
    WHEN MDCallName = 'OpenCust' THEN 'base.customer'
    WHEN MDCallName = 'OpenEmp' THEN 'base.employee'
    WHEN MDCallName = 'OpenDept' THEN 'base.dept'
    WHEN MDCallName = 'OpenArea' THEN 'base.area'
    -- 采购
    WHEN MDCallName = 'OpenPur_Order' THEN 'purchase.order'
    WHEN MDCallName = 'OpenPur_Receipt' THEN 'purchase.receipt'
    WHEN MDCallName = 'OpenPur_Returned' THEN 'purchase.return'
    WHEN MDCallName = 'OpenPur_AdjPrice' THEN 'purchase.adjPrice'
    WHEN MDCallName = 'OpenPur_AutoPO' THEN 'purchase.autoOrder'
    WHEN MDCallName = 'OpenPur_RACheck' THEN 'purchase.raCheck'
    -- 销售
    WHEN MDCallName = 'OpenSal_Inv' THEN 'sales.order'
    WHEN MDCallName = 'OpenSal_Deliver' THEN 'sales.outbound'
    WHEN MDCallName = 'OpenSal_Returned' THEN 'sales.return'
    WHEN MDCallName = 'OpenSal_AdjPrice' THEN 'sales.adjPrice'
    WHEN MDCallName = 'OpenSal_SAdjPrice' THEN 'sales.adjPrice'
    WHEN MDCallName = 'OpenSal_GDSTypeEx' THEN 'sales.gdsTypeEx'
    WHEN MDCallName = 'OpenSal_GuideMode' THEN 'sales.guideMode'
    WHEN MDCallName = 'OpenSal_StkTake' THEN 'sales.stkTake'
    -- 库存
    WHEN MDCallName = 'OpenStk_Move' THEN 'inventory.move'
    WHEN MDCallName = 'OpenStk_Tran' THEN 'inventory.check'
    WHEN MDCallName = 'OpenStk_Other' THEN 'inventory.oto'
    WHEN MDCallName = 'OpenStk_Receive' THEN 'inventory.oti'
    WHEN MDCallName = 'OpenStk_ReplenishApply' THEN 'inventory.replenish'
    WHEN MDCallName = 'OpenStockCycle' THEN 'inventory.stockCycle'
    WHEN MDCallName = 'OpenStk_GDSLabel' THEN 'inventory.gdsLabel'
    WHEN MDCallName = 'OpenStk_ActPlan' THEN 'inventory.actPlan'
    WHEN MDCallName = 'OpenStk_MatAct' THEN 'inventory.matAct'
    WHEN MDCallName = 'OpenStk_PhoneApply' THEN 'inventory.phoneApply'
    WHEN MDCallName = 'OpenBas_GDSMoveBatch' THEN 'inventory.moveBatch'
    WHEN MDCallName = 'OpenBas_GDSOpenRA' THEN 'inventory.openRA'
    WHEN MDCallName = 'OpenBas_StockDspStd' THEN 'inventory.stockDspStd'
    WHEN MDCallName = 'OpenGoodsStock' THEN 'inventory.stockQuery'
    WHEN MDCallName = 'OpenQStk_GDSStock' THEN 'inventory.stockQuery'
    -- 零售
    WHEN MDCallName = 'OpenPos' THEN 'retail.sale'
    -- 财务
    WHEN MDCallName = 'OpenAccPayCheck' THEN 'finance.payCheck'
    -- 报表
    WHEN MDCallName = 'OpenRptTotal' THEN 'report.total'
    WHEN MDCallName = 'OpenRptSalTotal' THEN 'report.sales'
    WHEN MDCallName = 'OpenRptPurTotal' THEN 'report.purchase'
    WHEN MDCallName = 'OpenRptVIPTotal' THEN 'report.vip'
    WHEN MDCallName = 'OpenRptStdQuery' THEN 'report.stdQuery'
    WHEN MDCallName = 'OpenPub_Stat' THEN 'report.pubStat'
    -- 系统
    WHEN MDCallName = 'OpenMenus' THEN 'system.menu'
    WHEN MDCallName = 'OpenSys_Report' THEN 'system.report'
    WHEN MDCallName = 'OpenSys_Parameters' THEN 'system.config'
    WHEN MDCallName = 'OpenSys_SetPassWord' THEN 'system.password'
    WHEN MDCallName = 'OpenSys_Company' THEN 'system.company'
    WHEN MDCallName = 'OpenSys_AccPer' THEN 'system.accPer'
    WHEN MDCallName = 'OpenSys_AutoMsg' THEN 'system.autoMsg'
    WHEN MDCallName = 'OpenSys_DynTerm' THEN 'system.dynTerm'
    WHEN MDCallName = 'OpenSys_FastQ' THEN 'system.fastQ'
    WHEN MDCallName = 'OpenSys_MWorkFlow' THEN 'system.workflow'
    WHEN MDCallName = 'OpenSys_StkParams' THEN 'system.stkParams'
    WHEN MDCallName = 'OpenSys_ServerNodeNet' THEN 'system.serverNode'
    WHEN MDCallName = 'OpenPowerSet' THEN 'system.role'
    WHEN MDCallName = 'OpenDataPack' THEN 'system.dataPack'
    WHEN MDCallName = 'OpenFieldEdr' THEN 'system.fieldEdr'
    WHEN MDCallName = 'OpenGridInfo' THEN 'system.gridInfo'
    WHEN MDCallName = 'OpenPhonePass' THEN 'system.phonePass'
    -- OA
    WHEN MDCallName = 'OpenOA_MyWork' THEN 'oa.myWork'
    WHEN MDCallName = 'OpenOther' THEN 'other'
    WHEN MDCallName = 'OpenAS_Asset' THEN 'asset'
    ELSE PermCode
END
WHERE NULLIF(MDCallName, '') IS NOT NULL
AND PermCode IS NULL;
GO

PRINT '--- PermCode 初始化结果 ---';
SELECT 
    '已设置权限码的菜单' AS Item, COUNT(*) AS Value 
FROM tSys_Menus WHERE NULLIF(PermCode, '') IS NOT NULL
UNION ALL
SELECT '启用但未设置权限码的菜单', COUNT(*) 
FROM tSys_Menus WHERE ISNULL(Used, 'Y') = 'Y' AND NULLIF(PermCode, '') IS NULL AND NULLIF(MDCallName, '') IS NOT NULL;
GO
