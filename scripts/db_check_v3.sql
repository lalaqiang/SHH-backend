-- ============================================================================
-- DB Schema Check v3 (NO Chinese anywhere, fresh filename to avoid cache)
-- Usage:
--   1. SSMS: File > Open > c:\Users\Administrator\Desktop\ERP\server-rust\scripts\db_check_v3.sql
--   2. Press F5, then copy results grid to me
--   OR via sqlcmd:
--   sqlcmd -S localhost -d TestERP -U sa -P sa123456 -C -i db_check_v3.sql -o out_v3.txt -W -h-1
-- ============================================================================
SET NOCOUNT ON
GO

PRINT '====== 1. Finance tables (tFin_* / tArd_* / tAcc_*) ======'
GO
SELECT t.TABLE_NAME AS TableName, 'EXISTS' AS Status
FROM INFORMATION_SCHEMA.TABLES t
WHERE t.TABLE_NAME IN (
    'tFin_Receivable','tFin_Payable','tFin_Receipt','tFin_Payment','tFin_CashFlow',
    'tArd_AR','tArd_PD','tArd_RK','tArd_Sale','tArd_SaleOrder','tArd_PDJ','tArd_Log',
    'tAcc_PayIn','tAcc_PayOut','tFin_GL','tFin_GLDetail','tFin_Period','tFin_Recon'
)
ORDER BY t.TABLE_NAME
GO

PRINT '------ Finance tables that DO NOT exist (code vs DB mismatch) ------'
GO
SELECT v.name AS MissingTable
FROM (VALUES
    ('tFin_Receivable'),('tFin_Payable'),('tFin_Receipt'),
    ('tFin_Payment'),('tFin_CashFlow'),
    ('tArd_AR'),('tArd_PD'),('tArd_RK'),('tArd_Sale'),
    ('tArd_SaleOrder'),('tArd_PDJ'),('tArd_Log'),
    ('tAcc_PayIn'),('tAcc_PayOut')
) AS v(name)
WHERE NOT EXISTS (
    SELECT 1 FROM sysobjects o WHERE o.name = v.name AND o.xtype = 'U'
)
GO

PRINT '------ REAL finance-related tables (search by prefix) ------'
GO
SELECT name AS TableName FROM sysobjects
WHERE xtype = 'U' AND (
    name LIKE 'tFin_%' OR name LIKE 'tArd_%' OR name LIKE 'tAcc_%' OR
    name LIKE 'tRec_%' OR name LIKE 'tPay_%' OR name LIKE 'tCash_%' OR
    name LIKE 'tReckoning_%' OR name LIKE 'tRecei%' OR name LIKE 'tPayab%'
)
ORDER BY name
GO

PRINT '====== 2. OA / workflow tables ======'
GO
SELECT t.TABLE_NAME AS TableName
FROM INFORMATION_SCHEMA.TABLES t
WHERE t.TABLE_NAME IN (
    'tOA_InfoDetail','tOA_InfoMenus','tOA_InfoType','tOA_InfoTypeEmp',
    'tOA_MyInfo','tOA_Bylaw','tOA_Email','tOA_EmailPath','tOA_EmailServer',
    'tOA_EmailToDtl','tOA_EmailUser','tOA_EUEmp','tOA_LineMan',
    'tSys_WorkFlow','tSys_MWorkFlow','tSys_Msg','tSys_AutoMsg','tSys_Warning'
)
ORDER BY t.TABLE_NAME
GO

PRINT '------ REAL OA/notice/workflow tables (search by prefix) ------'
GO
SELECT name AS TableName FROM sysobjects
WHERE xtype = 'U' AND (
    name LIKE 'tOA_%' OR name LIKE 'tNotice%' OR name LIKE 'tNews%'
    OR name LIKE 'tWorkFlow%' OR name LIKE 'tFlow_%' OR name LIKE 'tWork_%'
)
ORDER BY name
GO

PRINT '====== 3. System tables (tSys_*) ======'
GO
SELECT t.TABLE_NAME AS TableName
FROM INFORMATION_SCHEMA.TABLES t
WHERE t.TABLE_NAME IN (
    'tSys_Company','tSys_User','tSys_Menus','tSys_Rule','tSys_RuleEmp',
    'tSys_Params','tSys_Parameters','tSys_OperHis','tSys_OperLog',
    'tSys_Rpt','tSys_Dictionary','tSys_DocNo'
)
ORDER BY t.TABLE_NAME
GO

PRINT '====== 4. Sales / purchase / return / IO main tables ======'
GO
SELECT t.TABLE_NAME AS TableName
FROM INFORMATION_SCHEMA.TABLES t
WHERE t.TABLE_NAME IN (
    'tSal_Order','tSal_OrderDetail','tSal_Order_b',
    'tSal_Inv','tSal_InvDetail','tSal_Inv_b',
    'tSal_Return','tSal_ReturnDetail','tSal_Return_b',
    'tPur_Order','tPur_OrderDetail','tPur_Order_b',
    'tPur_Inv','tPur_InvDetail','tPur_Inv_b',
    'tPur_Return','tPur_ReturnDetail','tPur_Return_b',
    'tStk_IO','tStk_IO_b','tStk_IODetail',
    'tStk_Move','tStk_MoveDetail','tStk_Move_b',
    'tStk_Tran','tStk_TranDetail',
    'tStk_ReplenishApply','tStk_ReplenishApplyDetail'
)
ORDER BY t.TABLE_NAME
GO

