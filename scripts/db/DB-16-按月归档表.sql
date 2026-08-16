/* ============================================================================
 * DB-16-按月归档表.sql
 *
 * 目的：按月归档历史数据到 *_Arch 表，避免主表无限膨胀。
 *       替代分区表方案（2005 Express/Standard 不支持分区）。
 *
 * 归档策略：
 *   A 类（日志）：tSys_OperHis、tSys_PrintLog → 按日期归档
 *   B 类（月结）：tStk_StockYM → 按 AccYM 归档
 *   B 类（预占）：tStk_Reserve → State='X' 且 EDate 早于 N 天
 *   C 类（单据）：tPur_Order、tSal_Order、tStk_IO 等主从表
 *                → State in ('S','Y','C') 且 EDate 早于 N 天
 *
 * 安全机制：
 *   - @DryRun=1 仅统计不执行（默认）
 *   - 事务包装 + TRY/CATCH
 *   - 分批 DELETE TOP (@BatchSize) 避免长事务锁表
 *   - 归档表已存在记录不重复插入（按主键去重）
 *   - 单据归档必须 State in ('S','Y','C')，禁止归档编辑中单据
 *
 * 兼容性：SQL Server 2005+（禁用 MERGE / IIF / THROW / CONCAT / OFFSET-FETCH）
 *
 * 用法：
 *   -- 1. 试运行（默认 DryRun=1）
 *   EXEC sp_ArchiveAll
 *   -- 2. 正式归档日志（保留 180 天）
 *   EXEC sp_ArchiveAll @KeepDays=180, @DryRun=0
 *   -- 3. 仅归档日志，不动单据
 *   EXEC sp_ArchiveOperHis @KeepDays=180, @DryRun=0
 *   -- 4. 仅归档月结（保留近 24 个月）
 *   EXEC sp_ArchiveStockYM @KeepYM='202401', @DryRun=0
 *
 * 调度建议：
 *   - 每月 1 号凌晨 02:00 执行 sp_ArchiveAll @KeepDays=180, @DryRun=0
 *   - 配合 DB-14 sp_CleanHistoryData 做最终清理（归档后可缩短保留天数）
 *
 * 作者：ERP 团队 | 日期：2026-07-01
 * ========================================================================== */

SET NOCOUNT ON
GO

/* ----------------------------------------------------------------------------
 * 1. 辅助函数：确保归档表存在（结构与主表一致）
 *    2005 不支持 SELECT INTO 在 IF 内嵌套，用动态 SQL + OBJECT_ID 判断
 * -------------------------------------------------------------------------- */
IF OBJECT_ID('dbo.fn_EnsureArchTable', 'FN') IS NOT NULL
    DROP FUNCTION dbo.fn_EnsureArchTable
GO
CREATE FUNCTION dbo.fn_EnsureArchTable(
    @SourceTable NVARCHAR(128),
    @ArchTable NVARCHAR(128)
)
RETURNS BIT
AS
BEGIN
    DECLARE @Sql NVARCHAR(4000)
    DECLARE @Result BIT

    IF OBJECT_ID(QUOTENAME(@SourceTable)) IS NULL
        RETURN 0

    IF OBJECT_ID(QUOTENAME(@ArchTable)) IS NOT NULL
        RETURN 1

    -- 用 SELECT INTO 复制结构（不含数据，WHERE 1=0）
    SET @Sql = N'SELECT * INTO ' + QUOTENAME(@ArchTable) +
               N' FROM ' + QUOTENAME(@SourceTable) +
               N' WHERE 1=0'

    EXEC sp_executesql @Sql

    IF OBJECT_ID(QUOTENAME(@ArchTable)) IS NOT NULL
        SET @Result = 1
    ELSE
        SET @Result = 0

    RETURN @Result
END
GO

/* ----------------------------------------------------------------------------
 * 2. sp_ArchiveOperHis - 归档操作日志（按 OperDate）
 * -------------------------------------------------------------------------- */
IF OBJECT_ID('dbo.sp_ArchiveOperHis', 'P') IS NOT NULL
    DROP PROCEDURE dbo.sp_ArchiveOperHis
