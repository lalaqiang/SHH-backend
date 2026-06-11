-- 查询帐号000064的密码信息
USE TestERP;
GO

SELECT 
    EmpNo AS '工号',
    EmpName AS '姓名',
    PassWordStr AS '密码字段',
    LEN(PassWordStr) AS '密码长度',
    CASE 
        WHEN PassWordStr LIKE 'SHA256:%' THEN 'SHA256加密'
        WHEN LEN(PassWordStr) = 16 AND PassWordStr NOT LIKE '%[^0-9A-Fa-f]%' THEN 'XOR加密(16位十六进制)'
        WHEN LEN(PassWordStr) = 0 OR PassWordStr IS NULL THEN '空密码'
        ELSE '可能是明文或其他加密'
    END AS '加密类型'
FROM tBas_Emp
WHERE EmpNo = '000064';
GO

-- 查看所有用户的密码加密类型分布
SELECT 
    CASE 
        WHEN PassWordStr LIKE 'SHA256:%' THEN 'SHA256加密'
        WHEN LEN(PassWordStr) = 16 AND PassWordStr NOT LIKE '%[^0-9A-Fa-f]%' THEN 'XOR加密'
        WHEN LEN(PassWordStr) = 0 OR PassWordStr IS NULL THEN '空密码'
        ELSE '明文或其他'
    END AS '加密类型',
    COUNT(*) AS '用户数量'
FROM tBas_Emp
GROUP BY 
    CASE 
        WHEN PassWordStr LIKE 'SHA256:%' THEN 'SHA256加密'
        WHEN LEN(PassWordStr) = 16 AND PassWordStr NOT LIKE '%[^0-9A-Fa-f]%' THEN 'XOR加密'
        WHEN LEN(PassWordStr) = 0 OR PassWordStr IS NULL THEN '空密码'
        ELSE '明文或其他'
    END;
GO
