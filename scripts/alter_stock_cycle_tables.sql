-- ============================================================================
-- 补全 tStk_StockCycle 字段 + 创建 tStk_StockCycleDetail
-- ============================================================================

-- 补全主表字段
IF NOT EXISTS (SELECT 1 FROM sys.columns WHERE object_id = OBJECT_ID('tStk_StockCycle') AND name = 'CycleID')
    ALTER TABLE [tStk_StockCycle] ADD [CycleID] uniqueidentifier NULL;
GO
IF NOT EXISTS (SELECT 1 FROM sys.columns WHERE object_id = OBJECT_ID('tStk_StockCycle') AND name = 'CycleNo')
    ALTER TABLE [tStk_StockCycle] ADD [CycleNo] nvarchar(50) NULL;
GO
IF NOT EXISTS (SELECT 1 FROM sys.columns WHERE object_id = OBJECT_ID('tStk_StockCycle') AND name = 'CycleDate')
    ALTER TABLE [tStk_StockCycle] ADD [CycleDate] datetime NULL;
GO
IF NOT EXISTS (SELECT 1 FROM sys.columns WHERE object_id = OBJECT_ID('tStk_StockCycle') AND name = 'CycleType')
    ALTER TABLE [tStk_StockCycle] ADD [CycleType] int NOT NULL DEFAULT 3;
GO
IF NOT EXISTS (SELECT 1 FROM sys.columns WHERE object_id = OBJECT_ID('tStk_StockCycle') AND name = 'EmpID')
    ALTER TABLE [tStk_StockCycle] ADD [EmpID] uniqueidentifier NULL;
GO
IF NOT EXISTS (SELECT 1 FROM sys.columns WHERE object_id = OBJECT_ID('tStk_StockCycle') AND name = 'Note')
    ALTER TABLE [tStk_StockCycle] ADD [Note] nvarchar(500) NULL;
GO
IF NOT EXISTS (SELECT 1 FROM sys.columns WHERE object_id = OBJECT_ID('tStk_StockCycle') AND name = 'State')
    ALTER TABLE [tStk_StockCycle] ADD [State] char(1) NOT NULL DEFAULT 'N';
GO
IF NOT EXISTS (SELECT 1 FROM sys.columns WHERE object_id = OBJECT_ID('tStk_StockCycle') AND name = 'LUTime')
    ALTER TABLE [tStk_StockCycle] ADD [LUTime] datetime NULL;
GO
IF NOT EXISTS (SELECT 1 FROM sys.columns WHERE object_id = OBJECT_ID('tStk_StockCycle') AND name = 'EUser')
    ALTER TABLE [tStk_StockCycle] ADD [EUser] uniqueidentifier NULL;
GO
IF NOT EXISTS (SELECT 1 FROM sys.columns WHERE object_id = OBJECT_ID('tStk_StockCycle') AND name = 'EDate')
    ALTER TABLE [tStk_StockCycle] ADD [EDate] datetime NULL;
GO
IF NOT EXISTS (SELECT 1 FROM sys.columns WHERE object_id = OBJECT_ID('tStk_StockCycle') AND name = 'AUser')
    ALTER TABLE [tStk_StockCycle] ADD [AUser] uniqueidentifier NULL;
GO
IF NOT EXISTS (SELECT 1 FROM sys.columns WHERE object_id = OBJECT_ID('tStk_StockCycle') AND name = 'ADate')
    ALTER TABLE [tStk_StockCycle] ADD [ADate] datetime NULL;
GO
IF NOT EXISTS (SELECT 1 FROM sys.columns WHERE object_id = OBJECT_ID('tStk_StockCycle') AND name = 'CrDate')
    ALTER TABLE [tStk_StockCycle] ADD [CrDate] datetime NULL;
GO

-- 创建明细表
IF NOT EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tStk_StockCycleDetail' AND xtype = 'U')
CREATE TABLE [tStk_StockCycleDetail] (
    [CycleDetailID] uniqueidentifier PRIMARY KEY DEFAULT NEWID(),
    [CycleID]       uniqueidentifier NOT NULL,
    [RowNO]         int             NULL,
    [StkID]         uniqueidentifier NULL,
    [GDSID]         uniqueidentifier NULL,
    [GDSNO]         nvarchar(50)    NULL,
    [GDSDesc]       nvarchar(200)   NULL,
    [BarCode]       nvarchar(50)    NULL,
    [UnitNO]        nvarchar(20)    NULL,
    [AccQty]        float           NOT NULL DEFAULT 0,
    [RealQty]       float           NOT NULL DEFAULT 0,
    [DiffQty]       float           NOT NULL DEFAULT 0,
    [AInPrice]      float           NOT NULL DEFAULT 0,
    [Note]          nvarchar(500)   NULL
);
GO

IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tStk_StockCycleDtl_CycleID' AND object_id = OBJECT_ID('tStk_StockCycleDetail'))
    CREATE INDEX [IX_tStk_StockCycleDtl_CycleID] ON [tStk_StockCycleDetail]([CycleID]);
GO
IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tStk_StockCycleDtl_GDSID' AND object_id = OBJECT_ID('tStk_StockCycleDetail'))
    CREATE INDEX [IX_tStk_StockCycleDtl_GDSID] ON [tStk_StockCycleDetail]([GDSID]);
GO

-- 验证
SELECT 'tStk_StockCycle' AS tbl, c.name, t.name AS type FROM sys.columns c JOIN sys.types t ON c.user_type_id = t.user_type_id WHERE c.object_id = OBJECT_ID('tStk_StockCycle')
UNION ALL
SELECT 'tStk_StockCycleDetail', c.name, t.name FROM sys.columns c JOIN sys.types t ON c.user_type_id = t.user_type_id WHERE c.object_id = OBJECT_ID('tStk_StockCycleDetail')
ORDER BY 1, c.column_id;
GO