GO
CREATE PROCEDURE dbo.sp_ArchiveOperHis(
    @KeepDays INT = 180,
    @BatchSize INT = 5000,
    @DryRun INT = 1
)
AS
BEGIN
    SET NOCOUNT ON
    DECLARE @Cutoff DATETIME
    DECLARE @ArchTable NVARCHAR(128)
    DECLARE @Sql NVARCHAR(4000)
    DECLARE @Archived BIGINT
    DECLARE @Deleted BIGINT
    DECLARE @BatchDeleted INT

    SET @Cutoff = DATEADD(day, -@KeepDays, GETDATE())
    SET @ArchTable = N'tSys_OperHis_Arch'
    SET @Archived = 0
    SET @Deleted = 0

    PRINT '==== sp_ArchiveOperHis 开始 ===='
    PRINT '保留天数: ' + CONVERT(NVARCHAR(10), @KeepDays)
    PRINT '截止日期: ' + CONVERT(NVARCHAR(19), @Cutoff, 120)
    PRINT 'DryRun: ' + CONVERT(NVARCHAR(1), @DryRun)

    -- 检查源表
    IF OBJECT_ID('tSys_OperHis') IS NULL
    BEGIN
        PRINT '⚠ tSys_OperHis 表不存在，跳过'
        RETURN 0
    END

    -- 统计待归档行数
    SET @Sql = N'SELECT @cnt = COUNT(*) FROM tSys_OperHis WHERE OperDate < @cutoff'
    EXEC sp_executesql @Sql, N'@cutoff DATETIME, @cnt BIGINT OUTPUT',
                       @cutoff = @Cutoff, @cnt = @Archived OUTPUT
    PRINT '待归档行数: ' + CONVERT(NVARCHAR(20), @Archived)

    IF @DryRun = 1
    BEGIN
        PRINT ' DryRun 模式，未执行实际归档'
        RETURN @Archived
    END

    -- 确保归档表存在
    IF dbo.fn_EnsureArchTable('tSys_OperHis', @ArchTable) = 0
    BEGIN
        RAISERROR('创建归档表 %s 失败', 16, 1, @ArchTable)
        RETURN -1
    END

    -- 事务包裹：先 INSERT 到归档表，再分批 DELETE
    BEGIN TRY
        BEGIN TRAN

        -- 插入归档表（排除已在归档表中的主键，幂等）
        SET @Sql = N'INSERT INTO ' + QUOTENAME(@ArchTable) +
                   N' SELECT s.* FROM tSys_OperHis s' +
                   N' LEFT JOIN ' + QUOTENAME(@ArchTable) + N' a ON s.OperHisID = a.OperHisID' +
                   N' WHERE s.OperDate < @cutoff AND a.OperHisID IS NULL'
        EXEC sp_executesql @Sql, N'@cutoff DATETIME', @cutoff = @Cutoff

        -- 分批 DELETE
        SET @BatchDeleted = 1
        WHILE @BatchDeleted > 0
        BEGIN
            DELETE TOP (@BatchSize) FROM tSys_OperHis
            WHERE OperDate < @Cutoff
            AND OperHisID IN (SELECT OperHisID FROM tSys_OperHis WHERE OperDate < @Cutoff)

            SET @BatchDeleted = @@ROWCOUNT
            SET @Deleted = @Deleted + @BatchDeleted
        END

        COMMIT TRAN
        PRINT '已归档: ' + CONVERT(NVARCHAR(20), @Archived) + ' 行'
        PRINT '已删除: ' + CONVERT(NVARCHAR(20), @Deleted) + ' 行'
    END TRY
    BEGIN CATCH
        IF @@TRANCOUNT > 0 ROLLBACK TRAN
        PRINT '✗ 归档失败: ' + ERROR_MESSAGE()
        RETURN -1
    END CATCH

    RETURN @Archived
END
GO

/* ----------------------------------------------------------------------------
 * 3. sp_ArchivePrintLog - 归档打印日志（按 PrintDate）
 * -------------------------------------------------------------------------- */
IF OBJECT_ID('dbo.sp_ArchivePrintLog', 'P') IS NOT NULL
    DROP PROCEDURE dbo.sp_ArchivePrintLog
