/* ============================================================================
   DB-15-CHECK-约束（SQL Server 2005 兼容）
   ----------------------------------------------------------------------------
   模块：DB-15
   目标：为核心表的 State / Used / Kind / GDSStateNO / WorkState / CostCalc
        字段添加值域 CHECK 约束，防止脏数据写入
   设计要点：
     1) 动态遍历 sys.columns 自动发现含 State/Used 列的用户表，无需硬编码表名
     2) 特定表（tStk_IO.Kind、tStk_Move.Kind、tBas_Goods.GDSStateNO 等）使用显式约束
     3) WITH NOCHECK：不校验已有历史数据，仅约束新写入（兼容遗留库，避免因脏数据导致脚本失败）
        后续可执行 ALTER TABLE ... WITH CHECK CHECK CONSTRAINT ... 重新启用全量校验
     4) 幂等：先 DROP 再 ADD，重复执行安全
     5) TRY/CATCH 包裹，单表失败不中断整体执行
   用法：sqlcmd -S server -d TestERP -U sa -P xxx -i DB-15-CHECK-约束.sql
   ============================================================================ */

USE [TestERP];
GO
SET NOCOUNT ON;
SET QUOTED_IDENTIFIER ON;
SET ANSI_NULLS ON;
GO

PRINT N'========== DB-15 CHECK 约束开始 ==========';

-- ============================================================================
-- 1. State 字段约束（动态遍历所有含 State 列的用户表）
--    允许值: D=删除, E=编辑中, S=已审核, Y=已确认, C=已作废, N=新建
--    （基础表使用 S/D/Y/N 子集，单据表使用 D/E/S/Y/C/N，取并集兼容两类表）
-- ============================================================================
DECLARE @sql nvarchar(4000);
DECLARE @tbl nvarchar(128);

DECLARE cur_state CURSOR LOCAL FAST_FORWARD FOR
    SELECT t.name
    FROM sys.columns c
    JOIN sys.tables t ON c.object_id = t.object_id
    JOIN sys.types ty ON c.user_type_id = ty.user_type_id
    WHERE c.name = N'State'
      AND t.type = N'U'
      AND ty.name IN (N'nvarchar', N'varchar', N'nchar', N'char');
OPEN cur_state;
FETCH NEXT FROM cur_state INTO @tbl;
WHILE @@FETCH_STATUS = 0
BEGIN
    SET @sql = N'IF OBJECT_ID(N''CK_' + @tbl + N'_State'', N''C'') IS NOT NULL '
             + N'ALTER TABLE [' + @tbl + N'] DROP CONSTRAINT [CK_' + @tbl + N'_State];';
    EXEC sp_executesql @sql;
    SET @sql = N'ALTER TABLE [' + @tbl + N'] WITH NOCHECK '
             + N'ADD CONSTRAINT [CK_' + @tbl + N'_State] '
             + N'CHECK ([State] IN (N''D'', N''E'', N''S'', N''Y'', N''C'', N''N''));';
    BEGIN TRY
        EXEC sp_executesql @sql;
        PRINT N'  [OK] ' + @tbl + N'.State -> CK_' + @tbl + N'_State';
    END TRY
    BEGIN CATCH
        PRINT N'  [FAIL] ' + @tbl + N'.State: ' + ERROR_MESSAGE();
    END CATCH
    FETCH NEXT FROM cur_state INTO @tbl;
END
CLOSE cur_state;
DEALLOCATE cur_state;

-- ============================================================================
-- 2. Used 字段约束（动态遍历所有含 Used 列的用户表）
--    允许值: Y=启用, N=停用
-- ============================================================================
DECLARE cur_used CURSOR LOCAL FAST_FORWARD FOR
    SELECT t.name
    FROM sys.columns c
    JOIN sys.tables t ON c.object_id = t.object_id
    JOIN sys.types ty ON c.user_type_id = ty.user_type_id
    WHERE c.name = N'Used'
      AND t.type = N'U'
      AND ty.name IN (N'nvarchar', N'varchar', N'nchar', N'char');
OPEN cur_used;
FETCH NEXT FROM cur_used INTO @tbl;
WHILE @@FETCH_STATUS = 0
BEGIN
    SET @sql = N'IF OBJECT_ID(N''CK_' + @tbl + N'_Used'', N''C'') IS NOT NULL '
             + N'ALTER TABLE [' + @tbl + N'] DROP CONSTRAINT [CK_' + @tbl + N'_Used];';
    EXEC sp_executesql @sql;
    SET @sql = N'ALTER TABLE [' + @tbl + N'] WITH NOCHECK '
             + N'ADD CONSTRAINT [CK_' + @tbl + N'_Used] '
             + N'CHECK ([Used] IN (N''Y'', N''N''));';
    BEGIN TRY
        EXEC sp_executesql @sql;
        PRINT N'  [OK] ' + @tbl + N'.Used -> CK_' + @tbl + N'_Used';
    END TRY
    BEGIN CATCH
        PRINT N'  [FAIL] ' + @tbl + N'.Used: ' + ERROR_MESSAGE();
    END CATCH
    FETCH NEXT FROM cur_used INTO @tbl;
END
CLOSE cur_used;
DEALLOCATE cur_used;

