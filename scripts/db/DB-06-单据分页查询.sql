/* ============================================================================
   DB-06 单据查询分页存储过程（SQL Server 2005 兼容）
   ----------------------------------------------------------------------------
   模块：DB-06
   目标：为 tStk_IO / tStk_Move / tStk_Tran 提供通用分页查询接口
   设计：
     - sp_QueryIOPage：入出库单（按 Kind/State/仓库/日期/客户供应商过滤）
     - sp_QueryMovePage：调拨单（按 State/仓库/日期过滤）
     - sp_QueryTranPage：盘点单（按 State/仓库/日期过滤）
   2005 兼容：ROW_NUMBER() OVER + CTE 分页
   ============================================================================ */

USE [TestERP];
GO
SET NOCOUNT ON;
GO

/* ---------- 1. tStk_IO 入出库单分页 ---------- */
IF OBJECT_ID(N'sp_QueryIOPage', N'P') IS NOT NULL DROP PROCEDURE [dbo].[sp_QueryIOPage];
GO
CREATE PROCEDURE [dbo].[sp_QueryIOPage]
    @Kind     nvarchar(5)  = NULL,   -- RI/PD/SR/SD/POS/TH/PR/OT/ZP，NULL=全部
    @State    char(1)      = NULL,   -- D/N/S/Y/C，NULL=全部
    @StkID    nvarchar(40) = NULL,   -- 仓库
    @CustID   nvarchar(40) = NULL,   -- 客户（出库时）
    @SuppID   nvarchar(40) = NULL,   -- 供应商（入库时）
    @DateFrom nvarchar(8)  = NULL,   -- 起始日期 YYYYMMDD（对 IoDate）
    @DateTo   nvarchar(8)  = NULL,
    @Keyword  nvarchar(50) = NULL,   -- 匹配 IONo
    @PageNum  int = 1,
    @PageSize int = 50
AS
BEGIN
    SET NOCOUNT ON;
    DECLARE @Offset int; SET @Offset = (@PageNum - 1) * @PageSize;
    DECLARE @Total int;
    SELECT @Total = COUNT(*)
    FROM [dbo].[tStk_IO]
    WHERE (@Kind IS NULL OR Kind = @Kind)
      AND (@State IS NULL OR State = @State)
      AND (@StkID IS NULL OR StkID = @StkID)
      AND (@CustID IS NULL OR CustID = @CustID)
      AND (@SuppID IS NULL OR SuppID = @SuppID)
      AND (@DateFrom IS NULL OR CONVERT(nvarchar(8), IoDate, 112) >= @DateFrom)
      AND (@DateTo IS NULL OR CONVERT(nvarchar(8), IoDate, 112) <= @DateTo)
      AND (@Keyword IS NULL OR IONo LIKE N'%' + @Keyword + N'%');

    WITH page AS (
        SELECT  io.IOID, io.IONo, io.IoDate, io.Kind, io.State, io.StkID,
                io.CustID, io.SuppID, io.EmpID, io.SumQty, io.SumAmt,
                io.EUser, io.EDate, io.AUser, io.ADate,
                st.StkCode, st.StkName,
                cu.CustNO, cu.CustName,
                su.SuppNO, su.SuppName,
                ROW_NUMBER() OVER (ORDER BY io.IoDate DESC, io.IONo DESC) AS rn
        FROM [dbo].[tStk_IO] io
        LEFT JOIN [dbo].[tBas_Stock] st ON st.StkID = io.StkID
        LEFT JOIN [dbo].[tBas_Cust]  cu ON cu.CustID = io.CustID
        LEFT JOIN [dbo].[tBas_Supp]  su ON su.SuppID = io.SuppID
        WHERE (@Kind IS NULL OR io.Kind = @Kind)
          AND (@State IS NULL OR io.State = @State)
          AND (@StkID IS NULL OR io.StkID = @StkID)
          AND (@CustID IS NULL OR io.CustID = @CustID)
          AND (@SuppID IS NULL OR io.SuppID = @SuppID)
          AND (@DateFrom IS NULL OR CONVERT(nvarchar(8), io.IoDate, 112) >= @DateFrom)
          AND (@DateTo IS NULL OR CONVERT(nvarchar(8), io.IoDate, 112) <= @DateTo)
          AND (@Keyword IS NULL OR io.IONo LIKE N'%' + @Keyword + N'%')
    )
    SELECT * FROM page WHERE rn > @Offset AND rn <= @Offset + @PageSize ORDER BY rn;
    SELECT @Total AS TotalCount, @PageNum AS PageNum, @PageSize AS PageSize;
END
GO

/* ---------- 2. tStk_Move 调拨单分页 ---------- */
IF OBJECT_ID(N'sp_QueryMovePage', N'P') IS NOT NULL DROP PROCEDURE [dbo].[sp_QueryMovePage];
GO
CREATE PROCEDURE [dbo].[sp_QueryMovePage]
    @State    char(1)      = NULL,
    @StkID    nvarchar(40) = NULL,   -- 调出或调入仓
    @DateFrom nvarchar(8)  = NULL,
    @DateTo   nvarchar(8)  = NULL,
    @Keyword  nvarchar(50) = NULL,
    @PageNum  int = 1,
    @PageSize int = 50