GO
CREATE PROCEDURE dbo.sp_ArchivePrintLog(
    @KeepDays INT = 180,
    @BatchSize INT = 5000,
    @DryRun INT = 1
)
AS
BEGIN
    SET NOCOUNT ON
    DECLARE @Cutoff DATETIME
    DECLARE @ArchTable NVARCHAR(128)
    DECLARE @Sql NVARCHAR(4000)
    DECLARE @Archived BIGINT
    DECLARE @Deleted BIGINT
    DECLARE @BatchDeleted INT

    SET @Cutoff = DATEADD(day, -@KeepDays, GETDATE())
    SET @ArchTable = N'tSys_PrintLog_Arch'
    SET @Archived = 0
    SET @Deleted = 0

    PRINT '==== sp_ArchivePrintLog 开始 ===='
    PRINT '保留天数: ' + CONVERT(NVARCHAR(10), @KeepDays)
    PRINT '截止日期: ' + CONVERT(NVARCHAR(19), @Cutoff, 120)

    IF OBJECT_ID('tSys_PrintLog') IS NULL
    BEGIN
        PRINT '⚠ tSys_PrintLog 表不存在，跳过'
        RETURN 0
    END

    SET @Sql = N'SELECT @cnt = COUNT(*) FROM tSys_PrintLog WHERE PrintDate < @cutoff'
    EXEC sp_executesql @Sql, N'@cutoff DATETIME, @cnt BIGINT OUTPUT',
                       @cutoff = @Cutoff, @cnt = @Archived OUTPUT
    PRINT '待归档行数: ' + CONVERT(NVARCHAR(20), @Archived)

    IF @DryRun = 1
    BEGIN
        PRINT ' DryRun 模式，未执行实际归档'
        RETURN @Archived
    END

    IF dbo.fn_EnsureArchTable('tSys_PrintLog', @ArchTable) = 0
    BEGIN
        RAISERROR('创建归档表 %s 失败', 16, 1, @ArchTable)
        RETURN -1
    END

    BEGIN TRY
        BEGIN TRAN

        SET @Sql = N'INSERT INTO ' + QUOTENAME(@ArchTable) +
                   N' SELECT s.* FROM tSys_PrintLog s' +
                   N' LEFT JOIN ' + QUOTENAME(@ArchTable) + N' a ON s.LogID = a.LogID' +
                   N' WHERE s.PrintDate < @cutoff AND a.LogID IS NULL'
        EXEC sp_executesql @Sql, N'@cutoff DATETIME', @cutoff = @Cutoff

        SET @BatchDeleted = 1
        WHILE @BatchDeleted > 0
        BEGIN
            DELETE TOP (@BatchSize) FROM tSys_PrintLog
            WHERE PrintDate < @Cutoff

            SET @BatchDeleted = @@ROWCOUNT
            SET @Deleted = @Deleted + @BatchDeleted
        END

        COMMIT TRAN
        PRINT '已归档: ' + CONVERT(NVARCHAR(20), @Archived) + ' 行'
        PRINT '已删除: ' + CONVERT(NVARCHAR(20), @Deleted) + ' 行'
    END TRY
    BEGIN CATCH
        IF @@TRANCOUNT > 0 ROLLBACK TRAN
        PRINT '✗ 归档失败: ' + ERROR_MESSAGE()
        RETURN -1
    END CATCH

    RETURN @Archived
END
GO

/* ----------------------------------------------------------------------------
 * 4. sp_ArchiveStockYM - 归档月结表（按 AccYM）
 *    @KeepYM: 早于此月份的记录归档，如 '202401' 表示 2024-01 之前的全部归档
 * -------------------------------------------------------------------------- */
IF OBJECT_ID('dbo.sp_ArchiveStockYM', 'P') IS NOT NULL
    DROP PROCEDURE dbo.sp_ArchiveStockYM
GO
CREATE PROCEDURE dbo.sp_ArchiveStockYM(
    @KeepYM CHAR(6) = NULL,
    @DryRun INT = 1
)
AS
BEGIN
    SET NOCOUNT ON
    DECLARE @ArchTable NVARCHAR(128)
    DECLARE @Sql NVARCHAR(4000)
    DECLARE @Archived BIGINT

    SET @ArchTable = N'tStk_StockYM_Arch'
    SET @Archived = 0

    -- 默认保留近 24 个月
    IF @KeepYM IS NULL OR LEN(@KeepYM) <> 6
    BEGIN
        SET @KeepYM = CONVERT(CHAR(6), DATEADD(month, -24, GETDATE()), 112)
    END

    PRINT '==== sp_ArchiveStockYM 开始 ===='
    PRINT '保留起始月份: ' + @KeepYM
    PRINT 'DryRun: ' + CONVERT(NVARCHAR(1), @DryRun)

    IF OBJECT_ID('tStk_StockYM') IS NULL
    BEGIN
        PRINT '⚠ tStk_StockYM 表不存在，跳过'
        RETURN 0
    END

    SET @Sql = N'SELECT @cnt = COUNT(*) FROM tStk_StockYM WHERE AccYM < @keepym'
    EXEC sp_executesql @Sql, N'@keepym CHAR(6), @cnt BIGINT OUTPUT',
                       @keepym = @KeepYM, @cnt = @Archived OUTPUT
    PRINT '待归档行数: ' + CONVERT(NVARCHAR(20), @Archived)

    IF @DryRun = 1
    BEGIN
        PRINT ' DryRun 模式，未执行实际归档'
        RETURN @Archived
    END

    IF dbo.fn_EnsureArchTable('tStk_StockYM', @ArchTable) = 0
    BEGIN
        RAISERROR('创建归档表 %s 失败', 16, 1, @ArchTable)
        RETURN -1
    END

    BEGIN TRY
        BEGIN TRAN

        -- 幂等：排除已归档的主键（AccYM+StkID+GDSID 三联主键）
        SET @Sql = N'INSERT INTO ' + QUOTENAME(@ArchTable) +
                   N' SELECT s.* FROM tStk_StockYM s' +
                   N' LEFT JOIN ' + QUOTENAME(@ArchTable) +
                   N' a ON s.AccYM = a.AccYM AND s.StkID = a.StkID AND s.GDSID = a.GDSID' +
                   N' WHERE s.AccYM < @keepym AND a.AccYM IS NULL'
        EXEC sp_executesql @Sql, N'@keepym CHAR(6)', @keepym = @KeepYM

        DELETE FROM tStk_StockYM WHERE AccYM < @KeepYM

        COMMIT TRAN
        PRINT '已归档: ' + CONVERT(NVARCHAR(20), @Archived) + ' 行'
    END TRY
    BEGIN CATCH
        IF @@TRANCOUNT > 0 ROLLBACK TRAN
        PRINT '✗ 归档失败: ' + ERROR_MESSAGE()
        RETURN -1
    END CATCH

    RETURN @Archived
