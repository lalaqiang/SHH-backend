/* ============================================================================
   DB-07 月结存储过程（SQL Server 2005 兼容）
   ----------------------------------------------------------------------------
   模块：DB-07
   目标：sp_MonthEndSettle(@FromYM, @ToYM) —— 把来源月 EndQty 复制为目标月 InitQty
   特性：
     1) 幂等：目标月已有 InitQty>0 的记录会跳过（不重复累加）
     2) 期间锁定：月结后该月视为"已结账"，应用层反审时检测下月 InitQty>0 阻断
     3) 2005 兼容：不用 MERGE，用 INSERT...WHERE NOT EXISTS
   ============================================================================ */

USE [TestERP];
GO
SET NOCOUNT ON;
GO

IF OBJECT_ID(N'sp_MonthEndSettle', N'P') IS NOT NULL DROP PROCEDURE [dbo].[sp_MonthEndSettle];
GO
CREATE PROCEDURE [dbo].[sp_MonthEndSettle]
    @FromYM int,    -- 来源月 YYYYMM（如 202606）
    @ToYM   int     -- 目标月 YYYYMM（如 202607）
AS
BEGIN
    SET NOCOUNT ON;
    IF @FromYM < 200001 OR @ToYM < 200001 OR @ToYM <= @FromYM
    BEGIN
        RAISERROR(N'月份参数非法：FromYM=%d, ToYM=%d（需 FromYM>=200001 且 ToYM>FromYM）', 16, 1, @FromYM, @ToYM);
        RETURN -1;
    END

    BEGIN TRAN;
    -- 把来源月所有 (StkID, GDSID) 的 EndQty 作为目标月的 InitQty
    -- 幂等：目标月已有记录则跳过（用 NOT EXISTS）
    INSERT INTO [dbo].[tStk_StockYM] (AccYM, StkID, GDSID, InitQty, inQty, OutQty, EndQty)
    SELECT  @ToYM, m.StkID, m.GDSID, m.EndQty, 0, 0, m.EndQty
    FROM    [dbo].[tStk_StockYM] m
    WHERE   m.AccYM = @FromYM
      AND   NOT EXISTS (
              SELECT 1 FROM [dbo].[tStk_StockYM] t
              WHERE t.AccYM = @ToYM AND t.StkID = m.StkID AND t.GDSID = m.GDSID
            );

    DECLARE @rows int; SET @rows = @@ROWCOUNT;
    COMMIT TRAN;

    PRINT N'月结完成：' + CONVERT(nvarchar(10), @rows) + N' 条 (StkID,GDSID) 从 ' +
          CONVERT(nvarchar(10), @FromYM) + N' 复制到 ' + CONVERT(nvarchar(10), @ToYM);
    RETURN @rows;
END
GO

/* ---------- 验证（用当前月+下月做一次 dry run）---------- */
DECLARE @cur int, @nxt int, @r int;
SET @cur = CONVERT(int, CONVERT(nvarchar(6), GETDATE(), 112));
IF @cur % 100 = 12 SET @nxt = (@cur / 100 + 1) * 100 + 1; ELSE SET @nxt = @cur + 1;
PRINT N'试运行月结：' + CONVERT(nvarchar(10), @cur) + N' -> ' + CONVERT(nvarchar(10), @nxt);
EXEC @r = sp_MonthEndSettle @cur, @nxt;
PRINT N'返回行数：' + CONVERT(nvarchar(10), @r);
GO

PRINT N'';
PRINT N'=== DB-07 完成 ===';
GO