AS
BEGIN
    SET NOCOUNT ON;
    DECLARE @Offset int; SET @Offset = (@PageNum - 1) * @PageSize;
    DECLARE @Total int;
    SELECT @Total = COUNT(*)
    FROM [dbo].[tStk_Move]
    WHERE (@State IS NULL OR State = @State)
      AND (@StkID IS NULL OR FromStkID = @StkID OR ToStkID = @StkID)
      AND (@DateFrom IS NULL OR CONVERT(nvarchar(8), MoveDate, 112) >= @DateFrom)
      AND (@DateTo IS NULL OR CONVERT(nvarchar(8), MoveDate, 112) <= @DateTo)
      AND (@Keyword IS NULL OR MoveNO LIKE N'%' + @Keyword + N'%');

    WITH page AS (
        SELECT  m.MoveID, m.MoveNO, m.MoveDate, m.Kind, m.State,
                m.FromStkID, m.ToStkID, m.EmpID, m.RSumAmt,
                m.EUser, m.EDate, m.AUser, m.ADate,
                fs.StkCode AS FromStkCode, fs.StkName AS FromStkName,
                ts.StkCode AS ToStkCode,   ts.StkName AS ToStkName,
                ROW_NUMBER() OVER (ORDER BY m.MoveDate DESC, m.MoveNO DESC) AS rn
        FROM [dbo].[tStk_Move] m
        LEFT JOIN [dbo].[tBas_Stock] fs ON fs.StkID = m.FromStkID
        LEFT JOIN [dbo].[tBas_Stock] ts ON ts.StkID = m.ToStkID
        WHERE (@State IS NULL OR m.State = @State)
          AND (@StkID IS NULL OR m.FromStkID = @StkID OR m.ToStkID = @StkID)
          AND (@DateFrom IS NULL OR CONVERT(nvarchar(8), m.MoveDate, 112) >= @DateFrom)
          AND (@DateTo IS NULL OR CONVERT(nvarchar(8), m.MoveDate, 112) <= @DateTo)
          AND (@Keyword IS NULL OR m.MoveNO LIKE N'%' + @Keyword + N'%')
    )
    SELECT * FROM page WHERE rn > @Offset AND rn <= @Offset + @PageSize ORDER BY rn;
    SELECT @Total AS TotalCount, @PageNum AS PageNum, @PageSize AS PageSize;
END
GO

/* ---------- 3. tStk_Tran 盘点单分页 ---------- */
IF OBJECT_ID(N'sp_QueryTranPage', N'P') IS NOT NULL DROP PROCEDURE [dbo].[sp_QueryTranPage];
GO
CREATE PROCEDURE [dbo].[sp_QueryTranPage]
    @State    char(1)      = NULL,
    @StkID    nvarchar(40) = NULL,
    @DateFrom nvarchar(8)  = NULL,
    @DateTo   nvarchar(8)  = NULL,
    @Keyword  nvarchar(50) = NULL,
    @PageNum  int = 1,
    @PageSize int = 50
AS
BEGIN
    SET NOCOUNT ON;
    DECLARE @Offset int; SET @Offset = (@PageNum - 1) * @PageSize;
    DECLARE @Total int;
    SELECT @Total = COUNT(*)
    FROM [dbo].[tStk_Tran]
    WHERE (@State IS NULL OR State = @State)
      AND (@StkID IS NULL OR StkID = @StkID)
      AND (@DateFrom IS NULL OR CONVERT(nvarchar(8), TranDate, 112) >= @DateFrom)
      AND (@DateTo IS NULL OR CONVERT(nvarchar(8), TranDate, 112) <= @DateTo)
      AND (@Keyword IS NULL OR TranNo LIKE N'%' + @Keyword + N'%');

    WITH page AS (
        SELECT  t.TranID, t.TranNo, t.TranDate, t.State, t.StkID, t.EmpID,
                t.EUser, t.EDate, t.AUser, t.ADate,
                st.StkCode, st.StkName,
                ROW_NUMBER() OVER (ORDER BY t.TranDate DESC, t.TranNo DESC) AS rn
        FROM [dbo].[tStk_Tran] t
        LEFT JOIN [dbo].[tBas_Stock] st ON st.StkID = t.StkID
        WHERE (@State IS NULL OR t.State = @State)
          AND (@StkID IS NULL OR t.StkID = @StkID)
          AND (@DateFrom IS NULL OR CONVERT(nvarchar(8), t.TranDate, 112) >= @DateFrom)
          AND (@DateTo IS NULL OR CONVERT(nvarchar(8), t.TranDate, 112) <= @DateTo)
          AND (@Keyword IS NULL OR t.TranNo LIKE N'%' + @Keyword + N'%')
    )
    SELECT * FROM page WHERE rn > @Offset AND rn <= @Offset + @PageSize ORDER BY rn;
    SELECT @Total AS TotalCount, @PageNum AS PageNum, @PageSize AS PageSize;
END
GO

/* ---------- 验证 ---------- */
PRINT N'验证 sp_QueryIOPage（SD 出库，前 3 行）：';
EXEC sp_QueryIOPage @Kind = N'SD', @PageSize = 3;
PRINT N'验证 sp_QueryMovePage（前 3 行）：';
EXEC sp_QueryMovePage @PageSize = 3;
PRINT N'验证 sp_QueryTranPage（前 3 行）：';
EXEC sp_QueryTranPage @PageSize = 3;
GO

PRINT N'';
PRINT N'=== DB-06 完成 ===';
GO