END
GO

/* ----------------------------------------------------------------------------
 * 5. sp_ArchiveReserve - 归档已作废预占（State='X' 且 EDate 早于 N 天）
 * -------------------------------------------------------------------------- */
IF OBJECT_ID('dbo.sp_ArchiveReserve', 'P') IS NOT NULL
    DROP PROCEDURE dbo.sp_ArchiveReserve
GO
CREATE PROCEDURE dbo.sp_ArchiveReserve(
    @KeepDays INT = 90,
    @BatchSize INT = 5000,
    @DryRun INT = 1
)
AS
BEGIN
    SET NOCOUNT ON
    DECLARE @Cutoff DATETIME
    DECLARE @ArchTable NVARCHAR(128)
    DECLARE @Sql NVARCHAR(4000)
    DECLARE @Archived BIGINT

    SET @Cutoff = DATEADD(day, -@KeepDays, GETDATE())
    SET @ArchTable = N'tStk_Reserve_Arch'
    SET @Archived = 0

    PRINT '==== sp_ArchiveReserve 开始 ===='
    PRINT '保留天数: ' + CONVERT(NVARCHAR(10), @KeepDays)
    PRINT '截止日期: ' + CONVERT(NVARCHAR(19), @Cutoff, 120)

    IF OBJECT_ID('tStk_Reserve') IS NULL
    BEGIN
        PRINT '⚠ tStk_Reserve 表不存在，跳过'
        RETURN 0
    END

    -- State='X' 表示已作废；保留近 N 天以便排查问题
    SET @Sql = N'SELECT @cnt = COUNT(*) FROM tStk_Reserve WHERE State = ''X'' AND EDate < @cutoff'
    EXEC sp_executesql @Sql, N'@cutoff DATETIME, @cnt BIGINT OUTPUT',
                       @cutoff = @Cutoff, @cnt = @Archived OUTPUT
    PRINT '待归档行数: ' + CONVERT(NVARCHAR(20), @Archived)

    IF @DryRun = 1
    BEGIN
        PRINT ' DryRun 模式，未执行实际归档'
        RETURN @Archived
    END

    IF dbo.fn_EnsureArchTable('tStk_Reserve', @ArchTable) = 0
    BEGIN
        RAISERROR('创建归档表 %s 失败', 16, 1, @ArchTable)
        RETURN -1
    END

    BEGIN TRY
        BEGIN TRAN

        SET @Sql = N'INSERT INTO ' + QUOTENAME(@ArchTable) +
                   N' SELECT s.* FROM tStk_Reserve s' +
                   N' LEFT JOIN ' + QUOTENAME(@ArchTable) + N' a ON s.ReserveID = a.ReserveID' +
                   N' WHERE s.State = ''X'' AND s.EDate < @cutoff AND a.ReserveID IS NULL'
        EXEC sp_executesql @Sql, N'@cutoff DATETIME', @cutoff = @Cutoff

        DECLARE @BatchDeleted INT
        SET @BatchDeleted = 1
        WHILE @BatchDeleted > 0
        BEGIN
            DELETE TOP (@BatchSize) FROM tStk_Reserve
            WHERE State = 'X' AND EDate < @Cutoff

            SET @BatchDeleted = @@ROWCOUNT
        END

        COMMIT TRAN
        PRINT '已归档: ' + CONVERT(NVARCHAR(20), @Archived) + ' 行'
    END TRY
    BEGIN CATCH
        IF @@TRANCOUNT > 0 ROLLBACK TRAN
        PRINT '✗ 归档失败: ' + ERROR_MESSAGE()
        RETURN -1
    END CATCH

    RETURN @Archived
END
GO

/* ----------------------------------------------------------------------------
 * 6. sp_ArchiveDoc - 通用单据归档（主表+明细表）
 *    归档条件：State in ('S','Y','C') AND EDate < @Cutoff
 *    安全：禁止归档编辑中（'E','N'）单据
 * -------------------------------------------------------------------------- */
IF OBJECT_ID('dbo.sp_ArchiveDoc', 'P') IS NOT NULL
    DROP PROCEDURE dbo.sp_ArchiveDoc
