/* ============================================================================
   DB-08 软删除/恢复存储过程（SQL Server 2005 兼容）
   ----------------------------------------------------------------------------
   模块：DB-08
   目标：提供统一的软删除/恢复接口，按表自动选 State/Used 字段，级联明细
   过程：
     sp_SoftDelete @Table, @PK, @ID, @Void (0=State='D' / 1=State='C')
     sp_Restore   @Table, @PK, @ID
   策略：
     - Used 表：删除置 'N'，恢复置 'Y'
     - State 表：删除置 'D'（或 'C' 作废），恢复置 'N'
     - 级联：若该表有 detail_meta 映射的明细表且明细也有状态列，则同步
   注意：本过程是 DB 层兜底接口，应用层（generic_delete）已有更完整实现。
   ============================================================================ */

USE [TestERP];
GO
SET NOCOUNT ON;
GO

/* ---------- 辅助：根据表名返回状态字段（'State' / 'Used' / NULL）---------- */
IF OBJECT_ID(N'fn_GetStateField', N'FN') IS NOT NULL DROP FUNCTION [dbo].[fn_GetStateField];
GO
CREATE FUNCTION [dbo].[fn_GetStateField] (@Table nvarchar(128))
RETURNS nvarchar(10)
AS
BEGIN
    DECLARE @f nvarchar(10);
    SET @f = CASE
        WHEN @Table IN (N'tBas_Brand', N'tBas_Stock', N'tBas_GDSType', N'tBas_GDSProperty',
                        N'tBas_GDSKind', N'tBas_DeaType', N'tBas_Unit', N'tBas_SuppType',
                        N'tBas_CustType', N'tBas_Area', N'tBas_Dept', N'tBas_Duty',
                        N'tBas_Payment', N'tSys_Menus') THEN N'Used'
        WHEN @Table IN (N'tBas_Goods', N'tBas_Supp', N'tBas_Cust', N'tBas_Emp',
                        N'tPur_Order', N'tSal_Order', N'tSal_Inv', N'tStk_IO', N'tStk_Move',
                        N'tStk_ReplenishApply', N'tStk_StockCycle', N'tStk_Tran',
                        N'tAcc_PayOut', N'tAcc_PayIn', N'tSys_Rpt', N'tSys_User',
                        N'tSys_Rule', N'tSal_VIP') THEN N'State'
        ELSE NULL
    END
    RETURN @f;
END
GO

/* ---------- 辅助：根据主表返回明细表信息（detail_table, fk_col）---------- */
IF OBJECT_ID(N'fn_GetDetailMeta', N'FN') IS NOT NULL DROP FUNCTION [dbo].[fn_GetDetailMeta];
GO
CREATE FUNCTION [dbo].[fn_GetDetailMeta] (@Table nvarchar(128))
RETURNS nvarchar(260)  -- 'detail_table|fk_col' 或 NULL
AS
BEGIN
    DECLARE @r nvarchar(260);
    SET @r = CASE
        WHEN @Table = N'tSal_Order'          THEN N'tSal_OrderDetail|SOID'
        WHEN @Table = N'tSal_Inv'            THEN N'tSal_InvDetail|SIID'
        WHEN @Table = N'tPur_Order'          THEN N'tPur_OrderDetail|POID'
        WHEN @Table = N'tStk_IO'             THEN N'tStk_IODetail|IOID'
        WHEN @Table = N'tStk_Move'           THEN N'tStk_MoveDetail|MoveID'
        WHEN @Table = N'tStk_Tran'           THEN N'tStk_TranDetail|TranID'
        WHEN @Table = N'tStk_ReplenishApply' THEN N'tStk_ReplenishApplyDtl|ReplenishApplyID'
        ELSE NULL
    END
    RETURN @r;
END
GO

/* ---------- sp_SoftDelete ---------- */
IF OBJECT_ID(N'sp_SoftDelete', N'P') IS NOT NULL DROP PROCEDURE [dbo].[sp_SoftDelete];
GO
CREATE PROCEDURE [dbo].[sp_SoftDelete]
    @Table nvarchar(128),
    @PK    nvarchar(60),
    @ID    nvarchar(40),
    @Void  int = 0      -- 0=State='D'（软删），1=State='C'（作废）
