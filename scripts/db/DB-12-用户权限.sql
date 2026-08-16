/* ============================================================================
   DB-12 用户权限分配（SQL Server 2005 兼容）
   ----------------------------------------------------------------------------
   模块：DB-12
   目标：创建应用账号 + 只读账号，按最小权限原则分配 schema 权限
   账号：
     - erp_app：应用层专用，对 dbo schema 有 SELECT/INSERT/UPDATE/DELETE + EXEC
     - erp_readonly：报表/排查专用，仅 SELECT + EXEC
     - erp_dba：DBA 维护专用，db_owner
   注意：CREATE LOGIN 需 sysadmin 权限。脚本幂等（先判断存在）。
   ============================================================================ */

USE [master];
GO
SET NOCOUNT ON;
GO

/* ---------- 1. 创建登录账号 ---------- */
IF NOT EXISTS (SELECT 1 FROM sys.sql_logins WHERE name = N'erp_app')
BEGIN
    CREATE LOGIN [erp_app] WITH PASSWORD = N'Erp@App2026!Pwd', CHECK_POLICY = OFF;
    PRINT N'[OK] 已创建登录 erp_app';
END
ELSE PRINT N'[SKIP] 登录 erp_app 已存在';

IF NOT EXISTS (SELECT 1 FROM sys.sql_logins WHERE name = N'erp_readonly')
BEGIN
    CREATE LOGIN [erp_readonly] WITH PASSWORD = N'Erp@Read2026!Pwd', CHECK_POLICY = OFF;
    PRINT N'[OK] 已创建登录 erp_readonly';
END
ELSE PRINT N'[SKIP] 登录 erp_readonly 已存在';

IF NOT EXISTS (SELECT 1 FROM sys.sql_logins WHERE name = N'erp_dba')
BEGIN
    CREATE LOGIN [erp_dba] WITH PASSWORD = N'Erp@Dba2026!Pwd', CHECK_POLICY = OFF;
    PRINT N'[OK] 已创建登录 erp_dba';
END
ELSE PRINT N'[SKIP] 登录 erp_dba 已存在';
GO

/* ---------- 2. 在 TestERP 库中创建用户并授权 ---------- */
USE [TestERP];
GO

-- erp_app 用户：应用层全功能（CRUD + EXEC）
IF NOT EXISTS (SELECT 1 FROM sys.database_principals WHERE name = N'erp_app' AND type = N'S')
    CREATE USER [erp_app] FOR LOGIN [erp_app];
GRANT SELECT, INSERT, UPDATE, DELETE ON SCHEMA :: [dbo] TO [erp_app];
GRANT EXECUTE ON SCHEMA :: [dbo] TO [erp_app];
GRANT VIEW DEFINITION ON SCHEMA :: [dbo] TO [erp_app];
-- 不给 ALTER/DROP（表结构变更需 DBA）
PRINT N'[OK] erp_app 已授权（SELECT/INSERT/UPDATE/DELETE/EXECUTE on dbo）';

-- erp_readonly 用户：只读 + 执行存储过程
IF NOT EXISTS (SELECT 1 FROM sys.database_principals WHERE name = N'erp_readonly' AND type = N'S')
    CREATE USER [erp_readonly] FOR LOGIN [erp_readonly];
GRANT SELECT ON SCHEMA :: [dbo] TO [erp_readonly];
GRANT EXECUTE ON SCHEMA :: [dbo] TO [erp_readonly];
GRANT VIEW DEFINITION ON SCHEMA :: [dbo] TO [erp_readonly];
PRINT N'[OK] erp_readonly 已授权（SELECT/EXECUTE on dbo）';

-- erp_dba 用户：DBA，db_owner 角色
IF NOT EXISTS (SELECT 1 FROM sys.database_principals WHERE name = N'erp_dba' AND type = N'S')
    CREATE USER [erp_dba] FOR LOGIN [erp_dba];
ALTER ROLE [db_owner] ADD MEMBER [erp_dba];
PRINT N'[OK] erp_dba 已加入 db_owner 角色';
GO

/* ---------- 3. 验证 ---------- */
PRINT N'';
PRINT N'--- TestERP 用户与权限 ---';
SELECT  dp.name AS [用户],
        dp.type_desc AS [类型],
        ISNULL(rp.name, N'(无)') AS [角色]
FROM    sys.database_principals dp
LEFT JOIN sys.database_role_members drm ON drm.member_principal_id = dp.principal_id
LEFT JOIN sys.database_principals rp ON rp.principal_id = drm.role_principal_id
WHERE   dp.name IN (N'erp_app', N'erp_readonly', N'erp_dba')
ORDER BY dp.name;
GO

PRINT N'';
PRINT N'=== DB-12 完成 ===';
PRINT N'应用层用 erp_app 连接（CRUD+EXEC），报表用 erp_readonly（只读），DBA 用 erp_dba。';
PRINT N'注意：连接字符串需更新为对应账号。';
GO