GO
CREATE PROCEDURE dbo.sp_ArchiveDoc(
    @MainTable NVARCHAR(128),
    @PKField NVARCHAR(128),
    @DetailTable NVARCHAR(128),
    @DetailFKField NVARCHAR(128),
    @KeepDays INT = 365,
    @BatchSize INT = 2000,
    @DryRun INT = 1
)
AS
BEGIN
    SET NOCOUNT ON
    DECLARE @Cutoff DATETIME
    DECLARE @ArchMain NVARCHAR(128)
    DECLARE @ArchDetail NVARCHAR(128)
    DECLARE @Sql NVARCHAR(4000)
    DECLARE @ArchivedMain BIGINT
    DECLARE @ArchivedDetail BIGINT

    SET @Cutoff = DATEADD(day, -@KeepDays, GETDATE())
    SET @ArchMain = @MainTable + N'_Arch'
    SET @ArchDetail = @DetailTable + N'_Arch'
    SET @ArchivedMain = 0
    SET @ArchivedDetail = 0

    PRINT '==== sp_ArchiveDoc 归档单据: ' + @MainTable + ' ===='
    PRINT '保留天数: ' + CONVERT(NVARCHAR(10), @KeepDays)
    PRINT '截止日期: ' + CONVERT(NVARCHAR(19), @Cutoff, 120)

    IF OBJECT_ID(QUOTENAME(@MainTable)) IS NULL
    BEGIN
        PRINT '⚠ 主表 ' + @MainTable + ' 不存在，跳过'
        RETURN 0
    END

    -- 统计待归档主表行数（State in S/Y/C 表示已审核/已确认/已作废）
    SET @Sql = N'SELECT @cnt = COUNT(*) FROM ' + QUOTENAME(@MainTable) +
               N' WHERE State IN (''S'',''Y'',''C'') AND EDate < @cutoff'
    EXEC sp_executesql @Sql, N'@cutoff DATETIME, @cnt BIGINT OUTPUT',
                       @cutoff = @Cutoff, @cnt = @ArchivedMain OUTPUT
    PRINT '待归档主表行数: ' + CONVERT(NVARCHAR(20), @ArchivedMain)

    IF @DryRun = 1
    BEGIN
        -- 试运行时也统计明细行数
        IF OBJECT_ID(QUOTENAME(@DetailTable)) IS NOT NULL
        BEGIN
            SET @Sql = N'SELECT @cnt = COUNT(*) FROM ' + QUOTENAME(@DetailTable) +
                       N' d WHERE EXISTS (SELECT 1 FROM ' + QUOTENAME(@MainTable) +
                       N' m WHERE m.' + QUOTENAME(@PKField) + N' = d.' + QUOTENAME(@DetailFKField) +
                       N' AND m.State IN (''S'',''Y'',''C'') AND m.EDate < @cutoff)'
            EXEC sp_executesql @Sql, N'@cutoff DATETIME, @cnt BIGINT OUTPUT',
                               @cutoff = @Cutoff, @cnt = @ArchivedDetail OUTPUT
            PRINT '待归档明细行数: ' + CONVERT(NVARCHAR(20), @ArchivedDetail)
        END
        PRINT ' DryRun 模式，未执行实际归档'
        RETURN @ArchivedMain
    END

    -- 确保归档表存在
    IF dbo.fn_EnsureArchTable(@MainTable, @ArchMain) = 0
    BEGIN
        RAISERROR('创建主表归档表 %s 失败', 16, 1, @ArchMain)
        RETURN -1
    END

    IF OBJECT_ID(QUOTENAME(@DetailTable)) IS NOT NULL
    BEGIN
        IF dbo.fn_EnsureArchTable(@DetailTable, @ArchDetail) = 0
        BEGIN
            RAISERROR('创建明细归档表 %s 失败', 16, 1, @ArchDetail)
            RETURN -1
        END
    END

    BEGIN TRY
        BEGIN TRAN

        -- 1. 归档明细（先于主表，避免主表删除后找不到关联）
        IF OBJECT_ID(QUOTENAME(@DetailTable)) IS NOT NULL
        BEGIN
            -- 插入明细归档表（按主键去重，幂等）
            -- 注意：明细表无 State 字段，通过主表 State 过滤
            SET @Sql = N'INSERT INTO ' + QUOTENAME(@ArchDetail) +
                       N' SELECT d.* FROM ' + QUOTENAME(@DetailTable) + N' d' +
                       N' INNER JOIN ' + QUOTENAME(@MainTable) + N' m ON m.' + QUOTENAME(@PKField) + N' = d.' + QUOTENAME(@DetailFKField) +
                       N' LEFT JOIN ' + QUOTENAME(@ArchDetail) + N' a ON d.' + QUOTENAME(@PKField) + N' = a.' + QUOTENAME(@PKField) +
                       N' WHERE m.State IN (''S'',''Y'',''C'') AND m.EDate < @cutoff AND a.' + QUOTENAME(@PKField) + N' IS NULL'
            EXEC sp_executesql @Sql, N'@cutoff DATETIME', @cutoff = @Cutoff

            -- 分批删除明细
            DECLARE @BatchDeleted INT
            SET @BatchDeleted = 1
            WHILE @BatchDeleted > 0
            BEGIN
                -- 2005 支持 DELETE TOP
                SET @Sql = N'DELETE TOP (' + CONVERT(NVARCHAR(10), @BatchSize) + N') FROM ' + QUOTENAME(@DetailTable) +
                           N' WHERE ' + QUOTENAME(@DetailFKField) + N' IN (' +
                           N' SELECT ' + QUOTENAME(@PKField) + N' FROM ' + QUOTENAME(@MainTable) +
                           N' WHERE State IN (''S'',''Y'',''C'') AND EDate < @cutoff)'
                EXEC sp_executesql @Sql, N'@cutoff DATETIME', @cutoff = @Cutoff
                SET @BatchDeleted = @@ROWCOUNT
            END
        END

        -- 2. 归档主表
        SET @Sql = N'INSERT INTO ' + QUOTENAME(@ArchMain) +
                   N' SELECT s.* FROM ' + QUOTENAME(@MainTable) + N' s' +
                   N' LEFT JOIN ' + QUOTENAME(@ArchMain) + N' a ON s.' + QUOTENAME(@PKField) + N' = a.' + QUOTENAME(@PKField) +
                   N' WHERE s.State IN (''S'',''Y'',''C'') AND s.EDate < @cutoff AND a.' + QUOTENAME(@PKField) + N' IS NULL'
        EXEC sp_executesql @Sql, N'@cutoff DATETIME', @cutoff = @Cutoff

        -- 分批删除主表
        DECLARE @BatchMainDeleted INT
        SET @BatchMainDeleted = 1
        WHILE @BatchMainDeleted > 0
        BEGIN
            SET @Sql = N'DELETE TOP (' + CONVERT(NVARCHAR(10), @BatchSize) + N') FROM ' + QUOTENAME(@MainTable) +
                       N' WHERE State IN (''S'',''Y'',''C'') AND EDate < @cutoff'
            EXEC sp_executesql @Sql, N'@cutoff DATETIME', @cutoff = @Cutoff
            SET @BatchMainDeleted = @@ROWCOUNT
        END

        COMMIT TRAN
        PRINT '已归档主表: ' + CONVERT(NVARCHAR(20), @ArchivedMain) + ' 行'
        PRINT '已归档明细: ' + CONVERT(NVARCHAR(20), @ArchivedDetail) + ' 行'
    END TRY
    BEGIN CATCH
        IF @@TRANCOUNT > 0 ROLLBACK TRAN
        PRINT '✗ 归档失败: ' + ERROR_MESSAGE()
        RETURN -1
    END CATCH

    RETURN @ArchivedMain
