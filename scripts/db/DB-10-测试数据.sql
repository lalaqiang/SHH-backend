/* ============================================================================
   DB-10 初始化测试数据（SQL Server 2005 兼容）
   ----------------------------------------------------------------------------
   模块：DB-10
   目标：为基础资料表插入测试数据（商品/客户/供应商/仓库/员工/品牌/单位/分类）
        供开发调试用。所有 ID 用 TST_ 前缀 + 固定值，方便清理。
   注意：仅在空库或测试库执行；生产库不要执行。
   幂等：用 IF NOT EXISTS 守卫，重复执行不重复插入。
   ============================================================================ */

USE [TestERP];
GO
SET NOCOUNT ON;
GO

PRINT N'========================================';
PRINT N'DB-10 初始化测试数据开始';
PRINT N'时间：' + CONVERT(nvarchar(19), GETDATE(), 120);
PRINT N'========================================';
GO

/* ---------- 0. 固定测试 ID（用 NEWID() 一次性生成，存到变量）---------- */
-- 为保证幂等，用固定可识别 UUID（TST 前缀的 fake UUID）
DECLARE @wh1 nvarchar(40); SET @wh1 = N'11111111-1111-1111-1111-111111111111';  -- 测试仓库1
DECLARE @wh2 nvarchar(40); SET @wh2 = N'22222222-2222-2222-2222-222222222222';  -- 测试仓库2
DECLARE @g1 nvarchar(40);   SET @g1   = N'AAAAAAAA-AAAA-AAAA-AAAA-AAAAAAAAAAAA';  -- 测试商品1
DECLARE @g2 nvarchar(40);   SET @g2   = N'BBBBBBBB-BBBB-BBBB-BBBB-BBBBBBBBBBBB';  -- 测试商品2
DECLARE @c1 nvarchar(40);   SET @c1   = N'CCCCCCCC-CCCC-CCCC-CCCC-CCCCCCCCCCCC';  -- 测试客户1
DECLARE @s1 nvarchar(40);   SET @s1   = N'DDDDDDDD-DDDD-DDDD-DDDD-DDDDDDDDDDDD';  -- 测试供应商1
DECLARE @e1 nvarchar(40);   SET @e1   = N'EEEEEEEE-EEEE-EEEE-EEEE-EEEEEEEEEEEE';  -- 测试员工1
DECLARE @b1 nvarchar(40);   SET @b1   = N'FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF';  -- 测试品牌1

/* ---------- 1. 单位（UnitNO 是 varchar(5)，用短编码）---------- */
IF NOT EXISTS (SELECT 1 FROM tBas_Unit WHERE UnitNO = N'T01')
    INSERT INTO tBas_Unit (UnitID, UnitNO, UnitName, Used)
    VALUES (NEWID(), N'T01', N'测试单位-支', N'Y');
IF NOT EXISTS (SELECT 1 FROM tBas_Unit WHERE UnitNO = N'T02')
    INSERT INTO tBas_Unit (UnitID, UnitNO, UnitName, Used)
    VALUES (NEWID(), N'T02', N'测试单位-箱', N'Y');
PRINT N'[OK] 单位 ×2';
GO

/* ---------- 2. 品牌（无 BrandNO，用 BrandName 标识，先存 BrandID）---------- */
DECLARE @brand_id nvarchar(40);
IF NOT EXISTS (SELECT 1 FROM tBas_Brand WHERE BrandName = N'TST_测试品牌')
BEGIN
    SET @brand_id = N'FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF';
    INSERT INTO tBas_Brand (BrandID, BrandName, Used)
    VALUES (@brand_id, N'TST_测试品牌', N'Y');
END
ELSE
    SELECT @brand_id = BrandID FROM tBas_Brand WHERE BrandName = N'TST_测试品牌';
PRINT N'[OK] 品牌 ×1';
GO

/* ---------- 3. 商品分类 ---------- */
IF NOT EXISTS (SELECT 1 FROM tBas_GDSType WHERE GDSTypeCode = N'TST_T1')
    INSERT INTO tBas_GDSType (GDSTypeID, GDSTypeCode, GDSTypeName, Used)
    VALUES (NEWID(), N'TST_T1', N'测试分类-日化', N'Y');
PRINT N'[OK] 分类 ×1';
GO

/* ---------- 4. 仓库 ---------- */
IF NOT EXISTS (SELECT 1 FROM tBas_Stock WHERE StkCode = N'TST_W1')
    INSERT INTO tBas_Stock (StkID, StkCode, StkName, Used, State)
    VALUES (N'11111111-1111-1111-1111-111111111111', N'TST_W1', N'测试仓库-主仓', N'Y', N'Y');
IF NOT EXISTS (SELECT 1 FROM tBas_Stock WHERE StkCode = N'TST_W2')
    INSERT INTO tBas_Stock (StkID, StkCode, StkName, Used, State)
    VALUES (N'22222222-2222-2222-2222-222222222222', N'TST_W2', N'测试仓库-副仓', N'Y', N'Y');
PRINT N'[OK] 仓库 ×2';
GO

/* ---------- 5. 商品（关联品牌用子查询）---------- */
IF NOT EXISTS (SELECT 1 FROM tBas_Goods WHERE GDSNO = N'TST_G1')
    INSERT INTO tBas_Goods (GDSID, GDSNO, GDSDesc, GDSSpec, UnitNO, BrandID, State)
    VALUES (N'AAAAAAAA-AAAA-AAAA-AAAA-AAAAAAAAAAAA', N'TST_G1', N'测试商品-洗发水', N'500ml', N'T01',
            (SELECT TOP 1 BrandID FROM tBas_Brand WHERE BrandName=N'TST_测试品牌'), N'Y');
