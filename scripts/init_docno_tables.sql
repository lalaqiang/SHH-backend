-- ============================================================
-- 单据编号自增相关表
-- 适用 SQL Server 2005+
-- 说明:
--   tSys_DocNo      配置每种单据类型的编号规则（仅供参考/校验，实际生成由后端 doc_no.rs 完成）
--   tSys_DocNoSeq   单据序号记录表（doc_no.rs 实际使用此表，UPDATE...OUTPUT 原子自增）
--
-- ★ 重要：后端 server-rust/src/utils/doc_no.rs 的 generate_via_docnoseq
--   只使用 tSys_DocNoSeq 表（DocTypeID=前缀, PeriodKey=YYMM, CurrentSeq 自增）
--   不读取 tSys_DocNo 配置表。tSys_DocNo 仅用于文档/校验目的。
--   PeriodKey 格式为 YYMM（按月重置），与 doc_no.rs 的 chrono format("%y%m") 一致。
-- ============================================================

USE [TestERP]
GO

-- 1. 单据编号配置表（仅文档/校验用途，后端不读取）
IF NOT EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tSys_DocNo' AND xtype = 'U')
BEGIN
    CREATE TABLE tSys_DocNo (
        DocTypeID    nvarchar(30)  NOT NULL PRIMARY KEY,         -- 业务类型编码=单据前缀, 如 SO/PO/MV
        DocName      nvarchar(50)  NOT NULL DEFAULT '',           -- 中文名称
        Prefix       nvarchar(20)  NOT NULL DEFAULT '',           -- 单据前缀（与 DocTypeID 一致）
        TableName    nvarchar(60)  NOT NULL DEFAULT '',           -- 实际数据表名
        FieldName    nvarchar(60)  NOT NULL DEFAULT '',           -- 单据号字段名
        DateFormat   nvarchar(20)  NOT NULL DEFAULT 'YYMM',       -- 日期段格式: 后端固定 YYMM
        SeqPadding   int           NOT NULL DEFAULT 4,            -- 序号位数, 后端固定 4
        SeqStart     int           NOT NULL DEFAULT 1,            -- 序号起始值
        DateReset    char(1)       NOT NULL DEFAULT 'Y',          -- Y=按月重置
        PeriodType   nvarchar(10)  NOT NULL DEFAULT 'MONTH',      -- 后端固定 MONTH
        State        char(1)       NOT NULL DEFAULT 'Y',          -- Y=启用, N=停用
        Remark       nvarchar(200) NULL,
        LUTime       datetime      NULL DEFAULT GETDATE()
    )
END
GO

-- 2. 单据序号记录表（后端 doc_no.rs 实际使用此表）
--    并发安全：UPDATE...OUTPUT INSERTED.CurrentSeq 原子自增
IF NOT EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tSys_DocNoSeq' AND xtype = 'U')
BEGIN
    CREATE TABLE tSys_DocNoSeq (
        DocTypeID    nvarchar(30)  NOT NULL,                       -- 业务类型编码=单据前缀
        PeriodKey    nvarchar(20)  NOT NULL,                       -- YYMM（按月重置）
        CurrentSeq   bigint        NOT NULL DEFAULT 0,
        LUTime       datetime      NOT NULL DEFAULT GETDATE(),
        CONSTRAINT PK_tSys_DocNoSeq PRIMARY KEY (DocTypeID, PeriodKey)
    )
END
GO

-- 3. 初始化单据类型配置（TableName/FieldName 与实际表结构严格对齐）
--    单据号前缀与 client/src/config/enums.js DOC_NO_PREFIX 对齐

-- 销售模块
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'SO')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('SO', '销售订单', 'SO', 'tSal_Order', 'SoNo', 'YYMM', 4, 1, 'Y', 'MONTH', 'Y', '销售订单: SO+YYMM+4位序号')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'SI')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('SI', '销售出库单', 'SI', 'tSal_Inv', 'SINo', 'YYMM', 4, 1, 'Y', 'MONTH', 'Y', '销售出库/发票: SI+YYMM+4位序号')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'SQ')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('SQ', '销售报价单', 'SQ', 'tSal_Quote', 'QuoNo', 'YYMM', 4, 1, 'Y', 'MONTH', 'Y', '销售报价单')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'SR')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('SR', '销售退货单', 'SR', 'tStk_IO', 'IONo', 'YYMM', 4, 1, 'Y', 'MONTH', 'Y', '销售退货走库存表 Kind=SR')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'SS')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('SS', '门店销售单', 'SS', 'tStk_IO', 'IONo', 'YYMM', 4, 1, 'Y', 'MONTH', 'Y', '门店销售走库存表 Kind=SI')
GO

