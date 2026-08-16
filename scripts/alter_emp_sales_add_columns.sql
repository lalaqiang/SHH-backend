-- ============================================================================
-- 补全 tSal_EmpSales 缺失字段
-- 现有表已有: ID/EmpID/EmpNo/GDSNO/GDSDesc/Qty/Price/Amt/SaleDate/State/EUser/EDate/AUser/ADate
-- 缺失: GDSID(必填), EmpName(前端展示), Remark(备注), LUTime(修改时间)
-- ============================================================================

-- GDSID（必填字段，关联 tBas_Goods.GDSID）
IF NOT EXISTS (SELECT 1 FROM sys.columns WHERE object_id = OBJECT_ID('tSal_EmpSales') AND name = 'GDSID')
BEGIN
    ALTER TABLE [tSal_EmpSales] ADD [GDSID] uniqueidentifier NULL;
    PRINT 'OK: added column GDSID';
END

-- EmpName（前端展示用，冗余字段）
IF NOT EXISTS (SELECT 1 FROM sys.columns WHERE object_id = OBJECT_ID('tSal_EmpSales') AND name = 'EmpName')
BEGIN
    ALTER TABLE [tSal_EmpSales] ADD [EmpName] nvarchar(100) NULL;
    PRINT 'OK: added column EmpName';
END

-- Remark（备注）
IF NOT EXISTS (SELECT 1 FROM sys.columns WHERE object_id = OBJECT_ID('tSal_EmpSales') AND name = 'Remark')
BEGIN
    ALTER TABLE [tSal_EmpSales] ADD [Remark] nvarchar(500) NULL;
    PRINT 'OK: added column Remark';
END

-- LUTime（修改时间）
IF NOT EXISTS (SELECT 1 FROM sys.columns WHERE object_id = OBJECT_ID('tSal_EmpSales') AND name = 'LUTime')
BEGIN
    ALTER TABLE [tSal_EmpSales] ADD [LUTime] datetime NULL;
    PRINT 'OK: added column LUTime';
END
GO

-- 创建索引（IF NOT EXISTS 避免重复）
IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tSal_EmpSales_EmpID' AND object_id = OBJECT_ID('tSal_EmpSales'))
    CREATE INDEX [IX_tSal_EmpSales_EmpID] ON [tSal_EmpSales]([EmpID]);
GO

IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tSal_EmpSales_GDSID' AND object_id = OBJECT_ID('tSal_EmpSales'))
    CREATE INDEX [IX_tSal_EmpSales_GDSID] ON [tSal_EmpSales]([GDSID]);
GO

IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tSal_EmpSales_SaleDate' AND object_id = OBJECT_ID('tSal_EmpSales'))
    CREATE INDEX [IX_tSal_EmpSales_SaleDate] ON [tSal_EmpSales]([SaleDate]);
GO

IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tSal_EmpSales_State' AND object_id = OBJECT_ID('tSal_EmpSales'))
    CREATE INDEX [IX_tSal_EmpSales_State] ON [tSal_EmpSales]([State]);
GO

IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tSal_EmpSales_EUser' AND object_id = OBJECT_ID('tSal_EmpSales'))
    CREATE INDEX [IX_tSal_EmpSales_EUser] ON [tSal_EmpSales]([EUser]);
GO

-- 验证最终结构
SELECT c.name AS column_name, t.name AS type_name, c.max_length, c.is_nullable
FROM sys.columns c
JOIN sys.types t ON c.user_type_id = t.user_type_id
WHERE c.object_id = OBJECT_ID('tSal_EmpSales')
ORDER BY c.column_id;
GO
