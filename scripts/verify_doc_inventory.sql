-- ============================================================
-- 深华辉日化 ERP - 单据管理与库存管理完整性验证脚本
-- 验证范围：
--   1. 业务表是否存在
--   2. 主键/外键是否正确
--   3. 关键索引是否已建
--   4. tSys_DocNo 单据号规则是否完整
--   5. BTPID 业务类型数据
--   6. tStk_Reserve 预占表结构
-- ============================================================

SET NOCOUNT ON
DECLARE @ErrCount INT = 0
DECLARE @WarnCount INT = 0

PRINT '========================================================='
PRINT '  ERP 单据库存管理完整性验证'
PRINT '  执行时间: ' + CONVERT(VARCHAR, GETDATE(), 120)
PRINT '========================================================='
PRINT ''

-- ============================================================
-- 第1部分: 业务表存在性检查
-- ============================================================
PRINT '【第1部分】业务表存在性检查'
PRINT '-----------------------------------'

DECLARE @RequiredTables TABLE (TableName NVARCHAR(60), Module NVARCHAR(20))
INSERT INTO @RequiredTables VALUES
    -- 采购
    ('tPur_Order', '采购'), ('tPur_OrderDetail', '采购'),
    ('tPur_Inv', '采购'), ('tPur_InvDetail', '采购'),
    ('tPur_Return', '采购'), ('tPur_ReturnDetail', '采购'),
    ('tPur_Quote', '采购'), ('tPur_QuoteDetail', '采购'),
    ('tPur_AdjPrice', '采购'), ('tPur_AdjPriceDetail', '采购'),
    -- 销售
    ('tSal_Order', '销售'), ('tSal_OrderDetail', '销售'),
    ('tSal_Inv', '销售'), ('tSal_InvDetail', '销售'),
    ('tSal_Quote', '销售'), ('tSal_QuoteDetail', '销售'),
    ('tSal_AdjPrice', '销售'), ('tSal_AdjPriceDetail', '销售'),
    -- 库存
    ('tStk_IO', '库存'), ('tStk_IODetail', '库存'),
    ('tStk_Move', '库存'), ('tStk_MoveDetail', '库存'),
    ('tStk_Tran', '库存'), ('tStk_TranDetail', '库存'),
    ('tStk_Stock', '库存'),
    ('tStk_Qty', '库存'),
    ('tStk_Reserve', '库存'),
    ('tStk_StockHis', '库存'),
    ('tStk_StockTranHis', '库存'),
    ('tStk_StockYM', '库存'),
    ('tStk_StockCycle', '库存'), ('tStk_StockCycleDetail', '库存'),
    ('tStk_ReplenishApply', '库存'), ('tStk_ReplenishApplyDtl', '库存'),
    -- 基础资料
    ('tBas_Goods', '基础'), ('tBas_Unit', '基础'),
    ('tBas_Cust', '基础'), ('tBas_Supp', '基础'),
    ('tBas_Emp', '基础'), ('tBas_Dept', '基础'),
    ('tBas_Stock', '基础'), ('tBas_Brand', '基础'),
    -- 系统
    ('tSys_DocNo', '系统'), ('tSys_DocNoSeq', '系统'),
    ('tSys_OperHis', '系统'), ('tBas_BillType', '系统')

DECLARE @TblName NVARCHAR(60)
DECLARE cur CURSOR FOR SELECT TableName FROM @RequiredTables
OPEN cur
FETCH NEXT FROM cur INTO @TblName
WHILE @@FETCH_STATUS = 0
BEGIN
    IF NOT EXISTS (SELECT 1 FROM sysobjects WHERE name = @TblName AND xtype = 'U')
    BEGIN
        PRINT '  ❌ 缺失表: ' + @TblName
        SET @ErrCount = @ErrCount + 1
    END
    FETCH NEXT FROM cur INTO @TblName
END
CLOSE cur
DEALLOCATE cur

DECLARE @TotalTables INT = (SELECT COUNT(*) FROM @RequiredTables)
PRINT '  ✓ 已检查 ' + CAST(@TotalTables AS VARCHAR) + ' 张业务表'
PRINT ''

-- ============================================================
-- 第2部分: 主键检查
-- ============================================================
PRINT '【第2部分】主键字段检查'
PRINT '-----------------------------------'

