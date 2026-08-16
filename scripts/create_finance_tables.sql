-- ============================================================================
-- 创建缺失的财务表（tFin_Receipt / tFin_Payment / tFin_CashFlow + 明细表）
-- 字段与前端 columns 定义对齐：
--   收款单 tFin_Receipt: RecID/RecNO/RecDate/CustID/RecAmt/RecType/Remark
--   付款单 tFin_Payment: PayID/PayNO/PayDate/SuppID/PayAmt/PayType/Remark
--   现金流量 tFin_CashFlow: CFID/CFNO/CFDate/CFType/CFAmt/Remark
--   收款明细 tFin_ReceiptDtl: ReceiptDtlID/RecID/SourceDocID/GDSID/Qty/Price/Amt
--   付款明细 tFin_PaymentDtl: PaymentDtlID/PayID/SourceDocID/GDSID/Qty/Price/Amt
-- ============================================================================

-- 1. tFin_Receipt — 收款单主表
IF NOT EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tFin_Receipt' AND xtype = 'U')
BEGIN
    CREATE TABLE tFin_Receipt (
        RecID       uniqueidentifier PRIMARY KEY DEFAULT NEWID(),
        RecNO       nvarchar(50)     NULL,
        RecDate     datetime         NULL,
        CustID      uniqueidentifier NULL,
        DeptID      uniqueidentifier NULL,
        EmpID       uniqueidentifier NULL,
        StkID       uniqueidentifier NULL,
        RecAmt      decimal(18,2)    DEFAULT 0,
        RecType     varchar(20)      DEFAULT 'cash',
        BankName    nvarchar(100)    NULL,
        BankAccount nvarchar(50)     NULL,
        DocID       uniqueidentifier NULL,
        DocNo       nvarchar(50)     NULL,
        Remark      nvarchar(500)    NULL,
        State       char(1)          DEFAULT 'N',
        LUTime      datetime         DEFAULT GETDATE(),
        EUser       uniqueidentifier NULL,
        EDate       datetime         DEFAULT GETDATE(),
        AUser       uniqueidentifier NULL,
        ADate       datetime         NULL,
        SUser       uniqueidentifier NULL,
        SDate       datetime         NULL
    )
END
GO

-- 2. tFin_Payment — 付款单主表
IF NOT EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tFin_Payment' AND xtype = 'U')
BEGIN
    CREATE TABLE tFin_Payment (
        PayID       uniqueidentifier PRIMARY KEY DEFAULT NEWID(),
        PayNO       nvarchar(50)     NULL,
        PayDate     datetime         NULL,
        SuppID      uniqueidentifier NULL,
        DeptID      uniqueidentifier NULL,
        EmpID       uniqueidentifier NULL,
        StkID       uniqueidentifier NULL,
        PayAmt      decimal(18,2)    DEFAULT 0,
        PayType     varchar(20)      DEFAULT 'bank',
        BankName    nvarchar(100)    NULL,
        BankAccount nvarchar(50)     NULL,
        DocID       uniqueidentifier NULL,
        DocNo       nvarchar(50)     NULL,
        Remark      nvarchar(500)    NULL,
        State       char(1)          DEFAULT 'N',
        LUTime      datetime         DEFAULT GETDATE(),
        EUser       uniqueidentifier NULL,
        EDate       datetime         DEFAULT GETDATE(),
        AUser       uniqueidentifier NULL,
        ADate       datetime         NULL,
        SUser       uniqueidentifier NULL,
        SDate       datetime         NULL
    )
END
GO

-- 3. tFin_CashFlow — 现金流量表
IF NOT EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tFin_CashFlow' AND xtype = 'U')
BEGIN
    CREATE TABLE tFin_CashFlow (
        CFID        uniqueidentifier PRIMARY KEY DEFAULT NEWID(),
        CFNO        nvarchar(50)     NULL,
        CFDate      datetime         NULL,
        CFType      varchar(10)      DEFAULT 'IN',
        SuppID      uniqueidentifier NULL,
        CustID      uniqueidentifier NULL,
        DeptID      uniqueidentifier NULL,
        EmpID       uniqueidentifier NULL,
        CFAmt       decimal(18,2)    DEFAULT 0,
        BankName    nvarchar(100)    NULL,
        BankAccount nvarchar(50)     NULL,
        DocID       uniqueidentifier NULL,
        DocNo       nvarchar(50)     NULL,
        Remark      nvarchar(500)    NULL,
        State       char(1)          DEFAULT 'N',
        LUTime      datetime         DEFAULT GETDATE(),
        EUser       uniqueidentifier NULL,
        EDate       datetime         DEFAULT GETDATE(),
        AUser       uniqueidentifier NULL,
        ADate       datetime         NULL,
        SUser       uniqueidentifier NULL,
        SDate       datetime         NULL
    )