AS
BEGIN
    SET NOCOUNT ON;
    DECLARE @sf nvarchar(10);
    SET @sf = [dbo].[fn_GetStateField](@Table);
    IF @sf IS NULL
    BEGIN
        RAISERROR(N'表 [%s] 无软删字段（既无 State 也无 Used）', 16, 1, @Table);
        RETURN -1;
    END

    DECLARE @val nvarchar(2);
    IF @sf = N'Used' SET @val = N'N';
    ELSE IF @Void = 1 SET @val = N'C';
    ELSE SET @val = N'D';

    -- ★ 防注入：标识符用 QUOTENAME 包裹，值用 sp_executesql 参数化
    DECLARE @sql nvarchar(4000);
    SET @sql = N'UPDATE [dbo].' + QUOTENAME(@Table) + N' SET ' + QUOTENAME(@sf) +
               N' = @val WHERE ' + QUOTENAME(@PK) + N' = @id';
    EXEC sp_executesql @sql, N'@val nvarchar(2), @id nvarchar(40)', @val = @val, @id = @ID;
    DECLARE @rows int; SET @rows = @@ROWCOUNT;

    -- 级联明细（若明细表也有 State/Used 列）
    DECLARE @meta nvarchar(260);
    SET @meta = [dbo].[fn_GetDetailMeta](@Table);
    IF @meta IS NOT NULL AND @rows > 0
    BEGIN
        DECLARE @dt nvarchar(128), @fk nvarchar(60);
        SET @dt = LEFT(@meta, CHARINDEX(N'|', @meta) - 1);
        SET @fk  = SUBSTRING(@meta, CHARINDEX(N'|', @meta) + 1, 60);
        -- 检查明细表是否有状态列
        IF EXISTS (SELECT 1 FROM sys.columns WHERE object_id = OBJECT_ID(@dt) AND name = @sf)
        BEGIN
            SET @sql = N'UPDATE [dbo].' + QUOTENAME(@dt) + N' SET ' + QUOTENAME(@sf) +
                       N' = @val WHERE ' + QUOTENAME(@fk) + N' = @id';
            EXEC sp_executesql @sql, N'@val nvarchar(2), @id nvarchar(40)', @val = @val, @id = @ID;
            PRINT N'级联软删明细 ' + @dt + N' (' + @fk + N'=' + @ID + N')';
        END
    END

    RETURN @rows;
END
GO

/* ---------- sp_Restore ---------- */
IF OBJECT_ID(N'sp_Restore', N'P') IS NOT NULL DROP PROCEDURE [dbo].[sp_Restore];
GO
CREATE PROCEDURE [dbo].[sp_Restore]
    @Table nvarchar(128),
    @PK    nvarchar(60),
    @ID    nvarchar(40)
AS
BEGIN
    SET NOCOUNT ON;
    DECLARE @sf nvarchar(10);
    SET @sf = [dbo].[fn_GetStateField](@Table);
    IF @sf IS NULL
    BEGIN
        RAISERROR(N'表 [%s] 无软删字段', 16, 1, @Table);
        RETURN -1;
    END

    DECLARE @val nvarchar(2);
    IF @sf = N'Used' SET @val = N'Y'; ELSE SET @val = N'N';

    -- ★ 防注入：标识符用 QUOTENAME 包裹，值用 sp_executesql 参数化
    DECLARE @sql nvarchar(4000);
    SET @sql = N'UPDATE [dbo].' + QUOTENAME(@Table) + N' SET ' + QUOTENAME(@sf) +
               N' = @val WHERE ' + QUOTENAME(@PK) + N' = @id';
    EXEC sp_executesql @sql, N'@val nvarchar(2), @id nvarchar(40)', @val = @val, @id = @ID;
    DECLARE @rows int; SET @rows = @@ROWCOUNT;

    -- 级联恢复明细
    DECLARE @meta nvarchar(260);
    SET @meta = [dbo].[fn_GetDetailMeta](@Table);
    IF @meta IS NOT NULL AND @rows > 0
    BEGIN
        DECLARE @dt nvarchar(128), @fk nvarchar(60);
        SET @dt = LEFT(@meta, CHARINDEX(N'|', @meta) - 1);
        SET @fk  = SUBSTRING(@meta, CHARINDEX(N'|', @meta) + 1, 60);
        IF EXISTS (SELECT 1 FROM sys.columns WHERE object_id = OBJECT_ID(@dt) AND name = @sf)
        BEGIN
            SET @sql = N'UPDATE [dbo].' + QUOTENAME(@dt) + N' SET ' + QUOTENAME(@sf) +
                       N' = @val WHERE ' + QUOTENAME(@fk) + N' = @id';
            EXEC sp_executesql @sql, N'@val nvarchar(2), @id nvarchar(40)', @val = @val, @id = @ID;
            PRINT N'级联恢复明细 ' + @dt;
        END
    END
    RETURN @rows;
END
GO

PRINT N'=== DB-08 完成 ===';
PRINT N'sp_SoftDelete / sp_Restore 已就绪（带级联明细）';
GO