-- 关键主键校验
DECLARE @PKChecks TABLE (TableName NVARCHAR(60), PKField NVARCHAR(60))
INSERT INTO @PKChecks VALUES
    ('tPur_Order', 'POID'), ('tPur_Inv', 'PIID'), ('tPur_Return', 'PRID'),
    ('tPur_Quote', 'PQID'), ('tPur_AdjPrice', 'PAPID'),
    ('tSal_Order', 'SOID'), ('tSal_Inv', 'SIID'),
    ('tSal_Quote', 'SQID'), ('tSal_AdjPrice', 'SAPID'),
    ('tStk_IO', 'IOID'), ('tStk_Move', 'MoveID'),
    ('tStk_Tran', 'TranID'), ('tStk_Stock', 'GDSStockID'),
    ('tStk_Reserve', 'ReserveID'),
    ('tStk_ReplenishApply', 'ApplyID'),
    ('tStk_StockCycle', 'CycleID')

DECLARE @PKTbl NVARCHAR(60), @PKFld NVARCHAR(60)
DECLARE cur2 CURSOR FOR SELECT TableName, PKField FROM @PKChecks
OPEN cur2
FETCH NEXT FROM cur2 INTO @PKTbl, @PKFld
WHILE @@FETCH_STATUS = 0
BEGIN
    IF EXISTS (SELECT 1 FROM sysobjects WHERE name = @PKTbl AND xtype = 'U')
    BEGIN
        IF NOT EXISTS (
            SELECT 1 FROM sys.columns
            WHERE object_id = OBJECT_ID(@PKTbl) AND name = @PKFld
        )
        BEGIN
            PRINT '  ⚠ 主键字段不存在: ' + @PKTbl + '.' + @PKFld
            SET @WarnCount = @WarnCount + 1
        END
    END
    FETCH NEXT FROM cur2 INTO @PKTbl, @PKFld
END
CLOSE cur2
DEALLOCATE cur2

PRINT '  ✓ 主键检查完成（' + CAST(@WarnCount AS VARCHAR) + ' 个警告）'
PRINT ''

-- ============================================================
-- 第3部分: 单据号规则检查
-- ============================================================
PRINT '【第3部分】tSys_DocNo 单据号规则'
PRINT '-----------------------------------'

IF EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tSys_DocNo' AND xtype = 'U')
BEGIN
    SELECT
        DocTypeID AS [单据类型],
        DocName AS [名称],
        Prefix AS [前缀],
        TableName AS [表名],
        FieldName AS [单号字段],
        State AS [状态]
    FROM tSys_DocNo
    ORDER BY DocTypeID

    DECLARE @DocNoCount INT = (SELECT COUNT(*) FROM tSys_DocNo WHERE State = 'Y')
    PRINT '  ✓ 共 ' + CAST(@DocNoCount AS VARCHAR) + ' 个启用单据号规则'

    -- 检查关键单据号
    DECLARE @RequiredDocTypes TABLE (DocType NVARCHAR(30))
    INSERT INTO @RequiredDocTypes VALUES
        ('PO'), ('PI'), ('PR'), ('PRQ'), ('PAP'),
        ('SO'), ('SI'), ('SR'), ('SRQ'),
        ('WO'), ('WSO'), ('WR'), ('WQ'), ('SAP'),
        ('MV'), ('IO'), ('CHK'), ('CYC'), ('RPA'), ('ADJ'),
        ('PAY'), ('RCV'), ('CF'), ('POS')

    SELECT @WarnCount = @WarnCount + COUNT(*)
    FROM @RequiredDocTypes t
    WHERE NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = t.DocType)

    PRINT '  ' + CASE
        WHEN (SELECT COUNT(*) FROM @RequiredDocTypes t WHERE NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = t.DocType)) > 0
        THEN '⚠ 有 ' + CAST((SELECT COUNT(*) FROM @RequiredDocTypes t WHERE NOT EXISTS (SELECT 1 FROM tSys_DocNo WHERE DocTypeID = t.DocType)) AS VARCHAR) + ' 个必需单据号类型缺失'
        ELSE '✓ 所有必需单据号类型已配置'
    END
END
ELSE
BEGIN
    PRINT '  ❌ tSys_DocNo 表不存在！请先执行 init_docno_tables.sql'
    SET @ErrCount = @ErrCount + 1
END
PRINT ''

-- ============================================================
-- 第4部分: BTPID 业务类型数据
-- ============================================================
PRINT '【第4部分】BTPID 业务类型数据'
PRINT '-----------------------------------'

