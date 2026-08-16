/* ============================================================================
   DB-09 完整数据字典生成脚本（SQL Server 2005 兼容，修正版）
   ----------------------------------------------------------------------------
   说明：PRINT 不允许子查询，全部先 SELECT INTO 变量再 PRINT。
   ============================================================================ */

USE [TestERP];
GO
SET NOCOUNT ON;
GO

PRINT N'# 数据库完整字典（TestERP）';
DECLARE @now nvarchar(19); SET @now = CONVERT(nvarchar(19), GETDATE(), 120);
DECLARE @cnt int; SET @cnt = (SELECT COUNT(*) FROM sys.tables);
PRINT N'> 自动生成：' + @now + N'，从 INFORMATION_SCHEMA 实测';
PRINT N'> 数据库：TestERP，共 ' + CONVERT(nvarchar(10), @cnt) + N' 张用户表';
PRINT N'';
PRINT N'---';
PRINT N'';

DECLARE @tbl nvarchar(128);
DECLARE @line nvarchar(4000);
DECLARE @col nvarchar(128), @dtype nvarchar(160), @nullable nvarchar(3), @default nvarchar(400), @pk_flag nvarchar(3);

DECLARE cur_tbl CURSOR LOCAL FAST_FORWARD FOR
    SELECT name FROM sys.tables ORDER BY name;
OPEN cur_tbl;
FETCH NEXT FROM cur_tbl INTO @tbl;
WHILE @@FETCH_STATUS = 0
BEGIN
    SET @line = N'## ' + @tbl; PRINT @line;
    PRINT N'';
    PRINT N'| 列名 | 类型 | 可空 | 默认值 | PK |';
    PRINT N'|------|------|------|--------|----|';

    DECLARE cur_col CURSOR LOCAL FAST_FORWARD FOR
        SELECT  c.COLUMN_NAME,
                c.DATA_TYPE +
                    CASE
                        WHEN c.DATA_TYPE IN (N'char', N'varchar', N'nchar', N'nvarchar') AND c.CHARACTER_MAXIMUM_LENGTH > 0
                            THEN N'(' + CASE WHEN c.CHARACTER_MAXIMUM_LENGTH = -1 THEN N'MAX' ELSE CONVERT(nvarchar(10), c.CHARACTER_MAXIMUM_LENGTH) END + N')'
                        WHEN c.DATA_TYPE IN (N'decimal', N'numeric')
                            THEN N'(' + CONVERT(nvarchar(10), c.NUMERIC_PRECISION) + N',' + CONVERT(nvarchar(10), c.NUMERIC_SCALE) + N')'
                        ELSE N''
                    END AS dtype,
                CASE WHEN c.IS_NULLABLE = N'YES' THEN N'Y' ELSE N'N' END AS nullable,
                ISNULL((SELECT TOP 1 REPLACE(REPLACE(dc.definition, CHAR(13), N' '), CHAR(10), N' ')
                        FROM sys.default_constraints dc
                        WHERE dc.parent_object_id = OBJECT_ID(@tbl)
                          AND dc.parent_column_id = c.ORDINAL_POSITION), N'') AS dflt,
                CASE WHEN EXISTS (
                    SELECT 1 FROM sys.indexes i
                    JOIN sys.index_columns ic ON i.object_id = ic.object_id AND i.index_id = ic.index_id
                    WHERE i.object_id = OBJECT_ID(@tbl) AND i.is_primary_key = 1
                      AND ic.column_id = c.ORDINAL_POSITION
                ) THEN N'Y' ELSE N'' END AS pkf
        FROM    INFORMATION_SCHEMA.COLUMNS c
        WHERE   c.TABLE_NAME = @tbl
        ORDER BY c.ORDINAL_POSITION;

    OPEN cur_col;
    FETCH NEXT FROM cur_col INTO @col, @dtype, @nullable, @default, @pk_flag;
    WHILE @@FETCH_STATUS = 0
    BEGIN
        SET @line = N'| ' + @col + N' | ' + @dtype + N' | ' + @nullable + N' | ' + @default + N' | ' + @pk_flag + N' |';
        PRINT @line;
        FETCH NEXT FROM cur_col INTO @col, @dtype, @nullable, @default, @pk_flag;
    END
    CLOSE cur_col;
    DEALLOCATE cur_col;

    PRINT N'';
    FETCH NEXT FROM cur_tbl INTO @tbl;
END
CLOSE cur_tbl;
DEALLOCATE cur_tbl;

PRINT N'---';
PRINT N'> Y in PK = 主键列。数据类型为实测（UDDT 已展开为 base type + 长度）。';
GO
