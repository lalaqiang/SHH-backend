-- ============================================================
-- 深华辉日化 ERP 通用插入修复 - 一次性执行
-- ============================================================

-- 1. 业务表主键添加 NEWID() 默认值
IF OBJECT_ID('tPur_Order', 'U') IS NOT NULL BEGIN
    IF COL_LENGTH('tPur_Order', 'POID') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tPur_Order') AND c.name = 'POID')
        ALTER TABLE [tPur_Order] ADD CONSTRAINT [DF_tPur_Order_POID] DEFAULT NEWID() FOR [POID];
END

IF OBJECT_ID('tPur_OrderDetail', 'U') IS NOT NULL BEGIN
    IF COL_LENGTH('tPur_OrderDetail', 'PODetailID') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tPur_OrderDetail') AND c.name = 'PODetailID')
        ALTER TABLE [tPur_OrderDetail] ADD CONSTRAINT [DF_tPur_OrderDetail_PODetailID] DEFAULT NEWID() FOR [PODetailID];
END

IF OBJECT_ID('tSal_Order', 'U') IS NOT NULL BEGIN
    IF COL_LENGTH('tSal_Order', 'SOID') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tSal_Order') AND c.name = 'SOID')
        ALTER TABLE [tSal_Order] ADD CONSTRAINT [DF_tSal_Order_SOID] DEFAULT NEWID() FOR [SOID];
END

IF OBJECT_ID('tSal_Inv', 'U') IS NOT NULL BEGIN
    IF COL_LENGTH('tSal_Inv', 'SIID') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tSal_Inv') AND c.name = 'SIID')
        ALTER TABLE [tSal_Inv] ADD CONSTRAINT [DF_tSal_Inv_SIID] DEFAULT NEWID() FOR [SIID];
END

IF OBJECT_ID('tStk_IO', 'U') IS NOT NULL BEGIN
    IF COL_LENGTH('tStk_IO', 'IOID') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tStk_IO') AND c.name = 'IOID')
        ALTER TABLE [tStk_IO] ADD CONSTRAINT [DF_tStk_IO_IOID] DEFAULT NEWID() FOR [IOID];
END

IF OBJECT_ID('tStk_Move', 'U') IS NOT NULL BEGIN
    IF COL_LENGTH('tStk_Move', 'MoveID') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tStk_Move') AND c.name = 'MoveID')
        ALTER TABLE [tStk_Move] ADD CONSTRAINT [DF_tStk_Move_MoveID] DEFAULT NEWID() FOR [MoveID];
END

IF OBJECT_ID('tBas_Goods', 'U') IS NOT NULL BEGIN
    IF COL_LENGTH('tBas_Goods', 'GDSID') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tBas_Goods') AND c.name = 'GDSID')
        ALTER TABLE [tBas_Goods] ADD CONSTRAINT [DF_tBas_Goods_GDSID] DEFAULT NEWID() FOR [GDSID];
END

IF OBJECT_ID('tBas_Supp', 'U') IS NOT NULL BEGIN
    IF COL_LENGTH('tBas_Supp', 'SuppID') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tBas_Supp') AND c.name = 'SuppID')
        ALTER TABLE [tBas_Supp] ADD CONSTRAINT [DF_tBas_Supp_SuppID] DEFAULT NEWID() FOR [SuppID];
END

IF OBJECT_ID('tBas_Cust', 'U') IS NOT NULL BEGIN
    IF COL_LENGTH('tBas_Cust', 'CustID') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tBas_Cust') AND c.name = 'CustID')
        ALTER TABLE [tBas_Cust] ADD CONSTRAINT [DF_tBas_Cust_CustID] DEFAULT NEWID() FOR [CustID];
END

IF OBJECT_ID('tBas_Emp', 'U') IS NOT NULL BEGIN
    IF COL_LENGTH('tBas_Emp', 'EmpID') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tBas_Emp') AND c.name = 'EmpID')
        ALTER TABLE [tBas_Emp] ADD CONSTRAINT [DF_tBas_Emp_EmpID] DEFAULT NEWID() FOR [EmpID];
END

IF OBJECT_ID('tBas_Stock', 'U') IS NOT NULL BEGIN
    IF COL_LENGTH('tBas_Stock', 'StkID') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tBas_Stock') AND c.name = 'StkID')
        ALTER TABLE [tBas_Stock] ADD CONSTRAINT [DF_tBas_Stock_StkID] DEFAULT NEWID() FOR [StkID];
END

