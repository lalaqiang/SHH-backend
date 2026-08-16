-- ============================================================================
-- tSal_EmpSales — 员工销量录入（扁平表，无明细）
-- 主键使用 UNIQUEIDENTIFIER，与项目其他单据表风格一致
-- 字段与前端 SalesInput.vue columns + doc_graph.rs 必填字段对齐：
--   必填: ID, EmpID, GDSID, Qty, SaleDate
--   前端展示: EmpNo, EmpName, GDSNO, GDSDesc, Qty, Price, Amt, SaleDate, Remark
-- 注: sales_input.rs 的 create/update handler 是死代码（前端走 /api/doc/* 统一接口）
-- ============================================================================

IF NOT EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tSal_EmpSales' AND xtype = 'U')
BEGIN
    CREATE TABLE [tSal_EmpSales] (
        [ID]        uniqueidentifier PRIMARY KEY DEFAULT NEWID(),
        [EmpID]     uniqueidentifier NULL,             -- 关联 tBas_Emp.EmpID
        [EmpNo]     nvarchar(50)     NULL,             -- 员工工号（冗余，便于查询）
        [EmpName]   nvarchar(100)    NULL,             -- 员工姓名（冗余，便于查询）
        [GDSID]     uniqueidentifier NULL,             -- 关联 tBas_Goods.GDSID
        [GDSNO]     nvarchar(50)     NULL,             -- 商品编码（冗余）
        [GDSDesc]   nvarchar(200)    NULL,             -- 商品名称（冗余）
        [Qty]       float            NOT NULL DEFAULT 0,
        [Price]     float            NOT NULL DEFAULT 0,
        [Amt]       float            NOT NULL DEFAULT 0,
        [SaleDate]  datetime         NULL,             -- 销售日期
        [Remark]    nvarchar(500)    NULL,
        [State]     char(1)          NOT NULL DEFAULT 'N',  -- N=新建 S=已审核 D=删除 C=已作废
        [LUTime]    datetime         NULL DEFAULT GETDATE(),
        [EUser]     uniqueidentifier NULL,             -- 创建人 EmpID
        [EDate]     datetime         NULL DEFAULT GETDATE(),
        [AUser]     uniqueidentifier NULL,             -- 审核人 EmpID
        [ADate]     datetime         NULL
    );

    -- 索引
    CREATE INDEX [IX_tSal_EmpSales_EmpID]     ON [tSal_EmpSales]([EmpID]);
    CREATE INDEX [IX_tSal_EmpSales_GDSID]     ON [tSal_EmpSales]([GDSID]);
    CREATE INDEX [IX_tSal_EmpSales_SaleDate]  ON [tSal_EmpSales]([SaleDate]);
    CREATE INDEX [IX_tSal_EmpSales_State]     ON [tSal_EmpSales]([State]);
    CREATE INDEX [IX_tSal_EmpSales_EUser]     ON [tSal_EmpSales]([EUser]);

    PRINT 'OK: tSal_EmpSales created';
END
ELSE
BEGIN
    PRINT 'SKIP: tSal_EmpSales already exists';
END
GO