-- ============================================================================
-- 3. tStk_IO.Kind 库存入出库类型约束
-- ============================================================================
IF OBJECT_ID(N'tStk_IO', N'U') IS NOT NULL
BEGIN
    IF OBJECT_ID(N'CK_tStk_IO_Kind', N'C') IS NOT NULL
        ALTER TABLE [tStk_IO] DROP CONSTRAINT [CK_tStk_IO_Kind];
    BEGIN TRY
        ALTER TABLE [tStk_IO] WITH NOCHECK ADD CONSTRAINT [CK_tStk_IO_Kind]
        CHECK ([Kind] IN (N'RI', N'PD', N'OTI', N'DBI', N'TH', N'SD', N'SR', N'SI',
                          N'POS', N'OTO', N'DBO', N'O', N'REQ', N'PR', N'DB', N'ZP',
                          N'OT', N'ADJ'));
        PRINT N'  [OK] tStk_IO.Kind -> CK_tStk_IO_Kind';
    END TRY
    BEGIN CATCH
        PRINT N'  [FAIL] tStk_IO.Kind: ' + ERROR_MESSAGE();
    END CATCH
END
ELSE
    PRINT N'  [SKIP] tStk_IO 不存在';

-- ============================================================================
-- 4. tStk_Move.Kind 调拨类型约束
-- ============================================================================
IF OBJECT_ID(N'tStk_Move', N'U') IS NOT NULL
BEGIN
    IF OBJECT_ID(N'CK_tStk_Move_Kind', N'C') IS NOT NULL
        ALTER TABLE [tStk_Move] DROP CONSTRAINT [CK_tStk_Move_Kind];
    BEGIN TRY
        ALTER TABLE [tStk_Move] WITH NOCHECK ADD CONSTRAINT [CK_tStk_Move_Kind]
        CHECK ([Kind] IN (N'DB', N'TH', N'ZP'));
        PRINT N'  [OK] tStk_Move.Kind -> CK_tStk_Move_Kind';
    END TRY
    BEGIN CATCH
        PRINT N'  [FAIL] tStk_Move.Kind: ' + ERROR_MESSAGE();
    END CATCH
END
ELSE
    PRINT N'  [SKIP] tStk_Move 不存在';

-- ============================================================================
-- 5. tBas_Goods.GDSStateNO 商品品态约束 (0=停用,1=进销,2=新品,3=只销,4=止销)
-- ============================================================================
IF OBJECT_ID(N'tBas_Goods', N'U') IS NOT NULL
   AND COL_LENGTH(N'tBas_Goods', N'GDSStateNO') IS NOT NULL
BEGIN
    IF OBJECT_ID(N'CK_tBas_Goods_GDSStateNO', N'C') IS NOT NULL
        ALTER TABLE [tBas_Goods] DROP CONSTRAINT [CK_tBas_Goods_GDSStateNO];
    BEGIN TRY
        ALTER TABLE [tBas_Goods] WITH NOCHECK ADD CONSTRAINT [CK_tBas_Goods_GDSStateNO]
        CHECK ([GDSStateNO] IN (0, 1, 2, 3, 4));
        PRINT N'  [OK] tBas_Goods.GDSStateNO -> CK_tBas_Goods_GDSStateNO';
    END TRY
    BEGIN CATCH
        PRINT N'  [FAIL] tBas_Goods.GDSStateNO: ' + ERROR_MESSAGE();
    END CATCH
END
ELSE
    PRINT N'  [SKIP] tBas_Goods 或 GDSStateNO 列不存在';

-- ============================================================================
-- 6. tBas_Emp.WorkState 员工在职状态约束 (1=在职,2=试用,3=离职,4=退休,5=休假,6=其他)
-- ============================================================================
IF OBJECT_ID(N'tBas_Emp', N'U') IS NOT NULL
   AND COL_LENGTH(N'tBas_Emp', N'WorkState') IS NOT NULL
BEGIN
    IF OBJECT_ID(N'CK_tBas_Emp_WorkState', N'C') IS NOT NULL
        ALTER TABLE [tBas_Emp] DROP CONSTRAINT [CK_tBas_Emp_WorkState];
    BEGIN TRY
        ALTER TABLE [tBas_Emp] WITH NOCHECK ADD CONSTRAINT [CK_tBas_Emp_WorkState]
        CHECK ([WorkState] IN (1, 2, 3, 4, 5, 6));
        PRINT N'  [OK] tBas_Emp.WorkState -> CK_tBas_Emp_WorkState';
    END TRY
    BEGIN CATCH
        PRINT N'  [FAIL] tBas_Emp.WorkState: ' + ERROR_MESSAGE();
    END CATCH
END
ELSE
    PRINT N'  [SKIP] tBas_Emp 或 WorkState 列不存在';

-- ============================================================================
-- 7. tBas_Stock.CostCalc 成本核算约束 (Y/N)
-- ============================================================================
IF OBJECT_ID(N'tBas_Stock', N'U') IS NOT NULL
   AND COL_LENGTH(N'tBas_Stock', N'CostCalc') IS NOT NULL
BEGIN
    IF OBJECT_ID(N'CK_tBas_Stock_CostCalc', N'C') IS NOT NULL
        ALTER TABLE [tBas_Stock] DROP CONSTRAINT [CK_tBas_Stock_CostCalc];
    BEGIN TRY
        ALTER TABLE [tBas_Stock] WITH NOCHECK ADD CONSTRAINT [CK_tBas_Stock_CostCalc]
        CHECK ([CostCalc] IN (N'Y', N'N'));
        PRINT N'  [OK] tBas_Stock.CostCalc -> CK_tBas_Stock_CostCalc';
    END TRY
    BEGIN CATCH
        PRINT N'  [FAIL] tBas_Stock.CostCalc: ' + ERROR_MESSAGE();
    END CATCH
END
ELSE
    PRINT N'  [SKIP] tBas_Stock 或 CostCalc 列不存在';

PRINT N'========== DB-15 CHECK 约束完成 ==========';
GO