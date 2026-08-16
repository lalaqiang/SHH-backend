/* ============================================================================
   DB-03 选择性外键约束（SQL Server 2005 兼容）
   ----------------------------------------------------------------------------
   模块：DB-03
   目标：为核心"明细→主表"关系补全 FK 约束，防止明细行引用不存在的主表记录。
   策略：
     1) 只对强关系建 FK：明细表.FK → 主表.PK（如 tStk_IODetail.IOID → tStk_IO.IOID）
     2) 跳过弱关系：明细→基础资料（如 GDSID→tBas_Goods）——老库脏数据多，建 FK 会阻塞
     3) WITH NOCHECK：跳过历史脏数据，只约束新数据（不因老库执行失败）
     4) ON DELETE NO ACTION ON UPDATE NO ACTION：禁止 FK 级联删改（应用层管软删）
   注意：FK 会带来写入开销（每次 INSERT 明细查一次主表），但保护价值大于开销。
   幂等：用 sys.foreign_keys 判断。
   ----------------------------------------------------------------------------
   覆盖范围（共 18 个 FK）：
     第 1 节：库存单据主从（IODetail/MoveDetail/TranDetail/ReplenishApplyDtl）4 个
     第 2 节：采购/销售单据主从（PurOrder/PurQuote/SalOrder/SalInv/SalQuote）5 个
     第 3 节：装箱/物料动作/活动计划（IOBoxDtl/MatActDtl/ActPlanDtl）3 个
     第 4 节：补充缺失主从（PurInv/PurReturn/PurAdjPrice/SalReturn/StockCycle/OnlineOrder）6 个
   ============================================================================ */

USE [TestERP];
GO
SET NOCOUNT ON;
GO

PRINT N'========================================';
PRINT N'DB-03 选择性外键约束安装开始';
PRINT N'时间：' + CONVERT(nvarchar(19), GETDATE(), 120);
PRINT N'========================================';
GO

/* ---------- 辅助：幂等创建 FK ---------- */
IF OBJECT_ID(N'tmp_db03_ensure_fk', N'P') IS NOT NULL
    DROP PROCEDURE [dbo].[tmp_db03_ensure_fk];
GO
CREATE PROCEDURE [dbo].[tmp_db03_ensure_fk]
    @fk       nvarchar(128),
    @child    nvarchar(128),
    @child_col nvarchar(128),
    @parent   nvarchar(128),
    @parent_col nvarchar(128)
AS
BEGIN
    SET NOCOUNT ON;
    IF EXISTS (SELECT 1 FROM sys.foreign_keys WHERE name = @fk)
    BEGIN
        PRINT N'[SKIP] ' + @fk + N' 已存在';
        RETURN;
    END
    -- 校验子表/父表/列都存在（避免脚本对脏库执行失败）
    IF NOT EXISTS (SELECT 1 FROM sys.columns WHERE object_id=OBJECT_ID(@child) AND name=@child_col)
    BEGIN
        PRINT N'[WARN] ' + @child + N'.' + @child_col + N' 列不存在，跳过 ' + @fk;
        RETURN;
    END
    IF NOT EXISTS (SELECT 1 FROM sys.columns WHERE object_id=OBJECT_ID(@parent) AND name=@parent_col)
    BEGIN
        PRINT N'[WARN] ' + @parent + N'.' + @parent_col + N' 列不存在，跳过 ' + @fk;
        RETURN;
    END
    DECLARE @sql nvarchar(4000);
    SET @sql = N'ALTER TABLE [dbo].[' + @child + N'] WITH NOCHECK ADD CONSTRAINT [' + @fk +
               N'] FOREIGN KEY ([' + @child_col + N']) REFERENCES [dbo].[' + @parent +
               N'] ([' + @parent_col + N']) ON DELETE NO ACTION ON UPDATE NO ACTION';
    EXEC sp_executesql @sql;
    PRINT N'[OK] 已创建 FK ' + @fk + N': ' + @child + N'.' + @child_col + N' -> ' + @parent + N'.' + @parent_col;
END
GO

/* ============================================================================
   1. 库存单据主从关系（最关键）
   ============================================================================ */
EXEC tmp_db03_ensure_fk N'FK_IODetail_IO',       N'tStk_IODetail',       N'IOID',   N'tStk_IO',   N'IOID';
EXEC tmp_db03_ensure_fk N'FK_MoveDetail_Move',   N'tStk_MoveDetail',     N'MoveID', N'tStk_Move', N'MoveID';
EXEC tmp_db03_ensure_fk N'FK_TranDetail_Tran',   N'tStk_TranDetail',     N'TranID', N'tStk_Tran', N'TranID';
EXEC tmp_db03_ensure_fk N'FK_ReplenishDtl_Apply',N'tStk_ReplenishApplyDtl', N'ReplenishApplyID', N'tStk_ReplenishApply', N'ReplenishApplyID';
GO

/* ============================================================================
   2. 采购/销售单据主从关系
   ============================================================================ */
EXEC tmp_db03_ensure_fk N'FK_PurOrderDtl_Order', N'tPur_OrderDetail', N'POID', N'tPur_Order', N'POID';
EXEC tmp_db03_ensure_fk N'FK_PurQuoteDtl_Quote', N'tPur_QuoteDetail', N'PQID', N'tPur_Quote', N'PQID';
EXEC tmp_db03_ensure_fk N'FK_SalOrderDtl_Order', N'tSal_OrderDetail', N'SOID', N'tSal_Order', N'SOID';
EXEC tmp_db03_ensure_fk N'FK_SalInvDtl_Inv',     N'tSal_InvDetail',   N'SIID', N'tSal_Inv',   N'SIID';
EXEC tmp_db03_ensure_fk N'FK_SalQuoteDtl_Quote', N'tSal_QuoteDetail', N'SQID', N'tSal_Quote', N'SQID';
GO

