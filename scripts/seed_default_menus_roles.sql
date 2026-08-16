-- ============================================================================
-- D2: 初始化默认角色与角色权限分配
-- ============================================================================
-- 用途：新部署的 ERP 系统首次启动时，通过此脚本创建：
--   1. "系统管理员"角色（拥有所有菜单的所有权限）
--   2. "普通用户"角色（只读权限，作为新用户的默认角色）
--   3. 将 admin 用户分配到"系统管理员"角色
--
-- 触发：由 db/migrate.rs 自动执行（标记为 critical 迁移），也可手动运行
-- 幂等性：使用 IF NOT EXISTS 检查，重复执行不会创建重复记录
-- ============================================================================

USE TestERP;
GO

-- ============================================================================
-- 1. 创建默认角色（tSys_Rule）
-- ============================================================================

-- 1.1 "系统管理员"角色：拥有所有菜单的所有权限
IF NOT EXISTS (SELECT 1 FROM [dbo].[tSys_Rule] WHERE [RuleID] = '10000000-0000-1000-0000-000000000001')
BEGIN
    INSERT INTO [dbo].[tSys_Rule] ([RuleID], [RuleName], [Note], [Flg], [State])
    VALUES (
        '10000000-0000-1000-0000-000000000001',
        N'系统管理员',
        N'系统初始化角色，拥有所有菜单的所有权限（CRUD + 审核 + 打印 + 导出）。请勿删除。',
        N'admin',
        N'Y'
    );
    PRINT N'[seed_roles] 已创建"系统管理员"角色';
END
ELSE
BEGIN
    PRINT N'[seed_roles] "系统管理员"角色已存在，跳过';
END
GO

-- 1.2 "普通用户"角色：只读权限（仅 CanRead=1）
IF NOT EXISTS (SELECT 1 FROM [dbo].[tSys_Rule] WHERE [RuleID] = '10000000-0000-1000-0000-000000000002')
BEGIN
    INSERT INTO [dbo].[tSys_Rule] ([RuleID], [RuleName], [Note], [Flg], [State])
    VALUES (
        '10000000-0000-1000-0000-000000000002',
        N'普通用户',
        N'系统初始化角色，仅对所有菜单有只读权限（CanRead=1）。可在此基础上扩展。',
        N'user',
        N'Y'
    );
    PRINT N'[seed_roles] 已创建"普通用户"角色';
END
ELSE
BEGIN
    PRINT N'[seed_roles] "普通用户"角色已存在，跳过';
END
GO

-- ============================================================================
-- 2. 分配菜单权限到"系统管理员"角色（tSys_RuleMenu）
--    只对未分配的菜单执行 INSERT，避免重复
-- ============================================================================

-- 2.1 系统管理员：所有菜单全部 CRUD + 审核 + 打印 + 导出 权限
INSERT INTO [dbo].[tSys_RuleMenu]
    ([RuleMenuID], [RuleID], [MenuID], [CanRead], [CanCreate], [CanUpdate], [CanDelete], [CanAudit], [CanPrint], [CanExport], [LUTime])
SELECT
    NEWID(),
    '10000000-0000-1000-0000-000000000001',  -- 系统管理员角色
    m.[SYM_ID],
    1, 1, 1, 1, 1, 1, 1,                     -- 全部权限
    GETDATE()
FROM [dbo].[tSys_Menus] m
WHERE ISNULL(m.[Used], 'Y') = 'Y'
  AND m.[SYM_ID] IS NOT NULL
  AND NOT EXISTS (
      SELECT 1
      FROM [dbo].[tSys_RuleMenu] rm
      WHERE rm.[RuleID] = '10000000-0000-1000-0000-000000000001'
        AND rm.[MenuID] = m.[SYM_ID]
  );

PRINT N'[seed_roles] 已为"系统管理员"角色分配所有菜单权限';
GO

-- 2.2 普通用户：所有菜单的只读权限（CanRead=1，其他=0）
INSERT INTO [dbo].[tSys_RuleMenu]
    ([RuleMenuID], [RuleID], [MenuID], [CanRead], [CanCreate], [CanUpdate], [CanDelete], [CanAudit], [CanPrint], [CanExport], [LUTime])
SELECT
    NEWID(),
    '10000000-0000-1000-0000-000000000002',  -- 普通用户角色
    m.[SYM_ID],
    1, 0, 0, 0, 0, 0, 0,                     -- 仅 CanRead
    GETDATE()
FROM [dbo].[tSys_Menus] m
WHERE ISNULL(m.[Used], 'Y') = 'Y'
  AND m.[SYM_ID] IS NOT NULL
  AND NOT EXISTS (
      SELECT 1
      FROM [dbo].[tSys_RuleMenu] rm
      WHERE rm.[RuleID] = '10000000-0000-1000-0000-000000000002'
        AND rm.[MenuID] = m.[SYM_ID]
  );

PRINT N'[seed_roles] 已为"普通用户"角色分配只读菜单权限';
GO

-- ============================================================================
-- 3. 将 admin 用户分配到"系统管理员"角色（tSys_UserRule）
--    admin 用户在后端 permission_middleware 中通过 user_code='admin' 直接放行，
--    不依赖角色分配。但分配角色是为了：
--      - 在前端权限列表显示时一致
--      - 在系统角色管理界面可见
--      - 兼容未来权限模型变更
-- ============================================================================

IF NOT EXISTS (
    SELECT 1
    FROM [dbo].[tSys_UserRule] ur
    INNER JOIN [dbo].[tBas_Emp] e ON ur.[EmpID] = e.[EmpID]
    WHERE e.[EmpNo] = N'admin'
      AND ur.[RuleID] = '10000000-0000-1000-0000-000000000001'
)
BEGIN
    INSERT INTO [dbo].[tSys_UserRule] ([UserRuleID], [EmpID], [RuleID], [LUTime])
    SELECT
        NEWID(),
        e.[EmpID],
        '10000000-0000-1000-0000-000000000001',
        GETDATE()
    FROM [dbo].[tBas_Emp] e
    WHERE e.[EmpNo] = N'admin';

    IF @@ROWCOUNT > 0
        PRINT N'[seed_roles] 已将 admin 用户分配到"系统管理员"角色';
    ELSE
        PRINT N'[seed_roles] 警告：未找到 admin 用户（请先运行 seed_admin_user.sql）';
END
ELSE
BEGIN
    PRINT N'[seed_roles] admin 用户已分配到"系统管理员"角色，跳过';
END
GO