-- 采购模块
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'PO')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('PO', '采购订单', 'PO', 'tPur_Order', 'PoNo', 'YYMM', 4, 1, 'Y', 'MONTH', 'Y', '采购订单')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'PI')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('PI', '采购入库单', 'PI', 'tPur_Inv', 'PiNo', 'YYMM', 4, 1, 'Y', 'MONTH', 'Y', '采购入库: PI+YYMM+4位序号')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'PR')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('PR', '采购退货单', 'PR', 'tStk_IO', 'IONo', 'YYMM', 4, 1, 'Y', 'MONTH', 'Y', '采购退货走库存表 Kind=PR')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'PRQ')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('PRQ', '采购报价单', 'PRQ', 'tPur_Quote', 'PrQuoNo', 'YYMM', 4, 1, 'Y', 'MONTH', 'Y', '采购报价单')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'PAP')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('PAP', '采购调价单', 'PAP', 'tPur_AdjPrice', 'PAPNo', 'YYMM', 4, 1, 'Y', 'MONTH', 'Y', '采购调价单')
GO

-- 批发模块（共用 tSal_Order / tSal_Quote / tStk_IO，靠 BTPID 区分）
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'WO')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('WO', '批发订单', 'WO', 'tSal_Order', 'SoNo', 'YYMM', 4, 1, 'Y', 'MONTH', 'Y', '批发订单共用 tSal_Order，BTPID=WHOLESALE')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'WSO')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('WSO', '批发出库单', 'WSO', 'tStk_IO', 'IONo', 'YYMM', 4, 1, 'Y', 'MONTH', 'Y', '批发出库走库存表 Kind=SD，BTPID=WHOLESALE')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'WSR')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('WSR', '批发退货单', 'WSR', 'tStk_IO', 'IONo', 'YYMM', 4, 1, 'Y', 'MONTH', 'Y', '批发退货走库存表 Kind=SR，BTPID=WHOLESALE_RETURN')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'WQ')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('WQ', '批发报价单', 'WQ', 'tSal_Quote', 'QuoNo', 'YYMM', 4, 1, 'Y', 'MONTH', 'Y', '批发报价共用 tSal_Quote')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'SAP')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('SAP', '批发调价单', 'SAP', 'tSal_AdjPrice', 'SAPNo', 'YYMM', 4, 1, 'Y', 'MONTH', 'Y', '批发调价单')
GO

-- 库存模块
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'MV')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('MV', '调拨单', 'MV', 'tStk_Move', 'MoveNO', 'YYMM', 4, 1, 'Y', 'MONTH', 'Y', '库存调拨单')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'IO')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('IO', '库存出入库', 'IO', 'tStk_IO', 'IONo', 'YYMM', 4, 1, 'Y', 'MONTH', 'Y', '库存出入库单（零散入出库）')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'CHK')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('CHK', '库存盘点单', 'CHK', 'tStk_Tran', 'TranNo', 'YYMM', 4, 1, 'Y', 'MONTH', 'Y', '盘点走 tStk_Tran')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'CYC')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('CYC', '周期盘点单', 'CYC', 'tStk_StockCycle', 'CycleNo', 'YYMM', 4, 1, 'Y', 'MONTH', 'Y', '周期盘点')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'RPA')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('RPA', '补货申请单', 'RPA', 'tStk_ReplenishApply', 'ApplyNo', 'YYMM', 4, 1, 'Y', 'MONTH', 'Y', '补货申请')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'ADJ')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('ADJ', '调价单', 'ADJ', 'tPur_AdjPrice', 'PAPNo', 'YYMM', 4, 1, 'Y', 'MONTH', 'Y', '调价单（注：ADJ 前缀与库存调整可能歧义，建议细化）')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'ZP')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('ZP', '门店直配单', 'ZP', 'tStk_Move', 'MoveNO', 'YYMM', 4, 1, 'Y', 'MONTH', 'Y', '门店直配走 tStk_Move Kind=ZP')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'OTI')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('OTI', '零散入库单', 'OTI', 'tStk_IO', 'IONo', 'YYMM', 4, 1, 'Y', 'MONTH', 'Y', '零散入库走 tStk_IO Kind=OTI')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'OTO')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('OTO', '零散出库单', 'OTO', 'tStk_IO', 'IONo', 'YYMM', 4, 1, 'Y', 'MONTH', 'Y', '零散出库走 tStk_IO Kind=OTO')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'REQ')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('REQ', '领用申请单', 'REQ', 'tStk_IO', 'IONo', 'YYMM', 4, 1, 'Y', 'MONTH', 'Y', '领用申请走 tStk_IO Kind=REQ')
GO

-- 财务模块
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'PAY')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('PAY', '付款单', 'PAY', 'tFin_Payment', 'PayNo', 'YYMM', 4, 1, 'Y', 'MONTH', 'Y', '付款单')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'RCV')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('RCV', '收款单', 'RCV', 'tFin_Receipt', 'RcptNo', 'YYMM', 4, 1, 'Y', 'MONTH', 'Y', '收款单')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'CF')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('CF', '现金流量', 'CF', 'tFin_CashFlow', 'CFNo', 'YYMM', 4, 1, 'Y', 'MONTH', 'Y', '现金流量记录')
GO

-- POS / 门店
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'POS')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('POS', 'POS收银', 'POS', 'tStk_IO', 'IONo', 'YYMM', 4, 1, 'Y', 'MONTH', 'Y', 'POS收银走 tStk_IO Kind=POS')
GO
GO
