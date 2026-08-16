/* ============================================================================
   DB-02 核心业务表非聚集索引（SQL Server 2005 兼容）
   ----------------------------------------------------------------------------
   模块：DB-02
   目标：为 11 张完全无非聚集索引的表 + tStk_IODetail 补全关键索引，
        消除按外键/业务号/状态查询的全表扫描。
   现状（实测）：
     - tStk_IO 已有 12 个非聚集索引（充分）
     - tBas_Goods 已有 3 个（充分）
     - tStk_Stock 已有 (StkID,GDSID) 复合（充分）
     - 但 11 张明细/账本表完全无非聚集索引
     - tStk_IODetail 缺 IOID（最关键的 FK 查询列）
   设计原则：
     1) 只补"查询热点"列：外键、业务号、状态、日期
     2) 复合索引按等值列在前、范围列在后排列
     3) 2005 兼容：CREATE INDEX ... INCLUDE 不用（INCLUDE 是 2005+ 支持的，但保险起见不用）
   幂等：用 sys.indexes 判断是否存在。
   ============================================================================ */

USE [TestERP];
GO
SET NOCOUNT ON;
GO

PRINT N'========================================';
PRINT N'DB-02 核心业务表非聚集索引安装开始';
PRINT N'时间：' + CONVERT(nvarchar(19), GETDATE(), 120);
PRINT N'========================================';
GO

/* ---------- 辅助：幂等创建非聚集索引 ---------- */
-- 2005 不支持 CREATE INDEX IF NOT EXISTS，用动态 SQL + sys.indexes 判断
IF OBJECT_ID(N'tmp_db02_ensure_index', N'P') IS NOT NULL
    DROP PROCEDURE [dbo].[tmp_db02_ensure_index];
GO
CREATE PROCEDURE [dbo].[tmp_db02_ensure_index]
    @tbl     nvarchar(128),
    @idx     nvarchar(128),
    @cols    nvarchar(1000),
    @incl    nvarchar(1000) = NULL  -- 可选 INCLUDE 列（2005 支持）
AS
BEGIN
    SET NOCOUNT ON;
    IF EXISTS (SELECT 1 FROM sys.indexes WHERE name = @idx AND object_id = OBJECT_ID(@tbl))
    BEGIN
        PRINT N'[SKIP] ' + @idx + N' 已存在';
        RETURN;
    END
    DECLARE @sql nvarchar(4000);
    IF @incl IS NULL OR LEN(@incl) = 0
        SET @sql = N'CREATE NONCLUSTERED INDEX [' + @idx + N'] ON [dbo].[' + @tbl + N'] (' + @cols + N')';
    ELSE
        SET @sql = N'CREATE NONCLUSTERED INDEX [' + @idx + N'] ON [dbo].[' + @tbl + N'] (' + @cols + N') INCLUDE (' + @incl + N')';
    EXEC sp_executesql @sql;
    PRINT N'[OK] 已创建 ' + @idx + N' on ' + @tbl + N' (' + @cols + N')';
END
GO

/* ============================================================================
   1. tStk_IODetail —— 补 IOID（最关键，每次查单据明细都按此过滤）
   ============================================================================ */
EXEC tmp_db02_ensure_index N'tStk_IODetail', N'idx_IODetail_IOID',         N'IOID';
EXEC tmp_db02_ensure_index N'tStk_IODetail', N'idx_IODetail_GDSID_StkID',  N'GDSID, StkID';
GO

/* ============================================================================
   2. tStk_MoveDetail —— 完全无索引
   ============================================================================ */
EXEC tmp_db02_ensure_index N'tStk_MoveDetail', N'idx_MoveDetail_MoveID',  N'MoveID';
EXEC tmp_db02_ensure_index N'tStk_MoveDetail', N'idx_MoveDetail_GDSID',   N'GDSID';
GO

/* ============================================================================
   3. tStk_TranDetail —— 完全无索引
   ============================================================================ */
EXEC tmp_db02_ensure_index N'tStk_TranDetail', N'idx_TranDetail_TranID',  N'TranID';
EXEC tmp_db02_ensure_index N'tStk_TranDetail', N'idx_TranDetail_GDSID',   N'GDSID';
GO

/* ============================================================================
   4. tStk_Move —— 完全无索引
   ============================================================================ */
EXEC tmp_db02_ensure_index N'tStk_Move', N'idx_Move_MoveNo',       N'MoveNO';
EXEC tmp_db02_ensure_index N'tStk_Move', N'idx_Move_State',        N'State';
EXEC tmp_db02_ensure_index N'tStk_Move', N'idx_Move_FromToStk',    N'FromStkID, ToStkID';
GO

/* ============================================================================
   5. tStk_Tran —— 完全无索引
   ============================================================================ */