END
GO

/* ----------------------------------------------------------------------------
 * 7. sp_ArchiveAll - 主调度存储过程（统一入口）
 *    调用上述子过程，按优先级依次归档
 * -------------------------------------------------------------------------- */
IF OBJECT_ID('dbo.sp_ArchiveAll', 'P') IS NOT NULL
    DROP PROCEDURE dbo.sp_ArchiveAll
GO
CREATE PROCEDURE dbo.sp_ArchiveAll(
    @KeepDays INT = 180,        -- 日志/预占保留天数
    @DocKeepDays INT = 365,     -- 单据保留天数（更长）
    @KeepYM CHAR(6) = NULL,     -- 月结保留起始月，NULL=近24个月
    @BatchSize INT = 5000,
    @DryRun INT = 1
)
AS
BEGIN
    SET NOCOUNT ON
    DECLARE @StartTime DATETIME
    DECLARE @StepStart DATETIME
    DECLARE @Ret INT

    SET @StartTime = GETDATE()
    PRINT '============================================================'
    PRINT '  sp_ArchiveAll 全量归档开始 - ' + CONVERT(NVARCHAR(19), @StartTime, 120)
    PRINT '  日志保留: ' + CONVERT(NVARCHAR(10), @KeepDays) + ' 天'
    PRINT '  单据保留: ' + CONVERT(NVARCHAR(10), @DocKeepDays) + ' 天'
    PRINT '  DryRun: ' + CONVERT(NVARCHAR(1), @DryRun)
    PRINT '============================================================'

    -- 1. 操作日志
    SET @StepStart = GETDATE()
    EXEC @Ret = sp_ArchiveOperHis @KeepDays = @KeepDays, @BatchSize = @BatchSize, @DryRun = @DryRun
    PRINT '步骤 1 sp_ArchiveOperHis 返回: ' + CONVERT(NVARCHAR(10), @Ret) +
          ' 耗时: ' + CONVERT(NVARCHAR(10), DATEDIFF(second, @StepStart, GETDATE())) + ' 秒'

    -- 2. 打印日志
    SET @StepStart = GETDATE()
    EXEC @Ret = sp_ArchivePrintLog @KeepDays = @KeepDays, @BatchSize = @BatchSize, @DryRun = @DryRun
    PRINT '步骤 2 sp_ArchivePrintLog 返回: ' + CONVERT(NVARCHAR(10), @Ret) +
          ' 耗时: ' + CONVERT(NVARCHAR(10), DATEDIFF(second, @StepStart, GETDATE())) + ' 秒'

    -- 3. 月结归档
    SET @StepStart = GETDATE()
    EXEC @Ret = sp_ArchiveStockYM @KeepYM = @KeepYM, @DryRun = @DryRun
    PRINT '步骤 3 sp_ArchiveStockYM 返回: ' + CONVERT(NVARCHAR(10), @Ret) +
          ' 耗时: ' + CONVERT(NVARCHAR(10), DATEDIFF(second, @StepStart, GETDATE())) + ' 秒'

    -- 4. 已作废预占归档
    SET @StepStart = GETDATE()
    EXEC @Ret = sp_ArchiveReserve @KeepDays = @KeepDays, @BatchSize = @BatchSize, @DryRun = @DryRun
    PRINT '步骤 4 sp_ArchiveReserve 返回: ' + CONVERT(NVARCHAR(10), @Ret) +
          ' 耗时: ' + CONVERT(NVARCHAR(10), DATEDIFF(second, @StepStart, GETDATE())) + ' 秒'

    -- 5-12. 历史单据归档（主表+明细）
    -- 注意：单据归档需业务确认；建议先在测试库验证
    PRINT '--- 历史单据归档（保留 ' + CONVERT(NVARCHAR(10), @DocKeepDays) + ' 天）---'

    SET @StepStart = GETDATE()
    EXEC @Ret = sp_ArchiveDoc @MainTable = 'tPur_Order', @PKField = 'POID',
                              @DetailTable = 'tPur_OrderDetail', @DetailFKField = 'POID',
                              @KeepDays = @DocKeepDays, @BatchSize = @BatchSize, @DryRun = @DryRun
    PRINT '步骤 5 tPur_Order 返回: ' + CONVERT(NVARCHAR(10), @Ret) +
          ' 耗时: ' + CONVERT(NVARCHAR(10), DATEDIFF(second, @StepStart, GETDATE())) + ' 秒'

    SET @StepStart = GETDATE()
    EXEC @Ret = sp_ArchiveDoc @MainTable = 'tSal_Order', @PKField = 'SOID',
                              @DetailTable = 'tSal_OrderDetail', @DetailFKField = 'SOID',
                              @KeepDays = @DocKeepDays, @BatchSize = @BatchSize, @DryRun = @DryRun
    PRINT '步骤 6 tSal_Order 返回: ' + CONVERT(NVARCHAR(10), @Ret) +
          ' 耗时: ' + CONVERT(NVARCHAR(10), DATEDIFF(second, @StepStart, GETDATE())) + ' 秒'

    SET @StepStart = GETDATE()
    EXEC @Ret = sp_ArchiveDoc @MainTable = 'tSal_Inv', @PKField = 'SIID',
                              @DetailTable = 'tSal_InvDetail', @DetailFKField = 'SIID',
                              @KeepDays = @DocKeepDays, @BatchSize = @BatchSize, @DryRun = @DryRun
    PRINT '步骤 7 tSal_Inv 返回: ' + CONVERT(NVARCHAR(10), @Ret) +
          ' 耗时: ' + CONVERT(NVARCHAR(10), DATEDIFF(second, @StepStart, GETDATE())) + ' 秒'

    SET @StepStart = GETDATE()
    EXEC @Ret = sp_ArchiveDoc @MainTable = 'tStk_IO', @PKField = 'IOID',
                              @DetailTable = 'tStk_IODetail', @DetailFKField = 'IOID',
                              @KeepDays = @DocKeepDays, @BatchSize = @BatchSize, @DryRun = @DryRun
    PRINT '步骤 8 tStk_IO 返回: ' + CONVERT(NVARCHAR(10), @Ret) +
          ' 耗时: ' + CONVERT(NVARCHAR(10), DATEDIFF(second, @StepStart, GETDATE())) + ' 秒'

    SET @StepStart = GETDATE()
    EXEC @Ret = sp_ArchiveDoc @MainTable = 'tStk_Move', @PKField = 'MoveID',
                              @DetailTable = 'tStk_MoveDetail', @DetailFKField = 'MoveID',
                              @KeepDays = @DocKeepDays, @BatchSize = @BatchSize, @DryRun = @DryRun
    PRINT '步骤 9 tStk_Move 返回: ' + CONVERT(NVARCHAR(10), @Ret) +
          ' 耗时: ' + CONVERT(NVARCHAR(10), DATEDIFF(second, @StepStart, GETDATE())) + ' 秒'

    SET @StepStart = GETDATE()
    EXEC @Ret = sp_ArchiveDoc @MainTable = 'tStk_Tran', @PKField = 'TranID',
                              @DetailTable = 'tStk_TranDetail', @DetailFKField = 'TranID',
                              @KeepDays = @DocKeepDays, @BatchSize = @BatchSize, @DryRun = @DryRun
    PRINT '步骤 10 tStk_Tran 返回: ' + CONVERT(NVARCHAR(10), @Ret) +
          ' 耗时: ' + CONVERT(NVARCHAR(10), DATEDIFF(second, @StepStart, GETDATE())) + ' 秒'

    SET @StepStart = GETDATE()
    EXEC @Ret = sp_ArchiveDoc @MainTable = 'tStk_ReplenishApply', @PKField = 'ReplenishApplyID',
                              @DetailTable = 'tStk_ReplenishApplyDtl', @DetailFKField = 'ReplenishApplyID',
                              @KeepDays = @DocKeepDays, @BatchSize = @BatchSize, @DryRun = @DryRun
    PRINT '步骤 11 tStk_ReplenishApply 返回: ' + CONVERT(NVARCHAR(10), @Ret) +
          ' 耗时: ' + CONVERT(NVARCHAR(10), DATEDIFF(second, @StepStart, GETDATE())) + ' 秒'

    PRINT '============================================================'
    PRINT '  sp_ArchiveAll 全量归档完成 - ' + CONVERT(NVARCHAR(19), GETDATE(), 120)
    PRINT '  总耗时: ' + CONVERT(NVARCHAR(10), DATEDIFF(second, @StartTime, GETDATE())) + ' 秒'
    PRINT '============================================================'
