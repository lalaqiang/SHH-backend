/* ============================================================================
   DB-05 库存查询分页存储过程（SQL Server 2005 兼容）
   ----------------------------------------------------------------------------
   模块：DB-05
   目标：sp_QueryStockPage —— 按仓库/商品/分类/关键字过滤 + 分页查询库存
   2005 兼容：
     - ROW_NUMBER() OVER（2005 支持）+ CTE 分页（不用 OFFSET/FETCH，那是 2012+）
     - 不用 CONCAT，用 +
   ============================================================================ */

USE [TestERP];
GO
SET NOCOUNT ON;
GO

IF OBJECT_ID(N'sp_QueryStockPage', N'P') IS NOT NULL DROP PROCEDURE [dbo].[sp_QueryStockPage];
GO
CREATE PROCEDURE [dbo].[sp_QueryStockPage]
    @StkID    nvarchar(40) = NULL,    -- 仓库 ID（NULL=全部）
    @GDSID    nvarchar(40) = NULL,    -- 商品 ID（NULL=全部）
    @Keyword  nvarchar(100) = NULL,   -- 关键字（匹配商品编码/名称）
    @OnlyPositive char(1) = N'N',     -- Y=只看有库存的，N=全部
    @PageNum  int = 1,                -- 页码（从 1 开始）
    @PageSize int = 50                -- 每页行数
AS
BEGIN
    SET NOCOUNT ON;
    DECLARE @Offset int;
    SET @Offset = (@PageNum - 1) * @PageSize;

    -- 总行数（先算，用于前端分页器）
    DECLARE @Total int;
    SELECT  @Total = COUNT(*)
    FROM    [dbo].[tStk_Stock] s
    LEFT JOIN [dbo].[tBas_Goods] g ON g.GDSID = s.GDSID
    WHERE   (@StkID IS NULL OR s.StkID = @StkID)
      AND   (@GDSID IS NULL OR s.GDSID = @GDSID)
      AND   (@Keyword IS NULL OR g.GDSNO LIKE N'%' + @Keyword + N'%' OR g.GDSDesc LIKE N'%' + @Keyword + N'%')
      AND   (@OnlyPositive <> N'Y' OR ISNULL(s.Qty, 0) > 0);

    -- 用 CTE + ROW_NUMBER() 分页（2005 标准写法）
    -- 实测列名：tBas_Goods.UnitNO（不是 UnitID）、tBas_Stock.StkCode/StkName（不是 StkNO）
    WITH page AS (
        SELECT  s.GDSID, s.StkID, ISNULL(s.Qty, 0) AS Qty, ISNULL(s.QQty, 0) AS QQty,
                g.GDSNO, g.GDSDesc, g.BarCode,
                st.StkCode, st.StkName,
                u.UnitName,
                ROW_NUMBER() OVER (ORDER BY g.GDSNO, st.StkCode) AS rn
        FROM    [dbo].[tStk_Stock] s
        LEFT JOIN [dbo].[tBas_Goods]  g  ON g.GDSID = s.GDSID
        LEFT JOIN [dbo].[tBas_Stock]  st ON st.StkID = s.StkID
        LEFT JOIN [dbo].[tBas_Unit]   u  ON u.UnitNO = g.UnitNO
        WHERE   (@StkID IS NULL OR s.StkID = @StkID)
          AND   (@GDSID IS NULL OR s.GDSID = @GDSID)
          AND   (@Keyword IS NULL OR g.GDSNO LIKE N'%' + @Keyword + N'%' OR g.GDSDesc LIKE N'%' + @Keyword + N'%')
          AND   (@OnlyPositive <> N'Y' OR ISNULL(s.Qty, 0) > 0)
    )
    SELECT  GDSID, StkID, Qty, QQty, GDSNO, GDSDesc, BarCode, StkCode, StkName, UnitName
    FROM    page
    WHERE   rn > @Offset AND rn <= @Offset + @PageSize
    ORDER BY rn;

    -- 返回总数
    SELECT @Total AS TotalCount, @PageNum AS PageNum, @PageSize AS PageSize;
END
GO

/* ---------- 验证 ---------- */
PRINT N'验证 sp_QueryStockPage（首页 5 行）：';
EXEC sp_QueryStockPage @PageNum = 1, @PageSize = 5;
GO

PRINT N'';
PRINT N'=== DB-05 完成 ===';
GO
