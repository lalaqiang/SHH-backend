-- =====================================================================
-- 增加 tBas_Brand.BrandABC（品牌线别）列
-- 说明：原数据库中 tBas_Brand 表缺少 BrandABC 字段，但前端表单已使用
-- =====================================================================

IF OBJECT_ID('tBas_Brand', 'U') IS NOT NULL BEGIN
    IF COL_LENGTH('tBas_Brand', 'BrandABC') IS NULL
        ALTER TABLE [tBas_Brand] ADD [BrandABC] NVARCHAR(50) NULL;
END

PRINT 'tBas_Brand.BrandABC column ensured';
