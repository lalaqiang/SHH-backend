/* ============================================================================
   00_init_schema.sql — 新部署统一表创建入口
   ----------------------------------------------------------------------------
   用途：
     首次部署 ERP 系统时，按顺序执行所有数据库初始化脚本。
     本文件作为「入口」与「执行顺序清单」，本身只创建迁移版本表（tSys_Migration）
     和数据库本体（如不存在），后续脚本由 init_database.ps1 调用 sqlcmd 依次执行。

   执行顺序（必须严格遵守）：
     1. 本文件（00_init_schema.sql）
        - 创建数据库（如不存在）
        - 创建 tSys_Migration 表（migrate.rs 依赖）
     2. scripts/db/DB-01-触发器与约束.sql    —— 业务表触发器与约束
     3. scripts/db/DB-02-非聚集索引.sql       —— 性能索引
     4. scripts/db/DB-02-cleanup-duplicate-indexes.sql —— 清理重复索引
     5. scripts/db/DB-03-外键约束.sql          —— 外键关系
     6. scripts/db/DB-04-单据号生成.sql        —— 单据号序列表 tSys_DocNoSeq
     7. scripts/db/DB-05-库存分页查询.sql      —— 库存查询优化
     8. scripts/db/DB-06-单据分页查询.sql      —— 单据分页优化
     9. scripts/db/DB-07-月结.sql              —— 月结存储过程
    10. scripts/db/DB-07-rollback-月结回滚.sql —— 月结回滚
    11. scripts/db/DB-08-软删除与恢复.sql      —— 软删/恢复存储过程
    12. scripts/db/DB-09-数据字典.sql          —— 数据字典初始化
    13. scripts/db/DB-10-测试数据.sql          —— 测试数据（生产环境可跳过）
    14. scripts/db/DB-11-备份还原.sql          —— 备份/恢复存储过程
    15. scripts/db/DB-12-用户权限.sql          —— 应用账号与权限
    16. scripts/db/DB-13-14-运维.sql           —— 运维脚本
    17. scripts/db/DB-15-CHECK-约束.sql        —— CHECK 约束
    18. scripts/db/DB-16-按月归档表.sql        —— 归档表
    19. scripts/init_menu_permcode.sql         —— 菜单 PermCode 字段初始化
    20. scripts/seed_admin_user.sql            —— D1：创建 admin 用户
    21. scripts/seed_default_menus_roles.sql   —— D2：创建默认角色与权限

   说明：
     - 业务表（tBas_*, tPur_*, tSal_*, tStk_*, tFin_*）由 legacy ERP 数据库导出，
       或由 DBA 通过 DB-*.sql 系列脚本手工创建。
     - 本入口只负责「最小启动集」：tSys_Migration 表（应用层 migrate.rs 依赖）
     - 应用启动时 migrate.rs 会自动执行所有 001-011 迁移，无需手工干预。
   ============================================================================ */

USE master;
GO
SET NOCOUNT ON;
GO

/* ---------- 1. 创建数据库（如不存在） ---------- */
IF NOT EXISTS (SELECT 1 FROM sys.databases WHERE name = N'TestERP')
BEGIN
    CREATE DATABASE [TestERP];
    PRINT N'[00_init_schema] 数据库 TestERP 已创建';
END
ELSE
BEGIN
    PRINT N'[00_init_schema] 数据库 TestERP 已存在，跳过创建';
END
GO

/* ---------- 2. 切换到 TestERP 库，创建迁移版本表 ---------- */
USE [TestERP];
GO

IF NOT EXISTS (SELECT 1 FROM sys.tables WHERE name = N'tSys_Migration')
BEGIN
    CREATE TABLE [dbo].[tSys_Migration] (
        [Id]         INT IDENTITY(1,1) PRIMARY KEY,
        [Name]       NVARCHAR(200) NOT NULL UNIQUE,
        [AppliedAt]  DATETIME NOT NULL DEFAULT GETDATE()
    );
    PRINT N'[00_init_schema] 已创建 tSys_Migration 表（migrate.rs 依赖）';
END
ELSE
BEGIN
    PRINT N'[00_init_schema] tSys_Migration 表已存在，跳过';
END
GO

PRINT N'';
PRINT N'============================================================';
PRINT N'[00_init_schema] 入口脚本执行完毕';
PRINT N'接下来请依次执行以下脚本（或运行 init_database.ps1）：';
PRINT N'  1. scripts/db/DB-01 至 DB-16 系列';
PRINT N'  2. scripts/init_menu_permcode.sql';
PRINT N'  3. scripts/seed_admin_user.sql';
PRINT N'  4. scripts/seed_default_menus_roles.sql';
PRINT N'============================================================';
GO
