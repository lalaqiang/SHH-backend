-- 重建 tSys_Menus 表为新结构
-- 与前端 router/index.js 和 stores/app.js menuData 完全对齐
-- 生成时间: 2026-07-05

-- 备份旧菜单数据（按 Used=Y 过滤，只备份启用的）
IF NOT EXISTS (SELECT * FROM sys.tables WHERE name = 'tSys_Menus_BAK')
BEGIN
    SELECT * INTO tSys_Menus_BAK FROM tSys_Menus;
    PRINT '已备份旧菜单到 tSys_Menus_BAK';
END
GO

-- 清空旧菜单数据
DELETE FROM tSys_Menus;
PRINT '已清空旧菜单数据';
GO

-- 插入新结构菜单数据（使用事务，无 GO 分隔以保持事务上下文）
BEGIN TRANSACTION;

-- 模块: 基础资料 (base)
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('a87d6cc6-a1a2-581c-80fb-63f0ba47b633', '00000000-0000-0000-0000-000000000000', N'基础资料', N'001', N'/base/product', 0, N'Y', N'base', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('81e181a6-77f1-5dcf-813e-75e459f35a94', 'a87d6cc6-a1a2-581c-80fb-63f0ba47b633', N'商品管理', N'0010', N'/base/product', 0, N'Y', N'base.goods', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('893f4536-3182-594a-85b1-d05f53e7f43a', 'a87d6cc6-a1a2-581c-80fb-63f0ba47b633', N'商品分类', N'0020', N'/base/gds-type', 0, N'Y', N'base.gdsType', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('9c5f090c-c68c-5aa3-b080-22c685363000', 'a87d6cc6-a1a2-581c-80fb-63f0ba47b633', N'品牌管理', N'0030', N'/base/brand', 0, N'Y', N'base.brand', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('2f66be3d-e139-5ab5-a2e9-7ab03ae1c093', 'a87d6cc6-a1a2-581c-80fb-63f0ba47b633', N'仓库管理', N'0040', N'/base/warehouse', 0, N'Y', N'base.warehouse', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('618ca2ad-577b-5eb9-b729-39d946091a5f', 'a87d6cc6-a1a2-581c-80fb-63f0ba47b633', N'单位管理', N'0050', N'/base/unit', 0, N'Y', N'base.unit', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('d3dd74a8-d032-52bb-b007-d21dbcab6e2f', 'a87d6cc6-a1a2-581c-80fb-63f0ba47b633', N'供应商管理', N'0060', N'/base/supplier', 0, N'Y', N'base.supplier', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('06eb3dbf-81be-5840-8a20-e4f8949e9ccd', 'a87d6cc6-a1a2-581c-80fb-63f0ba47b633', N'客户管理', N'0070', N'/base/customer', 0, N'Y', N'base.customer', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('c25e3551-a439-53f3-8448-828fab0fabda', 'a87d6cc6-a1a2-581c-80fb-63f0ba47b633', N'员工管理', N'0080', N'/base/employee', 0, N'Y', N'base.employee', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('4b4b7917-3a3f-5d0d-adfa-5b7cf38761a6', 'a87d6cc6-a1a2-581c-80fb-63f0ba47b633', N'客户定价', N'0090', N'/base/pricing-template', 0, N'Y', N'base.pricingTemplate', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('26b84039-78d0-50d9-990a-565dbe37f6b0', 'a87d6cc6-a1a2-581c-80fb-63f0ba47b633', N'提成模板', N'0100', N'/base/commission-template', 0, N'Y', N'base.commissionTemplate', N'0');

PRINT N'已插入模块: 基础资料 (10 个子菜单)';

-- 模块: 单据管理 (document)
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('6d25e5bc-6dae-51c1-9f69-4d7911de4922', '00000000-0000-0000-0000-000000000000', N'单据管理', N'002', N'/purchase/order', 0, N'Y', N'document', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('fbb9c533-3367-55db-ae91-a5ec77834554', '6d25e5bc-6dae-51c1-9f69-4d7911de4922', N'采购订单', N'0010', N'/purchase/order', 0, N'Y', N'purchase.order', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('10b11f0e-fb32-5459-ac8b-a4e0b9fa34b8', '6d25e5bc-6dae-51c1-9f69-4d7911de4922', N'采购收货', N'0020', N'/purchase/receipt', 0, N'Y', N'purchase.receipt', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('c87e33b4-a7b7-594e-930a-aaaf4449d660', '6d25e5bc-6dae-51c1-9f69-4d7911de4922', N'采购退货', N'0030', N'/purchase/return', 0, N'Y', N'purchase.return', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('0a7872b1-8bea-5a64-845d-17cf2c454aec', '6d25e5bc-6dae-51c1-9f69-4d7911de4922', N'采购报价', N'0040', N'/purchase/quote', 0, N'Y', N'purchase.quote', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('07ee6eba-3777-56a8-a8d1-42d14d60d136', '6d25e5bc-6dae-51c1-9f69-4d7911de4922', N'采购调价', N'0050', N'/purchase/adjprice', 0, N'Y', N'purchase.adjPrice', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('6a173f86-39bd-53bb-b01b-94a2fac7286b', '6d25e5bc-6dae-51c1-9f69-4d7911de4922', N'销售订单', N'0060', N'/sales/order', 0, N'Y', N'sales.order', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('9197e8a9-3bd0-56fa-be20-e59ea982babb', '6d25e5bc-6dae-51c1-9f69-4d7911de4922', N'销售出库', N'0070', N'/sales/outbound', 0, N'Y', N'sales.outbound', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('994a683c-8d2b-572d-8054-f51c279a6a6c', '6d25e5bc-6dae-51c1-9f69-4d7911de4922', N'销售退货', N'0080', N'/sales/return', 0, N'Y', N'sales.return', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('533368bd-09b3-51b3-9346-d1e8614d9e85', '6d25e5bc-6dae-51c1-9f69-4d7911de4922', N'销售报价', N'0090', N'/sales/quote', 0, N'Y', N'sales.quote', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('b7ba573c-dc62-5c5c-bd1e-aef42198e333', '6d25e5bc-6dae-51c1-9f69-4d7911de4922', N'会员管理', N'0100', N'/sales/vip', 0, N'Y', N'sales.vip', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('ef0a8620-05a5-5418-99f8-897ff6a51496', '6d25e5bc-6dae-51c1-9f69-4d7911de4922', N'门店直配', N'0110', N'/inventory/zp-delivery', 0, N'Y', N'inventory.zpDelivery', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('c8918706-df83-5c48-a93a-d277d5539ad8', '6d25e5bc-6dae-51c1-9f69-4d7911de4922', N'门店退仓', N'0120', N'/inventory/store-return', 0, N'Y', N'inventory.storeReturn', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('4fbe169b-3640-5df3-b254-b6064099784e', '6d25e5bc-6dae-51c1-9f69-4d7911de4922', N'调拨单', N'0130', N'/inventory/move', 0, N'Y', N'inventory.move', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('8c8771db-c6a6-57ca-8bf0-1a64727582d6', '6d25e5bc-6dae-51c1-9f69-4d7911de4922', N'门店销售', N'0140', N'/retail/sale', 0, N'Y', N'retail.sale', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('5b3d968c-ca4c-5e73-807a-a8622fbb7b98', '6d25e5bc-6dae-51c1-9f69-4d7911de4922', N'收银台', N'0150', N'/retail/cashier', 0, N'Y', N'retail.cashier', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('d9c7250b-3b92-535d-93f8-18c1b2d7dc5d', '6d25e5bc-6dae-51c1-9f69-4d7911de4922', N'入出库单', N'0160', N'/inventory/io', 0, N'Y', N'inventory.io', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('87f1a35d-7bf5-583a-af1f-e1e377a253d7', '6d25e5bc-6dae-51c1-9f69-4d7911de4922', N'零散入库', N'0170', N'/inventory/oti-inbound', 0, N'Y', N'inventory.otiInbound', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('825adf18-53e5-5190-86d5-234390cea72a', '6d25e5bc-6dae-51c1-9f69-4d7911de4922', N'零散出库', N'0180', N'/inventory/oto-outbound', 0, N'Y', N'inventory.otoOutbound', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('da5319aa-2965-52a5-a72a-40b0da6a4657', '6d25e5bc-6dae-51c1-9f69-4d7911de4922', N'领用单', N'0190', N'/inventory/requisition', 0, N'Y', N'inventory.requisition', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('40b13b7d-ae2c-5a39-b7fe-54da84d3e63c', '6d25e5bc-6dae-51c1-9f69-4d7911de4922', N'盘点单', N'0200', N'/inventory/check', 0, N'Y', N'inventory.check', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('78739aee-9581-54b6-8094-0e296a66d92b', '6d25e5bc-6dae-51c1-9f69-4d7911de4922', N'单据关系图', N'0210', N'/document/relation-graph', 0, N'Y', N'document.relationGraph', N'0');

PRINT N'已插入模块: 单据管理 (21 个子菜单)';

-- 模块: 批发业务 (wholesale)
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('be84fa1d-13db-54b4-957f-17dec65ff96f', '00000000-0000-0000-0000-000000000000', N'批发业务', N'003', N'/wholesale/order', 0, N'Y', N'wholesale', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('bf68a155-b918-50a3-aeda-315ef49f877f', 'be84fa1d-13db-54b4-957f-17dec65ff96f', N'批发订单', N'0010', N'/wholesale/order', 0, N'Y', N'wholesale.order', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('be9e692b-72ca-56f7-ae18-d383856347a8', 'be84fa1d-13db-54b4-957f-17dec65ff96f', N'批发报价', N'0020', N'/wholesale/quote', 0, N'Y', N'wholesale.quote', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('f4161f2d-958b-5131-8df3-62d1053d4d88', 'be84fa1d-13db-54b4-957f-17dec65ff96f', N'批发出库', N'0030', N'/wholesale/outbound', 0, N'Y', N'wholesale.outbound', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('46bf0311-3454-5498-a009-499556b5522f', 'be84fa1d-13db-54b4-957f-17dec65ff96f', N'批发退货', N'0040', N'/wholesale/return', 0, N'Y', N'wholesale.return', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('7849308f-843a-5d9c-b9ea-92fc8fcd2258', 'be84fa1d-13db-54b4-957f-17dec65ff96f', N'批发调价', N'0050', N'/wholesale/adjprice', 0, N'Y', N'wholesale.adjPrice', N'0');

PRINT N'已插入模块: 批发业务 (5 个子菜单)';

-- 模块: 库存管理 (inventory)
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('c88a3610-58c8-5299-bd97-fcf07e5a2dd1', '00000000-0000-0000-0000-000000000000', N'库存管理', N'004', N'/inventory/stock', 0, N'Y', N'inventory', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('5309d41a-884f-5c1c-b905-41ab9bfabe98', 'c88a3610-58c8-5299-bd97-fcf07e5a2dd1', N'库存查询', N'0010', N'/inventory/stock', 0, N'Y', N'inventory.stock', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('18d36976-6f84-5423-9627-d8de6b7f0cce', 'c88a3610-58c8-5299-bd97-fcf07e5a2dd1', N'库存流水', N'0020', N'/inventory/flows', 0, N'Y', N'inventory.flows', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('9ebabf25-4898-572d-b76f-3966ccfc5024', 'c88a3610-58c8-5299-bd97-fcf07e5a2dd1', N'库存预警', N'0030', N'/inventory/alerts', 0, N'Y', N'inventory.alerts', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('c181c258-0807-51b9-9a30-69e40840644b', 'c88a3610-58c8-5299-bd97-fcf07e5a2dd1', N'缺货记录', N'0040', N'/inventory/shortages', 0, N'Y', N'inventory.shortages', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('54234d81-276b-5647-95b1-45d6ad3c6140', 'c88a3610-58c8-5299-bd97-fcf07e5a2dd1', N'库存调整', N'0050', N'/inventory/adjust', 0, N'Y', N'inventory.adjust', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('3378f27a-bdba-5815-89dc-582a31b593a6', 'c88a3610-58c8-5299-bd97-fcf07e5a2dd1', N'补货申请', N'0060', N'/inventory/replenish', 0, N'Y', N'inventory.replenish', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('4f867878-3bd9-5d80-a153-12da16c60d8b', 'c88a3610-58c8-5299-bd97-fcf07e5a2dd1', N'周期盘点', N'0070', N'/inventory/stock-cycle', 0, N'Y', N'inventory.stockCycle', N'0');

PRINT N'已插入模块: 库存管理 (7 个子菜单)';

-- 模块: 财务管理 (finance)
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('9a696a96-493c-55be-af11-9fd5bc0d6182', '00000000-0000-0000-0000-000000000000', N'财务管理', N'005', N'/finance/receipts', 0, N'Y', N'finance', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('6dc5ee85-8918-597c-ab17-f79e5992bec3', '9a696a96-493c-55be-af11-9fd5bc0d6182', N'收款单', N'0010', N'/finance/receipts', 0, N'Y', N'finance.receipts', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('cfcf9421-75e2-5bbc-979d-022e2fba33be', '9a696a96-493c-55be-af11-9fd5bc0d6182', N'付款单', N'0020', N'/finance/payments', 0, N'Y', N'finance.payments', N'0');

PRINT N'已插入模块: 财务管理 (2 个子菜单)';

-- 模块: 报表中心 (report)
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('a2405d4f-2c60-5a93-9c19-bac91c8db3bf', '00000000-0000-0000-0000-000000000000', N'报表中心', N'006', N'/report/sales-report', 0, N'Y', N'report', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('b247c719-21be-56c6-aa47-5dd47a9af3bd', 'a2405d4f-2c60-5a93-9c19-bac91c8db3bf', N'销售报表', N'0010', N'/report/sales-report', 0, N'Y', N'report.salesReport', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('631949cd-532e-5f45-b36d-973eee511a21', 'a2405d4f-2c60-5a93-9c19-bac91c8db3bf', N'采购报表', N'0020', N'/report/purchase-report', 0, N'Y', N'report.purchaseReport', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('2f41e6a0-8890-52ab-9941-6f8c9f970b75', 'a2405d4f-2c60-5a93-9c19-bac91c8db3bf', N'库存报表', N'0030', N'/report/inventory-report', 0, N'Y', N'report.inventoryReport', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('f34e634c-e99b-56ef-b6d9-3b8c32deab5d', 'a2405d4f-2c60-5a93-9c19-bac91c8db3bf', N'利润报表', N'0040', N'/decision/profit-analysis', 0, N'Y', N'report.profitAnalysis', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('9996fda6-7e2a-5259-ae25-a5dd07eaabe1', 'a2405d4f-2c60-5a93-9c19-bac91c8db3bf', N'提成报表', N'0050', N'/report/commission-report', 0, N'Y', N'report.commissionReport', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('894ced3a-a810-56c8-8275-a4de9f0a4efe', 'a2405d4f-2c60-5a93-9c19-bac91c8db3bf', N'提成明细', N'0060', N'/report/commission-details', 0, N'Y', N'report.commissionDetails', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('247e6bef-0f32-5fbf-b884-a53a20893df1', 'a2405d4f-2c60-5a93-9c19-bac91c8db3bf', N'财务报表', N'0070', N'/report/finance', 0, N'Y', N'report.finance', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('c15ced0d-2304-5d24-8cda-e580cd5a2633', 'a2405d4f-2c60-5a93-9c19-bac91c8db3bf', N'销售任务报表', N'0080', N'/report/sales-task', 0, N'Y', N'report.salesTask', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('c90424a4-792d-59b1-8012-d843a9d8ef18', 'a2405d4f-2c60-5a93-9c19-bac91c8db3bf', N'销售分析', N'0090', N'/decision/sales-analysis', 0, N'Y', N'report.salesAnalysis', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('e2b391f1-97ff-5591-8253-1d93564c9462', 'a2405d4f-2c60-5a93-9c19-bac91c8db3bf', N'采购分析', N'0100', N'/decision/purchase-analysis', 0, N'Y', N'report.purchaseAnalysis', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('deb7d73e-65c5-562c-9ac4-5d419223a782', 'a2405d4f-2c60-5a93-9c19-bac91c8db3bf', N'业务报表', N'0110', N'/report/business-report', 0, N'Y', N'report.businessReport', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('621fbb4b-9597-596c-a022-16f960aaa621', 'a2405d4f-2c60-5a93-9c19-bac91c8db3bf', N'自定义报表', N'0120', N'/report/custom', 0, N'Y', N'report.custom', N'0');

PRINT N'已插入模块: 报表中心 (12 个子菜单)';

-- 模块: 手机数据 (mobile-data)
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('3e00451b-84b9-5b3d-8060-d8dabaa6924b', '00000000-0000-0000-0000-000000000000', N'手机数据', N'007', N'/mobile-data', 0, N'Y', N'mobile', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('799c1040-10af-5997-8284-3f9cf1c325e4', '3e00451b-84b9-5b3d-8060-d8dabaa6924b', N'补货申请', N'0010', N'/mobile-data/replenishment', 0, N'Y', N'mobile.replenishment', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('19d1a5ce-61b6-5865-a3aa-be64c32dc4b5', '3e00451b-84b9-5b3d-8060-d8dabaa6924b', N'赠品赠送', N'0020', N'/mobile-data/gift-giving', 0, N'Y', N'mobile.giftGiving', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('8c96634a-bd8d-5591-8442-d381860382e4', '3e00451b-84b9-5b3d-8060-d8dabaa6924b', N'手机盘点', N'0030', N'/mobile-data/stock-check', 0, N'Y', N'mobile.stockCheck', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('e2593dc3-b7f1-56b4-bef2-4d3fe73a5a32', '3e00451b-84b9-5b3d-8060-d8dabaa6924b', N'特价申请', N'0040', N'/mobile-data/special-price', 0, N'Y', N'mobile.specialPrice', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('11ab1e3c-8140-52c6-b1b2-f6356ec8e2f5', '3e00451b-84b9-5b3d-8060-d8dabaa6924b', N'奖励产品', N'0050', N'/mobile-data/reward-product', 0, N'Y', N'mobile.rewardProduct', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('e6592626-314c-5a1b-9da8-09eb6ac1cefc', '3e00451b-84b9-5b3d-8060-d8dabaa6924b', N'销售任务', N'0060', N'/sales-task', 0, N'Y', N'mobile.salesTask', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('e022c1ff-2e6d-5802-baf8-ee5d54b92b64', '3e00451b-84b9-5b3d-8060-d8dabaa6924b', N'员工销量录入', N'0070', N'/sales/input', 0, N'Y', N'mobile.salesInput', N'0');

PRINT N'已插入模块: 手机数据 (7 个子菜单)';

-- 模块: 线上商城 (online)
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('5737664a-f7b1-5f75-8406-845448872a23', '00000000-0000-0000-0000-000000000000', N'线上商城', N'008', N'/online/shop', 0, N'Y', N'online', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('ade00a60-4786-5ba5-aec0-3336f1073659', '5737664a-f7b1-5f75-8406-845448872a23', N'线上下单', N'0010', N'/online/shop', 0, N'Y', N'online.shop', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('a3e472b3-f5cd-5610-9308-c06c8a0df80c', '5737664a-f7b1-5f75-8406-845448872a23', N'我的订单', N'0020', N'/online/my-orders', 0, N'Y', N'online.myOrders', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('bb33ee27-ad1e-5417-8d5f-33fd06b623ce', '5737664a-f7b1-5f75-8406-845448872a23', N'地址库', N'0030', N'/online/address-book', 0, N'Y', N'online.addressBook', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('f83fc5fd-d696-582c-81b9-5935d9621f7f', '5737664a-f7b1-5f75-8406-845448872a23', N'商城管理', N'0040', N'/online/manage', 0, N'Y', N'online.manage', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('b2b93de6-7b84-5709-a6c7-0568f527b022', '5737664a-f7b1-5f75-8406-845448872a23', N'支付配置', N'0050', N'/online/payment-configs', 0, N'Y', N'online.paymentConfigs', N'0');

PRINT N'已插入模块: 线上商城 (5 个子菜单)';

-- 模块: 系统设置 (system)
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('374e5e1a-8258-581e-8003-1880e7d3bd0c', '00000000-0000-0000-0000-000000000000', N'系统设置', N'009', N'/system/overview', 0, N'Y', N'system', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('151344b8-0ccf-5b54-837d-1d86446dab1d', '374e5e1a-8258-581e-8003-1880e7d3bd0c', N'系统总览', N'0010', N'/system/overview', 0, N'Y', N'system.overview', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('0ec35a5a-692b-58ad-a5b7-de3e076da166', '374e5e1a-8258-581e-8003-1880e7d3bd0c', N'系统配置', N'0020', N'/system/config', 0, N'Y', N'system.config', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('dbf93703-82df-5829-bf4a-03c6ff9ecc12', '374e5e1a-8258-581e-8003-1880e7d3bd0c', N'用户管理', N'0030', N'/system/user-management', 0, N'Y', N'system.user', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('d0709afc-729a-51c7-88e0-8ff840084846', '374e5e1a-8258-581e-8003-1880e7d3bd0c', N'角色权限', N'0040', N'/system/role-management', 0, N'Y', N'system.role', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('d0e8cbbb-821c-50ec-8932-b0a68b305918', '374e5e1a-8258-581e-8003-1880e7d3bd0c', N'菜单管理', N'0050', N'/system/menu-management', 0, N'Y', N'system.menu', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('25e10c5f-f914-5636-96fa-f5bde0db0d6d', '374e5e1a-8258-581e-8003-1880e7d3bd0c', N'操作日志', N'0060', N'/system/oper-log', 0, N'Y', N'system.operLog', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('64c03d0a-1ca4-5cb8-86c0-ff597dc17e8f', '374e5e1a-8258-581e-8003-1880e7d3bd0c', N'打印模板', N'0070', N'/system/print-template', 0, N'Y', N'system.printTemplate', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('4672cc9e-8b84-53d0-a77b-c6b258d7c905', '374e5e1a-8258-581e-8003-1880e7d3bd0c', N'打印日志', N'0080', N'/system/print-log', 0, N'Y', N'system.printLog', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('1ee1359f-7848-570e-9f4f-c57fa4092a43', '374e5e1a-8258-581e-8003-1880e7d3bd0c', N'数据导入导出', N'0090', N'/system/import-export', 0, N'Y', N'system.importExport', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('02d9959e-f914-5b7a-8b29-0cd511f471be', '374e5e1a-8258-581e-8003-1880e7d3bd0c', N'通知中心', N'0100', N'/system/notification', 0, N'Y', N'system.notification', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('00bc32d1-ceb0-50df-a16a-b7d994340e78', '374e5e1a-8258-581e-8003-1880e7d3bd0c', N'数据字典', N'0110', N'/system/dictionary', 0, N'Y', N'system.dictionary', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('01c9c583-504c-588f-abdf-a50e32974c7f', '374e5e1a-8258-581e-8003-1880e7d3bd0c', N'系统参数', N'0120', N'/system/params', 0, N'Y', N'system.params', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('7e10ec8a-4aed-5cf9-95d5-fd077b7a8697', '374e5e1a-8258-581e-8003-1880e7d3bd0c', N'数据备份', N'0130', N'/system/backup', 0, N'Y', N'system.backup', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('0b0338cf-2d0d-5f5d-8a90-cde9b5bc13b2', '374e5e1a-8258-581e-8003-1880e7d3bd0c', N'修改密码', N'0140', N'/user/change-password', 0, N'Y', N'system.changePassword', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('83e8f1d5-43fc-5ccc-ba8f-86e1ff8497c8', '374e5e1a-8258-581e-8003-1880e7d3bd0c', N'流程审批', N'0150', N'/oa/workflow', 0, N'Y', N'oa.workflow', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('cac7683a-4265-5829-a40e-4cfcf388fc14', '374e5e1a-8258-581e-8003-1880e7d3bd0c', N'公告通知', N'0160', N'/oa/notice', 0, N'Y', N'oa.notice', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('8b90d3cf-df70-5fea-8ce0-4bec81847bcf', '374e5e1a-8258-581e-8003-1880e7d3bd0c', N'OA邮件', N'0170', N'/oa/email', 0, N'Y', N'oa.email', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('60146263-8f77-5dfc-b309-ff7eec626a2a', '374e5e1a-8258-581e-8003-1880e7d3bd0c', N'IT报表', N'0180', N'/it/report', 0, N'Y', N'it.report', N'0');
INSERT INTO tSys_Menus (SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, SYM_PPT, Used, PermCode, Flg)
VALUES ('8642cf49-f2e8-5fa9-b854-eb6e484c2f97', '374e5e1a-8258-581e-8003-1880e7d3bd0c', N'综合查询', N'0190', N'/query/index', 0, N'Y', N'query.index', N'0');

PRINT N'已插入模块: 系统设置 (19 个子菜单)';

COMMIT TRANSACTION;

PRINT N'菜单重建完成: 9 个顶级模块, 88 个子菜单, 共 97 条记录';