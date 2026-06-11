-- ===================================================================
-- 补充单据编号配置 + 关键业务表索引
-- 适用：SQL Server 2016+ / Azure SQL
-- 执行：sqlcmd -S SERVER -d TestERP -U sa -P sa123456 -C -i add_docno_and_indexes.sql
-- 说明：
--   1. 补充 tSys_DocNo 中缺失的单据类型（销售报价、采购报价、调价单、
--      补货申请、周期盘点、批发出库等），使用 IF NOT EXISTS 检查，重复执行安全
--   2. 为高频业务表（tStk_Reserve、tStk_IO、tStk_IODetail、tStk_Stock、
--      tSal_Order、tPur_Order、tPur_Inv/Return、tSal_Quote、tPur_Quote）
--      补充业务查询索引，所有索引均通过 sys.indexes 检查，重复执行安全
-- ===================================================================

SET NOCOUNT ON;
PRINT '=== 补充单据编号与索引开始 ===';
PRINT '';

/* ============================================================
 * 1. 补充 tSys_DocNo 缺失的单据类型
 * ============================================================ */
PRINT '--- 1) tSys_DocNo 缺失单据类型 ---';
GO

-- 销售报价单 (零售)
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'SRQ')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('SRQ', '销售报价单(零售)', 'SRQ', 'tSal_Quote', 'SQNo', 'YYYYMMDD', 4, 1, 'Y', 'DAY', 'Y', '销售报价单(零售)')
PRINT '  SRQ: 销售报价单(零售)';
GO

-- 采购报价单
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'PRQ')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('PRQ', '采购报价单', 'PRQ', 'tPur_Quote', 'PqNo', 'YYYYMMDD', 4, 1, 'Y', 'DAY', 'Y', '采购报价单')
PRINT '  PRQ: 采购报价单';
GO

-- 采购调价单
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'PAP')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('PAP', '采购调价单', 'PAP', 'tPur_AdjPrice', 'PAPNo', 'YYYYMMDD', 4, 1, 'Y', 'DAY', 'Y', '采购调价单')
PRINT '  PAP: 采购调价单';
GO

-- 批发调价单
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'SAP')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('SAP', '批发调价单', 'SAP', 'tSal_AdjPrice', 'SAPNo', 'YYYYMMDD', 4, 1, 'Y', 'DAY', 'Y', '批发调价单')
PRINT '  SAP: 批发调价单';
GO

-- 补货申请
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'RPA')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('RPA', '补货申请', 'RPA', 'tStk_ReplenishApply', 'ReplenishApplyNo', 'YYYYMMDD', 4, 1, 'Y', 'DAY', 'Y', '补货申请单')
PRINT '  RPA: 补货申请';
GO

-- 周期盘点
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'CYC')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('CYC', '周期盘点', 'CYC', 'tStk_StockCycle', 'CycleNo', 'YYYYMMDD', 4, 1, 'Y', 'DAY', 'Y', '库存周期盘点单')
PRINT '  CYC: 周期盘点';
GO

-- 批发出库 (WSO 前缀)
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'WSO')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('WSO', '批发出库', 'WSO', 'tStk_IO', 'IONo', 'YYYYMMDD', 4, 1, 'Y', 'DAY', 'Y', '批发出库单')
PRINT '  WSO: 批发出库';
GO

/* ============================================================
 * 2. 关键业务表补充索引
 * ============================================================ */
PRINT '--- 2) 关键业务表索引 ---';
GO

-- 2.1 tStk_Reserve (预留/占用) - 按单据类型+单据ID+商品+仓库查询
IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tStk_Reserve_DocType_DocID' AND object_id = OBJECT_ID('tStk_Reserve'))
    CREATE NONCLUSTERED INDEX IX_tStk_Reserve_DocType_DocID
        ON tStk_Reserve (DocType, DocID, GDSID, StkID) INCLUDE (Qty, ReleasedQty, State)
PRINT '  IX_tStk_Reserve_DocType_DocID: 预留占用按单据+商品+仓库查询';
GO

-- 2.2 tStk_IODetail - 上游单据追溯（SouID 是源单主键）
IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tStk_IODetail_SouID' AND object_id = OBJECT_ID('tStk_IODetail'))
    CREATE NONCLUSTERED INDEX IX_tStk_IODetail_SouID
        ON tStk_IODetail (IOID, SouID) INCLUDE (GDSID, StkID, Qty, Price, Amt)