END
GO

/* ----------------------------------------------------------------------------
 * 8. 归档表恢复查询示例（手动执行，不封装为存储过程）
 *    需要恢复某条已归档记录时，可参考以下 SQL：
 *
 *    -- 恢复单条操作日志
 *    INSERT INTO tSys_OperHis
 *        SELECT * FROM tSys_OperHis_Arch WHERE OperHisID = 'xxx-uuid'
 *    DELETE FROM tSys_OperHis_Arch WHERE OperHisID = 'xxx-uuid'
 *
 *    -- 恢复某月全部月结
 *    INSERT INTO tStk_StockYM
 *        SELECT * FROM tStk_StockYM_Arch WHERE AccYM = '202312'
 *    DELETE FROM tStk_StockYM_Arch WHERE AccYM = '202312'
 *
 *    -- 恢复单据（主表+明细）
 *    BEGIN TRAN
 *    INSERT INTO tPur_Order SELECT * FROM tPur_Order_Arch WHERE POID = 'xxx'
 *    INSERT INTO tPur_OrderDetail SELECT * FROM tPur_OrderDetail_Arch
 *    WHERE POID = 'xxx'
 *    DELETE FROM tPur_Order_Arch WHERE POID = 'xxx'
 *    DELETE FROM tPur_OrderDetail_Arch WHERE POID = 'xxx'
 *    COMMIT
 * -------------------------------------------------------------------------- */

