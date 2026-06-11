-- =====================================================================
-- 回滚：删除 tBas_Goods.BrandABC 列（如果存在）
-- 商品页的"品牌线别"仅作为前端显示字段，不在 tBas_Goods 中存储
-- 通过 generic/query 的 LEFT JOIN tBas_Brand 实时取值
-- =====================================================================

IF OBJECT_ID('tBas_Goods', 'U') IS NOT NULL BEGIN
    IF COL_LENGTH('tBas_Goods', 'BrandABC') IS NOT NULL
        ALTER TABLE [tBas_Goods] DROP COLUMN [BrandABC];
END

PRINT 'tBas_Goods.BrandABC dropped (if existed)';
