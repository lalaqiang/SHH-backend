/* ============================================================================
   DB-11 备份还原 SQL 脚本（SQL Server 2005 兼容）
   ----------------------------------------------------------------------------
   模块：DB-11
   包含：完整备份 / 差异备份 / 事务日志备份 / 还原 / 分离附加
   注意：脚本含占位路径，需按实际环境修改后再执行。
   ============================================================================ */

USE [master];
GO
SET NOCOUNT ON;
GO

/* ====================================================================
   1. 完整备份（FULL BACKUP）
   建议每天凌晨执行一次。备份文件按日期命名。
   ==================================================================== */
-- 实际执行前修改 @bak_path 为实际备份目录
DECLARE @bak_path nvarchar(260);
DECLARE @sql nvarchar(4000);
SET @bak_path = N'D:\Backup\TestERP\';   -- ★ 修改为实际目录

-- 自动建目录（xp_cmdshell 需启用；如未启用可手动建目录后注释此行）
-- EXEC xp_cmdshell N'mkdir "' + @bak_path + '"', no_output;

-- 完整备份
SET @sql = N'BACKUP DATABASE [TestERP] TO DISK = N''' + @bak_path +
           N'TestERP_FULL_' + REPLACE(REPLACE(CONVERT(nvarchar(19), GETDATE(), 120), N':', N''), N' ', N'_') +
           N'.bak'' WITH INIT, COMPRESSION, NAME = N''TestERP-Full Database Backup''';
-- EXEC sp_executesql @sql;   -- ★ 取消注释后执行
PRINT N'[示例] 完整备份命令：';
PRINT @sql;
GO

/* ====================================================================
   2. 差异备份（DIFFERENTIAL）
   建议每 6 小时执行一次（在完整备份之后）。
   ==================================================================== */
PRINT N'';
PRINT N'[示例] 差异备份命令（修改路径后执行）：';
PRINT N'BACKUP DATABASE [TestERP] TO DISK = N''D:\Backup\TestERP\TestERP_DIFF_日期.bak'' WITH DIFFERENTIAL, INIT;';
GO

/* ====================================================================
   3. 事务日志备份（LOG）
   建议每小时执行一次（恢复模式须为 FULL/BULK_LOGGED）。
   ==================================================================== */
PRINT N'';
PRINT N'[示例] 日志备份命令（需 DB 恢复模式为 FULL）：';
PRINT N'BACKUP LOG [TestERP] TO DISK = N''D:\Backup\TestERP\TestERP_LOG_日期.trn'' WITH INIT;';
GO

/* ====================================================================
   4. 还原（RESTORE）
   演示从完整备份还原（覆盖现有库）。
   ==================================================================== */
PRINT N'';
PRINT N'[示例] 还原命令（★ 谨慎执行，会覆盖现有库）：';
PRINT N'RESTORE DATABASE [TestERP] FROM DISK = N''D:\Backup\TestERP\TestERP_FULL_xxx.bak'' WITH REPLACE, RECOVERY;';
GO

/* ====================================================================
   5. 分离 / 附加（用于迁移或快速备份整个 mdf/ldf）
   ==================================================================== */
PRINT N'';
PRINT N'[示例] 分离（断开连接，便于拷贝 mdf/ldf 文件）：';
PRINT N'EXEC sp_detach_db @dbname = N''TestERP'';';
PRINT N'';
PRINT N'[示例] 附加（拷贝到新机器后挂回）：';
PRINT N'CREATE DATABASE [TestERP] ON (FILENAME = N''D:\Data\TestERP.mdf''), (FILENAME = N''D:\Data\TestERP_log.ldf'') FOR ATTACH;';
GO

/* ====================================================================
   6. 查看现有备份历史
   ==================================================================== */
PRINT N'';
PRINT N'--- msdb 中记录的备份历史（最近 10 条）---';
SELECT TOP 10
    database_name,
    CASE type WHEN N'D' THEN N'完整' WHEN N'L' THEN N'日志' WHEN N'I' THEN N'差异' ELSE type END AS backup_type,
    backup_start_date,
    backup_finish_date,
    CAST(backup_size / 1024 / 1024 AS int) AS size_mb,
    physical_device_name
FROM msdb.dbo.backupset b
JOIN msdb.dbo.backupmediafamily m ON b.media_set_id = m.media_set_id
WHERE database_name = N'TestERP'
ORDER BY backup_start_date DESC;
GO

PRINT N'';
PRINT N'=== DB-11 完成 ===';
PRINT N'所有命令为示例，需修改路径后取消注释执行。';
PRINT N'生产建议：配置 SQL Agent 作业定时跑完整+差异+日志备份。';
GO
