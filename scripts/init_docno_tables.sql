-- ============================================================
-- 单据编号自增相关表
-- 适用 SQL Server 2005+
-- 说明:
--   tSys_DocNo   配置每种单据类型的编号规则
--   tSys_DocNoSeq 单据序号记录 (使用 UPDLOCK 串行化保证并发安全)
-- ============================================================

-- 1. 单据编号配置表
IF NOT EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tSys_DocNo' AND xtype = 'U')
BEGIN
    CREATE TABLE tSys_DocNo (
        DocTypeID    nvarchar(30)  NOT NULL PRIMARY KEY,         -- 业务类型编码, 如 SO/PO/MV
        DocName      nvarchar(50)  NOT NULL DEFAULT '',           -- 中文名称
        Prefix       nvarchar(20)  NOT NULL DEFAULT '',           -- 单据前缀
        TableName    nvarchar(60)  NOT NULL DEFAULT '',           -- 实际数据表名
        FieldName    nvarchar(60)  NOT NULL DEFAULT '',           -- 单据号字段名
        DateFormat   nvarchar(20)  NOT NULL DEFAULT 'YYYYMMDD',   -- 日期段格式: YYYYMMDD / YYYYMM / YYMMDD / YYMM / NONE
        SeqPadding   int           NOT NULL DEFAULT 4,            -- 序号位数, 0 表示不补零
        SeqStart     int           NOT NULL DEFAULT 1,            -- 序号起始值
        DateReset    char(1)       NOT NULL DEFAULT 'Y',          -- Y=按日/月重置, N=一直累加
        PeriodType   nvarchar(10)  NOT NULL DEFAULT 'DAY',        -- DAY / MONTH / NONE
        State        char(1)       NOT NULL DEFAULT 'Y',          -- Y=启用, N=停用
        Remark       nvarchar(200) NULL,
        LUTime       datetime      NULL DEFAULT GETDATE()
    )
END
GO

-- 2. 单据序号记录表 (并发安全, 同一 doc_type + period_key 同一时刻仅一写)
IF NOT EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tSys_DocNoSeq' AND xtype = 'U')
BEGIN
    CREATE TABLE tSys_DocNoSeq (
        DocTypeID    nvarchar(30)  NOT NULL,
        PeriodKey    nvarchar(20)  NOT NULL,
        CurrentSeq   bigint        NOT NULL DEFAULT 0,
        LUTime       datetime      NOT NULL DEFAULT GETDATE(),
        CONSTRAINT PK_tSys_DocNoSeq PRIMARY KEY (DocTypeID, PeriodKey)
    )
END
GO

-- 3. 初始化单据类型配置 (覆盖系统全部单据)
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'SO')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('SO', '销售订单', 'SO', 'tSal_Order', 'OrderNo', 'YYYYMMDD', 4, 1, 'Y', 'DAY', 'Y', '销售订单: SO + YYYYMMDD + 4位序号')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'SI')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('SI', '销售出库单', 'SI', 'tSal_Inv', 'InvNo', 'YYYYMMDD', 4, 1, 'Y', 'DAY', 'Y', '销售出库单: SI + YYYYMMDD + 4位序号')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'SQ')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('SQ', '销售报价单', 'SQ', 'tSal_Quote', 'QuoteNo', 'YYYYMMDD', 4, 1, 'Y', 'DAY', 'Y', '销售报价单')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'SR')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('SR', '销售退货单', 'SR', 'tSal_Return', 'SalRetNo', 'YYYYMMDD', 4, 1, 'Y', 'DAY', 'Y', '销售退货单')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'SS')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('SS', '门店销售单', 'SS', 'tSal_StoreSal', 'SalNo', 'YYYYMMDD', 4, 1, 'Y', 'DAY', 'Y', '门店销售单')
GO

IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'PO')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('PO', '采购订单', 'PO', 'tPur_Order', 'PoNo', 'YYYYMMDD', 4, 1, 'Y', 'DAY', 'Y', '采购订单')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'PI')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('PI', '采购入库单', 'PI', 'tPur_In', 'PiNo', 'YYYYMMDD', 4, 1, 'Y', 'DAY', 'Y', '采购入库单')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'PR')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('PR', '采购退货单', 'PR', 'tPur_Return', 'PrNo', 'YYYYMMDD', 4, 1, 'Y', 'DAY', 'Y', '采购退货单')
GO

IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'WO')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('WO', '批发订单', 'WO', 'tSal_Inv', 'SINo', 'YYYYMMDD', 4, 1, 'Y', 'DAY', 'Y', '批发订单: WO + YYYYMMDD + 4位序号')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'WI')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('WI', '批发出库单', 'WI', 'tSal_Out', 'OutNo', 'YYYYMMDD', 4, 1, 'Y', 'DAY', 'Y', '批发出库单')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'WR')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('WR', '批发退货单', 'WR', 'tSal_Return', 'SalRetNo', 'YYYYMMDD', 4, 1, 'Y', 'DAY', 'Y', '批发退货单')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'WQ')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('WQ', '批发报价单', 'WQ', 'tSal_Quote', 'QuoteNo', 'YYYYMMDD', 4, 1, 'Y', 'DAY', 'Y', '批发报价单')
GO

IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'MV')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('MV', '调拨单', 'MV', 'tStk_Move', 'MoveNo', 'YYYYMMDD', 4, 1, 'Y', 'DAY', 'Y', '库存调拨单')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'IO')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('IO', '库存出入库', 'IO', 'tStk_IO', 'IoNo', 'YYYYMMDD', 4, 1, 'Y', 'DAY', 'Y', '库存出入库单')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'CHK')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('CHK', '库存盘点单', 'CHK', 'tStk_Chk', 'ChkNo', 'YYYYMMDD', 4, 1, 'Y', 'DAY', 'Y', '库存盘点单')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'ADJ')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('ADJ', '调价单', 'ADJ', 'tStk_AdjPrice', 'AdjNo', 'YYYYMMDD', 4, 1, 'Y', 'DAY', 'Y', '商品调价单')
GO

IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'PAY')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('PAY', '付款单', 'PAY', 'tFin_Payment', 'PayNo', 'YYYYMMDD', 4, 1, 'Y', 'DAY', 'Y', '付款单')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'RCV')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('RCV', '收款单', 'RCV', 'tFin_Receipt', 'RcptNo', 'YYYYMMDD', 4, 1, 'Y', 'DAY', 'Y', '收款单')
GO
IF NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = 'CF')
INSERT INTO tSys_DocNo (DocTypeID, DocName, Prefix, TableName, FieldName, DateFormat, SeqPadding, SeqStart, DateReset, PeriodType, State, Remark)
VALUES ('CF', '现金流量', 'CF', 'tFin_CashFlow', 'CFNO', 'YYYYMMDD', 4, 1, 'Y', 'DAY', 'Y', '现金流量记录')
GO
