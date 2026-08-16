/* ============================================================================
   DB-07-rollback 月结回滚存储过程
   ----------------------------------------------------------------------------
   模块：DB-07-rollback
   目标：提供月结回滚（反月结）能力，删除指定月份的期初结存记录
   背景：
     - month_end_settle 将 from_ym 的 EndQty 作为 to_ym 的 InitQty 写入 StockYM
     - 回滚时需删除 to_ym 的 StockYM 记录，使该月恢复"未结存"状态
   安全策略：
     1) 如果 to_ym 的 inQty 或 OutQty 不为 0，说明该月已有业务活动，默认拒绝回滚
     2) @Force=1 可强制回滚（危险，仅管理员使用）
     3) 回滚后该月数据视为"未月结"，需重新执行月结
   ============================================================================ */

USE [TestERP];
GO
SET NOCOUNT ON;
GO

IF OBJECT_ID(N'sp_MonthSettleRollback', N'P') IS NOT NULL
    DROP PROCEDURE [dbo].[sp_MonthSettleRollback];
GO

/* ============================================================================
   sp_MonthSettleRollback
   参数：
     @ToYM    int  -- 要回滚的目标月份 YYYYMM（如 202606）
     @Force   int  -- 0=安全模式（默认），1=强制回滚（忽略业务数据检查）
   返回：
     >0 = 删除的行数
     -1 = 参数错误
     -2 = 该月已有业务活动（inQty/OutQty 非0），拒绝回滚
     -3 = 该月无 StockYM 记录，无需回滚
   ============================================================================ */
CREATE PROCEDURE [dbo].[sp_MonthSettleRollback]
    @ToYM   int,
    @Force  int = 0
AS
BEGIN
    SET NOCOUNT ON;

    -- 1) 参数校验
    IF @ToYM < 200001 OR @ToYM > 209912
    BEGIN
        RAISERROR(N'@ToYM 格式应为 YYYYMM（如 202606）', 16, 1);
        RETURN -1;
    END

    -- 2) 检查该月是否有 StockYM 记录
    DECLARE @cnt int, @hasBiz int;
    SELECT @cnt = COUNT(*),
           @hasBiz = SUM(CASE WHEN ISNULL(inQty, 0) <> 0 OR ISNULL(OutQty, 0) <> 0 THEN 1 ELSE 0 END)
    FROM [dbo].[tStk_StockYM]
    WHERE AccYM = @ToYM;

    IF @cnt = 0
    BEGIN
        PRINT N'月份 ' + CAST(@ToYM AS nvarchar(10)) + N' 无 StockYM 记录，无需回滚';
        RETURN -3;
    END

    -- 3) 安全检查：该月是否已有业务活动
    IF @Force = 0 AND @hasBiz > 0
    BEGIN
        RAISERROR(N'月份 [%d] 已有 %d 条业务记录（inQty/OutQty 非0），拒绝回滚。如需强制回滚请使用 @Force=1', 16, 1, @ToYM, @hasBiz);
        RETURN -2;
    END

    -- 4) 执行回滚：删除该月的 StockYM 记录
    DELETE FROM [dbo].[tStk_StockYM]
    WHERE AccYM = @ToYM;

    DECLARE @rows int; SET @rows = @@ROWCOUNT;
    PRINT N'月结回滚完成：删除月份 ' + CAST(@ToYM AS nvarchar(10)) + N' 的 ' + CAST(@rows AS nvarchar(10)) + N' 条 StockYM 记录';
    RETURN @rows;
END
GO

PRINT N'=== DB-07-rollback 完成 ===';
PRINT N'sp_MonthSettleRollback 已就绪（月结回滚，安全模式默认拒绝有业务活动的月份）';
GO