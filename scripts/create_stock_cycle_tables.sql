-- ============================================================================
-- tStk_StockCycle / tStk_StockCycleDetail — 周期盘点主表 + 明细表
-- 字段与前端 StockCycle.vue columns + DocDetailTable check-mode 对齐：
--   主表: CycleID/CycleNo/CycleDate/CycleType/StkID/BStkID/EmpID/Cycle/Note/State
--   明细: CycleDetailID/CycleID/RowNO/GDSID/GDSNO/GDSDesc/BarCode/UnitNO
--         AccQty(账存)/RealQty(实存)/DiffQty(差异)/AInPrice(成本价)/Note
-- 后端 post_stock_cycle 按 DiffQty 过账（与 post_stock_tran 一致）
-- ============================================================================

-- 1. 周期盘点主表
IF NOT EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tStk_StockCycle' AND xtype = 'U')
BEGIN
    CREATE TABLE [tStk_StockCycle] (
        [CycleID]    uniqueidentifier PRIMARY KEY DEFAULT NEWID(),
        [CycleNo]    nvarchar(50)     NULL,             -- 盘点单号
        [CycleDate]  datetime         NULL,             -- 盘点日期
        [CycleType]  int              NOT NULL DEFAULT 3, -- 1日盘/2周盘/3月盘/4季盘/5年盘
        [StkID]      uniqueidentifier NULL,             -- 仓库
        [BStkID]     uniqueidentifier NULL,             -- 基准仓库
        [EmpID]      uniqueidentifier NULL,             -- 盘点人
        [Cycle]      int              NULL DEFAULT 30,  -- 周期天数
        [Note]       nvarchar(500)    NULL,             -- 备注
        [State]      char(1)          NOT NULL DEFAULT 'N', -- N=新建 S=已审核 D=删除 C=已作废
        [LUTime]     datetime         NULL DEFAULT GETDATE(),
        [EUser]      uniqueidentifier NULL,             -- 创建人 EmpID
        [EDate]      datetime         NULL DEFAULT GETDATE(),
        [AUser]      uniqueidentifier NULL,             -- 审核人 EmpID
        [ADate]      datetime         NULL,
        [CrDate]     datetime         NULL DEFAULT GETDATE()  -- 创建时间（前端用 CrDate）
    );

    CREATE INDEX [IX_tStk_StockCycle_StkID]     ON [tStk_StockCycle]([StkID]);
    CREATE INDEX [IX_tStk_StockCycle_CycleDate] ON [tStk_StockCycle]([CycleDate]);
    CREATE INDEX [IX_tStk_StockCycle_State]     ON [tStk_StockCycle]([State]);
    CREATE INDEX [IX_tStk_StockCycle_EUser]     ON [tStk_StockCycle]([EUser]);

    PRINT 'OK: tStk_StockCycle created';
END
ELSE
    PRINT 'SKIP: tStk_StockCycle already exists';
GO

-- 2. 周期盘点明细表（字段参考 tStk_TranDetail）
IF NOT EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tStk_StockCycleDetail' AND xtype = 'U')
BEGIN
    CREATE TABLE [tStk_StockCycleDetail] (
        [CycleDetailID] uniqueidentifier PRIMARY KEY DEFAULT NEWID(),
        [CycleID]       uniqueidentifier NOT NULL,     -- 关联主表
        [RowNO]         int             NULL,          -- 行号
        [StkID]         uniqueidentifier NULL,         -- 仓库（冗余，便于查询）
        [GDSID]         uniqueidentifier NULL,         -- 商品 ID
        [GDSNO]         nvarchar(50)    NULL,          -- 商品编码（冗余）
        [GDSDesc]       nvarchar(200)   NULL,          -- 商品名称（冗余）
        [BarCode]       nvarchar(50)    NULL,          -- 条码（冗余）
        [UnitNO]        nvarchar(20)    NULL,          -- 单位
        [AccQty]        float           NOT NULL DEFAULT 0,  -- 账存（前端 DocDetailTable check-mode 用）
        [RealQty]       float           NOT NULL DEFAULT 0,  -- 实存
        [DiffQty]       float           NOT NULL DEFAULT 0,  -- 差异 = RealQty - AccQty（后端过账用）
        [AInPrice]      float           NOT NULL DEFAULT 0,  -- 成本价
        [Note]          nvarchar(500)   NULL                 -- 备注
    );

    CREATE INDEX [IX_tStk_StockCycleDtl_CycleID] ON [tStk_StockCycleDetail]([CycleID]);
    CREATE INDEX [IX_tStk_StockCycleDtl_GDSID]   ON [tStk_StockCycleDetail]([GDSID]);

    PRINT 'OK: tStk_StockCycleDetail created';
END
ELSE
    PRINT 'SKIP: tStk_StockCycleDetail already exists';
GO

-- 验证
SELECT c.name AS column_name, t.name AS type_name, c.is_nullable
FROM sys.columns c
JOIN sys.types t ON c.user_type_id = t.user_type_id
WHERE c.object_id = OBJECT_ID('tStk_StockCycle')
ORDER BY c.column_id;
GO

SELECT c.name AS column_name, t.name AS type_name, c.is_nullable
FROM sys.columns c
JOIN sys.types t ON c.user_type_id = t.user_type_id
WHERE c.object_id = OBJECT_ID('tStk_StockCycleDetail')
ORDER BY c.column_id;
GO