IF EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tBas_BillType' AND xtype = 'U')
BEGIN
    SELECT
        BTPID AS [BTPID],
        BTPName AS [业务类型],
        BTPCode AS [代码],
        InOut AS [进出],
        Kind AS [Kind],
        CodePreFix AS [单据前缀],
        State AS [状态]
    FROM tBas_BillType
    ORDER BY BTPID

    -- 检查批发/零售 BTPID 是否存在
    IF NOT EXISTS (SELECT 1 FROM tBas_BillType WHERE BTPID = '6D8E9880-30BC-41F0-A8DE-E27263453DE4')
    BEGIN
        PRINT '  ❌ 批发 BTPID (6D8E9880-...) 缺失！请执行 add_wholesale_btpid.sql'
        SET @ErrCount = @ErrCount + 1
    END
    ELSE
        PRINT '  ✓ 批发 BTPID (6D8E9880-...) 已存在'

    IF NOT EXISTS (SELECT 1 FROM tBas_BillType WHERE BTPID = 'DE94C869-A125-44FD-A0B2-B93CB7749E37')
    BEGIN
        PRINT '  ⚠ 零售 BTPID (DE94C869-...) 缺失，建议补充'
        SET @WarnCount = @WarnCount + 1
    END
    ELSE
        PRINT '  ✓ 零售 BTPID (DE94C869-...) 已存在'
END
ELSE
BEGIN
    PRINT '  ❌ tBas_BillType 表不存在！'
    SET @ErrCount = @ErrCount + 1
END
PRINT ''

-- ============================================================
-- 第5部分: 关键索引检查
-- ============================================================
PRINT '【第5部分】关键索引检查'
PRINT '-----------------------------------'

DECLARE @IndexChecks TABLE (IdxName NVARCHAR(60), TblName NVARCHAR(60))
INSERT INTO @IndexChecks VALUES
    ('IX_tStk_Reserve_DocType_DocID', 'tStk_Reserve'),
    ('IX_tStk_IODetail_SouID', 'tStk_IODetail'),
    ('IX_tStk_IO_Kind_BTPID_StkID', 'tStk_IO'),
    ('IX_tStk_Stock_GDSID_StkID', 'tStk_Stock'),
    ('IX_tSal_Order_SoNo', 'tSal_Order'),
    ('IX_tSal_Order_BTPID', 'tSal_Order'),
    ('IX_tPur_Order_BTPID', 'tPur_Order'),
    ('IX_tStk_IO_IONo', 'tStk_IO')

DECLARE @IdxName NVARCHAR(60), @IdxTbl NVARCHAR(60)
DECLARE @MissingIdx INT = 0
DECLARE cur3 CURSOR FOR SELECT IdxName, TblName FROM @IndexChecks
OPEN cur3
FETCH NEXT FROM cur3 INTO @IdxName, @IdxTbl
WHILE @@FETCH_STATUS = 0
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM sys.indexes
        WHERE name = @IdxName AND object_id = OBJECT_ID(@IdxTbl)
    )
    BEGIN
        PRINT '  ⚠ 缺失索引: ' + @IdxTbl + '.' + @IdxName
        SET @MissingIdx = @MissingIdx + 1
        SET @WarnCount = @WarnCount + 1
    END
    FETCH NEXT FROM cur3 INTO @IdxName, @IdxTbl
END
CLOSE cur3
DEALLOCATE cur3

PRINT '  ' + CASE
    WHEN @MissingIdx > 0 THEN '⚠ 共 ' + CAST(@MissingIdx AS VARCHAR) + ' 个索引缺失，请执行 add_docno_and_indexes.sql'
    ELSE '✓ 所有关键索引已就位'
END
PRINT ''

-- ============================================================
-- 第6部分: 业务链路数据量统计
-- ============================================================
PRINT '【第6部分】业务数据量统计'
PRINT '-----------------------------------'

DECLARE @Tables4Count TABLE (TblName NVARCHAR(60), DispName NVARCHAR(40))
INSERT INTO @Tables4Count VALUES
    ('tPur_Order', '采购订单'),
    ('tPur_Inv', '采购入库'),
    ('tPur_Return', '采购退货'),
    ('tPur_Quote', '采购报价'),
    ('tPur_AdjPrice', '采购调价'),
    ('tSal_Order', '销售订单'),
    ('tSal_Inv', '销售发票'),
    ('tSal_Quote', '销售报价'),
    ('tSal_AdjPrice', '批发调价'),
    ('tStk_IO', '入出库单'),
    ('tStk_Move', '调拨单'),
    ('tStk_Tran', '盘点单'),
    ('tStk_StockCycle', '周期盘点'),
    ('tStk_ReplenishApply', '补货申请'),
    ('tStk_Stock', '库存余额'),
    ('tStk_Reserve', '库存预占')

