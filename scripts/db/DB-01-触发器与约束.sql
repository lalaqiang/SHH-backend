/* ============================================================================
   DB-01 触发器与 CHECK 约束（SQL Server 2005 兼容）
   ----------------------------------------------------------------------------
   模块：DB-01
   目标：为库存账本三件套补全 DB 层完整性保护
   设计要点：
     1) 严格 SQL Server 2005 兼容：禁用 MERGE(2008+)、IIF、THROW、内联DECLARE赋值、
        sysdatetime()、FORMAT、CONCAT。所有时间用 GETDATE()。
     2) CHECK 约束（3 个）：纯增益，无副作用，必装。
        - CK_Stock_Qty_NonNeg：Qty ≥ 0 AND QQty ≥ 0
        - CK_Stock_Qty_GE_QQty：Qty ≥ QQty（核心不变量）
        - CK_IODetail_Qty_NotZero：入出库明细 Qty ≠ 0
     3) 触发器（2 个，标为可选）：老 Delphi 客户端调 pStk_IOPass 已写 Stock，
        Rust 应用层 post_ledger 也写 Stock。触发器若安装会"三方共写"。
        决策：仅产出脚本，默认不安装。安装前需先确认老客户端是否停用。
     4) 2005 写法：MERGE 全部改写为 IF EXISTS / UPDATE / INSERT UPSERT 模式。
   幂等：全部用 IF EXISTS 判断后再 CREATE，重复执行安全。
   用法：sqlcmd -S server -d TestERP -U sa -P xxx -i DB-01-触发器与约束.sql
   ============================================================================ */

USE [TestERP];
GO
SET NOCOUNT ON;
SET QUOTED_IDENTIFIER ON;
SET ANSI_NULLS ON;
GO

PRINT N'========================================';
PRINT N'DB-01 触发器与 CHECK 约束安装开始';
PRINT N'目标库：' + DB_NAME();
PRINT N'时间：' + CONVERT(nvarchar(19), GETDATE(), 120);
PRINT N'========================================';
GO

/* ============================================================================
   第一部分：CHECK 约束（3 个，必装）
   用 WITH NOCHECK 跳过历史脏数据，只约束新数据，避免老库执行失败。
   ============================================================================ */

-- 1.1 tStk_Stock.Qty ≥ 0 AND QQty ≥ 0
IF NOT EXISTS (SELECT 1 FROM sys.check_constraints WHERE name = N'CK_Stock_Qty_NonNeg')
BEGIN
    ALTER TABLE [dbo].[tStk_Stock] WITH NOCHECK
    ADD CONSTRAINT [CK_Stock_Qty_NonNeg]
    CHECK (ISNULL([Qty], 0) >= 0 AND ISNULL([QQty], 0) >= 0);
    PRINT N'[OK] 已创建 CK_Stock_Qty_NonNeg (tStk_Stock: Qty>=0 AND QQty>=0)';
END
ELSE
    PRINT N'[SKIP] CK_Stock_Qty_NonNeg 已存在';
GO

-- 1.2 tStk_Stock.Qty ≥ QQty（核心不变量：可用量不超过账面总量）
IF NOT EXISTS (SELECT 1 FROM sys.check_constraints WHERE name = N'CK_Stock_Qty_GE_QQty')
BEGIN
    ALTER TABLE [dbo].[tStk_Stock] WITH NOCHECK
    ADD CONSTRAINT [CK_Stock_Qty_GE_QQty]
    CHECK (ISNULL([QQty], 0) <= ISNULL([Qty], 0));
    PRINT N'[OK] 已创建 CK_Stock_Qty_GE_QQty (tStk_Stock: Qty>=QQty)';
END
ELSE
    PRINT N'[SKIP] CK_Stock_Qty_GE_QQty 已存在';
GO

-- 1.3 tStk_IODetail.Qty ≠ 0（防漏填空行）
IF NOT EXISTS (SELECT 1 FROM sys.check_constraints WHERE name = N'CK_IODetail_Qty_NotZero')
BEGIN
    ALTER TABLE [dbo].[tStk_IODetail] WITH NOCHECK
    ADD CONSTRAINT [CK_IODetail_Qty_NotZero]
    CHECK ([Qty] <> 0);
    PRINT N'[OK] 已创建 CK_IODetail_Qty_NotZero (tStk_IODetail: Qty<>0)';
END
ELSE
    PRINT N'[SKIP] CK_IODetail_Qty_NotZero 已存在';
GO

/* ============================================================================
   第二部分：触发器（可选，默认产出但不自动启用）
   ----------------------------------------------------------------------------
   ⚠️ 冲突提示：
     - 老 Delphi 客户端走 pStk_IOPass：已更新 tStk_Stock
     - Rust 应用层走 post_ledger：已更新 tStk_Stock
     - 若再装触发器，会"三方共写"同一行。MERGE 是幂等的（同 delta 加多次），
       但若三方 delta 重复会导致 Qty 被多记。
     - 因此触发器默认不启用。仅当确认"老客户端停用 + 应用层有 bug 漏写"时才启用。
   安装方式：手动执行下方代码块。卸载方式见 DB-01 文档。
   触发条件：tStk_IODetail 写入后，仅当主表 State IN ('S','Y') 时才过账。
   2005 写法：MERGE → IF EXISTS / UPDATE / INSERT。
   ============================================================================ */

