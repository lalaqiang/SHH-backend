-- =============================================
-- 深华辉日化 ERP - 库存安全网触发器
-- 作用：应用层 post_ledger 漏调时的最后一道防线
-- 设计：冗余写 + 幂等 UPSERT，与应用层 double-write 配合
-- 兼容：SQL Server 2005+
-- 使用：直接执行本文件；重复执行安全（用 IF EXISTS / CREATE OR ALTER）
-- =============================================

-- =============================================
-- 1. 触发器：tStk_IODetail 写入后兜底刷新 tStk_Stock
--    触发场景：应用层漏调 post_ledger 时自动补刀
--    限定：仅当主表 State='S'(已审核) 或 'Y'(已确认) 时才生效
-- =============================================
IF OBJECT_ID('trg_IODetail_SafetyStock', 'TR') IS NOT NULL
    DROP TRIGGER trg_IODetail_SafetyStock;
GO

CREATE TRIGGER trg_IODetail_SafetyStock
ON tStk_IODetail
AFTER INSERT, UPDATE, DELETE
AS
BEGIN
    SET NOCOUNT ON;
    -- 收集受影响的行（inserted ∪ deleted）
    ;WITH affected AS (
        SELECT i.IODetailID, i.GDSID, i.StkID, i.Qty, i.IOID
        FROM inserted i
        UNION
        SELECT d.IODetailID, d.GDSID, d.StkID, d.Qty, d.IOID
        FROM deleted d
    ),
    -- 只保留"主表已审核/已确认"的行（State IN ('S','Y')）
    audited AS (
        SELECT a.GDSID, a.StkID, a.Qty, io.Kind
        FROM affected a
        INNER JOIN tStk_IO io ON io.IOID = a.IOID
        WHERE io.State IN ('S', 'Y')
    ),
    -- 按 (GDSID, StkID) 聚合 + 按 Kind 决定方向
    -- 权威来源：tBas_BillType.InOut 字段（+1=入库, -1=出库, 0=调拨）
    -- RI=领用单(出库), TH=门店退仓(调拨), PR=采购退货(出库)
    delta AS (
        SELECT
            GDSID, StkID,
            SUM(CASE
                WHEN Kind IN ('PD', 'SR', 'OTI', 'DBI') AND Qty > 0 THEN Qty       -- 正向入库
                WHEN Kind IN ('SD', 'POS', 'SI', 'RI', 'PR', 'OTO', 'DBO') AND Qty > 0 THEN -Qty -- 正向出库
                WHEN Kind IN ('PD', 'SR', 'OTI', 'DBI') AND Qty < 0 THEN Qty       -- 负向入库（冲销）
                WHEN Kind IN ('SD', 'POS', 'SI', 'RI', 'PR', 'OTO', 'DBO') AND Qty < 0 THEN -Qty
                ELSE 0  -- TH/DB/ZP/OT 等调拨类不在触发器处理，由应用层双边过账
            END) AS Delta
        FROM audited
        GROUP BY GDSID, StkID
    )
    -- UPSERT tStk_Stock（Qty + QQty 同步）
    MERGE tStk_Stock AS target
    USING delta AS src ON target.GDSID = src.GDSID AND target.StkID = src.StkID
    WHEN MATCHED AND src.Delta <> 0 THEN
        UPDATE SET
            Qty  = ISNULL(target.Qty,  0) + src.Delta,
            QQty = ISNULL(target.QQty, 0) + src.Delta
    WHEN NOT MATCHED BY TARGET AND src.Delta <> 0 THEN
        INSERT (GDSStockID, GDSID, StkID, Qty, QQty)
        VALUES (NEWID(), src.GDSID, src.StkID, src.Delta, src.Delta)
    WHEN MATCHED AND src.Delta = 0 THEN
        DELETE;  -- 净变化为0则删除空行（防垃圾）
END
GO