IF NOT EXISTS (SELECT 1 FROM tBas_Goods WHERE GDSNO = N'TST_G2')
    INSERT INTO tBas_Goods (GDSID, GDSNO, GDSDesc, GDSSpec, UnitNO, BrandID, State)
    VALUES (N'BBBBBBBB-BBBB-BBBB-BBBB-BBBBBBBBBBBB', N'TST_G2', N'测试商品-沐浴露', N'1L', N'T01',
            (SELECT TOP 1 BrandID FROM tBas_Brand WHERE BrandName=N'TST_测试品牌'), N'Y');
PRINT N'[OK] 商品 ×2';
GO

/* ---------- 6. 客户（AreaID NOT NULL，先建测试地区）---------- */
IF NOT EXISTS (SELECT 1 FROM tBas_Area WHERE AreaName = N'TST_测试地区')
    INSERT INTO tBas_Area (AreaID, AreaName, Used)
    VALUES (NEWID(), N'TST_测试地区', N'Y');
IF NOT EXISTS (SELECT 1 FROM tBas_Cust WHERE CustNo = N'TST_C1')
    INSERT INTO tBas_Cust (CustID, CustNo, CustName, AreaID, State)
    VALUES (N'CCCCCCCC-CCCC-CCCC-CCCC-CCCCCCCCCCCC', N'TST_C1', N'测试客户-张三超市',
            (SELECT TOP 1 AreaID FROM tBas_Area WHERE AreaName=N'TST_测试地区'), N'Y');
PRINT N'[OK] 客户 ×1';
GO

/* ---------- 7. 供应商 ---------- */
IF NOT EXISTS (SELECT 1 FROM tBas_Supp WHERE SuppNO = N'TST_S1')
    INSERT INTO tBas_Supp (SuppID, SuppNO, SuppName, State)
    VALUES (N'DDDDDDDD-DDDD-DDDD-DDDD-DDDDDDDDDDDD', N'TST_S1', N'测试供应商-日化批发', N'Y');
PRINT N'[OK] 供应商 ×1';
GO

/* ---------- 8. 员工 ---------- */
IF NOT EXISTS (SELECT 1 FROM tBas_Emp WHERE EmpNO = N'TST_E1')
    INSERT INTO tBas_Emp (EmpID, EmpNO, EmpName, State)
    VALUES (N'EEEEEEEE-EEEE-EEEE-EEEE-EEEEEEEEEEEE', N'TST_E1', N'测试员工-李四', N'Y');
PRINT N'[OK] 员工 ×1';
GO

/* ---------- 9. 初始库存 ---------- */
IF NOT EXISTS (SELECT 1 FROM tStk_Stock WHERE GDSID=N'AAAAAAAA-AAAA-AAAA-AAAA-AAAAAAAAAAAA' AND StkID=N'11111111-1111-1111-1111-111111111111')
    INSERT INTO tStk_Stock (GDSStockID, GDSID, StkID, Qty, QQty)
    VALUES (NEWID(), N'AAAAAAAA-AAAA-AAAA-AAAA-AAAAAAAAAAAA', N'11111111-1111-1111-1111-111111111111', 100, 100);
IF NOT EXISTS (SELECT 1 FROM tStk_Stock WHERE GDSID=N'BBBBBBBB-BBBB-BBBB-BBBB-BBBBBBBBBBBB' AND StkID=N'11111111-1111-1111-1111-111111111111')
    INSERT INTO tStk_Stock (GDSStockID, GDSID, StkID, Qty, QQty)
    VALUES (NEWID(), N'BBBBBBBB-BBBB-BBBB-BBBB-BBBBBBBBBBBB', N'11111111-1111-1111-1111-111111111111', 50, 50);
PRINT N'[OK] 初始库存 ×2';
GO

/* ---------- 验证（注意列名：Brand 用 BrandName 标识，无 BrandNO）---------- */
PRINT N'';
PRINT N'--- 测试数据汇总 ---';
SELECT N'单位' AS 类型, COUNT(*) AS 数 FROM tBas_Unit WHERE UnitNO LIKE N'T0_'
UNION ALL SELECT N'品牌', COUNT(*) FROM tBas_Brand WHERE BrandName LIKE N'TST_%'
UNION ALL SELECT N'分类', COUNT(*) FROM tBas_GDSType WHERE GDSTypeCode LIKE N'TST_%'
UNION ALL SELECT N'仓库', COUNT(*) FROM tBas_Stock WHERE StkCode LIKE N'TST_%'
UNION ALL SELECT N'商品', COUNT(*) FROM tBas_Goods WHERE GDSNO LIKE N'TST_%'
UNION ALL SELECT N'客户', COUNT(*) FROM tBas_Cust WHERE CustNo LIKE N'TST_%'
UNION ALL SELECT N'供应商', COUNT(*) FROM tBas_Supp WHERE SuppNO LIKE N'TST_%'
UNION ALL SELECT N'员工', COUNT(*) FROM tBas_Emp WHERE EmpNO LIKE N'TST_%'
UNION ALL SELECT N'库存', COUNT(*) FROM tStk_Stock WHERE StkID IN (N'11111111-1111-1111-1111-111111111111',N'22222222-2222-2222-2222-222222222222');
GO

PRINT N'';
PRINT N'=== DB-10 完成 ===';
PRINT N'所有测试数据用 TST_ 前缀，可用 LIKE N''TST_%'' 一次性清理。';
GO
