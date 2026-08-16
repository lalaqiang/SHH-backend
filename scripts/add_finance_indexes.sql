-- ============================================================================
-- 财务表补充索引（tFin_CashFlow / tFin_Receipt / tFin_Payment / 明细表）
-- 重点优化：
--   1. tFin_ReceiptDtl.SourceDocID / tFin_PaymentDtl.SourceDocID
--      → 核销查询 LEFT JOIN 聚合已审核金额的关键索引（频繁使用）
--   2. tFin_CashFlow 多字段索引（CFDate+State, CFType+State）
--      → 列表查询与汇总报表过滤
--   3. tFin_Receipt/Payment 的 RecDate/PayDate、EmpID、单号索引
--      → 对账单期间查询、按业务员汇总、按单号定位
-- 全部使用 IF NOT EXISTS 幂等模式，可重复执行
-- ============================================================================
-- 必需的 SET 选项：带筛选 (WHERE) 和 INCLUDE 的索引要求以下选项全部 ON
-- 否则报错 1934：CREATE INDEX 失败，因为下列 SET 选项的设置不正确
SET QUOTED_IDENTIFIER ON;
SET ANSI_NULLS ON;
SET ANSI_PADDING ON;
SET ANSI_WARNINGS ON;
SET CONCAT_NULL_YIELDS_NULL ON;
SET ARITHABORT ON;
SET NUMERIC_ROUNDABORT OFF;
GO

-- ---------------- tFin_CashFlow ----------------
IF EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tFin_CashFlow' AND xtype = 'U')
   AND NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tFin_CashFlow_CFDate' AND object_id = OBJECT_ID('tFin_CashFlow'))
    CREATE INDEX IX_tFin_CashFlow_CFDate ON tFin_CashFlow(CFDate) WHERE CFDate IS NOT NULL
GO

IF EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tFin_CashFlow' AND xtype = 'U')
   AND NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tFin_CashFlow_State' AND object_id = OBJECT_ID('tFin_CashFlow'))
    CREATE INDEX IX_tFin_CashFlow_State ON tFin_CashFlow(State)
GO

IF EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tFin_CashFlow' AND xtype = 'U')
   AND NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tFin_CashFlow_CFType_State' AND object_id = OBJECT_ID('tFin_CashFlow'))
    CREATE INDEX IX_tFin_CashFlow_CFType_State ON tFin_CashFlow(CFType, State)
GO

IF EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tFin_CashFlow' AND xtype = 'U')
   AND NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tFin_CashFlow_SuppID' AND object_id = OBJECT_ID('tFin_CashFlow'))
    CREATE INDEX IX_tFin_CashFlow_SuppID ON tFin_CashFlow(SuppID) WHERE SuppID IS NOT NULL
GO

IF EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tFin_CashFlow' AND xtype = 'U')
   AND NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tFin_CashFlow_CustID' AND object_id = OBJECT_ID('tFin_CashFlow'))
    CREATE INDEX IX_tFin_CashFlow_CustID ON tFin_CashFlow(CustID) WHERE CustID IS NOT NULL
GO

IF EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tFin_CashFlow' AND xtype = 'U')
   AND NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tFin_CashFlow_EmpID' AND object_id = OBJECT_ID('tFin_CashFlow'))
    CREATE INDEX IX_tFin_CashFlow_EmpID ON tFin_CashFlow(EmpID) WHERE EmpID IS NOT NULL
GO

IF EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tFin_CashFlow' AND xtype = 'U')
   AND NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tFin_CashFlow_CFNO' AND object_id = OBJECT_ID('tFin_CashFlow'))
    CREATE INDEX IX_tFin_CashFlow_CFNO ON tFin_CashFlow(CFNO) WHERE CFNO IS NOT NULL
GO

-- ---------------- tFin_Receipt ----------------
IF EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tFin_Receipt' AND xtype = 'U')
   AND NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tFin_Receipt_RecDate' AND object_id = OBJECT_ID('tFin_Receipt'))
    CREATE INDEX IX_tFin_Receipt_RecDate ON tFin_Receipt(RecDate) WHERE RecDate IS NOT NULL
GO

IF EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tFin_Receipt' AND xtype = 'U')
   AND NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tFin_Receipt_RecNO' AND object_id = OBJECT_ID('tFin_Receipt'))
    CREATE INDEX IX_tFin_Receipt_RecNO ON tFin_Receipt(RecNO) WHERE RecNO IS NOT NULL
GO

IF EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tFin_Receipt' AND xtype = 'U')
   AND NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tFin_Receipt_EmpID' AND object_id = OBJECT_ID('tFin_Receipt'))
    CREATE INDEX IX_tFin_Receipt_EmpID ON tFin_Receipt(EmpID) WHERE EmpID IS NOT NULL
GO