EXEC tmp_db02_ensure_index N'tStk_Tran', N'idx_Tran_TranNo',  N'TranNo';
EXEC tmp_db02_ensure_index N'tStk_Tran', N'idx_Tran_State',   N'State';
EXEC tmp_db02_ensure_index N'tStk_Tran', N'idx_Tran_StkID',   N'StkID';
GO

/* ============================================================================
   6. tStk_StockTranHis —— 完全无索引（按 GDSID+StkID UPSERT 查找）
   ============================================================================ */
EXEC tmp_db02_ensure_index N'tStk_StockTranHis', N'idx_StockTranHis_GDSID_StkID', N'GDSID, StkID';
EXEC tmp_db02_ensure_index N'tStk_StockTranHis', N'idx_StockTranHis_TranID',      N'TranID';
GO

/* ============================================================================
   7. tStk_StockYM —— 完全无索引（按 AccYM+StkID+GDSID UPSERT 查找）
   ============================================================================ */
EXEC tmp_db02_ensure_index N'tStk_StockYM', N'idx_StockYM_AccYM_StkID_GDSID', N'AccYM, StkID, GDSID';
GO

/* ============================================================================
   8. tStk_Qty —— 完全无索引（按 GDSID+StkID 查找）
   ============================================================================ */
EXEC tmp_db02_ensure_index N'tStk_Qty', N'idx_Qty_GDSID_StkID', N'GDSID, StkID';
GO

/* ============================================================================
   9. tSys_OperHis —— 完全无索引
   实测列：DocID/EmpID/MenusID/OperDate/OperHisID/OpenMsg（老 ERP 结构）
   ============================================================================ */
EXEC tmp_db02_ensure_index N'tSys_OperHis', N'idx_OperHis_OperDate',        N'OperDate';
EXEC tmp_db02_ensure_index N'tSys_OperHis', N'idx_OperHis_EmpID_OperDate',  N'EmpID, OperDate';
EXEC tmp_db02_ensure_index N'tSys_OperHis', N'idx_OperHis_MenusID',         N'MenusID, OperDate';
GO

/* ============================================================================
   10. tSal_OrderDetail / tPur_OrderDetail —— 完全无索引
   ============================================================================ */
EXEC tmp_db02_ensure_index N'tSal_OrderDetail', N'idx_SODetail_SOID',   N'SOID';
EXEC tmp_db02_ensure_index N'tSal_OrderDetail', N'idx_SODetail_GDSID',  N'GDSID';
EXEC tmp_db02_ensure_index N'tPur_OrderDetail', N'idx_PODetail_POID',   N'POID';
EXEC tmp_db02_ensure_index N'tPur_OrderDetail', N'idx_PODetail_GDSID',  N'GDSID';
GO

/* ============================================================================
   11. tStk_ReplenishApplyDtl —— 完全无索引
   ============================================================================ */
EXEC tmp_db02_ensure_index N'tStk_ReplenishApplyDtl', N'idx_ReplenishDtl_ApplyID', N'ReplenishApplyID';
GO

/* ============================================================================
   12. tStk_Reserve —— 预占表（按 DocType+DocID+GDSID 查找）
   ============================================================================ */
IF OBJECT_ID(N'tStk_Reserve', N'U') IS NOT NULL
BEGIN
    EXEC tmp_db02_ensure_index N'tStk_Reserve', N'idx_Reserve_DocType_DocID', N'DocType, DocID';
    EXEC tmp_db02_ensure_index N'tStk_Reserve', N'idx_Reserve_GDSID_StkID',   N'GDSID, StkID, State';
END
GO

/* ============================================================================
   清理临时过程 + 验证
   ============================================================================ */
DROP PROCEDURE [dbo].[tmp_db02_ensure_index];
GO

PRINT N'';
PRINT N'--- 本次新增的非聚集索引 ---';
SELECT  OBJECT_NAME(i.object_id) AS [表],
        i.name                   AS [索引],
        STUFF((SELECT N', ' + COL_NAME(ic.object_id, ic.column_id)
               FROM sys.index_columns ic
               WHERE ic.object_id = i.object_id AND ic.index_id = i.index_id
                 AND ic.is_included_column = 0
               ORDER BY ic.key_ordinal
               FOR XML PATH(N'')), 1, 2, N'') AS [列]
FROM    sys.indexes i
WHERE   i.type = 2  -- NONCLUSTERED
  AND   i.is_primary_key = 0
  AND   i.name LIKE N'idx[_]%'
  AND   OBJECT_NAME(i.object_id) IN (
    N'tStk_IODetail', N'tStk_MoveDetail', N'tStk_TranDetail',
    N'tStk_Move', N'tStk_Tran', N'tStk_StockTranHis', N'tStk_StockYM',
    N'tStk_Qty', N'tSys_OperHis', N'tSal_OrderDetail', N'tPur_OrderDetail',
    N'tStk_ReplenishApplyDtl', N'tStk_Reserve')
ORDER BY [表], [索引];
GO

PRINT N'';
PRINT N'=== DB-02 完成 ===';
GO
