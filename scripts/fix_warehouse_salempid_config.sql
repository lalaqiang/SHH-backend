-- 修复 admin 用户 tBas_Stock 列配置中 SalEmpID 被错误隐藏的问题
-- 根因：用户在列设置中把 SalEmpID 的 formIn 设为 false，导致编辑表单中看不到该字段，
--       修改保存时 SalEmpID 不变，操作日志显示"暂无变更明细数据"
-- 修复：将 ConfigData JSON 中 SalEmpID 的 formIn 改回 true
--
-- 执行方法（任选其一）：
--   1. 在 SQL Server Management Studio 中执行
--   2. 通过 sqlcmd 命令行执行
--   3. 通过后端 /generic/query 接口执行（不推荐，应使用专用接口）
--
-- 注意：此脚本可重复执行（幂等），已 formIn=true 的不会被改变

DECLARE @AdminEmpID VARCHAR(36) = '1f9fe0a8-df94-490d-8890-41e5167748a4'
DECLARE @TableName NVARCHAR(100) = N'tBas_Stock'

-- 1. 查看修复前的配置（诊断用）
SELECT 'BEFORE' AS Phase, EmpID, TableName,
       JSON_VALUE(ConfigData, '$.columns') AS ColumnsJson
FROM tSys_TableColumnConfig
WHERE EmpID = @AdminEmpID AND TableName = @TableName

-- 2. 修复：将 SalEmpID 的 formIn 改为 true
--    使用 JSON_MODIFY 修改嵌套数组中的对象（SQL Server 2017+）
UPDATE tSys_TableColumnConfig
SET ConfigData = (
    SELECT
        -- 重新构造 columns 数组，把 SalEmpID 的 formIn 强制设为 true
        JSON_MODIFY(
            ConfigData,
            '$.columns',
            (
                SELECT
                    -- 对 SalEmpID 字段强制 formIn=true，其他字段保持原值
                    CASE
                        WHEN JSON_VALUE(c.value, '$.prop') = 'SalEmpID'
                        THEN JSON_MODIFY(c.value, '$.formIn', CAST(1 AS BIT))
                        ELSE c.value
                    END AS value
                FROM OPENJSON(JSON_QUERY(ConfigData, '$.columns')) AS c
                FOR JSON PATH
            )
        )
    FROM tSys_TableColumnConfig
    WHERE EmpID = @AdminEmpID AND TableName = @TableName
)
WHERE EmpID = @AdminEmpID AND TableName = @TableName
  AND ConfigData LIKE '%SalEmpID%'
  AND JSON_VALUE(ConfigData, '$.columns') IS NOT NULL

-- 3. 查看修复后的配置（验证）
SELECT 'AFTER' AS Phase, EmpID, TableName,
       JSON_VALUE(ConfigData, '$.columns') AS ColumnsJson
FROM tSys_TableColumnConfig
WHERE EmpID = @AdminEmpID AND TableName = @TableName
