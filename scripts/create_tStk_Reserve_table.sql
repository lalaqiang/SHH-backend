-- tStk_Reserve：库存预占表（销售订单审核时写入，销售出库审核时按源单释放）
-- 销售订单审核：QQty += qty（tStk_Stock），同时写入一条 tStk_Reserve 记录（State='A'）
-- 销售出库审核（有源 SO）：QQty -= qty（释放预占），同时更新 tStk_Reserve.ReleasedQty += qty（满额改 State='X'）
-- 销售订单反审：QQty -= qty，同时 tStk_Reserve.State='X'
-- 销售出库反审（有源 SO）：QQty += qty（重预占），同时回退 tStk_Reserve.ReleasedQty -= qty（恢复 State='A'）

IF NOT EXISTS (SELECT 1 FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_NAME = 'tStk_Reserve')
BEGIN
    CREATE TABLE tStk_Reserve (
        ReserveID   uniqueidentifier NOT NULL DEFAULT NEWID(),
        DocType     varchar(10)  NOT NULL,          -- 单据类型前缀，如 'SO'
        DocID       varchar(40)  NOT NULL,          -- 源单主键（SOID）
        DocNo       varchar(30)  NOT NULL,          -- 源单号（SoNo）
        DetailID    varchar(40)  NULL,              -- 明细行主键（SODetailID）
        GDSID       uniqueidentifier NOT NULL,
        StkID       uniqueidentifier NOT NULL,
        Qty         decimal(18,4) NOT NULL DEFAULT 0,    -- 预占数量
        ReleasedQty decimal(18,4) NOT NULL DEFAULT 0,    -- 已释放数量（出库后递增，等于 Qty 时 State='X'）
        State       char(1)      NOT NULL DEFAULT 'A',   -- 'A'=Active, 'X'=已释放/已作废
        EDate       datetime     NULL,
        EUser       uniqueidentifier NULL,
        CONSTRAINT PK_tStk_Reserve PRIMARY KEY (ReserveID)
    );

    CREATE INDEX IX_tStk_Reserve_DocID   ON tStk_Reserve (DocID, GDSID, StkID);
    CREATE INDEX IX_tStk_Reserve_GDSStk  ON tStk_Reserve (GDSID, StkID, State);
    CREATE INDEX IX_tStk_Reserve_DocType ON tStk_Reserve (DocType, DocID, State);

    PRINT 'tStk_Reserve 表创建成功';
END
ELSE
BEGIN
    PRINT 'tStk_Reserve 表已存在，跳过创建';
END