-- 2. EUser 默认值 (tStk_IO / tStk_Move)
IF OBJECT_ID('tStk_IO', 'U') IS NOT NULL BEGIN
    IF COL_LENGTH('tStk_IO', 'EUser') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tStk_IO') AND c.name = 'EUser')
        ALTER TABLE [tStk_IO] ADD CONSTRAINT [DF_tStk_IO_EUser] DEFAULT '00000000-0000-0000-0000-000000000000' FOR [EUser];
END

IF OBJECT_ID('tStk_Move', 'U') IS NOT NULL BEGIN
    IF COL_LENGTH('tStk_Move', 'EUser') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tStk_Move') AND c.name = 'EUser')
        ALTER TABLE [tStk_Move] ADD CONSTRAINT [DF_tStk_Move_EUser] DEFAULT '00000000-0000-0000-0000-000000000000' FOR [EUser];
END

-- 3. 顺序字段默认值 0
IF OBJECT_ID('tBas_Goods', 'U') IS NOT NULL BEGIN
    IF COL_LENGTH('tBas_Goods', 'gdsSD') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tBas_Goods') AND c.name = 'gdsSD')
        ALTER TABLE [tBas_Goods] ADD CONSTRAINT [DF_tBas_Goods_gdsSD] DEFAULT 0 FOR [gdsSD];
END

IF OBJECT_ID('tBas_Supp', 'U') IS NOT NULL BEGIN
    IF COL_LENGTH('tBas_Supp', 'suppSD') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tBas_Supp') AND c.name = 'suppSD')
        ALTER TABLE [tBas_Supp] ADD CONSTRAINT [DF_tBas_Supp_suppSD] DEFAULT 0 FOR [suppSD];
END

IF OBJECT_ID('tBas_Cust', 'U') IS NOT NULL BEGIN
    IF COL_LENGTH('tBas_Cust', 'custSD') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tBas_Cust') AND c.name = 'custSD')
        ALTER TABLE [tBas_Cust] ADD CONSTRAINT [DF_tBas_Cust_custSD] DEFAULT 0 FOR [custSD];
END

IF OBJECT_ID('tBas_Emp', 'U') IS NOT NULL BEGIN
    IF COL_LENGTH('tBas_Emp', 'empSD') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tBas_Emp') AND c.name = 'empSD')
        ALTER TABLE [tBas_Emp] ADD CONSTRAINT [DF_tBas_Emp_empSD] DEFAULT 0 FOR [empSD];
END

IF OBJECT_ID('tBas_Stock', 'U') IS NOT NULL BEGIN
    IF COL_LENGTH('tBas_Stock', 'stkSD') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tBas_Stock') AND c.name = 'stkSD')
        ALTER TABLE [tBas_Stock] ADD CONSTRAINT [DF_tBas_Stock_stkSD] DEFAULT 0 FOR [stkSD];
END

-- 4. 客户表 AreaID 默认值
IF OBJECT_ID('tBas_Cust', 'U') IS NOT NULL BEGIN
    IF COL_LENGTH('tBas_Cust', 'AreaID') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tBas_Cust') AND c.name = 'AreaID')
        ALTER TABLE [tBas_Cust] ADD CONSTRAINT [DF_tBas_Cust_AreaID] DEFAULT '00000000-0000-0000-0000-000000000000' FOR [AreaID];
END

-- 5. OrderState / EDate 默认值（多个表）
IF OBJECT_ID('tPur_Order', 'U') IS NOT NULL BEGIN
    IF COL_LENGTH('tPur_Order', 'OrderState') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tPur_Order') AND c.name = 'OrderState')
        ALTER TABLE [tPur_Order] ADD CONSTRAINT [DF_tPur_Order_OrderState] DEFAULT 'A' FOR [OrderState];
    IF COL_LENGTH('tPur_Order', 'EDate') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tPur_Order') AND c.name = 'EDate')
        ALTER TABLE [tPur_Order] ADD CONSTRAINT [DF_tPur_Order_EDate] DEFAULT GETDATE() FOR [EDate];
END

IF OBJECT_ID('tSal_Order', 'U') IS NOT NULL BEGIN
    IF COL_LENGTH('tSal_Order', 'OrderState') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tSal_Order') AND c.name = 'OrderState')
        ALTER TABLE [tSal_Order] ADD CONSTRAINT [DF_tSal_Order_OrderState] DEFAULT 'A' FOR [OrderState];
    IF COL_LENGTH('tSal_Order', 'EDate') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tSal_Order') AND c.name = 'EDate')
        ALTER TABLE [tSal_Order] ADD CONSTRAINT [DF_tSal_Order_EDate] DEFAULT GETDATE() FOR [EDate];