PRINT '  IX_tStk_IODetail_SouID: 入出库明细按上游单据追溯';
GO

-- 2.3 tStk_IO - 业务类型+业务ID+仓库+日期组合查询
IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tStk_IO_Kind_BTPID_StkID' AND object_id = OBJECT_ID('tStk_IO'))
    CREATE NONCLUSTERED INDEX IX_tStk_IO_Kind_BTPID_StkID
        ON tStk_IO (Kind, BTPID, StkID, IoDate) INCLUDE (IOID, IONo, State)
PRINT '  IX_tStk_IO_Kind_BTPID_StkID: 入出库单按类型+业务+仓库+日期查询';
GO

-- 2.4 tStk_IO - 单号精确查询
IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tStk_IO_IONo' AND object_id = OBJECT_ID('tStk_IO'))
    CREATE NONCLUSTERED INDEX IX_tStk_IO_IONo
        ON tStk_IO (IONo)
PRINT '  IX_tStk_IO_IONo: 入出库单号精确查询';
GO

-- 2.5 tStk_Stock (商品库存余额) - 按商品+仓库查询
IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tStk_Stock_GDSID_StkID' AND object_id = OBJECT_ID('tStk_Stock'))
    CREATE NONCLUSTERED INDEX IX_tStk_Stock_GDSID_StkID
        ON tStk_Stock (GDSID, StkID) INCLUDE (Qty, QQty)
PRINT '  IX_tStk_Stock_GDSID_StkID: 商品库存按商品+仓库查询';
GO

-- 2.6 tSal_Order - 单号精确查询
IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tSal_Order_SoNo' AND object_id = OBJECT_ID('tSal_Order'))
    CREATE NONCLUSTERED INDEX IX_tSal_Order_SoNo
        ON tSal_Order (SoNo)
PRINT '  IX_tSal_Order_SoNo: 销售订单号精确查询';
GO

-- 2.7 tSal_Order - 业务+客户+日期组合查询
IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tSal_Order_BTPID' AND object_id = OBJECT_ID('tSal_Order'))
    CREATE NONCLUSTERED INDEX IX_tSal_Order_BTPID
        ON tSal_Order (BTPID, CustID, SoDate) INCLUDE (SOID, SoNo, State, SumAmt)
PRINT '  IX_tSal_Order_BTPID: 销售订单按业务+客户+日期查询';
GO

-- 2.8 tPur_Order - 业务+供应商+日期组合查询
IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tPur_Order_BTPID' AND object_id = OBJECT_ID('tPur_Order'))
    CREATE NONCLUSTERED INDEX IX_tPur_Order_BTPID
        ON tPur_Order (BTPID, SuppID, PoDate) INCLUDE (POID, PoNo, State, SumAmt)
PRINT '  IX_tPur_Order_BTPID: 采购订单按业务+供应商+日期查询';
GO

-- 2.9 tPur_Inv / tPur_Return - 单号精确查询
IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tPur_Inv_PiNo' AND object_id = OBJECT_ID('tPur_Inv'))
    CREATE NONCLUSTERED INDEX IX_tPur_Inv_PiNo
        ON tPur_Inv (PiNo)
PRINT '  IX_tPur_Inv_PiNo: 采购入库单号精确查询';
GO

IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tPur_Return_PrNo' AND object_id = OBJECT_ID('tPur_Return'))
    CREATE NONCLUSTERED INDEX IX_tPur_Return_PrNo
        ON tPur_Return (PrNo)
PRINT '  IX_tPur_Return_PrNo: 采购退货单号精确查询';
GO

-- 2.10 tSal_Quote / tPur_Quote - 单号精确查询
IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tSal_Quote_SQNo' AND object_id = OBJECT_ID('tSal_Quote'))
    CREATE NONCLUSTERED INDEX IX_tSal_Quote_SQNo
        ON tSal_Quote (SQNo)
PRINT '  IX_tSal_Quote_SQNo: 销售报价单号精确查询';
GO

IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tPur_Quote_PqNo' AND object_id = OBJECT_ID('tPur_Quote'))
    CREATE NONCLUSTERED INDEX IX_tPur_Quote_PqNo
        ON tPur_Quote (PqNo)
PRINT '  IX_tPur_Quote_PqNo: 采购报价单号精确查询';
GO

PRINT '';
PRINT '=== 补充单据编号与索引完成 ===';
GO