DECLARE @CntTbl NVARCHAR(60), @CntDisp NVARCHAR(40), @Cnt INT
DECLARE cur4 CURSOR FOR SELECT TblName, DispName FROM @Tables4Count
OPEN cur4
FETCH NEXT FROM cur4 INTO @CntTbl, @CntDisp
WHILE @@FETCH_STATUS = 0
BEGIN
    IF EXISTS (SELECT 1 FROM sysobjects WHERE name = @CntTbl AND xtype = 'U')
    BEGIN
        DECLARE @SQL NVARCHAR(200) = 'SELECT @cnt = COUNT(*) FROM ' + @CntTbl
        EXEC sp_executesql @SQL, N'@cnt INT OUTPUT', @cnt OUTPUT
        PRINT '  ' + @CntDisp + ' (' + @CntTbl + '): ' + CAST(@Cnt AS VARCHAR) + ' 条'
    END
    FETCH NEXT FROM cur4 INTO @CntTbl, @CntDisp
END
CLOSE cur4
DEALLOCATE cur4
PRINT ''

-- ============================================================
-- 第7部分: 库存预占一致性
-- ============================================================
PRINT '【第7部分】库存预占一致性'
PRINT '-----------------------------------'

IF EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tStk_Reserve' AND xtype = 'U')
BEGIN
    -- 检查是否有 ReleasedQty > Qty 的异常数据
    DECLARE @BadReserve INT
    SELECT @BadReserve = COUNT(*)
    FROM tStk_Reserve
    WHERE ReleasedQty > Qty

    IF @BadReserve > 0
        PRINT '  ⚠ 有 ' + CAST(@BadReserve AS VARCHAR) + ' 条预占数据 ReleasedQty > Qty，请检查'
    ELSE
        PRINT '  ✓ 预占数据一致性正常'

    -- 统计未释放的预占
    DECLARE @ActiveReserve INT, @ActiveQty DECIMAL(18, 4)
    SELECT
        @ActiveReserve = COUNT(*),
        @ActiveQty = SUM(Qty - ISNULL(ReleasedQty, 0))
    FROM tStk_Reserve
    WHERE State = 'A' AND Qty > ISNULL(ReleasedQty, 0)

    PRINT '  活跃预占: ' + CAST(@ActiveReserve AS VARCHAR) + ' 条，剩余量: ' + CAST(@ActiveQty AS VARCHAR)
END
PRINT ''

-- ============================================================
-- 第8部分: BTPID 数据完整性
-- ============================================================
PRINT '【第8部分】BTPID 数据完整性（共用表）'
PRINT '-----------------------------------'

-- 检查 tSal_Order 中是否有 BTPID 为空的记录（这些会导致数据串台）
IF EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tSal_Order' AND xtype = 'U')
BEGIN
    DECLARE @EmptyBtpidSalOrder INT
    SELECT @EmptyBtpidSalOrder = COUNT(*)
    FROM tSal_Order
    WHERE ISNULL(BTPID, '00000000-0000-0000-0000-000000000000') = '00000000-0000-0000-0000-000000000000'

    IF @EmptyBtpidSalOrder > 0
        PRINT '  ⚠ tSal_Order 有 ' + CAST(@EmptyBtpidSalOrder AS VARCHAR) + ' 条 BTPID 为空的记录（运行 add_wholesale_btpid.sql 回填）'
    ELSE
        PRINT '  ✓ tSal_Order 所有记录 BTPID 已设置'
END

IF EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tStk_IO' AND xtype = 'U')
BEGIN
    DECLARE @EmptyBtpidIO INT
    SELECT @EmptyBtpidIO = COUNT(*)
    FROM tStk_IO
    WHERE ISNULL(BTPID, '00000000-0000-0000-0000-000000000000') = '00000000-0000-0000-0000-000000000000'

    IF @EmptyBtpidIO > 0
        PRINT '  ⚠ tStk_IO 有 ' + CAST(@EmptyBtpidIO AS VARCHAR) + ' 条 BTPID 为空的记录'
    ELSE
        PRINT '  ✓ tStk_IO 所有记录 BTPID 已设置'
END
PRINT ''

-- ============================================================
-- 汇总
-- ============================================================
PRINT '========================================================='
PRINT '  验证完成'
PRINT '  时间: ' + CONVERT(VARCHAR, GETDATE(), 120)
PRINT '  错误数: ' + CAST(@ErrCount AS VARCHAR)
PRINT '  警告数: ' + CAST(@WarnCount AS VARCHAR)
PRINT '  结论: ' + CASE
    WHEN @ErrCount = 0 AND @WarnCount = 0 THEN '✓ 全部通过'
    WHEN @ErrCount = 0 THEN '⚠ 通过（有警告）'
    ELSE '❌ 未通过'
END
PRINT '========================================================='