END
GO

-- 4. tFin_ReceiptDtl — 收款明细表（核销到具体销售单据）
IF NOT EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tFin_ReceiptDtl' AND xtype = 'U')
BEGIN
    CREATE TABLE tFin_ReceiptDtl (
        ReceiptDtlID  uniqueidentifier PRIMARY KEY DEFAULT NEWID(),
        RecID         uniqueidentifier NULL,
        SourceDocID   uniqueidentifier NULL,
        SourceDocNo   nvarchar(50)     NULL,
        GDSID         uniqueidentifier NULL,
        Qty           decimal(18,4)    DEFAULT 0,
        Price         decimal(18,4)    DEFAULT 0,
        Amt           decimal(18,2)    DEFAULT 0,
        Note          nvarchar(500)    NULL,
        Rowno         int              IDENTITY(1,1)
    )
END
GO

-- 5. tFin_PaymentDtl — 付款明细表（核销到具体采购单据）
IF NOT EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tFin_PaymentDtl' AND xtype = 'U')
BEGIN
    CREATE TABLE tFin_PaymentDtl (
        PaymentDtlID  uniqueidentifier PRIMARY KEY DEFAULT NEWID(),
        PayID         uniqueidentifier NULL,
        SourceDocID   uniqueidentifier NULL,
        SourceDocNo   nvarchar(50)     NULL,
        GDSID         uniqueidentifier NULL,
        Qty           decimal(18,4)    DEFAULT 0,
        Price         decimal(18,4)    DEFAULT 0,
        Amt           decimal(18,2)    DEFAULT 0,
        Note          nvarchar(500)    NULL,
        Rowno         int              IDENTITY(1,1)
    )
END
GO

-- 6. 创建索引
IF EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tFin_Receipt' AND xtype = 'U')
   AND NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tFin_Receipt_CustID' AND object_id = OBJECT_ID('tFin_Receipt'))
    CREATE INDEX IX_tFin_Receipt_CustID ON tFin_Receipt(CustID) WHERE CustID IS NOT NULL
GO

IF EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tFin_Receipt' AND xtype = 'U')
   AND NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tFin_Receipt_State' AND object_id = OBJECT_ID('tFin_Receipt'))
    CREATE INDEX IX_tFin_Receipt_State ON tFin_Receipt(State)
GO

IF EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tFin_Payment' AND xtype = 'U')
   AND NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tFin_Payment_SuppID' AND object_id = OBJECT_ID('tFin_Payment'))
    CREATE INDEX IX_tFin_Payment_SuppID ON tFin_Payment(SuppID) WHERE SuppID IS NOT NULL
GO

IF EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tFin_Payment' AND xtype = 'U')
   AND NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tFin_Payment_State' AND object_id = OBJECT_ID('tFin_Payment'))
    CREATE INDEX IX_tFin_Payment_State ON tFin_Payment(State)
GO

IF EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tFin_ReceiptDtl' AND xtype = 'U')
   AND NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tFin_ReceiptDtl_RecID' AND object_id = OBJECT_ID('tFin_ReceiptDtl'))
    CREATE INDEX IX_tFin_ReceiptDtl_RecID ON tFin_ReceiptDtl(RecID)
GO

IF EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tFin_PaymentDtl' AND xtype = 'U')
   AND NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tFin_PaymentDtl_PayID' AND object_id = OBJECT_ID('tFin_PaymentDtl'))
    CREATE INDEX IX_tFin_PaymentDtl_PayID ON tFin_PaymentDtl(PayID)
GO

PRINT '===== 财务表创建完成 ====='
GO