-- 2.1 触发器 trg_IODetail_SafetyStock（产出但注释，默认不启用）
/*
IF OBJECT_ID(N'trg_IODetail_SafetyStock', N'TR') IS NOT NULL
    DROP TRIGGER [dbo].[trg_IODetail_SafetyStock];
GO
CREATE TRIGGER [dbo].[trg_IODetail_SafetyStock]
ON [dbo].[tStk_IODetail]
AFTER INSERT, UPDATE, DELETE
AS
BEGIN
    SET NOCOUNT ON;
    -- 收集受影响行（inserted UNION deleted 去重按 IODetailID）
    DECLARE @g NVARCHAR(40), @s NVARCHAR(40), @delta decimal(18,2);
    DECLARE @exists int;

    -- 入库类：RI/PD/SR/OT 出库类：SD/POS/SI/TH/PR/ZP
    -- 用游标逐 (GDSID, StkID) 聚合 delta（2005 无 CTE 聚合 UPDATE，用临时表更稳）
    SELECT  i.GDSID, i.StkID, i.Qty, i.IOID, N'I' AS op
    INTO    #aff
    FROM    inserted i
    UNION ALL
    SELECT  d.GDSID, d.StkID, -d.Qty AS Qty, d.IOID, N'D' AS op
    FROM    deleted d;

    -- 只保留主表已审核的行
    SELECT  a.GDSID, a.StkID,
            SUM(CASE WHEN a.Qty > 0 THEN a.Qty ELSE a.Qty END) AS d
    INTO    #delta
    FROM    #aff a
    INNER JOIN [dbo].[tStk_IO] io ON io.IOID = a.IOID
    WHERE   io.State IN (N'S', N'Y')
    GROUP BY a.GDSID, a.StkID;

    -- UPSERT（2005：IF EXISTS / UPDATE / INSERT，非 MERGE）
    DECLARE cur CURSOR LOCAL FAST_FORWARD FOR SELECT GDSID, StkID, d FROM #delta WHERE d <> 0;
    OPEN cur;
    FETCH NEXT FROM cur INTO @g, @s, @delta;
    WHILE @@FETCH_STATUS = 0
    BEGIN
        SELECT @exists = COUNT(*) FROM [dbo].[tStk_Stock]
         WHERE GDSID = @g AND StkID = @s;
        IF @exists = 1
            UPDATE [dbo].[tStk_Stock]
               SET Qty = ISNULL(Qty, 0) + @delta,
                   QQty = ISNULL(QQty, 0) + @delta
             WHERE GDSID = @g AND StkID = @s;
        ELSE
            INSERT INTO [dbo].[tStk_Stock] (GDSStockID, GDSID, StkID, Qty, QQty)
            VALUES (NEWID(), @g, @s, @delta, @delta);
        FETCH NEXT FROM cur INTO @g, @s, @delta;
    END
    CLOSE cur;
    DEALLOCATE cur;

    DROP TABLE #aff;
    DROP TABLE #delta;
END
GO
*/

-- 2.2 触发器 trg_Stock_AfterChange（同步 tStk_Qty 物化快照，同样默认不启用）
/*
IF OBJECT_ID(N'trg_Stock_AfterChange', N'TR') IS NOT NULL
    DROP TRIGGER [dbo].[trg_Stock_AfterChange];
GO
CREATE TRIGGER [dbo].[trg_Stock_AfterChange]
ON [dbo].[tStk_Stock]
AFTER INSERT, UPDATE, DELETE
AS
BEGIN
    SET NOCOUNT ON;
    -- inserted：UPSERT 到 tStk_Qty
    UPDATE q
       SET q.Qty = i.Qty, q.LUTime = GETDATE()
    FROM [dbo].[tStk_Qty] q
    INNER JOIN inserted i ON q.GDSID = i.GDSID AND q.StkID = i.StkID;

    INSERT INTO [dbo].[tStk_Qty] (GDSID, StkID, Qty, LUTime)
    SELECT i.GDSID, i.StkID, i.Qty, GETDATE()
    FROM inserted i
    WHERE NOT EXISTS (SELECT 1 FROM [dbo].[tStk_Qty] q
                      WHERE q.GDSID = i.GDSID AND q.StkID = i.StkID);

    -- deleted：从 tStk_Qty 同步删除
    DELETE q
    FROM [dbo].[tStk_Qty] q
    INNER JOIN deleted d ON q.GDSID = d.GDSID AND q.StkID = d.StkID
    WHERE NOT EXISTS (SELECT 1 FROM [dbo].[tStk_Stock] s
                      WHERE s.GDSID = d.GDSID AND s.StkID = d.StkID);
END
GO
*/

/* ============================================================================
   第三部分：验证
   ============================================================================ */
PRINT N'';
PRINT N'--- 本次安装结果 ---';
SELECT  o.type_desc AS [类型],
        o.name      AS [对象名]
FROM    sys.objects o
WHERE   o.name IN (N'CK_Stock_Qty_NonNeg',
                   N'CK_Stock_Qty_GE_QQty',
                   N'CK_IODetail_Qty_NotZero',
                   N'trg_IODetail_SafetyStock',
                   N'trg_Stock_AfterChange')
ORDER BY o.type_desc, o.name;
GO

PRINT N'';
PRINT N'=== DB-01 完成 ===';
PRINT N'已安装 3 个 CHECK 约束（核心库存不变量保护）。';
PRINT N'触发器已产出（注释状态），默认不启用，避免与应用层/老存储过程三方冲突。';
PRINT N'如需启用触发器，请先阅读 docs/DB-01-触发器与约束.md 的冲突分析。';
GO