PRINT '====== 5. Inventory three-piece ======'
GO
SELECT t.TABLE_NAME AS TableName
FROM INFORMATION_SCHEMA.TABLES t
WHERE t.TABLE_NAME IN (
    'tStk_Stock','tStk_StockTranHis','tStk_StockYM',
    'tStk_Qty','tStk_IO','tStk_Move','tStk_Tran','tStk_ReplenishApply'
)
ORDER BY t.TABLE_NAME
GO

PRINT '------ tStk_Stock columns ------'
GO
SELECT c.COLUMN_NAME, c.DATA_TYPE, c.CHARACTER_MAXIMUM_LENGTH, c.IS_NULLABLE
FROM INFORMATION_SCHEMA.COLUMNS c
WHERE c.TABLE_NAME = 'tStk_Stock'
ORDER BY c.ORDINAL_POSITION
GO

PRINT '------ tStk_StockYM columns ------'
GO
SELECT c.COLUMN_NAME, c.DATA_TYPE, c.CHARACTER_MAXIMUM_LENGTH, c.IS_NULLABLE
FROM INFORMATION_SCHEMA.COLUMNS c
WHERE c.TABLE_NAME = 'tStk_StockYM'
ORDER BY c.ORDINAL_POSITION
GO

PRINT '------ tStk_StockTranHis columns ------'
GO
SELECT c.COLUMN_NAME, c.DATA_TYPE, c.CHARACTER_MAXIMUM_LENGTH, c.IS_NULLABLE
FROM INFORMATION_SCHEMA.COLUMNS c
WHERE c.TABLE_NAME = 'tStk_StockTranHis'
ORDER BY c.ORDINAL_POSITION
GO

PRINT '------ tStk_Qty columns ------'
GO
SELECT c.COLUMN_NAME, c.DATA_TYPE, c.CHARACTER_MAXIMUM_LENGTH, c.IS_NULLABLE
FROM INFORMATION_SCHEMA.COLUMNS c
WHERE c.TABLE_NAME = 'tStk_Qty'
ORDER BY c.ORDINAL_POSITION
GO

PRINT '------ tStk_IO columns ------'
GO
SELECT c.COLUMN_NAME, c.DATA_TYPE, c.CHARACTER_MAXIMUM_LENGTH, c.IS_NULLABLE
FROM INFORMATION_SCHEMA.COLUMNS c
WHERE c.TABLE_NAME = 'tStk_IO'
ORDER BY c.ORDINAL_POSITION
GO

PRINT '------ tStk_IODetail columns ------'
GO
SELECT c.COLUMN_NAME, c.DATA_TYPE, c.CHARACTER_MAXIMUM_LENGTH, c.IS_NULLABLE
FROM INFORMATION_SCHEMA.COLUMNS c
WHERE c.TABLE_NAME = 'tStk_IODetail'
ORDER BY c.ORDINAL_POSITION
GO

PRINT '====== 6. Triggers & CHECK constraints (safety net) ======'
GO
SELECT name AS ObjectName, type_desc AS ObjectType
FROM sys.objects
WHERE name IN (
    'trg_IODetail_SafetyStock',
    'trg_MoveDetail_SafetyStock',
    'trg_TranDetail_SafetyStock',
    'trg_Stock_AfterChange',
    'CK_Stock_Qty_NonNeg',
    'CK_Stock_Qty_GE_QQty',
    'CK_IODetail_Qty_NotZero'
)
ORDER BY type_desc, name
GO

PRINT '------ ALL triggers in current DB ------'
GO
SELECT name AS TriggerName, OBJECT_NAME(parent_id) AS OnTable, type_desc
FROM sys.triggers
WHERE is_disabled = 0
ORDER BY name
GO

PRINT '====== 7. DocNo / Numbering tables ======'
GO
SELECT name AS TableName FROM sysobjects
WHERE xtype = 'U' AND (name LIKE 'tSys_Doc%' OR name LIKE 'tDoc%' OR name LIKE 'tNum_%')
ORDER BY name
GO

PRINT '====== 8. Business PK survey (t% tables, first column only) ======'
GO
SELECT
    c.TABLE_NAME,
    c.COLUMN_NAME  AS PK1
FROM INFORMATION_SCHEMA.COLUMNS c
JOIN INFORMATION_SCHEMA.TABLES t
  ON t.TABLE_NAME = c.TABLE_NAME AND t.TABLE_TYPE = 'BASE TABLE'
WHERE c.ORDINAL_POSITION = 1
  AND c.TABLE_NAME LIKE 't%'
ORDER BY c.TABLE_NAME
GO

PRINT '====== DONE ======'
GO