IF EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tFin_Receipt' AND xtype = 'U')
   AND NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tFin_Receipt_CustID_State' AND object_id = OBJECT_ID('tFin_Receipt'))
    CREATE INDEX IX_tFin_Receipt_CustID_State ON tFin_Receipt(CustID, State) WHERE CustID IS NOT NULL
GO

-- ---------------- tFin_Payment ----------------
IF EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tFin_Payment' AND xtype = 'U')
   AND NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tFin_Payment_PayDate' AND object_id = OBJECT_ID('tFin_Payment'))
    CREATE INDEX IX_tFin_Payment_PayDate ON tFin_Payment(PayDate) WHERE PayDate IS NOT NULL
GO

IF EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tFin_Payment' AND xtype = 'U')
   AND NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tFin_Payment_PayNO' AND object_id = OBJECT_ID('tFin_Payment'))
    CREATE INDEX IX_tFin_Payment_PayNO ON tFin_Payment(PayNO) WHERE PayNO IS NOT NULL
GO

IF EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tFin_Payment' AND xtype = 'U')
   AND NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tFin_Payment_EmpID' AND object_id = OBJECT_ID('tFin_Payment'))
    CREATE INDEX IX_tFin_Payment_EmpID ON tFin_Payment(EmpID) WHERE EmpID IS NOT NULL
GO

IF EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tFin_Payment' AND xtype = 'U')
   AND NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tFin_Payment_SuppID_State' AND object_id = OBJECT_ID('tFin_Payment'))
    CREATE INDEX IX_tFin_Payment_SuppID_State ON tFin_Payment(SuppID, State) WHERE SuppID IS NOT NULL
GO

-- ---------------- tFin_ReceiptDtl（核销明细表）----------------
-- SourceDocID 是核销聚合查询（LEFT JOIN 已审核收款单）的关键索引
IF EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tFin_ReceiptDtl' AND xtype = 'U')
   AND NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tFin_ReceiptDtl_SourceDocID' AND object_id = OBJECT_ID('tFin_ReceiptDtl'))
    CREATE INDEX IX_tFin_ReceiptDtl_SourceDocID ON tFin_ReceiptDtl(SourceDocID) WHERE SourceDocID IS NOT NULL
GO

-- ---------------- tFin_PaymentDtl（付款核销明细表）----------------
IF EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tFin_PaymentDtl' AND xtype = 'U')
   AND NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tFin_PaymentDtl_SourceDocID' AND object_id = OBJECT_ID('tFin_PaymentDtl'))
    CREATE INDEX IX_tFin_PaymentDtl_SourceDocID ON tFin_PaymentDtl(SourceDocID) WHERE SourceDocID IS NOT NULL
GO

-- ============================================================================
-- 补充：tStk_IO 上的 AR/AP 派生查询专用索引
-- 背景：派生 AR/AP 从 tStk_IO 按 Kind 派生（SD/SI/POS/SR → AR；PD/PR → AP）
--   - get_customer_ar / get_overdue_accounts：按 State+Kind 全表聚合，已有
--     IX_tStk_IO_Kind_State_Date 可覆盖（见 optimize_indexes.sql）
--   - get_customer_ar_detail / get_supplier_ap_detail：按 CustID/SuppID 单查
--     现有索引前缀是 Kind，无法直接 seek 到指定客户/供应商，需补复合索引
-- 索引列顺序说明：
--   (CustID/SuppID, State, Kind) —— 先按客户/供应商等值定位，再按状态+类型筛选
--   INCLUDE 字段避免 Key Lookup 回表（覆盖索引）
-- ============================================================================

-- 客户应收明细查询：WHERE CustID=? AND State IN ('S','Y') AND Kind IN ('SD','SI','POS','SR')
IF EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tStk_IO' AND xtype = 'U')
   AND NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tStk_IO_CustID_State_Kind' AND object_id = OBJECT_ID('tStk_IO'))
    CREATE INDEX IX_tStk_IO_CustID_State_Kind ON tStk_IO(CustID, State, Kind)
    INCLUDE (IOID, IONo, IoDate, SumAmt, SumQty, Note)
    WHERE CustID IS NOT NULL
GO

-- 供应商应付明细查询：WHERE SuppID=? AND State IN ('S','Y') AND Kind IN ('PD','PR')
IF EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tStk_IO' AND xtype = 'U')
   AND NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tStk_IO_SuppID_State_Kind' AND object_id = OBJECT_ID('tStk_IO'))
    CREATE INDEX IX_tStk_IO_SuppID_State_Kind ON tStk_IO(SuppID, State, Kind)
    INCLUDE (IOID, IONo, IoDate, SumAmt, SumQty, Note)
    WHERE SuppID IS NOT NULL
GO

PRINT '===== 财务表补充索引创建完成 ====='
GO