/* ============================================================================
   3. 装箱/物料动作等辅助主从
   ============================================================================ */
IF OBJECT_ID(N'tStk_IOBoxDtl', N'U') IS NOT NULL
    EXEC tmp_db03_ensure_fk N'FK_IOBoxDtl_IO', N'tStk_IOBoxDtl', N'DocID', N'tStk_IO', N'IOID';
EXEC tmp_db03_ensure_fk N'FK_MatActDtl_MatAct', N'tStk_MatActDtl', N'MatActID', N'tStk_MatAct', N'MatActID';
EXEC tmp_db03_ensure_fk N'FK_ActPlanDtl_ActPlan', N'tStk_ActPlanDtl', N'ActPlanID', N'tStk_ActPlan', N'ActPlanID';
GO

/* ============================================================================
   4. 补充缺失的单据主从关系（Sprint6-34）
   ----------------------------------------------------------------------------
   涵盖：采购入库/退货/调价、销售退货、周期盘点、线上订单
   策略与 1-3 节一致：WITH NOCHECK + ON DELETE/UPDATE NO ACTION
   安全性：tmp_db03_ensure_fk 已做 sys.columns 列存在性预检，脏库自动跳过
   ============================================================================ */
-- 采购入库（tPur_InvDetail.PIID -> tPur_Inv.PIID）
IF OBJECT_ID(N'tPur_InvDetail', N'U') IS NOT NULL
    EXEC tmp_db03_ensure_fk N'FK_PurInvDtl_Inv', N'tPur_InvDetail', N'PIID', N'tPur_Inv', N'PIID';

-- 采购退货（tPur_ReturnDetail.PRID -> tPur_Return.PRID）
IF OBJECT_ID(N'tPur_ReturnDetail', N'U') IS NOT NULL
    EXEC tmp_db03_ensure_fk N'FK_PurReturnDtl_Return', N'tPur_ReturnDetail', N'PRID', N'tPur_Return', N'PRID';

-- 采购调价（tPur_AdjPriceDetail.PAID -> tPur_AdjPrice.PAID）
IF OBJECT_ID(N'tPur_AdjPriceDetail', N'U') IS NOT NULL
    EXEC tmp_db03_ensure_fk N'FK_PurAdjPriceDtl_AdjPrice', N'tPur_AdjPriceDetail', N'PAID', N'tPur_AdjPrice', N'PAID';

-- 销售退货（tSal_ReturnDetail.SRID -> tSal_Return.SRID）
IF OBJECT_ID(N'tSal_ReturnDetail', N'U') IS NOT NULL
    EXEC tmp_db03_ensure_fk N'FK_SalReturnDtl_Return', N'tSal_ReturnDetail', N'SRID', N'tSal_Return', N'SRID';

-- 周期盘点（tStk_StockCycleDetail.StockCycleID -> tStk_StockCycle.StockCycleID）
IF OBJECT_ID(N'tStk_StockCycleDetail', N'U') IS NOT NULL
    EXEC tmp_db03_ensure_fk N'FK_StockCycleDtl_StockCycle', N'tStk_StockCycleDetail', N'StockCycleID', N'tStk_StockCycle', N'StockCycleID';

-- 线上订单（tOnline_OrderDetail.OnlineOrderID -> tOnline_Order.OnlineOrderID）
IF OBJECT_ID(N'tOnline_OrderDetail', N'U') IS NOT NULL
    EXEC tmp_db03_ensure_fk N'FK_OnlineOrderDtl_Order', N'tOnline_OrderDetail', N'OnlineOrderID', N'tOnline_Order', N'OnlineOrderID';
GO

/* ============================================================================
   清理临时过程 + 验证
   ============================================================================ */
DROP PROCEDURE [dbo].[tmp_db03_ensure_fk];
GO

PRINT N'';
PRINT N'--- 本次新增的外键（WITH NOCHECK）---';
SELECT  fk.name AS [外键名],
        OBJECT_NAME(fk.parent_object_id) AS [子表],
        COL_NAME(fkc.parent_object_id, fkc.parent_column_id) AS [子列],
        OBJECT_NAME(fk.referenced_object_id) AS [父表],
        COL_NAME(fkc.referenced_object_id, fkc.referenced_column_id) AS [父列]
FROM    sys.foreign_keys fk
JOIN    sys.foreign_key_columns fkc ON fk.object_id = fkc.constraint_object_id
WHERE   fk.name LIKE N'FK[_]%'
  AND   fk.name NOT IN (N'FK_TSYS_DAT_REFERENCE_TSYS_DAT', N'FK_TSYS_DAT_REFERENCE_TSYS_OPE')
ORDER BY [子表], [外键名];
GO

PRINT N'';
PRINT N'=== DB-03 完成 ===';
PRINT N'所有 FK 用 WITH NOCHECK 创建（跳过历史脏数据，只约束新数据）。';
PRINT N'ON DELETE/UPDATE NO ACTION：禁止 FK 级联，软删由应用层管。';
GO