END

IF OBJECT_ID('tSal_Inv', 'U') IS NOT NULL BEGIN
    IF COL_LENGTH('tSal_Inv', 'OrderState') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tSal_Inv') AND c.name = 'OrderState')
        ALTER TABLE [tSal_Inv] ADD CONSTRAINT [DF_tSal_Inv_OrderState] DEFAULT 'A' FOR [OrderState];
    IF COL_LENGTH('tSal_Inv', 'EDate') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tSal_Inv') AND c.name = 'EDate')
        ALTER TABLE [tSal_Inv] ADD CONSTRAINT [DF_tSal_Inv_EDate] DEFAULT GETDATE() FOR [EDate];
END

IF OBJECT_ID('tStk_IO', 'U') IS NOT NULL BEGIN
    IF COL_LENGTH('tStk_IO', 'OrderState') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tStk_IO') AND c.name = 'OrderState')
        ALTER TABLE [tStk_IO] ADD CONSTRAINT [DF_tStk_IO_OrderState] DEFAULT 'A' FOR [OrderState];
    IF COL_LENGTH('tStk_IO', 'EDate') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tStk_IO') AND c.name = 'EDate')
        ALTER TABLE [tStk_IO] ADD CONSTRAINT [DF_tStk_IO_EDate] DEFAULT GETDATE() FOR [EDate];
END

IF OBJECT_ID('tStk_Move', 'U') IS NOT NULL BEGIN
    IF COL_LENGTH('tStk_Move', 'OrderState') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tStk_Move') AND c.name = 'OrderState')
        ALTER TABLE [tStk_Move] ADD CONSTRAINT [DF_tStk_Move_OrderState] DEFAULT 'A' FOR [OrderState];
    IF COL_LENGTH('tStk_Move', 'EDate') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tStk_Move') AND c.name = 'EDate')
        ALTER TABLE [tStk_Move] ADD CONSTRAINT [DF_tStk_Move_EDate] DEFAULT GETDATE() FOR [EDate];
END

IF OBJECT_ID('tFin_Payment', 'U') IS NOT NULL BEGIN
    IF COL_LENGTH('tFin_Payment', 'EDate') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tFin_Payment') AND c.name = 'EDate')
        ALTER TABLE [tFin_Payment] ADD CONSTRAINT [DF_tFin_Payment_EDate] DEFAULT GETDATE() FOR [EDate];
END

IF OBJECT_ID('tFin_Receipt', 'U') IS NOT NULL BEGIN
    IF COL_LENGTH('tFin_Receipt', 'EDate') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tFin_Receipt') AND c.name = 'EDate')
        ALTER TABLE [tFin_Receipt] ADD CONSTRAINT [DF_tFin_Receipt_EDate] DEFAULT GETDATE() FOR [EDate];
END

IF OBJECT_ID('tFin_Payable', 'U') IS NOT NULL BEGIN
    IF COL_LENGTH('tFin_Payable', 'EDate') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tFin_Payable') AND c.name = 'EDate')
        ALTER TABLE [tFin_Payable] ADD CONSTRAINT [DF_tFin_Payable_EDate] DEFAULT GETDATE() FOR [EDate];
END

IF OBJECT_ID('tFin_Receivable', 'U') IS NOT NULL BEGIN
    IF COL_LENGTH('tFin_Receivable', 'EDate') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tFin_Receivable') AND c.name = 'EDate')
        ALTER TABLE [tFin_Receivable] ADD CONSTRAINT [DF_tFin_Receivable_EDate] DEFAULT GETDATE() FOR [EDate];
END

-- 6. tBas_Stock SortOrder/Used 等默认值
IF OBJECT_ID('tBas_Stock', 'U') IS NOT NULL BEGIN
    IF COL_LENGTH('tBas_Stock', 'Used') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tBas_Stock') AND c.name = 'Used')
        ALTER TABLE [tBas_Stock] ADD CONSTRAINT [DF_tBas_Stock_Used] DEFAULT 'Y' FOR [Used];
END

IF OBJECT_ID('tBas_Goods', 'U') IS NOT NULL BEGIN
    IF COL_LENGTH('tBas_Goods', 'GDSStateNO') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tBas_Goods') AND c.name = 'GDSStateNO')
        ALTER TABLE [tBas_Goods] ADD CONSTRAINT [DF_tBas_Goods_GDSStateNO] DEFAULT 2 FOR [GDSStateNO];
END

PRINT 'All defaults applied successfully';