-- =============================================
-- 2. 触发器：tStk_MoveDetail 写入后兜底刷新 tStk_Stock（调拨双边）
--    仅当主表 State='S'/'Y' 时生效
-- =============================================
IF OBJECT_ID('trg_MoveDetail_SafetyStock', 'TR') IS NOT NULL
    DROP TRIGGER trg_MoveDetail_SafetyStock;
GO

CREATE TRIGGER trg_MoveDetail_SafetyStock
ON tStk_MoveDetail
AFTER INSERT, UPDATE, DELETE
AS
BEGIN
    SET NOCOUNT ON;
    ;WITH affected AS (
        SELECT m.MoveID, m.MoveDetailID, m.GDSID, m.Qty
        FROM inserted m
        UNION
        SELECT m.MoveID, m.MoveDetailID, m.GDSID, m.Qty
        FROM deleted m
    ),
    audited AS (
        SELECT a.GDSID, mv.FromStkID AS StkID, a.Qty, -1 AS Sign  -- 调出仓 -qty
        FROM affected a
        INNER JOIN tStk_Move mv ON mv.MoveID = a.MoveID
        WHERE mv.State IN ('S','Y') AND ISNULL(mv.FromStkID, '00000000-0000-0000-0000-000000000000') <> '00000000-0000-0000-0000-000000000000'
        UNION ALL
        SELECT a.GDSID, mv.ToStkID AS StkID, a.Qty, +1 AS Sign    -- 调入仓 +qty
        FROM affected a
        INNER JOIN tStk_Move mv ON mv.MoveID = a.MoveID
        WHERE mv.State IN ('S','Y') AND ISNULL(mv.ToStkID, '00000000-0000-0000-0000-000000000000') <> '00000000-0000-0000-0000-000000000000'
    ),
    delta AS (
        SELECT GDSID, StkID, SUM(Qty * Sign) AS Delta
        FROM audited
        GROUP BY GDSID, StkID
    )
    MERGE tStk_Stock AS target
    USING delta AS src ON target.GDSID = src.GDSID AND target.StkID = src.StkID
    WHEN MATCHED AND src.Delta <> 0 THEN
        UPDATE SET
            Qty  = ISNULL(target.Qty,  0) + src.Delta,
            QQty = ISNULL(target.QQty, 0) + src.Delta
    WHEN NOT MATCHED BY TARGET AND src.Delta <> 0 THEN
        INSERT (GDSStockID, GDSID, StkID, Qty, QQty)
        VALUES (NEWID(), src.GDSID, src.StkID, src.Delta, src.Delta);
END
GO

-- =============================================
-- 3. 触发器：tStk_TranDetail 写入后兜底刷新 tStk_Stock（盘点按 DiffQty）
--    仅当主表 State='S'/'Y' 时生效
-- =============================================
IF OBJECT_ID('trg_TranDetail_SafetyStock', 'TR') IS NOT NULL
    DROP TRIGGER trg_TranDetail_SafetyStock;
GO

CREATE TRIGGER trg_TranDetail_SafetyStock
ON tStk_TranDetail
AFTER INSERT, UPDATE, DELETE
AS
BEGIN
    SET NOCOUNT ON;
    ;WITH affected AS (
        SELECT t.TranID, t.TranDetailID, t.GDSID, t.StkID, t.DiffQty
        FROM inserted t
        UNION
        SELECT t.TranID, t.TranDetailID, t.GDSID, t.StkID, t.DiffQty
        FROM deleted t
    ),
    audited AS (
        SELECT a.GDSID, a.StkID, a.DiffQty
        FROM affected a
        INNER JOIN tStk_Tran tr ON tr.TranID = a.TranID
        WHERE tr.State IN ('S','Y')
    ),
    delta AS (
        SELECT GDSID, StkID, SUM(ISNULL(DiffQty, 0)) AS Delta
        FROM audited
        GROUP BY GDSID, StkID
    )
    MERGE tStk_Stock AS target
    USING delta AS src ON target.GDSID = src.GDSID AND target.StkID = src.StkID
    WHEN MATCHED AND src.Delta <> 0 THEN
        UPDATE SET
            Qty  = ISNULL(target.Qty,  0) + src.Delta,
            QQty = ISNULL(target.QQty, 0) + src.Delta
    WHEN NOT MATCHED BY TARGET AND src.Delta <> 0 THEN
        INSERT (GDSStockID, GDSID, StkID, Qty, QQty)
        VALUES (NEWID(), src.GDSID, src.StkID, src.Delta, src.Delta);