PRINT 'DB-16 按月归档脚本安装完成'
PRINT '存储过程清单：'
PRINT '  - fn_EnsureArchTable(@SourceTable, @ArchTable)'
PRINT '  - sp_ArchiveOperHis(@KeepDays, @BatchSize, @DryRun)'
PRINT '  - sp_ArchivePrintLog(@KeepDays, @BatchSize, @DryRun)'
PRINT '  - sp_ArchiveStockYM(@KeepYM, @DryRun)'
PRINT '  - sp_ArchiveReserve(@KeepDays, @BatchSize, @DryRun)'
PRINT '  - sp_ArchiveDoc(@MainTable, @PKField, @DetailTable, @DetailFKField, @KeepDays, @BatchSize, @DryRun)'
PRINT '  - sp_ArchiveAll(@KeepDays, @DocKeepDays, @KeepYM, @BatchSize, @DryRun)'
PRINT ''
PRINT '首次使用建议：'
PRINT '  1. EXEC sp_ArchiveAll @DryRun=1  -- 试运行查看待归档量'
PRINT '  2. EXEC sp_ArchiveOperHis @KeepDays=180, @DryRun=0  -- 先归档日志'
PRINT '  3. EXEC sp_ArchiveAll @KeepDays=180, @DocKeepDays=365, @DryRun=0  -- 全量归档'
PRINT '  4. 配合 DB-14 sp_CleanHistoryData 做最终清理'
GO
