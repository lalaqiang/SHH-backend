-- 创建权限相关表（包含 CanExport 字段）
USE TestERP;
GO

-- 6. tSys_RuleMenu — 角色菜单权限关联
IF NOT EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tSys_RuleMenu' AND xtype = 'U')
BEGIN
    CREATE TABLE tSys_RuleMenu (
        RuleMenuID uniqueidentifier PRIMARY KEY DEFAULT NEWID(),
        RuleID     uniqueidentifier NULL,
        MenuID     uniqueidentifier NULL,
        CanRead    int              DEFAULT 1,
        CanCreate  int              DEFAULT 0,
        CanUpdate  int              DEFAULT 0,
        CanDelete  int              DEFAULT 0,
        CanAudit   int              DEFAULT 0,
        CanPrint   int              DEFAULT 0,
        CanExport  int              DEFAULT 0,
        LUTime     datetime         DEFAULT GETDATE()
    );
    PRINT 'tSys_RuleMenu 表已创建';
END
ELSE
BEGIN
    -- 表已存在，确保 CanExport 字段存在
    IF NOT EXISTS (SELECT 1 FROM sys.columns WHERE object_id = OBJECT_ID('tSys_RuleMenu') AND name = 'CanExport')
    BEGIN
        ALTER TABLE tSys_RuleMenu ADD CanExport int DEFAULT 0;
        PRINT 'tSys_RuleMenu.CanExport 字段已添加';
    END
END
GO

-- 7. tSys_UserRule — 用户角色关联
IF NOT EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tSys_UserRule' AND xtype = 'U')
BEGIN
    CREATE TABLE tSys_UserRule (
        UserRuleID uniqueidentifier PRIMARY KEY DEFAULT NEWID(),
        EmpID      uniqueidentifier NULL,
        RuleID     uniqueidentifier NULL,
        LUTime     datetime         DEFAULT GETDATE()
    );
    PRINT 'tSys_UserRule 表已创建';
END
GO

PRINT '权限表创建完成';
GO
