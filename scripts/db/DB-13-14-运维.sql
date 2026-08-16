/* ============================================================================
   DB-13 统计信息更新作业 + DB-14 数据清理存储过程（SQL Server 2005 兼容）
   ----------------------------------------------------------------------------
   合并产出 DB-13 + DB-14（两个都是运维类小模块）
   DB-13：sp_UpdateAllStats —— 全库统计信息刷新（索引变更后必须做）
   DB-14：sp_CleanHistoryData —— 清理历史操作日志/临时表
   ============================================================================ */

USE [TestERP];
GO
SET NOCOUNT ON;
GO

/* ============================================================================
   DB-13: sp_UpdateAllStats
   目标：刷新全库统计信息（DB-02 加了 24 个索引后，统计信息需更新才能让查询优化器用上新索引）
   建议：索引变更后立即执行一次；之后每天凌晨定时执行一次（sp_updatestats）
   ============================================================================ */
IF OBJECT_ID(N'sp_UpdateAllStats', N'P') IS NOT NULL DROP PROCEDURE [dbo].[sp_UpdateAllStats];
GO
CREATE PROCEDURE [dbo].[sp_UpdateAllStats]
AS
BEGIN
    SET NOCOUNT ON;
    DECLARE @start datetime; SET @start = GETDATE();
    -- sp_updatestats 是系统存储过程，按需更新所有表的统计信息
    EXEC sp_updatestats;
    DECLARE @dur int; SET @dur = DATEDIFF(second, @start, GETDATE());
    PRINT N'统计信息更新完成，耗时 ' + CONVERT(nvarchar(10), @dur) + N' 秒';
END
GO

PRINT N'[OK] sp_UpdateAllStats 已创建';
GO

/* ============================================================================
   DB-14: sp_CleanHistoryData
   目标：清理历史数据，释放空间
   参数：
     @KeepDays int = 90   —— 操作日志保留天数（默认 90 天）
     @DryRun   int = 1    —— 1=只统计不删（dry run），0=实际删除
   清理目标：
     - tSys_OperHis：按 OperDate 早于 N 天
     - tStk_TranFasTmp：临时表全清
   注意：不清理业务单据（tStk_IO 等），单据是业务数据不能按时间删
   ============================================================================ */
IF OBJECT_ID(N'sp_CleanHistoryData', N'P') IS NOT NULL DROP PROCEDURE [dbo].[sp_CleanHistoryData];
GO
CREATE PROCEDURE [dbo].[sp_CleanHistoryData]
    @KeepDays int = 90,
    @DryRun   int = 1
AS
BEGIN
    SET NOCOUNT ON;
    DECLARE @cutoff datetime;
    SET @cutoff = DATEADD(day, -@KeepDays, GETDATE());

    PRINT N'清理截止日期：' + CONVERT(nvarchar(19), @cutoff, 120) +
          N'（保留 ' + CONVERT(nvarchar(10), @KeepDays) + N' 天）' +
          CASE WHEN @DryRun = 1 THEN N' [DRY RUN]' ELSE N' [实际删除]' END;

    -- 1. tSys_OperHis（操作日志）
    DECLARE @cnt1 int;
    SELECT @cnt1 = COUNT(*) FROM [dbo].[tSys_OperHis] WHERE OperDate < @cutoff;
    IF @DryRun = 1
        PRINT N'  [将删除] tSys_OperHis：' + CONVERT(nvarchar(10), @cnt1) + N' 行';
    ELSE
    BEGIN
        DELETE FROM [dbo].[tSys_OperHis] WHERE OperDate < @cutoff;
        PRINT N'  [已删除] tSys_OperHis：' + CONVERT(nvarchar(10), @cnt1) + N' 行';
    END

    -- 2. tStk_TranFasTmp（快速录入临时表，全清）
    DECLARE @cnt2 int;
    IF OBJECT_ID(N'tStk_TranFasTmp', N'U') IS NOT NULL
    BEGIN
        SELECT @cnt2 = COUNT(*) FROM [dbo].[tStk_TranFasTmp];
        IF @DryRun = 1
            PRINT N'  [将删除] tStk_TranFasTmp：' + CONVERT(nvarchar(10), @cnt2) + N' 行（全部临时数据）';
        ELSE
        BEGIN
            DELETE FROM [dbo].[tStk_TranFasTmp];
            PRINT N'  [已删除] tStk_TranFasTmp：' + CONVERT(nvarchar(10), @cnt2) + N' 行';
        END
    END

    -- 3. tSys_PrintLog（打印日志）
    DECLARE @cnt3 int;
    IF OBJECT_ID(N'tSys_PrintLog', N'U') IS NOT NULL
    BEGIN
        SELECT @cnt3 = COUNT(*) FROM [dbo].[tSys_PrintLog] WHERE PrintDate < @cutoff;
        IF @DryRun = 1
            PRINT N'  [将删除] tSys_PrintLog：' + CONVERT(nvarchar(10), @cnt3) + N' 行';
        ELSE
        BEGIN
            DELETE FROM [dbo].[tSys_PrintLog] WHERE PrintDate < @cutoff;
            PRINT N'  [已删除] tSys_PrintLog：' + CONVERT(nvarchar(10), @cnt3) + N' 行';
        END
    END

    PRINT N'清理完成。';
END
GO

PRINT N'[OK] sp_CleanHistoryData 已创建';
GO

/* ---------- 验证 ---------- */
PRINT N'';
PRINT N'--- 验证：sp_UpdateAllStats（首次执行，因 DB-02 加了索引）---';
EXEC sp_UpdateAllStats;
PRINT N'';
PRINT N'--- 验证：sp_CleanHistoryData（DryRun 模式）---';
EXEC sp_CleanHistoryData @KeepDays = 90, @DryRun = 1;
GO

PRINT N'';
PRINT N'=== DB-13 + DB-14 完成 ===';
PRINT N'sp_UpdateAllStats：索引变更后立即执行；之后每天定时（SQL Agent 作业）。';
PRINT N'sp_CleanHistoryData：先 DryRun 看行数，确认后 @DryRun=0 实际删除。';
GO
