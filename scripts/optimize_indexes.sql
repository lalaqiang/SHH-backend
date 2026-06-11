-- ===================================================================
-- ERP 高频表索引优化脚本
-- 适用：SQL Server 2016+ / Azure SQL
-- 执行：sqlcmd -S SERVER -d TestERP -U sa -P sa123456 -C -i optimize_indexes.sql
-- 说明：所有索引均使用 IF NOT EXISTS 检查，重复执行安全
-- ===================================================================

SET NOCOUNT ON;
PRINT '=== ERP 索引优化开始 ===';
PRINT '';

-- -------------------------------------------------------------------
-- 1. tStk_Stock (商品库存主表) - 高频读写
-- -------------------------------------------------------------------
PRINT '--- 1) tStk_Stock ---';

IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tStk_Stock_StkID_GDSID' AND object_id = OBJECT_ID('tStk_Stock'))
  CREATE NONCLUSTERED INDEX IX_tStk_Stock_StkID_GDSID ON tStk_Stock (StkID, GDSID) INCLUDE (Qty, QQty, AInPrice);
PRINT '  IX_tStk_Stock_StkID_GDSID: 仓库商品组合查询（库存查询/汇总）';

IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tStk_Stock_GDSID' AND object_id = OBJECT_ID('tStk_Stock'))
  CREATE NONCLUSTERED INDEX IX_tStk_Stock_GDSID ON tStk_Stock (GDSID) INCLUDE (StkID, Qty);
PRINT '  IX_tStk_Stock_GDSID: 按商品查询所有仓库库存';

-- -------------------------------------------------------------------
-- 2. tStk_StockTranHis (库存流水历史) - 单据过账时写入
-- -------------------------------------------------------------------
PRINT '--- 2) tStk_StockTranHis ---';

IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tStk_StockTranHis_GDSID_StkID_Date' AND object_id = OBJECT_ID('tStk_StockTranHis'))
  CREATE NONCLUSTERED INDEX IX_tStk_StockTranHis_GDSID_StkID_Date ON tStk_StockTranHis (GDSID, StkID, TranDate DESC) INCLUDE (Qty, InOutFlag);
PRINT '  IX_tStk_StockTranHis_GDSID_StkID_Date: 库存流水查询';

IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tStk_StockTranHis_SouID' AND object_id = OBJECT_ID('tStk_StockTranHis'))
  CREATE NONCLUSTERED INDEX IX_tStk_StockTranHis_SouID ON tStk_StockTranHis (SouType, SouID);
PRINT '  IX_tStk_StockTranHis_SouID: 单据反审回滚查询';

IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tStk_StockTranHis_StkID_Date' AND object_id = OBJECT_ID('tStk_StockTranHis'))
  CREATE NONCLUSTERED INDEX IX_tStk_StockTranHis_StkID_Date ON tStk_StockTranHis (StkID, TranDate DESC);
PRINT '  IX_tStk_StockTranHis_StkID_Date: 仓库出入库流水';

-- -------------------------------------------------------------------
-- 3. tStk_IO (入出库单主表) + tStk_IODetail (明细)
-- -------------------------------------------------------------------
PRINT '--- 3) tStk_IO / tStk_IODetail ---';

IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tStk_IO_Kind_State_Date' AND object_id = OBJECT_ID('tStk_IO'))
  CREATE NONCLUSTERED INDEX IX_tStk_IO_Kind_State_Date ON tStk_IO (Kind, State, IODate DESC) INCLUDE (IONo, CustID, SuppID);
PRINT '  IX_tStk_IO_Kind_State_Date: 单据按类型/状态/日期查询';

IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tStk_IODetail_IOID' AND object_id = OBJECT_ID('tStk_IODetail'))
  CREATE NONCLUSTERED INDEX IX_tStk_IODetail_IOID ON tStk_IODetail (IOID) INCLUDE (GDSID, Qty, Price, SumAmt);
PRINT '  IX_tStk_IODetail_IOID: 单据明细查询';

IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tStk_IODetail_GDSID' AND object_id = OBJECT_ID('tStk_IODetail'))
  CREATE NONCLUSTERED INDEX IX_tStk_IODetail_GDSID ON tStk_IODetail (GDSID);
PRINT '  IX_tStk_IODetail_GDSID: 商品销售/采购汇总';

IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tStk_IODetail_SouID' AND object_id = OBJECT_ID('tStk_IODetail'))
  CREATE NONCLUSTERED INDEX IX_tStk_IODetail_SouID ON tStk_IODetail (SouType, SouID);
PRINT '  IX_tStk_IODetail_SouID: 上下游单据追溯';

-- -------------------------------------------------------------------
-- 4. tStk_StockYM (库存月结账) - 月末结账
-- -------------------------------------------------------------------
PRINT '--- 4) tStk_StockYM ---';

IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tStk_StockYM_StkID_GDSID_YM' AND object_id = OBJECT_ID('tStk_StockYM'))
  CREATE NONCLUSTERED INDEX IX_tStk_StockYM_StkID_GDSID_YM ON tStk_StockYM (StkID, GDSID, YearMonth);
PRINT '  IX_tStk_StockYM_StkID_GDSID_YM: 月结账查询';

-- -------------------------------------------------------------------
-- 5. tSal_Order / tPur_Order (销售/采购订单主表)
-- -------------------------------------------------------------------
PRINT '--- 5) tSal_Order / tPur_Order ---';

IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tSal_Order_CustID_State' AND object_id = OBJECT_ID('tSal_Order'))
  CREATE NONCLUSTERED INDEX IX_tSal_Order_CustID_State ON tSal_Order (CustID, State) INCLUDE (SoNo, SoDate, SumAmt);
PRINT '  IX_tSal_Order_CustID_State: 客户订单查询';

IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tSal_OrderDetail_SOID' AND object_id = OBJECT_ID('tSal_OrderDetail'))
  CREATE NONCLUSTERED INDEX IX_tSal_OrderDetail_SOID ON tSal_OrderDetail (SOID) INCLUDE (GDSID, Qty, QQty, Price);
PRINT '  IX_tSal_OrderDetail_SOID: 销售订单明细';

IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tPur_Order_SuppID_State' AND object_id = OBJECT_ID('tPur_Order'))
  CREATE NONCLUSTERED INDEX IX_tPur_Order_SuppID_State ON tPur_Order (SuppID, State) INCLUDE (PoNo, PoDate, SumAmt);
PRINT '  IX_tPur_Order_SuppID_State: 供应商订单查询';

IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tPur_OrderDetail_POID' AND object_id = OBJECT_ID('tPur_OrderDetail'))
  CREATE NONCLUSTERED INDEX IX_tPur_OrderDetail_POID ON tPur_OrderDetail (POID) INCLUDE (GDSID, Qty, InQty, Price);
PRINT '  IX_tPur_OrderDetail_POID: 采购订单明细';

-- -------------------------------------------------------------------
-- 6. tSys_OperHis (操作日志) - 高频写入
-- -------------------------------------------------------------------
PRINT '--- 6) tSys_OperHis ---';

IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tSys_OperHis_OperUser_Date' AND object_id = OBJECT_ID('tSys_OperHis'))
  CREATE NONCLUSTERED INDEX IX_tSys_OperHis_OperUser_Date ON tSys_OperHis (OperUser, OperDate DESC) INCLUDE (OperType, TableName);
PRINT '  IX_tSys_OperHis_OperUser_Date: 用户操作日志查询';

IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tSys_OperHis_TableName_KeyValue' AND object_id = OBJECT_ID('tSys_OperHis'))
  CREATE NONCLUSTERED INDEX IX_tSys_OperHis_TableName_KeyValue ON tSys_OperHis (TableName, KeyValue, OperDate DESC);
PRINT '  IX_tSys_OperHis_TableName_KeyValue: 单据操作历史';

-- -------------------------------------------------------------------
-- 7. 基础资料表（高基数小表：State/Used 索引 + 名称索引）
-- -------------------------------------------------------------------
PRINT '--- 7) 基础资料 ---';

IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tBas_Goods_State_GDSStateNO' AND object_id = OBJECT_ID('tBas_Goods'))
  CREATE NONCLUSTERED INDEX IX_tBas_Goods_State_GDSStateNO ON tBas_Goods (State, GDSStateNO) INCLUDE (GDSNO, GDSDesc, BrandID);
PRINT '  IX_tBas_Goods_State_GDSStateNO: 商品按状态/品态查询';

IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tBas_Cust_State' AND object_id = OBJECT_ID('tBas_Cust'))
  CREATE NONCLUSTERED INDEX IX_tBas_Cust_State ON tBas_Cust (State) INCLUDE (CustNO, CustName, CustTypeID, SalEmpID);
PRINT '  IX_tBas_Cust_State: 客户按状态查询';

IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tBas_Supp_State' AND object_id = OBJECT_ID('tBas_Supp'))
  CREATE NONCLUSTERED INDEX IX_tBas_Supp_State ON tBas_Supp (State) INCLUDE (SuppNO, SuppName, SuppTypeID);
PRINT '  IX_tBas_Supp_State: 供应商按状态查询';

-- -------------------------------------------------------------------
-- 8. 统计信息更新
-- -------------------------------------------------------------------
PRINT '';
PRINT '--- 8) 更新统计信息 ---';

UPDATE STATISTICS tStk_Stock WITH FULLSCAN;
PRINT '  tStk_Stock 统计信息已更新';

UPDATE STATISTICS tStk_StockTranHis WITH FULLSCAN;
PRINT '  tStk_StockTranHis 统计信息已更新';

UPDATE STATISTICS tStk_IODetail WITH FULLSCAN;
PRINT '  tStk_IODetail 统计信息已更新';

UPDATE STATISTICS tSal_OrderDetail WITH FULLSCAN;
PRINT '  tSal_OrderDetail 统计信息已更新';

UPDATE STATISTICS tPur_OrderDetail WITH FULLSCAN;
PRINT '  tPur_OrderDetail 统计信息已更新';

-- -------------------------------------------------------------------
-- 9. 索引使用情况检查（仅输出，不删除）
-- -------------------------------------------------------------------
PRINT '';
PRINT '--- 9) 索引碎片检查（>30% 需重建）---';
SELECT
    OBJECT_NAME(ips.object_id) AS TableName,
    i.name AS IndexName,
    ips.avg_fragmentation_in_percent AS FragPct,
    ips.page_count AS PageCount,
    CASE
        WHEN ips.avg_fragmentation_in_percent > 30 THEN '建议 REBUILD'
        WHEN ips.avg_fragmentation_in_percent > 10 THEN '建议 REORGANIZE'
        ELSE 'OK'
    END AS Recommendation
FROM sys.dm_db_index_physical_stats(DB_ID(), NULL, NULL, NULL, 'LIMITED') ips
INNER JOIN sys.indexes i ON ips.object_id = i.object_id AND ips.index_id = i.index_id
WHERE ips.page_count > 100
  AND i.name IS NOT NULL
  AND OBJECT_NAME(ips.object_id) LIKE 't%'
ORDER BY ips.avg_fragmentation_in_percent DESC;

PRINT '';
PRINT '=== ERP 索引优化完成 ===';
PRINT '说明：以上索引使用 IF NOT EXISTS 检查，重复执行安全';
PRINT '建议：每月维护窗口执行一次 REBUILD 高碎片索引';
