-- ============================================================================
-- D1: 初始化 admin 用户
-- ============================================================================
-- 用途：新部署的 ERP 系统首次启动时，通过此脚本创建可登录的系统管理员账号
-- 触发：由 db/migrate.rs 自动执行（标记为 critical 迁移），也可手动运行
--
-- 默认账号：
--   工号 (EmpNo)   : admin
--   密码 (password): 123456
--
-- 安全说明：
--   1. 密码使用 SHA256+静态盐格式存储（兼容 verify_password 旧格式）
--   2. 首次登录后会被自动升级为 bcrypt 格式（更安全）
--   3. 上线后请立即通过"修改密码"功能更换为强密码
--   4. EmpID 使用固定 UUID（便于幂等：重复执行不会创建多个 admin）
-- ============================================================================

IF NOT EXISTS (SELECT 1 FROM [dbo].[tBas_Emp] WHERE [EmpNo] = N'admin')
BEGIN
    INSERT INTO [dbo].[tBas_Emp]
    (
        [EmpID], [EmpNo], [EmpName], [Sex], [DeptID], [DutyID], [StkID],
        [Tel], [IDCode], [Birthday], [InDate], [OutDate],
        [WorkState], [AllowLogin], [State], [Note],
        [EUser], [EDate],
        [PYCode], [HomeTel], [Email], [Addr],
        [PassWordStr]
    )
    VALUES
    (
        '00000000-0000-1000-0000-000000000001',  -- 固定 UUID，便于幂等
        N'admin',                                  -- 工号
        N'系统管理员',                              -- 姓名
        1,                                         -- 性别（BIT：1=男，0=女）
        NULL,                                      -- DeptID（部门，留空，admin 不属于任何部门）
        NULL,                                      -- DutyID（职务，留空）
        NULL,                                      -- StkID（仓库，留空）
        NULL,                                      -- Tel（电话，可后续补）
        NULL,                                      -- IDCode（身份证号）
        NULL,                                      -- Birthday
        GETDATE(),                                 -- InDate（入职日期=今天）
        NULL,                                      -- OutDate（离职日期）
        N'1',                                      -- WorkState：1=在职
        N'Y',                                      -- AllowLogin：允许登录
        N'S',                                      -- State：S=已审核（基础资料状态）
        N'系统初始管理员账号（seed_admin_user.sql 自动创建）',
        '00000000-0000-1000-0000-000000000001',    -- EUser：创建人（自引用）
        GETDATE(),                                 -- EDate：创建时间
        N'admin',                                  -- PYCode（拼音码）
        NULL,                                      -- HomeTel
        N'admin@erp.local',                        -- Email
        NULL,                                      -- Addr（住址）
        N'SHA256:eeaeaefac2f357bc46a8337007e3d0472e36795c2c6b68114c4585f43d97cb60'  -- PassWordStr：密码 123456
    );
    PRINT N'[seed_admin_user] 已创建 admin 账号（工号=admin，密码=123456）';
END
ELSE
BEGIN
    PRINT N'[seed_admin_user] admin 账号已存在，跳过';
END
GO
