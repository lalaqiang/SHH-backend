/* ============================================================================
   DB-02-cleanup 清理重复索引
   ----------------------------------------------------------------------------
   问题：多个脚本为同一表+字段组合创建了不同名称的索引，导致：
     1. 浪费存储空间
     2. INSERT/UPDATE/DELETE 性能下降（每次写操作更新多个索引）
     3. 查询优化器可能选择次优索引

   重复索引汇总：
   ┌─────────────────────┬──────────────────────┬──────────────────────────────┬──────────────────────────────────────────┐
   │ 表                  │ 字段组合             │ 重复索引名                   │ 保留（最优）                             │
   ├─────────────────────┼──────────────────────┼──────────────────────────────┼──────────────────────────────────────────┤
   │ tStk_Reserve        │ (DocType, DocID)     │ idx_Reserve_DocType_DocID    │ IX_tStk_Reserve_DocType_DocID            │
   │                     │                      │ IX_tStk_Reserve_Doc          │   (含 GDSID,StkID + INCLUDE)             │
   ├─────────────────────┼──────────────────────┼──────────────────────────────┼──────────────────────────────────────────┤
   │ tStk_Reserve        │ (GDSID, StkID)       │ IX_tStk_Reserve_Gds          │ idx_Reserve_GDSID_StkID                  │
   │                     │                      │                              │   (含 State)                             │
   ├─────────────────────┼──────────────────────┼──────────────────────────────┼──────────────────────────────────────────┤
   │ tStk_IODetail       │ (IOID)               │ idx_IODetail_IOID            │ IX_tStk_IODetail_IOID                    │
   │                     │                      │                              │   (含 INCLUDE GDSID,Qty,Price,SumAmt)    │
   ├─────────────────────┼──────────────────────┼──────────────────────────────┼──────────────────────────────────────────┤
   │ tStk_StockTranHis   │ (GDSID, StkID)       │ idx_StockTranHis_GDSID_StkID │ IX_tStk_StockTranHis_GDSID_StkID_Date    │
   │                     │                      │                              │   (含 TranDate DESC + INCLUDE)           │
   └─────────────────────┴──────────────────────┴──────────────────────────────┴──────────────────────────────────────────┘

   策略：DROP 次优索引，保留有 INCLUDE 的覆盖索引版本
   ============================================================================ */

USE [TestERP];
GO
SET NOCOUNT ON;
GO

-- 辅助存储过程：安全 DROP 索引（不存在则跳过）
IF OBJECT_ID(N'tmp_db02cleanup_drop_index', N'P') IS NOT NULL
    DROP PROCEDURE [dbo].[tmp_db02cleanup_drop_index];
GO
CREATE PROCEDURE [dbo].[tmp_db02cleanup_drop_index]
    @Table nvarchar(128),
    @Index nvarchar(128)
AS
BEGIN
    SET NOCOUNT ON;
    IF EXISTS (
        SELECT 1 FROM sys.indexes
        WHERE name = @Index
          AND object_id = OBJECT_ID(@Table)
          AND is_primary_key = 0       -- 不碰主键
          AND is_unique_constraint = 0  -- 不碰唯一约束
    )
    BEGIN
        DECLARE @sql nvarchar(1000);
        SET @sql = N'DROP INDEX [' + @Index + N'] ON [dbo].[' + @Table + N']';
        EXEC sp_executesql @sql;
        PRINT N'  [DROP] ' + @Table + N'.' + @Index;
    END
    ELSE
    BEGIN
        PRINT N'  [SKIP] ' + @Table + N'.' + @Index + N' (不存在)';
    END
END
GO

PRINT N'=== 清理重复索引开始 ===';

-- ── 1. tStk_Reserve (DocType, DocID) ──────────────────────────────
-- 保留：IX_tStk_Reserve_DocType_DocID（含 GDSID,StkID + INCLUDE Qty,ReleasedQty,State）
-- 删除：idx_Reserve_DocType_DocID（DB-02 创建，无 INCLUDE）
-- 删除：IX_tStk_Reserve_Doc（init_new_tables 创建，只有 DocType,DocID 两列）
PRINT N'1. tStk_Reserve (DocType, DocID) 重复索引清理';
EXEC tmp_db02cleanup_drop_index N'tStk_Reserve', N'idx_Reserve_DocType_DocID';
EXEC tmp_db02cleanup_drop_index N'tStk_Reserve', N'IX_tStk_Reserve_Doc';
GO

-- ── 2. tStk_Reserve (GDSID, StkID) ───────────────────────────────
-- 保留：idx_Reserve_GDSID_StkID（DB-02 创建，含 State 三列复合）
-- 删除：IX_tStk_Reserve_Gds（init_new_tables 创建，只有 GDSID,StkID 两列）
PRINT N'2. tStk_Reserve (GDSID, StkID) 重复索引清理';
EXEC tmp_db02cleanup_drop_index N'tStk_Reserve', N'IX_tStk_Reserve_Gds';
GO

-- ── 3. tStk_IODetail (IOID) ──────────────────────────────────────
-- 保留：IX_tStk_IODetail_IOID（optimize_indexes 创建，含 INCLUDE GDSID,Qty,Price,SumAmt）
-- 删除：idx_IODetail_IOID（DB-02 创建，无 INCLUDE）
PRINT N'3. tStk_IODetail (IOID) 重复索引清理';
EXEC tmp_db02cleanup_drop_index N'tStk_IODetail', N'idx_IODetail_IOID';
GO

-- ── 4. tStk_StockTranHis (GDSID, StkID) ──────────────────────────
-- 保留：IX_tStk_StockTranHis_GDSID_StkID_Date（optimize_indexes 创建，含 TranDate DESC + INCLUDE）
-- 删除：idx_StockTranHis_GDSID_StkID（DB-02 创建，无 TranDate 无 INCLUDE）
PRINT N'4. tStk_StockTranHis (GDSID, StkID) 重复索引清理';
EXEC tmp_db02cleanup_drop_index N'tStk_StockTranHis', N'idx_StockTranHis_GDSID_StkID';
GO

-- 清理临时存储过程
DROP PROCEDURE [dbo].[tmp_db02cleanup_drop_index];
GO

PRINT N'=== 清理重复索引完成 ===';
PRINT N'提示：后续应统一索引管理，避免多脚本各自创建索引。';
GO