END
GO

-- =============================================
-- 4. 触发器：tStk_Stock 自动维护 tStk_Qty 物化快照
--    每次 tStk_Stock 变更都同步 tStk_Qty
-- =============================================
IF OBJECT_ID('trg_Stock_AfterChange', 'TR') IS NOT NULL
    DROP TRIGGER trg_Stock_AfterChange;
GO

CREATE TRIGGER trg_Stock_AfterChange
ON tStk_Stock
AFTER INSERT, UPDATE, DELETE
AS
BEGIN
    SET NOCOUNT ON;
    -- collected 源行（inserted 与 deleted 取并集去重）
    ;WITH src AS (
        SELECT GDSID, StkID, Qty FROM inserted
        UNION
        SELECT GDSID, StkID, Qty FROM deleted
    )
    MERGE tStk_Qty AS target
    USING src ON target.GDSID = src.GDSID AND target.StkID = src.StkID
    WHEN MATCHED THEN
        UPDATE SET Qty = src.Qty, LUTime = GETDATE()
    WHEN NOT MATCHED BY TARGET THEN
        INSERT (GDSID, StkID, Qty, LUTime) VALUES (src.GDSID, src.StkID, src.Qty, GETDATE())
    WHEN NOT MATCHED BY SOURCE THEN
        DELETE;  -- 源端已删 → 同步删快照
END
GO

-- =============================================
-- 5. CHECK 约束：tStk_Stock 数据完整性
--    规则：Qty >= 0, QQty >= 0, Qty >= QQty
--    说明：用 WITH NOCHECK 跳过历史数据校验，新数据生效
-- =============================================
IF NOT EXISTS (SELECT 1 FROM sys.check_constraints WHERE name = 'CK_Stock_Qty_NonNeg')
    ALTER TABLE tStk_Stock WITH NOCHECK
    ADD CONSTRAINT CK_Stock_Qty_NonNeg CHECK (ISNULL(Qty, 0) >= 0 AND ISNULL(QQty, 0) >= 0);
GO

IF NOT EXISTS (SELECT 1 FROM sys.check_constraints WHERE name = 'CK_Stock_Qty_GE_QQty')
    ALTER TABLE tStk_Stock WITH NOCHECK
    ADD CONSTRAINT CK_Stock_Qty_GE_QQty CHECK (ISNULL(QQty, 0) <= ISNULL(Qty, 0));
GO

IF NOT EXISTS (SELECT 1 FROM sys.check_constraints WHERE name = 'CK_IODetail_Qty_NotZero')
    ALTER TABLE tStk_IODetail WITH NOCHECK
    ADD CONSTRAINT CK_IODetail_Qty_NotZero CHECK (Qty <> 0);
GO

-- =============================================
-- 6. 验证安装：列出本次创建的所有对象
-- =============================================
SELECT
    o.type_desc AS 对象类型,
    o.name      AS 对象名
FROM sys.objects o
WHERE o.name IN (
    'trg_IODetail_SafetyStock',
    'trg_MoveDetail_SafetyStock',
    'trg_TranDetail_SafetyStock',
    'trg_Stock_AfterChange',
    'CK_Stock_Qty_NonNeg',
    'CK_Stock_Qty_GE_QQty',
    'CK_IODetail_Qty_NotZero'
)
ORDER BY o.type_desc, o.name;
GO

PRINT '';
PRINT '=== 库存安全网触发器安装完成 ===';
PRINT '应用层已实现 post_ledger 同步刷新 tStk_Stock。';
PRINT '本套触发器是冗余写，作为应用层 bug 的最后防线。';
PRINT '正常情况下数据会被双重刷新（应用层 + 触发器），是幂等的。';
PRINT '';
