use axum::extract::{State, Json, Multipart, Extension};
use axum::response::Response;
use serde::Deserialize;
use tiberius::Row;
use crate::config::Config;
use crate::db::get_pool;
use crate::error::Result;
use crate::utils::{ApiResponse, build_pagination_sql_with_sort};
use crate::handlers::base_data::try_get_value;
use crate::middleware::auth::Claims;

#[derive(Deserialize)]
pub struct WhereCondition {
    pub field: String,
    pub op: String,
    pub value: serde_json::Value,
}

#[derive(Deserialize)]
pub struct GenericQueryParams {
    pub table: String,
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
    pub keyword_fields: Option<Vec<String>>,
    pub sort_prop: Option<String>,
    pub sort_order: Option<String>,
    pub wheres: Option<Vec<WhereCondition>>,
    pub include_deleted: Option<bool>,
    /// 仅显示已删除/已停用行（与 include_deleted 互斥）
    pub only_deleted: Option<bool>,
}

#[derive(Deserialize)]
pub struct GenericDeleteParams {
    pub table: String,
    pub primary_key: String,
    pub ids: Vec<String>,
    pub state_field: Option<String>,
    /// true = 物理删除（DELETE FROM），false = 软删除（更新状态字段）
    pub permanent: Option<bool>,
    /// true = 作废（State='C'，业务作废保留可查），false/unset = 删除（State='D'，软删）
    pub void: Option<bool>,
}

#[derive(Deserialize)]
pub struct GenericCreateParams {
    pub table: String,
    pub data: serde_json::Map<String, serde_json::Value>,
}

#[derive(Deserialize)]
pub struct GenericUpdateParams {
    pub table: String,
    pub primary_key: String,
    pub id: String,
    pub data: serde_json::Map<String, serde_json::Value>,
}

fn row_to_json(row: &Row) -> serde_json::Value {
    let columns = row.columns();
    let mut map = serde_json::Map::new();
    for col in columns {
        let name = col.name().to_string();
        if name == "_rn" {
            continue;
        }
        let val = try_get_value(row, &name);
        map.insert(name, val);
    }
    serde_json::Value::Object(map)
}

/// 哪些列即使值为空也要以 '' 写入（NOT NULL 文本列，避免 '' 被误转 NULL）
fn default_empty_string_cols() -> std::collections::HashSet<String> {
    [
        "PHelp".to_string(),
        "PValue".to_string(),
        "CheckSQL".to_string(),
        "PTerm".to_string(),
    ].into_iter().collect()
}

fn json_to_sql_value(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::Null => None,
        serde_json::Value::String(s) => {
            // Treat empty/whitespace-only strings as NULL — SQL Server cannot
            // convert '' to uniqueidentifier / datetime / numeric, and any
            // nullable column should accept NULL gracefully.
            if s.trim().is_empty() { None } else { Some(s.clone()) }
        }
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(if *b { "1".to_string() } else { "0".to_string() }),
        _ => Some(v.to_string()),
    }
}

/// Returns the primary-key column name for a given table. The PK column
/// is treated specially by the generic insert/update logic (must be a
/// non-empty uniqueidentifier when present, and is auto-generated as a
/// fresh UUID when the client omits it or sends an empty string).
fn get_primary_key_for_table(table: &str) -> Option<&'static str> {
    match table {
        "tPur_Order" => Some("POID"),
        "tPur_OrderDetail" => Some("PODetailID"),
        "tPur_Quote" => Some("PQID"),
        "tPur_AdjPrice" => Some("PAPID"),
        "tPur_AdjPriceDetail" => Some("PAPDetailID"),
        "tSal_Order" => Some("SOID"),
        "tSal_OrderDetail" => Some("SODetailID"),
        "tSal_Inv" => Some("SIID"),
        "tSal_InvDetail" => Some("SIDetailID"),
        "tSal_Quote" => Some("SQID"),
        "tSal_QuoteDetail" => Some("SQDetailID"),
        "tSal_AdjPrice" => Some("SAPID"),
        "tSal_AdjPriceDetail" => Some("SAPDetailID"),
        "tStk_IO" => Some("IOID"),
        "tStk_IODetail" => Some("IODetailID"),
        "tStk_Move" => Some("MoveID"),
        "tStk_MoveDetail" => Some("MoveDetailID"),
        "tStk_ReplenishApply" => Some("ApplyID"),
        "tStk_ReplenishApplyDetail" | "tStk_ReplenishApplyDtl" => Some("ApplyDtlID"),
        "tStk_StockCycle" => Some("CycleID"),
        "tStk_StockCycleDetail" => Some("CycleDetailID"),
        "tStk_Qty" => Some("QtyID"),
        "tBas_Goods" => Some("GDSID"),
        "tBas_Supp" => Some("SuppID"),
        "tBas_Cust" => Some("CustID"),
        "tBas_Emp" => Some("EmpID"),
        "tBas_Stock" => Some("StkID"),
        "tStk_Stock" => Some("GDSStockID"),
        "tBas_Dept" => Some("DeptID"),
        "tBas_Duty" => Some("DutyID"),
        "tBas_Brand" => Some("BrandID"),
        "tBas_GDSType" => Some("GDSTypeID"),
        "tBas_GDSProperty" => Some("GDSPropertyID"),
        "tBas_GDSKind" => Some("GDSKindID"),
        "tBas_DeaType" => Some("DeaTypeID"),
        "tBas_Unit" => Some("UnitID"),
        "tBas_SuppType" => Some("SuppTypeID"),
        "tBas_CustType" => Some("CustTypeID"),
        "tBas_Area" => Some("AreaID"),
        "tBas_Payment" => Some("PaymentID"),
        "tBas_CommTemplate" => Some("CommTplID"),
        "tBas_PriceTemplate" => Some("PriceTplID"),
        "tBas_EmpCommission" => Some("CommID"),
        "tBas_CustPrice" => Some("CustPriceID"),
        "tBas_CustPriceTac" => Some("CustPriceTacID"),
        "tBas_Dictionary" | "tBas_Dict" => Some("DictID"),
        "tFin_Payment_TEST" => Some("PaymentID"),
        "tFin_Receipt_TEST" => Some("ReceiptID"),
        "tFin_Payable" => Some("PayableID"),
        "tFin_Receivable" => Some("ReceivableID"),
        "tFin_CashFlow" => Some("CashFlowID"),
        "tSys_User" => Some("UserID"),
        "tSys_Rule" => Some("RuleID"),
        "tSys_Msg" => Some("MsgID"),
        "tSys_Parameters" | "tSys_Params" => Some("ParametersID"),
        "tSys_Rpt" => Some("RptID"),
        "tSys_RptPrintHis" => Some("PrintHisID"),
        "tSys_RptPrintNum" => Some("PrintNumID"),
        "tSys_Menus" => Some("MenuID"),
        "tSys_OperHis" => Some("OperHisID"),
        "tStk_Qty" => None,
        "tSys_OperHis" => Some("OperHisID"),
        "tSys_DataPack" => Some("DataPackID"),
        "tSys_Notification" => Some("NotifyID"),
        "tSys_TableColumnConfig" => Some("ColumnConfigID"),
        "tSys_UploadFile" => Some("FileID"),
        "tSys_Permission" => Some("PermissionID"),
        "tSys_PrintTemplate" => Some("TemplateID"),
        "tSys_Backup" => Some("BackupID"),
        "tSys_AutoMsg" => Some("AutoMsgID"),
        "tSys_AutoMsgRule" => Some("RuleID"),
        "tSys_Company" => Some("CompanyID"),
        "tSys_RuleMenu" => Some("RuleMenuID"),
        "tSys_UserRule" => Some("UserRuleID"),
        "tSys_RulePermission" => Some("RulePermID"),
        "tSys_RuleStock" => Some("RuleStockID"),
        "tOA_Notice" => Some("NoticeID"),
        "tOA_Workflow" => Some("WFID"),
        "tOA_Email" => Some("EmailID"),
        "tOnline_Goods" => Some("OnlineGDSID"),
        "tOnline_Order" => Some("OnlineOrderID"),
        "tOnline_OrderDetail" => Some("OnlineOrderDtlID"),
        "tOnline_Address" => Some("AddressID"),
        "tOnline_PaymentConfig" => Some("PaymentCfgID"),
        "tSal_VIP" => Some("VIPID"),
        "tSal_SaleTask" => Some("TaskID"),
        "tSal_EmpSales" => Some("ID"),
        "tStk_Tran" => Some("TranID"),
        "tStk_TranDetail" => Some("TranDetailID"),
        "tRpt_Custom" => Some("CustomRptID"),
        "tSys_ITReport" => Some("ITRptID"),
        "vStk_IOFlow" => Some("IODetailID"),
        _ => None,
    }
}

fn get_state_field_for_table(table: &str) -> Option<&'static str> {
    match table {
        "tBas_Brand" | "tBas_Stock" | "tBas_GDSType" | "tBas_GDSProperty" | "tBas_GDSKind" 
        | "tBas_DeaType" | "tBas_Unit" | "tBas_SuppType" | "tBas_CustType" | "tBas_Area"
        | "tBas_Dept" | "tBas_Duty" | "tBas_Payment"
        | "tBas_CommTemplate" | "tBas_PriceTemplate"
        | "tSys_Menus" => Some("Used"),
        "tBas_Goods" | "tBas_Supp" | "tBas_Cust" | "tBas_Emp" 
        | "tPur_Order" | "tSal_Order" | "tSal_Inv" | "tStk_IO" | "tStk_Move"
        | "tStk_ReplenishApply" | "tStk_StockCycle"
        | "tArd_PD" | "tArd_AR" | "tAcc_PayOut" | "tAcc_PayIn"
        | "tSys_Rpt" | "tSys_Msg" | "tSys_DataPack"
        | "tSys_User" | "tSys_Rule" | "tSal_VIP" => Some("State"),
        "tSys_OperLog" | "tSys_OperHis" | "tSys_Dictionary" | "tStk_Qty" => None,
        _ => None,
    }
}

fn get_joins_for_table(table: &str) -> (String, String) {
    match table {
        "tBas_Emp" => (
            "t.*".to_string(),
            "LEFT JOIN [tBas_Dept] d ON t.[DeptID] = d.[DeptID] \
             LEFT JOIN [tBas_Duty] du ON t.[DutyID] = du.[DutyID] \
             LEFT JOIN [tBas_Stock] s ON t.[StkID] = s.[StkID]".to_string()
        ),
        "tBas_Supp" => (
            "t.*, st.[SuppTypeName], dt.[DeaTypeName], e.[EmpName]".to_string(),
            "LEFT JOIN [tBas_SuppType] st ON t.[SuppTypeID] = st.[SuppTypeID] \
             LEFT JOIN [tBas_DeaType] dt ON t.[DeaTypeID] = dt.[DeaTypeID] \
             LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID]".to_string()
        ),
        "tBas_Cust" => (
            "t.*, ct.[CustTypeName], a.[AreaName], e.[EmpName]".to_string(),
            "LEFT JOIN [tBas_CustType] ct ON t.[CustTypeID] = ct.[CustTypeID] \
             LEFT JOIN [tBas_Area] a ON t.[AreaID] = a.[AreaID] \
             LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID]".to_string()
        ),
        "tBas_Goods" => (
            "t.*, gt.[GDSTypeName], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote], gk.[GDSTypeName] AS GDSKindName, \
             dt.[DeaTypeName], s.[SuppName], u.[UnitName], sk.[StkName]".to_string(),
            "LEFT JOIN [tBas_GDSType] gt ON t.[GDSTypeID] = gt.[GDSTypeID] \
             LEFT JOIN [tBas_Brand] b ON t.[BrandID] = b.[BrandID] \
             LEFT JOIN [tBas_GDSType] gk ON t.[GDSKindID] = gk.[GDSTypeID] \
             LEFT JOIN [tBas_DeaType] dt ON t.[DeaTypeID] = dt.[DeaTypeID] \
             LEFT JOIN [tBas_Supp] s ON t.[SuppID] = s.[SuppID] \
             LEFT JOIN [tBas_Unit] u ON t.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Stock] sk ON t.[StkID] = sk.[StkID]".to_string()
        ),
        "tBas_Stock" => (
            "t.*, e.[EmpName] AS [SalEmpName]".to_string(),
            "LEFT JOIN [tBas_Emp] e ON t.[SalEmpID] = e.[EmpID]".to_string()
        ),
        "tPur_Order" => (
            "t.*, s.[SuppName], d.[DeptName], e.[EmpName], sk.[StkName]".to_string(),
            "LEFT JOIN [tBas_Supp] s ON t.[SuppID] = s.[SuppID] \
             LEFT JOIN [tBas_Dept] d ON t.[DeptID] = d.[DeptID] \
             LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] \
             LEFT JOIN [tBas_Stock] sk ON t.[StkID] = sk.[StkID]".to_string()
        ),
        "tStk_IO" => (
            "t.*, s.[SuppName], c.[CustName], d.[DeptName], e.[EmpName], sk.[StkName]".to_string(),
            "LEFT JOIN [tBas_Supp] s ON t.[SuppID] = s.[SuppID] \
             LEFT JOIN [tBas_Cust] c ON t.[CustID] = c.[CustID] \
             LEFT JOIN [tBas_Dept] d ON t.[DeptID] = d.[DeptID] \
             LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] \
             LEFT JOIN [tBas_Stock] sk ON t.[StkID] = sk.[StkID]".to_string()
        ),
        "tSal_Inv" => (
            "t.*, c.[CustName], d.[DeptName], e.[EmpName], sk.[StkName]".to_string(),
            "LEFT JOIN [tBas_Cust] c ON t.[CustID] = c.[CustID] \
             LEFT JOIN [tBas_Dept] d ON t.[DeptID] = d.[DeptID] \
             LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] \
             LEFT JOIN [tBas_Stock] sk ON t.[StkID] = sk.[StkID]".to_string()
        ),
        "tStk_Move" => (
            "t.*, fs.[StkName] AS [FromStkName], ts.[StkName] AS [ToStkName], e.[EmpName]".to_string(),
            "LEFT JOIN [tBas_Stock] fs ON t.[FromStkID] = fs.[StkID] \
             LEFT JOIN [tBas_Stock] ts ON t.[ToStkID] = ts.[StkID] \
             LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID]".to_string()
        ),
        "tStk_ReplenishApply" => (
            "t.*, sk.[StkName], e.[EmpName]".to_string(),
            "LEFT JOIN [tBas_Stock] sk ON t.[StkID] = sk.[StkID] \
             LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID]".to_string()
        ),
        "tStk_Qty" => (
            "t.*, sk.[StkName], g.[GDSDesc], g.[GDSSpec], g.[GDSNO], g.[BarCode], \
             g.[AInPrice], g.[BPrice], g.[SPrice], g.[UnitNO], g.[WarnQty], \
             gt.[GDSTypeName], b.[BrandName], u.[UnitName]".to_string(),
            "LEFT JOIN [tBas_Stock] sk ON t.[StkID] = sk.[StkID] \
             LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_GDSType] gt ON g.[GDSTypeID] = gt.[GDSTypeID] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        "tSal_EmpSales" => (
            "t.*, d.[DeptName], b.[BrandName]".to_string(),
            "LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] \
             LEFT JOIN [tBas_Dept] d ON e.[DeptID] = d.[DeptID] \
             LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        "tSys_Rpt" => (
            "t.*".to_string(),
            "".to_string()
        ),
        "tSys_RptPrintHis" => (
            "t.*".to_string(),
            "".to_string()
        ),
        "tSys_RptPrintNum" => (
            "t.*".to_string(),
            "".to_string()
        ),
        "tSys_Msg" => (
            "t.*, e.[EmpName] AS [ToUserName], fe.[EmpName] AS [FromUserName]".to_string(),
            "LEFT JOIN [tBas_Emp] e ON t.[TEmpID] = e.[EmpID] LEFT JOIN [tBas_Emp] fe ON t.[FEmpID] = fe.[EmpID]".to_string()
        ),
        "tSys_Parameters" => (
            "t.*".to_string(),
            "".to_string()
        ),
        "tBas_CustPriceTac" => (
            "t.*, c.[CustName], b.[BrandName]".to_string(),
            "LEFT JOIN [tBas_Cust] c ON t.[CustID] = c.[CustID] LEFT JOIN [tBas_Brand] b ON t.[BrandID] = b.[BrandID]".to_string()
        ),
        "tSys_DataPack" => (
            "t.*".to_string(),
            "".to_string()
        ),
        "tOnline_Goods" => (
            "t.*, g.[GDSDesc] AS [GoodsGDSDesc], g.[GDSNO] AS [GoodsGDSNO], g.[GDSSpec] AS [GoodsGDSSpec], g.[GDSBarCode] AS [GoodsBarCode], s.[StkName]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] LEFT JOIN [tBas_Stock] s ON t.[StkID] = s.[StkID]".to_string()
        ),
        "tOnline_Order" => (
            "t.*, e.[EmpName]".to_string(),
            "LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID]".to_string()
        ),
        "tOnline_OrderDetail" => (
            "t.*, g.[GDSDesc] AS [GoodsGDSDesc], g.[GDSNO] AS [GoodsGDSNO]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID]".to_string()
        ),
        "tOnline_Address" => (
            "t.*".to_string(),
            "".to_string()
        ),
        "tOnline_PaymentConfig" => (
            "t.*".to_string(),
            "".to_string()
        ),
        "tAcc_PayOut" => (
            "t.*, s.[SuppName], e.[EmpName], d.[DeptName], k.[StkName]".to_string(),
            "LEFT JOIN [tBas_Supp] s ON t.[SuppID] = s.[SuppID] LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] LEFT JOIN [tBas_Dept] d ON t.[DeptID] = d.[DeptID] LEFT JOIN [tBas_Stock] k ON t.[StkID] = k.[StkID]".to_string()
        ),
        "tAcc_PayIn" => (
            "t.*, c.[CustName], e.[EmpName], d.[DeptName], k.[StkName]".to_string(),
            "LEFT JOIN [tBas_Cust] c ON t.[CustID] = c.[CustID] LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] LEFT JOIN [tBas_Dept] d ON t.[DeptID] = d.[DeptID] LEFT JOIN [tBas_Stock] k ON t.[StkID] = k.[StkID]".to_string()
        ),
        "tArd_PD" => (
            "t.*, s.[SuppName], d.[DeptName]".to_string(),
            "LEFT JOIN [tBas_Supp] s ON t.[SuppID] = s.[SuppID] LEFT JOIN [tBas_Dept] d ON t.[DeptID] = d.[DeptID]".to_string()
        ),
        "tFin_Receivable" => (
            "t.*, c.[CustName], d.[DeptName]".to_string(),
            "LEFT JOIN [tBas_Cust] c ON t.[CustID] = c.[CustID] LEFT JOIN [tBas_Dept] d ON t.[DeptID] = d.[DeptID]".to_string()
        ),
        "tFin_CashFlow_DISABLED" => ( // tFin_CashFlow 表不存在
            "".to_string(),
            "".to_string()
        ),
        "tSys_RuleMenu" => (
            "t.*".to_string(),
            "".to_string()
        ),
        "tSys_UserRule" => (
            "t.*, e.[EmpName], r.[RuleName]".to_string(),
            "LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] LEFT JOIN [tSys_Rule] r ON t.[RuleID] = r.[RuleID]".to_string()
        ),
        "tSys_TableColumnConfig" => (
            "t.*".to_string(),
            "".to_string()
        ),
        "tSys_UploadFile" => (
            "t.*".to_string(),
            "".to_string()
        ),
        "tSal_Order" => (
            "t.*, c.[CustName], d.[DeptName], e.[EmpName], sk.[StkName]".to_string(),
            "LEFT JOIN [tBas_Cust] c ON t.[CustID] = c.[CustID] \
             LEFT JOIN [tBas_Dept] d ON t.[DeptID] = d.[DeptID] \
             LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] \
             LEFT JOIN [tBas_Stock] sk ON t.[StkID] = sk.[StkID]".to_string()
        ),
        "tStk_StockCycle" => (
            "t.*, sk.[StkName], e.[EmpName]".to_string(),
            "LEFT JOIN [tBas_Stock] sk ON t.[StkID] = sk.[StkID] \
             LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID]".to_string()
        ),
        "tSal_OrderDetail" => (
            "t.*, ISNULL(NULLIF(t.[GDSNO], ''), g.[GDSNO]) AS [GoodsGDSNO], ISNULL(NULLIF(t.[GDSDesc], ''), g.[GDSDesc]) AS [GoodsGDSDesc], u.[UnitName], g.[BrandID], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        "tPur_OrderDetail" => (
            "t.*, ISNULL(NULLIF(t.[GDSNO], ''), g.[GDSNO]) AS [GoodsGDSNO], ISNULL(NULLIF(t.[GDSDesc], ''), g.[GDSDesc]) AS [GoodsGDSDesc], u.[UnitName], g.[BrandID], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        "tSal_InvDetail" => (
            "t.*, ISNULL(NULLIF(t.[GDSNO], ''), g.[GDSNO]) AS [GoodsGDSNO], ISNULL(NULLIF(t.[GDSDesc], ''), g.[GDSDesc]) AS [GoodsGDSDesc], u.[UnitName], g.[BrandID], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        "tStk_IODetail" => (
            "t.*, ISNULL(NULLIF(t.[GDSNO], ''), g.[GDSNO]) AS [GoodsGDSNO], ISNULL(NULLIF(t.[GDSDesc], ''), g.[GDSDesc]) AS [GoodsGDSDesc], u.[UnitName], g.[BrandID], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote], io.[IONo], io.[Kind], io.[IoDate], io.[State] AS [IOState], io.[SuppID] AS [IOSuppID], io.[CustID] AS [IOCustID], io.[EmpID] AS [IOEmpID], io.[DeptID] AS [IODeptID], io.[StkID] AS [IOStkID], io.[Note] AS [IONote], s.[SuppName], c.[CustName], e.[EmpName], d.[DeptName], sk.[StkName]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID] \
             LEFT JOIN [tStk_IO] io ON t.[IOID] = io.[IOID] \
             LEFT JOIN [tBas_Supp] s ON io.[SuppID] = s.[SuppID] \
             LEFT JOIN [tBas_Cust] c ON io.[CustID] = c.[CustID] \
             LEFT JOIN [tBas_Emp] e ON io.[EmpID] = e.[EmpID] \
             LEFT JOIN [tBas_Dept] d ON io.[DeptID] = d.[DeptID] \
             LEFT JOIN [tBas_Stock] sk ON io.[StkID] = sk.[StkID]".to_string()
        ),
        "tStk_MoveDetail" => (
            "t.*, ISNULL(NULLIF(t.[GDSNO], ''), g.[GDSNO]) AS [GoodsGDSNO], ISNULL(NULLIF(t.[GDSDesc], ''), g.[GDSDesc]) AS [GoodsGDSDesc], u.[UnitName], g.[BrandID], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        "tStk_ReplenishApplyDetail" => (
            "t.*, ISNULL(NULLIF(t.[GDSNO], ''), g.[GDSNO]) AS [GoodsGDSNO], ISNULL(NULLIF(t.[GDSDesc], ''), g.[GDSDesc]) AS [GoodsGDSDesc], u.[UnitName], g.[BrandID], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        "tStk_ReplenishApplyDtl" => (
            "t.*, ISNULL(NULLIF(t.[GDSNO], ''), g.[GDSNO]) AS [GoodsGDSNO], ISNULL(NULLIF(t.[GDSDesc], ''), g.[GDSDesc]) AS [GoodsGDSDesc], u.[UnitName], g.[BrandID], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        "tStk_StockCycleDetail" => (
            "t.*, ISNULL(NULLIF(t.[GDSNO], ''), g.[GDSNO]) AS [GoodsGDSNO], ISNULL(NULLIF(t.[GDSDesc], ''), g.[GDSDesc]) AS [GoodsGDSDesc], u.[UnitName], g.[BrandID], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        "tStk_TranDetail" => (
            "t.*, ISNULL(NULLIF(t.[GDSNO], ''), g.[GDSNO]) AS [GoodsGDSNO], ISNULL(NULLIF(t.[GDSDesc], ''), g.[GDSDesc]) AS [GoodsGDSDesc], u.[UnitName], g.[BrandID], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        "tPur_QuoteDetail" => (
            "t.*, ISNULL(NULLIF(t.[GDSNO], ''), g.[GDSNO]) AS [GoodsGDSNO], ISNULL(NULLIF(t.[GDSDesc], ''), g.[GDSDesc]) AS [GoodsGDSDesc], u.[UnitName], g.[BrandID], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        "tPur_AdjPriceDetail" => (
            "t.*, ISNULL(NULLIF(t.[GDSNO], ''), g.[GDSNO]) AS [GoodsGDSNO], ISNULL(NULLIF(t.[GDSDesc], ''), g.[GDSDesc]) AS [GoodsGDSDesc], u.[UnitName], g.[BrandID], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        "tSal_QuoteDetail" => (
            "t.*, ISNULL(NULLIF(t.[GDSNO], ''), g.[GDSNO]) AS [GoodsGDSNO], ISNULL(NULLIF(t.[GDSDesc], ''), g.[GDSDesc]) AS [GoodsGDSDesc], u.[UnitName], g.[BrandID], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        "tSal_AdjPriceDetail" => (
            "t.*, ISNULL(NULLIF(t.[GDSNO], ''), g.[GDSNO]) AS [GoodsGDSNO], ISNULL(NULLIF(t.[GDSDesc], ''), g.[GDSDesc]) AS [GoodsGDSDesc], u.[UnitName], g.[BrandID], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        "tBas_EmpCommission" => (
            "t.*, e.[EmpName], sk.[StkName]".to_string(),
            "LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] \
             LEFT JOIN [tBas_Stock] sk ON t.[StkID] = sk.[StkID]".to_string()
        ),
        "tBas_CustPrice" => (
            "t.*, c.[CustName], g.[GDSDesc] AS [GoodsGDSDesc], g.[GDSNO] AS [GoodsGDSNO]".to_string(),
            "LEFT JOIN [tBas_Cust] c ON t.[CustID] = c.[CustID] \
             LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID]".to_string()
        ),
        "tSal_SaleTask" => (
            "t.*, e.[EmpName], sk.[StkName]".to_string(),
            "LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] \
             LEFT JOIN [tBas_Stock] sk ON t.[StkID] = sk.[StkID]".to_string()
        ),
        "tSys_User" => (
            "t.*, e.[EmpName], r.[RuleName], s.[StkName]".to_string(),
            "LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] \
             LEFT JOIN [tSys_Rule] r ON t.[RuleID] = r.[RuleID] \
             LEFT JOIN [tBas_Stock] s ON t.[StkID] = s.[StkID]".to_string()
        ),
        "tSys_Rule" => (
            "t.*".to_string(),
            "".to_string()
        ),
        "tBas_CommTemplate" => (
            "t.*, s.[StkName]".to_string(),
            "LEFT JOIN [tBas_Stock] s ON t.[StkID] = s.[StkID]".to_string()
        ),
        "tBas_PriceTemplate" => (
            "t.*, c.[CustName], b.[BrandName]".to_string(),
            "LEFT JOIN [tBas_Cust] c ON t.[CustID] = c.[CustID] \
             LEFT JOIN [tBas_Brand] b ON t.[BrandID] = b.[BrandID]".to_string()
        ),
        "tSal_VIP" => (
            "t.*, s.[StkName]".to_string(),
            "LEFT JOIN [tBas_Stock] s ON t.[StkID] = s.[StkID]".to_string()
        ),
        "tSys_OperLog" => (
            "t.*, e.[EmpName] AS [OperatorName]".to_string(),
            "LEFT JOIN [tBas_Emp] e ON t.[OperatorID] = e.[EmpID]".to_string()
        ),
        "tSys_OperHis" => (
            "t.*, e.[EmpName] AS [OperatorName]".to_string(),
            "LEFT JOIN [tBas_Emp] e ON t.[OperatorID] = e.[EmpID]".to_string()
        ),
        "tSys_Menus" => (
            "t.*".to_string(),
            "".to_string()
        ),
        "tSys_Dictionary_DISABLED" => ( // tSys_Dictionary 表不存在
            "t.*".to_string(),
            "".to_string()
        ),
        "tPur_Inv" => (
            "t.*, s.[SuppName], d.[DeptName], e.[EmpName], sk.[StkName]".to_string(),
            "LEFT JOIN [tBas_Supp] s ON t.[SuppID] = s.[SuppID] \
             LEFT JOIN [tBas_Dept] d ON t.[DeptID] = d.[DeptID] \
             LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] \
             LEFT JOIN [tBas_Stock] sk ON t.[StkID] = sk.[StkID]".to_string()
        ),
        "tPur_Quote" => (
            "t.*, s.[SuppName], e.[EmpName]".to_string(),
            "LEFT JOIN [tBas_Supp] s ON t.[SuppID] = s.[SuppID] \
             LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID]".to_string()
        ),
        "tPur_AdjPrice" => (
            "t.*, e.[EmpName]".to_string(),
            "LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID]".to_string()
        ),
        "tSal_Quote" => (
            "t.*, c.[CustName], e.[EmpName], d.[DeptName], sk.[StkName]".to_string(),
            "LEFT JOIN [tBas_Cust] c ON t.[CustID] = c.[CustID] \
             LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] \
             LEFT JOIN [tBas_Dept] d ON t.[DeptID] = d.[DeptID] \
             LEFT JOIN [tBas_Stock] sk ON t.[StkID] = sk.[StkID]".to_string()
        ),
        "tSal_AdjPrice" => (
            "t.*, e.[EmpName], d.[DeptName]".to_string(),
            "LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] \
             LEFT JOIN [tBas_Dept] d ON t.[DeptID] = d.[DeptID]".to_string()
        ),
        "tPur_QuoteDetail" => (
            "t.*, ISNULL(NULLIF(t.[GDSNO], ''), g.[GDSNO]) AS [GoodsGDSNO], ISNULL(NULLIF(t.[GDSDesc], ''), g.[GDSDesc]) AS [GoodsGDSDesc], u.[UnitName], g.[BrandID], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        "tPur_AdjPriceDetail" => (
            "t.*, ISNULL(NULLIF(t.[GDSNO], ''), g.[GDSNO]) AS [GoodsGDSNO], ISNULL(NULLIF(t.[GDSDesc], ''), g.[GDSDesc]) AS [GoodsGDSDesc], u.[UnitName], g.[BrandID], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        "tSal_AdjPriceDetail" => (
            "t.*, ISNULL(NULLIF(t.[GDSNO], ''), g.[GDSNO]) AS [GoodsGDSNO], ISNULL(NULLIF(t.[GDSDesc], ''), g.[GDSDesc]) AS [GoodsGDSDesc], u.[UnitName], g.[BrandID], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        "tSal_QuoteDetail" => (
            "t.*, ISNULL(NULLIF(t.[GDSNO], ''), g.[GDSNO]) AS [GoodsGDSNO], ISNULL(NULLIF(t.[GDSDesc], ''), g.[GDSDesc]) AS [GoodsGDSDesc], u.[UnitName], g.[BrandID], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        "tStk_ReplenishApplyDtl" => (
            "t.*, ISNULL(NULLIF(t.[GDSNO], ''), g.[GDSNO]) AS [GoodsGDSNO], ISNULL(NULLIF(t.[GDSDesc], ''), g.[GDSDesc]) AS [GoodsGDSDesc], u.[UnitName], g.[BrandID], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        "tStk_Tran" => (
            "t.*, sk.[StkName], e.[EmpName]".to_string(),
            "LEFT JOIN [tBas_Stock] sk ON t.[StkID] = sk.[StkID] \
             LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID]".to_string()
        ),
        "tStk_TranDetail" => (
            "t.*, ISNULL(NULLIF(t.[GDSNO], ''), g.[GDSNO]) AS [GoodsGDSNO], ISNULL(NULLIF(t.[GDSDesc], ''), g.[GDSDesc]) AS [GoodsGDSDesc], u.[UnitName], g.[BrandID], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        "tStk_Stock" => (
            "t.*, sk.[StkName], sk.[StkCode], g.[GDSNO], g.[GDSDesc], g.[GDSSpec], g.[BarCode], \
             g.[AInPrice], g.[BPrice], g.[SPrice], g.[UnitNO], g.[TopStkQty], g.[BttomStkQty], g.[GDSStateNO], g.[State], \
             g.[GDSTypeID], g.[BrandID], g.[SuppID], \
             gt.[GDSTypeName], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote], u.[UnitName], s.[SuppName]".to_string(),
            "LEFT JOIN [tBas_Stock] sk ON t.[StkID] = sk.[StkID] \
             LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_GDSType] gt ON g.[GDSTypeID] = gt.[GDSTypeID] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID] \
             LEFT JOIN [tBas_Supp] s ON g.[SuppID] = s.[SuppID]".to_string()
        ),
        "tOA_Notice" => (
            "t.*, e.[EmpName] AS [CreatorName]".to_string(),
            "LEFT JOIN [tBas_Emp] e ON t.[CreatorID] = e.[EmpID]".to_string()
        ),
        "tOA_Workflow" => (
            "t.*, ce.[EmpName] AS [CreatorName], ae.[EmpName] AS [ApproverName]".to_string(),
            "LEFT JOIN [tBas_Emp] ce ON t.[CreatorID] = ce.[EmpID] \
             LEFT JOIN [tBas_Emp] ae ON t.[ApproverID] = ae.[EmpID]".to_string()
        ),
        "tOA_Email" => (
            "t.*, e.[EmpName] AS [SenderName]".to_string(),
            "LEFT JOIN [tBas_Emp] e ON t.[SenderID] = e.[EmpID]".to_string()
        ),
        "tSys_Params" => (
            "t.*".to_string(),
            "".to_string()
        ),
        "tSys_ITReport" => (
            "t.*".to_string(),
            "".to_string()
        ),
        "tBas_Dict" => (
            "t.*".to_string(),
            "".to_string()
        ),
        "tRpt_Custom" => (
            "t.*".to_string(),
            "".to_string()
        ),
        "tSys_Permission" => (
            "t.*".to_string(),
            "".to_string()
        ),
        "tSys_RulePermission" => (
            "t.*, r.[RuleName], p.[PermName]".to_string(),
            "LEFT JOIN [tSys_Rule] r ON t.[RuleID] = r.[RuleID] \
             LEFT JOIN [tSys_Permission] p ON t.[PermissionID] = p.[PermissionID]".to_string()
        ),
        "tSys_RuleStock" => (
            "t.*, r.[RuleName], s.[StkName]".to_string(),
            "LEFT JOIN [tSys_Rule] r ON t.[RuleID] = r.[RuleID] \
             LEFT JOIN [tBas_Stock] s ON t.[StkID] = s.[StkID]".to_string()
        ),
        "vStk_IOFlow" => (
            "t.*".to_string(),
            "".to_string()
        ),
        _ => (
            "t.*".to_string(),
            "".to_string()
        ),
    }
}

struct BuiltQuery {
    sql: String,
    params: Vec<Option<String>>,
}

fn get_field_prefix_for_table<'a>(table: &str, field: &str) -> &'a str {
    match table {
        "tBas_Goods" => {
            match field {
                "GDSTypeName" | "GDSTypeID" => "gt",
                "GDSKindName" => "gk",
                "BrandName" | "BrandABC" | "BrandNote" | "BrandID" => "b",
                "DeaTypeName" | "DeaTypeID" => "dt",
                "SuppName" | "SuppID" => "s",
                "UnitName" | "UnitNO" => "u",
                "StkName" | "StkID" => "sk",
                "GDSPropertyName" | "GDSPropertyID" => "gp",
                _ => "t",
            }
        }
        "tStk_Qty" | "tStk_Stock" => {
            match field {
                "GDSNO" | "GDSDesc" | "GDSSpec" | "BarCode" | "GDSTypeID" | "BrandID" | "SuppID"
                | "UnitNO" | "AInPrice" | "BPrice" | "SPrice" | "VPrice" | "CPrice" | "WarnQty"
                | "GDSStateNO" | "State" => "g",
                "StkName" | "StkCode" => "sk",
                "GDSTypeName" => "gt",
                "BrandName" | "BrandABC" | "BrandNote" => "b",
                "UnitName" => "u",
                "SuppName" => "s",
                _ => "t",
            }
        }
        "tPur_OrderDetail" | "tSal_OrderDetail" | "tSal_InvDetail" | "tStk_IODetail"
        | "tStk_MoveDetail" | "tStk_ReplenishApplyDetail" | "tStk_ReplenishApplyDtl"
        | "tStk_StockCycleDetail" | "tStk_TranDetail" | "tPur_QuoteDetail"
        | "tPur_AdjPriceDetail" | "tSal_QuoteDetail" | "tSal_AdjPriceDetail" => {
            match field {
                "GDSNO" | "GDSDesc" | "GoodsGDSNO" | "GoodsGDSDesc" | "GDSSpec" | "BarCode"
                | "GDSTypeID" | "BrandID" | "SuppID" | "UnitNO" | "AInPrice" | "BPrice"
                | "SPrice" | "VPrice" | "CPrice" | "WarnQty" | "GDSStateNO" | "State" => "g",
                "UnitName" => "u",
                "BrandName" | "BrandABC" | "BrandNote" => "b",
                "IONo" | "Kind" | "IoDate" | "IOState" | "IOSuppID" | "IOCustID" | "IOEmpID"
                | "IODeptID" | "IOStkID" | "IONote" => "io",
                "SuppName" => "s",
                "CustName" => "c",
                "EmpName" => "e",
                "DeptName" => "d",
                "StkName" => "sk",
                _ => "t",
            }
        }
        _ => "t",
    }
}

/// Returns the list of field names that come from JOIN (not from the main table).
/// These fields should be excluded from INSERT/UPDATE to avoid overwriting
/// the main table's own redundant Name columns with wrong data.
fn get_join_fields_for_table(table: &str) -> Vec<&'static str> {
    match table {
        "tBas_Goods" => vec!["GDSTypeName", "GDSPropertyName", "BrandName", "BrandABC", "BrandNote", "GDSKindName", "DeaTypeName", "SuppName", "UnitName", "StkName"],
        "tBas_Supp" => vec!["SuppTypeName", "DeaTypeName", "EmpName"],
        "tBas_Cust" => vec!["CustTypeName", "AreaName", "EmpName"],
        "tBas_Emp" => vec!["DeptName", "DutyName", "StkName"],
        "tBas_Stock" => vec!["SalEmpName"],
        "tPur_Order" | "tPur_Inv" => vec!["SuppName", "DeptName", "EmpName", "StkName"],
        "tPur_Quote" => vec!["SuppName", "EmpName"],
        "tPur_AdjPrice" => vec!["EmpName"],
        "tSal_Order" | "tSal_Inv" => vec!["CustName", "DeptName", "EmpName", "StkName"],
        "tSal_Quote" => vec!["CustName", "EmpName", "DeptName", "StkName"],
        "tSal_AdjPrice" => vec!["EmpName", "DeptName"],
        "tStk_IO" => vec!["SuppName", "CustName", "DeptName", "EmpName", "StkName"],
        "tStk_Move" => vec!["FromStkName", "ToStkName", "EmpName"],
        "tStk_ReplenishApply" | "tStk_StockCycle" => vec!["StkName", "EmpName"],
        "tStk_Qty" => vec!["StkName", "GDSDesc", "GDSSpec", "GDSNO", "BarCode", "AInPrice", "BPrice", "SPrice", "UnitNO", "GDSTypeName", "BrandName", "BrandABC", "BrandNote", "UnitName", "WarnQty"],
        "tStk_Stock" => vec!["StkName", "StkCode", "GDSNO", "GDSDesc", "GDSSpec", "BarCode", "AInPrice", "BPrice", "SPrice", "UnitNO", "WarnQty", "GDSStateNO", "State", "GDSTypeID", "BrandID", "SuppID", "GDSTypeName", "BrandName", "BrandABC", "BrandNote", "UnitName", "SuppName"],
        "tAcc_PayOut" => vec!["SuppName", "EmpName", "DeptName", "StkName"],
        "tAcc_PayIn" => vec!["CustName", "EmpName", "DeptName", "StkName"],
        "tArd_PD" => vec!["SuppName", "DeptName"],
        "tArd_AR" => vec!["CustName", "DeptName"],
        // tFin_CashFlow 表不存在
        "tSys_User" => vec!["EmpName", "RuleName", "StkName"],
        "tSys_OperHis" => vec!["OperatorName"], // tSys_OperLog 不存在
        "tSys_Msg" => vec!["ToUserName", "FromUserName"],
        "tSys_UserRule" => vec!["EmpName", "RuleName"],
        "tSys_RulePermission" => vec!["RuleName", "PermName"],
        "tSys_RuleStock" => vec!["RuleName", "StkName"],
        "tBas_CustPriceTac" => vec!["CustName", "BrandName"],
        "tBas_PriceTemplate" => vec!["CustName", "BrandName"],
        "tBas_CommTemplate" => vec!["StkName"],
        "tBas_EmpCommission" | "tSal_SaleTask" => vec!["EmpName", "StkName"],
        "tBas_CustPrice" => vec!["CustName", "GDSDesc", "GDSNO"],
        "tSal_EmpSales" => vec!["DeptName", "BrandName"],
        "tSal_VIP" => vec!["StkName"],
        "tStk_Tran" => vec!["StkName", "EmpName"],
        "tOnline_Goods" => vec!["GDSDesc", "GDSNO", "GDSSpec", "GDSBarCode", "StkName"],
        "tOnline_Order" => vec!["EmpName"],
        "tOnline_OrderDetail" => vec!["GDSDesc", "GDSNO"],
        "tPur_OrderDetail" | "tSal_OrderDetail" | "tSal_InvDetail" | "tStk_IODetail" | "tStk_MoveDetail"
        | "tStk_ReplenishApplyDetail" | "tStk_StockCycleDetail" | "tPur_QuoteDetail"
        | "tPur_AdjPriceDetail" | "tSal_AdjPriceDetail" | "tSal_QuoteDetail" | "tStk_ReplenishApplyDtl"
        | "tStk_TranDetail" => vec!["GDSDesc", "GDSNO", "GDSSpec", "UnitName", "BrandID", "BrandName", "BrandABC", "BrandNote", "GoodsGDSNO", "GoodsGDSDesc", "IONo", "Kind", "IoDate", "IOState", "IOSuppID", "IOCustID", "IOEmpID", "IODeptID", "IOStkID", "IONote", "SuppName", "CustName", "EmpName", "DeptName", "StkName"],
        "tOA_Notice" => vec!["CreatorName"],
        "tOA_Workflow" => vec!["CreatorName", "ApproverName"],
        "tOA_Email" => vec!["SenderName"],
        _ => vec![],
    }
}

fn get_identity_columns_for_table(table: &str) -> Vec<&'static str> {
    match table {
        "tBas_Goods" => vec!["gdsSD"],
        "tBas_Supp" => vec!["suppSD"],
        "tBas_Cust" => vec!["custSD"],
        "tBas_Emp" => vec!["empSD"],
        "tBas_Stock" => vec!["stkSD"],
        _ => vec![],
    }
}

/// Returns the list of column names that cannot be set in INSERT/UPDATE
/// (IDENTITY columns and computed columns). The list is fetched live from
/// `sys.columns` so it works for any table — no hard-coding required.
async fn fetch_readonly_columns(
    conn: &mut bb8::PooledConnection<'static, bb8_tiberius::ConnectionManager>,
    table: &str,
) -> std::collections::HashSet<String> {
    let mut result: std::collections::HashSet<String> = std::collections::HashSet::new();
    // SQL Server identifier safety: brackets only — table comes from the
    // request body so we must guard against injection.
    if !table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return result;
    }
    let sql = format!(
        "SELECT name FROM sys.columns \
         WHERE object_id = OBJECT_ID('[{}]') \
           AND (is_identity = 1 OR is_computed = 1)",
        table
    );
    if let Ok(rows) = conn.query(&sql, &[]).await {
        match rows.into_first_result().await {
            Ok(vec) => {
                for row in vec {
                    let name: Option<&str> = row.try_get("name").ok().flatten();
                    if let Some(n) = name {
                        // 统一存为小写，避免 tiberius 返回的列名大小写与
                        // sys.columns.name 不一致时比较失败
                        result.insert(n.to_lowercase());
                    }
                }
            }
            Err(_) => {}
        }
    }
    result
}

/// 检查表是否存在指定列（不区分大小写）
async fn has_column(
    conn: &mut bb8::PooledConnection<'static, bb8_tiberius::ConnectionManager>,
    table: &str,
    column: &str,
) -> bool {
    if !table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return false;
    }
    let sql = format!(
        "SELECT 1 FROM sys.columns WHERE object_id = OBJECT_ID('[{}]') AND name = @p1",
        table
    );
    if let Ok(rows) = conn.query(&sql, &[&column]).await {
        if let Ok(vec) = rows.into_first_result().await {
            return !vec.is_empty();
        }
    }
    false
}

/// 根据员工编号（EmpNo）查找其 UUID（用于自动填充 EUser 等审计字段）
async fn lookup_user_uuid(
    conn: &mut bb8::PooledConnection<'static, bb8_tiberius::ConnectionManager>,
    emp_no: &str,
) -> Option<String> {
    if emp_no.is_empty() {
        return None;
    }
    let sql = "SELECT TOP 1 CAST(EmpID AS NVARCHAR(36)) AS EmpID FROM tBas_Emp WHERE EmpNo = @p1";
    if let Ok(rows) = conn.query(sql, &[&emp_no]).await {
        if let Ok(vec) = rows.into_first_result().await {
            if let Some(row) = vec.into_iter().next() {
                let id: Option<&str> = row.try_get("EmpID").ok().flatten();
                if let Some(s) = id {
                    let s = s.trim();
                    if !s.is_empty() {
                        return Some(s.to_string());
                    }
                }
            }
        }
    }
    None
}

fn build_base_query(
    table: &str,
    keyword: &Option<String>,
    keyword_fields: &Option<Vec<String>>,
    wheres: &Option<Vec<WhereCondition>>,
    include_deleted: bool,
    only_deleted: bool,
) -> BuiltQuery {
    let mut conditions = Vec::new();
    let mut params: Vec<Option<String>> = Vec::new();
    let mut param_idx = 1;

    if only_deleted {
        // 只显示已删除/已停用行
        if let Some(state_field) = get_state_field_for_table(table) {
            match state_field {
                "Used" => {
                    conditions.push("t.[Used] = 'N'".to_string());
                }
                _ => {
                    conditions.push("t.[State] = 'D'".to_string());
                }
            }
        }
    } else if !include_deleted {
        if let Some(state_field) = get_state_field_for_table(table) {
            match state_field {
                "Used" => {
                    conditions.push("t.[Used] <> 'N'".to_string());
                }
                _ => {
                    conditions.push("t.[State] <> 'D'".to_string());
                }
            }
        }
    }

    if let Some(kw) = keyword {
        if !kw.is_empty() {
            if let Some(fields) = keyword_fields {
                if !fields.is_empty() {
                    let kw_conditions: Vec<String> = fields.iter()
                        .map(|f| {
                            let pidx = param_idx;
                            param_idx += 1;
                            params.push(Some(format!("%{}%", kw)));
                            let prefix = get_field_prefix_for_table(table, f);
                            format!("CAST({}.[{}] AS varchar(max)) LIKE @p{}", prefix, f, pidx)
                        })
                        .collect();
                    conditions.push(format!("({})", kw_conditions.join(" OR ")));
                }
            }
        }
    }

    if let Some(wc_list) = wheres {
        for wc in wc_list {
            let op = match wc.op.as_str() {
                "eq" | "=" => "=",
                "ne" | "<>" | "!=" => "<>",
                "gt" | ">" => ">",
                "lt" | "<" => "<",
                "gte" | ">=" => ">=",
                "lte" | "<=" => "<=",
                "like" | "LIKE" => "LIKE",
                _ => "=",
            };
            let pidx = param_idx;
            param_idx += 1;

            if op == "LIKE" {
                if let serde_json::Value::String(s) = &wc.value {
                    params.push(Some(format!("%{}%", s)));
                } else {
                    params.push(json_to_sql_value(&wc.value));
                }
                let prefix = get_field_prefix_for_table(table, &wc.field);
                conditions.push(format!("{}.[{}] LIKE @p{}", prefix, wc.field, pidx));
            } else {
                params.push(json_to_sql_value(&wc.value));
                let prefix = get_field_prefix_for_table(table, &wc.field);
                conditions.push(format!("{}.[{}] {} @p{}", prefix, wc.field, op, pidx));
            }
        }
    }

    let (select_cols, join_clause) = get_joins_for_table(table);

    let sql = if conditions.is_empty() {
        if join_clause.is_empty() {
            format!("SELECT {} FROM [{}] t", select_cols, table)
        } else {
            format!("SELECT {} FROM [{}] t {}", select_cols, table, join_clause)
        }
    } else {
        if join_clause.is_empty() {
            format!("SELECT {} FROM [{}] t WHERE {}", select_cols, table, conditions.join(" AND "))
        } else {
            format!("SELECT {} FROM [{}] t {} WHERE {}", select_cols, table, join_clause, conditions.join(" AND "))
        }
    };

    BuiltQuery { sql, params }
}

pub async fn generic_query(
    State(_config): State<Config>,
    Json(params): Json<GenericQueryParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    if params.table.is_empty() {
        return Ok(Json(ApiResponse::err("表名不能为空")));
    }
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Ok(Json(ApiResponse::err(&format!("数据库连接失败: {}", e)))),
    };

    // Auto-create tStk_Qty table if it doesn't exist
    if params.table == "tStk_Qty" {
        let create_sql = "IF NOT EXISTS (SELECT * FROM sysobjects WHERE name='tStk_Qty' AND xtype='U') \
                          CREATE TABLE [tStk_Qty] ([QtyID] uniqueidentifier PRIMARY KEY DEFAULT NEWID(), \
                          [GDSID] uniqueidentifier NULL, [StkID] uniqueidentifier NULL, \
                          [Qty] decimal(18,4) DEFAULT 0, [LUTime] datetime DEFAULT GETDATE())";
        let _ = conn.execute(create_sql, &[]).await;
    }

    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(50), 5000);

    let built = build_base_query(&params.table, &params.keyword, &params.keyword_fields, &params.wheres, params.include_deleted.unwrap_or(false), params.only_deleted.unwrap_or(false));

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", built.sql);
    let param_refs: Vec<&dyn tiberius::ToSql> = built.params.iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();
    let mut total: i32 = 0;
    match conn.query(&count_sql, &param_refs).await {
        Ok(count_stream) => {
            match count_stream.into_row().await {
                Ok(Some(row)) => {
                    let v = try_get_value(&row, "cnt");
                    total = match v {
                        serde_json::Value::Number(n) => n.as_i64().unwrap_or(0) as i32,
                        serde_json::Value::String(s) => s.parse::<i32>().unwrap_or(0),
                        _ => 0,
                    };
                }
                Ok(None) => {}
                Err(e) => {
                    eprintln!("[generic_query] COUNT 失败: table={} err={} sql={}", params.table, e, count_sql);
                    let err_msg = format!("查询表 [{}] 失败（可能是表不存在或字段错误）: {}\nSQL: {}", params.table, e, count_sql);
                    return Ok(Json(ApiResponse::err(&err_msg)));
                }
            }
        }
        Err(e) => {
            eprintln!("[generic_query] COUNT 失败: table={} err={} sql={}", params.table, e, count_sql);
            let err_msg = format!("查询表 [{}] 失败: {} (请确认表是否存在)\nSQL: {}", params.table, e, count_sql);
            return Ok(Json(ApiResponse::err(&err_msg)));
        }
    }

    let paginated_sql = build_pagination_sql_with_sort(&built.sql, page, page_size, params.sort_prop.as_deref(), params.sort_order.as_deref());
    match conn.query(&paginated_sql, &param_refs).await {
        Ok(data_stream) => {
            match data_stream.into_first_result().await {
                Ok(rows) => {
                    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
                    Ok(Json(ApiResponse::ok_paginated(data, total as u64, page, page_size)))
                }
                Err(e) => {
                    eprintln!("[generic_query] 数据读取失败: table={} err={} sql={}", params.table, e, paginated_sql);
                    Ok(Json(ApiResponse::err(&format!("读取数据失败: {}", e))))
                }
            }
        }
        Err(e) => {
            eprintln!("[generic_query] 数据查询失败: table={} err={} sql={}", params.table, e, paginated_sql);
            Ok(Json(ApiResponse::err(&format!("执行查询失败: {} (表[{}]可能不存在)", e, params.table))))
        }
    }
}

pub async fn generic_export(
    State(_config): State<Config>,
    Json(params): Json<GenericQueryParams>,
) -> Json<ApiResponse<Vec<serde_json::Value>>> {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("数据库连接失败: {}", e))),
    };

    let built = build_base_query(&params.table, &params.keyword, &params.keyword_fields, &params.wheres, params.include_deleted.unwrap_or(false), params.only_deleted.unwrap_or(false));
    let export_sql = format!("SELECT TOP 10000 * FROM ({}) t", built.sql);
    let param_refs: Vec<&dyn tiberius::ToSql> = built.params.iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();

    let data_stream = match conn.query(&export_sql, &param_refs).await {
        Ok(s) => s,
        Err(e) => return Json(ApiResponse::err(&format!("导出查询失败 [{}]: {}", params.table, e))),
    };
    let rows = match data_stream.into_first_result().await {
        Ok(r) => r,
        Err(e) => return Json(ApiResponse::err(&format!("导出读取数据失败: {}", e))),
    };
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Json(ApiResponse::ok(data))
}

pub async fn generic_delete(
    State(_config): State<Config>,
    Json(params): Json<GenericDeleteParams>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("数据库连接失败: {}", e))),
    };

    if params.ids.is_empty() {
        return Json(ApiResponse::err("请选择要操作的记录"));
    }

    // physicalDelete = true：执行物理删除（DELETE FROM）
    // physicalDelete = false：执行软删除（UPDATE State='D' / Used='N'）
    let physical_delete = params.permanent.unwrap_or(false);

    if physical_delete {
        tracing::info!("[彻底删除 v2] permanent=true ids={:?} table={}", params.ids, params.table);
        // 物理删除前：引用检查。若商品已被其它单据/库存引用，强制阻止物理删除，
        // 避免破坏外键完整性。返回被引用的表名 + 引用条数，便于用户清理。
        match check_references_blocking(&mut conn, &params.table, &params.ids, true).await {
            Ok(hits) => {
                if !hits.is_empty() {
                    return Json(ApiResponse::err(&format!(
                        "该商品已被以下数据引用，无法彻底删除：\n{}\n请先清理引用数据（删除/作废相关单据和库存）后再试。",
                        hits.join("\n")
                    )));
                }
            }
            Err(e) => return Json(ApiResponse::err(&format!("引用检查失败: {}", e))),
        }

        // 引用检查通过，执行物理删除
        for id in &params.ids {
            let sql = format!(
                "DELETE FROM [{}] WHERE [{}] = @p1",
                params.table, params.primary_key
            );
            let id_str = id.as_str();
            if let Err(e) = conn.execute(&sql, &[&id_str]).await {
                return Json(ApiResponse::err(&format!("彻底删除失败 [{}]: {}（可能存在 SQL Server 外键约束）", params.table, e)));
            }
        }
        return Json(ApiResponse::msg(&format!("成功彻底删除 {} 条记录", params.ids.len())));
    }

    // 软删除：更新状态字段
    let state_field = params.state_field.as_deref().unwrap_or("State");
    // void=true 时 State 置 'C'（业务作废，保留可查）；否则按字段类型取默认（Used='N' 停用，State='D' 软删）
    let void_flag = params.void.unwrap_or(false);
    let delete_value = if state_field == "Used" {
        "N"
    } else if void_flag {
        "C"
    } else {
        "D"
    };

    // 软删时检查业务引用但不阻止（有库存的商品可以停用，不影响库存数据）
    // 引用信息作为警告附加到成功消息中，便于用户感知
    let mut ref_warnings: Vec<String> = Vec::new();
    if let Ok(hits) = check_references_blocking(&mut conn, &params.table, &params.ids, false).await {
        if !hits.is_empty() {
            let label = if state_field == "Used" { "停用" } else { "作废" };
            ref_warnings.push(format!("该记录已被以下数据引用（{}不影响现有数据）：\n{}", label, hits.join("\n")));
        }
    }

    for id in &params.ids {
        let sql = format!(
            "UPDATE [{}] SET [{}] = @p1 WHERE [{}] = @p2",
            params.table, state_field, params.primary_key
        );
        let id_str = id.as_str();
        if let Err(e) = conn.execute(&sql, &[&delete_value, &id_str]).await {
            return Json(ApiResponse::err(&format!("删除失败 [{}]: {}", params.table, e)));
        }
    }

    let label = if state_field == "Used" { "停用" } else { "作废" };
    let mut msg = format!("成功{}{}条记录", label, params.ids.len());
    if !ref_warnings.is_empty() {
        msg.push_str("\n\n");
        msg.push_str(&ref_warnings.join("\n\n"));
    }
    Json(ApiResponse::msg(&msg))
}

/// 恢复软删除的记录：Used='Y' / State='N'
#[derive(Deserialize)]
pub struct GenericRestoreParams {
    pub table: String,
    pub primary_key: String,
    pub ids: Vec<String>,
    pub state_field: Option<String>,
}

pub async fn generic_restore(
    State(_config): State<Config>,
    Json(params): Json<GenericRestoreParams>,
) -> Json<ApiResponse<serde_json::Value>> {
    if params.table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') == false {
        return Json(ApiResponse::err("表名非法"));
    }
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("数据库连接失败: {}", e))),
    };

    // 自动选状态字段：先看客户端传入，再按表查找，最后兜底
    let sf: String = if let Some(s) = params.state_field.as_deref() {
        if !s.is_empty() { s.to_string() } else {
            match get_state_field_for_table(&params.table) { Some(x) => x.to_string(), None => return Json(ApiResponse::err(&format!("表 [{}] 没有软删字段，无法恢复", params.table))) }
        }
    } else {
        match get_state_field_for_table(&params.table) { Some(x) => x.to_string(), None => return Json(ApiResponse::err(&format!("表 [{}] 没有软删字段，无法恢复", params.table))) }
    };
    let restore_value = if sf == "Used" { "Y" } else { "N" };

    let mut ok = 0usize;
    for id in &params.ids {
        let sql = format!(
            "UPDATE [{}] SET [{}] = @p1 WHERE [{}] = @p2",
            params.table, sf, params.primary_key
        );
        let id_str = id.as_str();
        match conn.execute(&sql, &[&restore_value, &id_str]).await {
            Ok(r) => ok += r.rows_affected().len(),
            Err(e) => return Json(ApiResponse::err(&format!("恢复失败 [{}]: {}", params.table, e))),
        }
    }
    let label = if sf == "Used" { "启用" } else { "反作废" };
    Json(ApiResponse::msg(&format!("成功{}{}条记录", label, ok)))
}

/// 软删除/物理删除前的业务引用检查
/// 返回被引用的表名+条数（人类可读的中文描述）
/// `strict=true` 物理删模式：所有引用都阻止（避免破坏外键完整性）
/// `strict=false` 软删模式：默认不阻止，返回引用清单作为参考信息（不阻塞操作）
///                       物理引用（如 tStk_Stock 有真实库存）只在 strict=true 时阻止
async fn check_references_blocking(
    conn: &mut bb8::PooledConnection<'static, bb8_tiberius::ConnectionManager>,
    table: &str,
    ids: &[String],
    strict: bool,
) -> std::result::Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    let references = get_references_for_table(table);
    if references.is_empty() {
        return Ok(vec![]);
    }

    let mut hits: Vec<String> = Vec::new();
    for (ref_table, ref_col, ref_label) in &references {
        let in_list = ids
            .iter()
            .map(|s| format!("'{}'", s.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(",");
        let total_sql = format!(
            "SELECT COUNT(*) AS cnt FROM [{}] WHERE [{}] IN ({})",
            ref_table, ref_col, in_list
        );
        match conn.query(&total_sql, &[]).await {
            Ok(mut stream) => {
                if let Ok(Some(row)) = stream.into_row().await {
                    let v = try_get_value(&row, "cnt");
                    let cnt = match v {
                        serde_json::Value::Number(n) => n.as_i64().unwrap_or(0),
                        serde_json::Value::String(s) => s.parse::<i64>().unwrap_or(0),
                        _ => 0,
                    };
                    if cnt > 0 {
                        hits.push(format!("  · {} ({}): {} 条", ref_label, ref_table, cnt));
                    }
                }
            }
            Err(e) => {
                tracing::warn!("引用检查跳过 [{}].[{}]: {}", ref_table, ref_col, e);
            }
        }
    }
    Ok(hits)
}

/// 维护「哪些表通过哪个字段引用了哪张主表」的关系。
/// 物理删除前会逐一查询这些引用表，统计引用条数，
/// 若有任何引用就阻止物理删除（避免破坏外键完整性）。
/// 格式：(引用表, 引用字段, 友好名称)
fn get_references_for_table(table: &str) -> Vec<(&'static str, &'static str, &'static str)> {
    match table {
        "tBas_Goods" => vec![
            ("tStk_Stock", "GDSID", "商品库存余额"),
            ("tStk_Qty", "GDSID", "商品即时库存"),
            ("tStk_Reserve", "GDSID", "库存预留"),
            ("tStk_IODetail", "GDSID", "出入库明细"),
            ("tStk_MoveDetail", "GDSID", "调拨明细"),
            ("tStk_ReplenishApply", "GDSID", "补货申请"),
            ("tStk_ReplenishApplyDtl", "GDSID", "补货申请明细"),
            ("tSal_InvDetail", "GDSID", "销售发票明细"),
            ("tPur_OrderDetail", "GDSID", "采购订单明细"),
            ("tOnline_Goods", "GDSID", "线上商城商品"),
            ("tOnline_OrderDetail", "GDSID", "线上订单明细"),
        ],
        "tBas_Supp" => vec![
            ("tBas_Goods", "SuppID", "商品资料"),
            ("tPur_Order", "SuppID", "采购订单"),
            ("tPur_Quote", "SuppID", "采购报价"),
            ("tPur_AdjPrice", "SuppID", "采购调价"),
            ("tArd_PD", "SuppID", "应付款"),
            ("tAcc_PayOut", "SuppID", "付款单"),
            ("tStk_IO", "SuppID", "出入库单"),
        ],
        "tBas_Cust" => vec![
            ("tSal_Inv", "CustID", "销售发票"),
            ("tSal_InvDetail", "CustID", "销售发票明细"),
            ("tSal_Order", "CustID", "销售订单"),
            ("tSal_Quote", "CustID", "销售报价"),
            ("tFin_Receivable", "CustID", "应收款"),
            ("tFin_Receipt", "CustID", "收款单"),
            ("tOnline_Order", "CustID", "线上订单"),
            ("tBas_Goods", "CustID", "商品资料"),
        ],
        "tBas_Stock" => vec![
            ("tStk_Stock", "StkID", "商品库存余额"),
            ("tStk_Qty", "StkID", "商品即时库存"),
            ("tStk_IO", "StkID", "出入库单"),
            ("tStk_Move", "FromStkID", "调拨单(发出)"),
            ("tStk_Move", "ToStkID", "调拨单(接收)"),
            ("tBas_Emp", "StkID", "员工"),
            ("tBas_Goods", "StkID", "商品资料"),
        ],
        "tBas_Brand" => vec![
            ("tBas_Goods", "BrandID", "商品资料"),
        ],
        "tBas_Unit" => vec![
            ("tBas_Goods", "UnitNO", "商品资料"),
        ],
        "tBas_Emp" => vec![
            ("tStk_IO", "EmpID", "出入库单"),
            ("tStk_Move", "EmpID", "调拨单"),
            ("tPur_Order", "EmpID", "采购订单"),
            ("tSal_Order", "EmpID", "销售订单"),
            ("tSal_Inv", "EmpID", "销售发票"),
        ],
        "tBas_GDSType" => vec![
            ("tBas_Goods", "GDSTypeID", "商品资料"),
        ],
        "tBas_GDSProperty" => vec![
            ("tBas_Goods", "GDSPropertyID", "商品资料"),
        ],
        "tBas_GDSKind" => vec![
            ("tBas_Goods", "GDSKindID", "商品资料"),
        ],
        "tBas_DeaType" => vec![],
        "tBas_SuppType" => vec![
            ("tBas_Supp", "SuppTypeID", "供应商资料"),
        ],
        "tBas_CustType" => vec![
            ("tBas_Cust", "CustTypeID", "客户资料"),
        ],
        "tBas_Area" => vec![
            ("tBas_Cust", "AreaID", "客户资料"),
            ("tBas_Supp", "AreaID", "供应商资料"),
        ],
        "tBas_Dept" => vec![
            ("tBas_Emp", "DeptID", "员工资料"),
        ],
        "tBas_Duty" => vec![
            ("tBas_Emp", "DutyID", "员工资料"),
        ],
        "tBas_Payment" => vec![
            ("tBas_Cust", "PLID", "客户资料"),
        ],
        _ => vec![],
    }
}

/// 树形查询参数
#[derive(Deserialize)]
pub struct GenericTreeParams {
    pub table: String,
    pub primary_key: Option<String>,
    pub parent_field: Option<String>,
    pub name_field: Option<String>,
    pub count_table: Option<String>,
    pub count_field: Option<String>,
    pub extra_fields: Option<String>,
    pub state_field: Option<String>,
}

/// 树形查询：将扁平的父子关系数据组装成树形结构返回
pub async fn generic_tree(
    State(_config): State<Config>,
    Json(params): Json<GenericTreeParams>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("数据库连接失败: {}", e))),
    };

    let pk = params.primary_key.as_deref().unwrap_or("ID");
    let pf = params.parent_field.as_deref().unwrap_or("ParentID");
    let nf = params.name_field.as_deref().unwrap_or("Name");
    let sf = params.state_field.as_deref().unwrap_or("State");
    let extra = params.extra_fields.as_deref().unwrap_or("");

    let mut select_parts = vec![
        format!("[{}]", pk),
        format!("[{}]", pf),
        format!("[{}]", nf),
        format!("[{}]", sf),
    ];
    if !extra.is_empty() {
        for f in extra.split(',') {
            let f = f.trim();
            if !f.is_empty() && !select_parts.iter().any(|s| s == &format!("[{}]", f)) {
                select_parts.push(format!("[{}]", f));
            }
        }
    }

    // 根据状态字段类型决定过滤条件
    // Used 字段: Y=启用, N=停用 → 过滤 <> 'N'
    // State 字段: S=复审, D=删除, Y=已审核, N=新建 → 过滤 <> 'D'
    let state_filter = if sf == "Used" {
        "<> 'N'".to_string()
    } else {
        "<> 'D'".to_string()
    };

    let sql = format!(
        "SELECT {} FROM [{}] WHERE [{}] {} ORDER BY [{}]",
        select_parts.join(", "),
        params.table,
        sf,
        state_filter,
        pk
    );

    let flat_rows = match conn.query(&sql, &[]).await {
        Ok(s) => match s.into_first_result().await {
            Ok(rows) => rows,
            Err(e) => return Json(ApiResponse::err(&format!("读取数据失败: {}", e))),
        },
        Err(e) => return Json(ApiResponse::err(&format!("查询失败: {}", e))),
    };

    let mut flat: Vec<serde_json::Map<String, serde_json::Value>> = Vec::new();
    for row in &flat_rows {
        flat.push(row_to_json_map(row));
    }

    // 统计关联数量
    let mut count_map: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    if let (Some(ct), Some(cf)) = (&params.count_table, &params.count_field) {
        let count_sql = format!(
            "SELECT [{}], COUNT(*) AS cnt FROM [{}] WHERE [{}] IS NOT NULL GROUP BY [{}]",
            cf, ct, cf, cf
        );
        if let Ok(count_stream) = conn.query(&count_sql, &[]).await {
            if let Ok(count_rows) = count_stream.into_first_result().await {
                for row in &count_rows {
                    // 使用 try_get_value 安全提取，避免类型不匹配时 panic
                    let key = value_to_string(&try_get_value(row, cf));
                    if key.is_empty() || key == "null" {
                        continue;
                    }
                    let cnt = value_to_string(&try_get_value(row, "cnt"))
                        .parse::<i64>()
                        .unwrap_or(0);
                    count_map.insert(key, cnt);
                }
            }
        }
    }

    // 建立节点索引
    let pk_lower = pk.to_lowercase();
    let pf_lower = pf.to_lowercase();

    let mut node_map: std::collections::HashMap<String, serde_json::Value> = std::collections::HashMap::new();
    for item in &flat {
        if let Some(id_val) = item.get(&pk_lower) {
            let id_str = value_to_string(id_val);
            let mut node = serde_json::Map::new();
            for (k, v) in item {
                node.insert(k.clone(), v.clone());
            }
            let count = count_map.get(&id_str).copied().unwrap_or(0);
            node.insert("product_count".to_string(), serde_json::Value::Number(count.into()));
            node.insert("children".to_string(), serde_json::Value::Array(vec![]));
            node_map.insert(id_str, serde_json::Value::Object(node));
        }
    }

    // 组装父子关系
    let mut root_ids: Vec<String> = Vec::new();
    let mut children_map: std::collections::HashMap<String, Vec<serde_json::Value>> = std::collections::HashMap::new();

    for item in &flat {
        let id_str = item.get(&pk_lower)
            .map(|v| value_to_string(v))
            .unwrap_or_default();

        let parent_str = item.get(&pf_lower)
            .map(|v| value_to_string(v))
            .unwrap_or_default();

        if !parent_str.is_empty() && parent_str != "null" {
            children_map.entry(parent_str).or_default().push(
                node_map.get(&id_str).cloned().unwrap_or(serde_json::Value::Object(serde_json::Map::new()))
            );
        } else {
            root_ids.push(id_str);
        }
    }

    // 递归构建树
    fn build_tree(
        id: &str,
        node_map: &std::collections::HashMap<String, serde_json::Value>,
        children_map: &std::collections::HashMap<String, Vec<serde_json::Value>>,
        pk_lower: &str,
    ) -> serde_json::Value {
        let mut node = match node_map.get(id) {
            Some(n) => n.clone(),
            None => return serde_json::Value::Null,
        };

        if let Some(children) = children_map.get(id) {
            let child_trees: Vec<serde_json::Value> = children.iter()
                .filter_map(|c| {
                    let cid = c.as_object()
                        .and_then(|obj| obj.get(pk_lower))
                        .map(|v| value_to_string(v))
                        .unwrap_or_default();
                    if cid.is_empty() { return None; }
                    Some(build_tree(&cid, node_map, children_map, pk_lower))
                })
                .filter(|v| !v.is_null())
                .collect();

            if let Some(obj) = node.as_object_mut() {
                let self_count = obj.get("product_count").and_then(|v| v.as_i64()).unwrap_or(0);
                let children_count: i64 = child_trees.iter()
                    .filter_map(|c| c.get("product_count").and_then(|v| v.as_i64()))
                    .sum();
                obj.insert("product_count".to_string(), serde_json::Value::Number((self_count + children_count).into()));

                if !child_trees.is_empty() {
                    obj.insert("children".to_string(), serde_json::Value::Array(child_trees));
                } else {
                    obj.remove("children");
                }
            }
        }

        node
    }

    let tree: Vec<serde_json::Value> = root_ids.iter()
        .filter_map(|id| {
            let v = build_tree(id, &node_map, &children_map, &pk_lower);
            if v.is_null() { None } else { Some(v) }
        })
        .collect();

    Json(ApiResponse::ok(serde_json::Value::Array(tree)))
}

/// 辅助：serde_json::Value -> String（用于 ID 比较）
fn value_to_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        _ => v.to_string().trim_matches('"').to_string(),
    }
}

/// 辅助：Row -> serde_json::Map
fn row_to_json_map(row: &Row) -> serde_json::Map<String, serde_json::Value> {
    let columns = row.columns();
    let mut map = serde_json::Map::new();
    for col in columns {
        let name = col.name().to_string();
        if name == "_rn" { continue; }
        let val = try_get_value(row, &name);
        map.insert(name.to_lowercase(), val);
    }
    map
}

pub async fn generic_create(
    State(_config): State<Config>,
    Extension(claims): Extension<Claims>,
    Json(params): Json<GenericCreateParams>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("数据库连接失败: {}", e))),
    };

    if params.data.is_empty() {
        return Json(ApiResponse::err("没有提供新增数据"));
    }

    let join_fields = get_join_fields_for_table(&params.table);
    let pk_col = get_primary_key_for_table(&params.table);
    let readonly_fields = fetch_readonly_columns(&mut conn, &params.table).await;
    let empty_str_cols = default_empty_string_cols();
    // ★ 自动补充审计字段：EUser（按当前登录用户 EmpNo 查 UUID）/ EDate（当前时间）
    // 仅当表存在这些列且客户端未传值时才填充。业务字段一律不补。
    let user_uuid = lookup_user_uuid(&mut conn, &claims.user_code).await;
    let now_str = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let mut columns = Vec::new();
    let mut placeholders = Vec::new();
    let mut values: Vec<Option<String>> = Vec::new();

    // If the PK is omitted by the client (or sent as an empty string),
    // auto-generate a fresh UUID so SQL Server does not have to convert ''.
    let mut generated_pk: Option<String> = None;
    if let Some(pk) = pk_col {
        let needs_generate = match params.data.get(pk) {
            None => true,
            Some(serde_json::Value::String(s)) if s.trim().is_empty() => true,
            Some(serde_json::Value::Null) => true,
            _ => false,
        };
        if needs_generate {
            let new_id = uuid::Uuid::new_v4().to_string();
            generated_pk = Some(new_id.clone());
            columns.push(format!("[{}]", pk));
            placeholders.push(format!("@p{}", columns.len()));
            values.push(Some(new_id));
        }
    }

    for (key, val) in params.data.iter() {
        let key_lc = key.to_lowercase();
        // Skip fields that come from JOIN (not own columns)
        if join_fields.contains(&key.as_str()) { continue; }
        // Skip IDENTITY / computed columns — SQL Server rejects explicit inserts
        // (would need IDENTITY_INSERT ON). The DB will auto-fill them.
        if readonly_fields.contains(&key_lc) { continue; }
        // If PK was already auto-generated above, ignore whatever the client sent.
        if let Some(pk) = pk_col {
            if key_lc == pk.to_lowercase() && generated_pk.is_some() { continue; }
        }
        columns.push(format!("[{}]", key));
        placeholders.push(format!("@p{}", columns.len()));
        let mut v = json_to_sql_value(val);
        // NOT NULL 文本列：空值用 '' 写入而不是 NULL
        if v.is_none() && empty_str_cols.contains(key) {
            v = Some(String::new());
        }
        values.push(v);
    }

    // ★ 审计字段自动填充：仅当表存在 EDate / EUser 列，且客户端未提供时追加
    let provided_keys: std::collections::HashSet<String> = params.data.keys()
        .map(|k| k.to_lowercase())
        .collect();
    let mut pushed_audit = false;
    if !provided_keys.contains("edate") && has_column(&mut conn, &params.table, "EDate").await {
        columns.push("[EDate]".to_string());
        placeholders.push(format!("@p{}", columns.len()));
        values.push(Some(now_str.clone()));
        pushed_audit = true;
    }
    if !provided_keys.contains("euser") && has_column(&mut conn, &params.table, "EUser").await {
        if let Some(ref uid) = user_uuid {
            columns.push("[EUser]".to_string());
            placeholders.push(format!("@p{}", columns.len()));
            values.push(Some(uid.clone()));
            pushed_audit = true;
        }
    }
    let _ = pushed_audit;

    if columns.is_empty() {
        return Json(ApiResponse::err("没有可插入的字段"));
    }

    let sql = format!(
        "INSERT INTO [{}] ({}) VALUES ({})",
        params.table,
        columns.join(", "),
        placeholders.join(", ")
    );

    let param_refs: Vec<&dyn tiberius::ToSql> = values.iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();

    match conn.execute(&sql, &param_refs).await {
        Ok(_) => {
            // Echo back the (possibly generated) primary key so the frontend
            // can use it for follow-up detail inserts / navigation.
            if let Some(pk) = pk_col {
                let id_value = generated_pk.clone()
                    .or_else(|| {
                        params.data.get(pk)
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.trim().is_empty())
                            .map(|s| s.to_string())
                    });
                if let Some(id) = id_value {
                    return Json(ApiResponse::ok(serde_json::json!({
                        pk: id,
                        "id": id,
                    })));
                }
            }
            Json(ApiResponse::msg("新增成功"))
        }
        Err(e) => Json(ApiResponse::err(&format!("新增数据到表 [{}] 失败: {} (请确认表和字段是否存在)\nSQL: {}", params.table, e, sql)))
    }
}

pub async fn generic_update(
    State(_config): State<Config>,
    Json(params): Json<GenericUpdateParams>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("数据库连接失败: {}", e))),
    };

    if params.data.is_empty() {
        return Json(ApiResponse::err("没有提供更新数据"));
    }

    let join_fields = get_join_fields_for_table(&params.table);
    let readonly_fields = fetch_readonly_columns(&mut conn, &params.table).await;
    let mut set_clauses = Vec::new();
    let mut values: Vec<Option<String>> = Vec::new();

    for (key, val) in params.data.iter() {
        let key_lc = key.to_lowercase();
        if key_lc == params.primary_key.to_lowercase() { continue; }
        // Skip fields that come from JOIN (not own columns) to avoid overwriting redundant Name columns
        if join_fields.contains(&key.as_str()) { continue; }
        // Skip identity / computed columns — SQL Server rejects updates to them
        if readonly_fields.contains(&key_lc) { continue; }
        // 防御性跳过 null/空值：避免 NOT NULL 列被误设为 NULL（例如表单联动字段未及时回填）
        // 如果业务确实需要将某列置为 NULL，请走专用接口
        if val.is_null() {
            continue;
        }
        if let serde_json::Value::String(s) = val {
            if s.trim().is_empty() {
                continue;
            }
        }
        set_clauses.push(format!("[{}] = @p{}", key, set_clauses.len() + 1));
        values.push(json_to_sql_value(val));
    }

    if set_clauses.is_empty() {
        return Json(ApiResponse::err("没有提供需要更新的字段"));
    }

    let pk_param_idx = values.len() + 1;
    let sql = format!(
        "UPDATE [{}] SET {} WHERE [{}] = @p{}",
        params.table,
        set_clauses.join(", "),
        params.primary_key,
        pk_param_idx
    );

    values.push(Some(params.id));

    let param_refs: Vec<&dyn tiberius::ToSql> = values.iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();

    match conn.execute(&sql, &param_refs).await {
        Ok(_) => Json(ApiResponse::msg("更新成功")),
        Err(e) => Json(ApiResponse::err(&format!("更新表 [{}] 数据失败: {}", params.table, e)))
    }
}

#[derive(Deserialize)]
pub struct GenericImportParams {
    pub table: String,
    pub data: Vec<serde_json::Map<String, serde_json::Value>>,
}

pub async fn generic_import(
    State(_config): State<Config>,
    Json(params): Json<GenericImportParams>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("数据库连接失败: {}", e))),
    };

    if params.data.is_empty() {
        return Json(ApiResponse::err("没有提供导入数据"));
    }

    let join_fields = get_join_fields_for_table(&params.table);
    let readonly_fields = fetch_readonly_columns(&mut conn, &params.table).await;
    let mut success_count = 0u32;
    let mut error_msgs: Vec<String> = Vec::new();
    for row in &params.data {
        let mut columns = Vec::new();
        let mut placeholders = Vec::new();
        let mut values: Vec<Option<String>> = Vec::new();

        for (key, val) in row.iter() {
            // Skip fields that come from JOIN (not own columns)
            if join_fields.contains(&key.as_str()) { continue; }
            // Skip IDENTITY / computed columns
            if readonly_fields.contains(key) { continue; }
            columns.push(format!("[{}]", key));
            placeholders.push(format!("@p{}", columns.len()));
            values.push(json_to_sql_value(val));
        }

        if columns.is_empty() {
            error_msgs.push("无可插入字段".to_string());
            continue;
        }

        let sql = format!(
            "INSERT INTO [{}] ({}) VALUES ({})",
            params.table,
            columns.join(", "),
            placeholders.join(", ")
        );

        let param_refs: Vec<&dyn tiberius::ToSql> = values.iter()
            .map(|v| v as &dyn tiberius::ToSql)
            .collect();

        match conn.execute(&sql, &param_refs).await {
            Ok(_) => success_count += 1,
            Err(e) => {
                error_msgs.push(format!("{:?}", e));
            }
        }
    }

    if !error_msgs.is_empty() {
        return Json(ApiResponse::ok(serde_json::json!({
            "imported": success_count,
            "failed": error_msgs.len(),
            "errors": error_msgs,
        })));
    }
    Json(ApiResponse::msg(&format!("成功导入{}条记录", success_count)))
}

#[derive(Deserialize)]
pub struct BatchUpdateParams {
    pub table: String,
    pub primary_key: String,
    pub ids: Vec<String>,
    pub updates: serde_json::Map<String, serde_json::Value>,
}

pub async fn generic_batch_update(
    State(_config): State<Config>,
    Json(params): Json<BatchUpdateParams>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("数据库连接失败: {}", e))),
    };

    if params.ids.is_empty() {
        return Json(ApiResponse::err("请选择要更新的记录"));
    }
    if params.updates.is_empty() {
        return Json(ApiResponse::err("没有提供更新数据"));
    }

    let join_fields = get_join_fields_for_table(&params.table);
    let readonly_fields = fetch_readonly_columns(&mut conn, &params.table).await;
    let mut set_clauses = Vec::new();
    let mut values: Vec<Option<String>> = Vec::new();

    for (key, val) in params.updates.iter() {
        if key == &params.primary_key {
            continue;
        }
        // Skip fields that come from JOIN (not own columns)
        if join_fields.contains(&key.as_str()) { continue; }
        // Skip identity / computed columns
        if readonly_fields.contains(key) { continue; }
        set_clauses.push(format!("[{}] = @p{}", key, set_clauses.len() + 1));
        values.push(json_to_sql_value(val));
    }

    if set_clauses.is_empty() {
        return Json(ApiResponse::err("没有提供需要更新的字段"));
    }

    let mut updated_count = 0u32;
    for id in &params.ids {
        let pk_param_idx = values.len() + 1;
        let sql = format!(
            "UPDATE [{}] SET {} WHERE [{}] = @p{}",
            params.table,
            set_clauses.join(", "),
            params.primary_key,
            pk_param_idx
        );

        let mut all_values = values.clone();
        all_values.push(Some(id.clone()));

        let param_refs: Vec<&dyn tiberius::ToSql> = all_values.iter()
            .map(|v| v as &dyn tiberius::ToSql)
            .collect();

        match conn.execute(&sql, &param_refs).await {
            Ok(_) => updated_count += 1,
            Err(e) => {
                tracing::warn!("批量更新行失败: {:?}", e);
            }
        }
    }

    Json(ApiResponse::ok(serde_json::json!({ "updated_count": updated_count })))
}

#[derive(Deserialize)]
pub struct ImportTemplateParams {
    pub table: String,
}

pub async fn generic_import_template(
    State(_config): State<Config>,
    Json(params): Json<ImportTemplateParams>,
) -> Response {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => {
            let body = serde_json::json!({"success":false,"message":&format!("数据库连接失败: {}", e)}).to_string();
            return axum::response::Response::builder()
                .status(500)
                .header("Content-Type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap();
        }
    };

    let sql = format!(
        "SELECT COLUMN_NAME, DATA_TYPE FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_NAME = @p1 ORDER BY ORDINAL_POSITION"
    );
    let table_name = params.table.trim_start_matches('[').trim_start_matches(']');
    let stream = match conn.query(&sql, &[&table_name]).await {
        Ok(s) => s,
        Err(e) => {
            let body = serde_json::json!({"success":false,"message":&format!("查询表结构失败: {}", e)}).to_string();
            return axum::response::Response::builder()
                .status(500)
                .header("Content-Type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap();
        }
    };
    let rows = match stream.into_first_result().await {
        Ok(r) => r,
        Err(e) => {
            let body = serde_json::json!({"success":false,"message":&format!("读取列信息失败: {}", e)}).to_string();
            return axum::response::Response::builder()
                .status(500)
                .header("Content-Type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap();
        }
    };

    let mut headers = Vec::new();
    for row in &rows {
        let v = try_get_value(row, "COLUMN_NAME");
        if let serde_json::Value::String(s) = v {
            if !s.is_empty() {
                headers.push(s);
            }
        }
    }

    let csv = format!("\u{FEFF}{}\n", headers.join(","));

    axum::response::Response::builder()
        .status(200)
        .header("Content-Type", "text/csv; charset=utf-8")
        .header("Content-Disposition", format!("attachment; filename={}_template.csv", params.table))
        .body(axum::body::Body::from(csv))
        .unwrap()
}

pub async fn generic_import_excel(
    State(_config): State<Config>,
    mut multipart: Multipart,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut table_name = String::new();
    let mut file_data: Vec<u8> = Vec::new();

    loop {
        let field = match multipart.next_field().await {
            Ok(f) => f,
            Err(e) => return Json(ApiResponse::err(&format!("读取上传文件失败: {}", e))),
        };
        let Some(field) = field else { break; };
        let name = field.name().unwrap_or("").to_string();
        if name == "table" {
            table_name = field.text().await.unwrap_or_default();
        } else if name == "file" {
            let bytes = match field.bytes().await {
                Ok(b) => b,
                Err(e) => return Json(ApiResponse::err(&format!("读取文件内容失败: {}", e))),
            };
            file_data = bytes.to_vec();
        }
    }

    if table_name.is_empty() || file_data.is_empty() {
        return Json(ApiResponse::err("缺少表名或文件"));
    }

    let text = match String::from_utf8(file_data.clone()) {
        Ok(s) => s,
        Err(_) => return Json(ApiResponse::err("文件编码不支持，请使用UTF-8编码的CSV文件")),
    };

    let lines: Vec<&str> = text.split('\n').filter(|l| !l.trim().is_empty()).collect();
    if lines.len() < 2 {
        return Json(ApiResponse::err("文件内容为空"));
    }

    let header_line = lines[0].trim().trim_start_matches('\u{FEFF}');
    let headers: Vec<&str> = parse_csv_line(header_line);

    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("数据库连接失败: {}", e))),
    };

    let join_fields = get_join_fields_for_table(&table_name);
    let readonly_fields = fetch_readonly_columns(&mut conn, &table_name).await;
    let mut success_count = 0u32;
    let mut errors: Vec<String> = Vec::new();

    for (row_idx, line) in lines.iter().skip(1).enumerate() {
        let values = parse_csv_line(line.trim());
        let mut row_data = serde_json::Map::new();
        for (i, header) in headers.iter().enumerate() {
            let val = values.get(i).unwrap_or(&"");
            if !val.is_empty() {
                row_data.insert(header.to_string(), serde_json::Value::String(val.to_string()));
            }
        }

        if row_data.is_empty() {
            continue;
        }

        let mut columns = Vec::new();
        let mut placeholders = Vec::new();
        let mut sql_values: Vec<Option<String>> = Vec::new();

        for (key, val) in row_data.iter() {
            // Skip fields that come from JOIN (not own columns)
            if join_fields.contains(&key.as_str()) { continue; }
            // Skip IDENTITY / computed columns
            if readonly_fields.contains(key) { continue; }
            columns.push(format!("[{}]", key));
            placeholders.push(format!("@p{}", columns.len()));
            sql_values.push(json_to_sql_value(val));
        }

        if columns.is_empty() {
            errors.push(format!("第{}行: 无可插入字段", row_idx + 2));
            continue;
        }

        let sql = format!(
            "INSERT INTO [{}] ({}) VALUES ({})",
            table_name,
            columns.join(", "),
            placeholders.join(", ")
        );

        let param_refs: Vec<&dyn tiberius::ToSql> = sql_values.iter()
            .map(|v| v as &dyn tiberius::ToSql)
            .collect();

        match conn.execute(&sql, &param_refs).await {
            Ok(_) => success_count += 1,
            Err(e) => {
                errors.push(format!("第{}行: {}", row_idx + 2, e));
            }
        }
    }

    Json(ApiResponse::ok(serde_json::json!({
        "success_count": success_count,
        "error_count": errors.len(),
        "errors": errors
    })))
}

fn parse_csv_line(line: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;

    for (i, ch) in line.char_indices() {
        if ch == '"' {
            in_quotes = !in_quotes;
        } else if ch == ',' && !in_quotes {
            result.push(line[start..i].trim().trim_matches('"'));
            start = i + 1;
        }
    }
    result.push(line[start..].trim().trim_matches('"'));
    result
}

#[derive(Deserialize)]
pub struct ExportExcelParams {
    pub table: String,
    pub keyword: Option<String>,
    pub keyword_fields: Option<Vec<String>>,
    pub wheres: Option<Vec<WhereCondition>>,
    pub include_deleted: Option<bool>,
    pub only_deleted: Option<bool>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

pub async fn generic_export_excel(
    State(_config): State<Config>,
    Json(params): Json<ExportExcelParams>,
) -> Response {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => {
            let body = serde_json::json!({"success":false,"message":&format!("数据库连接失败: {}", e)}).to_string();
            return axum::response::Response::builder()
                .status(500)
                .header("Content-Type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap();
        }
    };

    let built = build_base_query(&params.table, &params.keyword, &params.keyword_fields, &params.wheres, params.include_deleted.unwrap_or(false), params.only_deleted.unwrap_or(false));
    let export_sql = format!("SELECT TOP 50000 * FROM ({}) t", built.sql);
    let param_refs: Vec<&dyn tiberius::ToSql> = built.params.iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();

    let data_stream = match conn.query(&export_sql, &param_refs).await {
        Ok(s) => s,
        Err(e) => {
            let body = serde_json::json!({"success":false,"message":&format!("导出查询失败: {}", e)}).to_string();
            return axum::response::Response::builder()
                .status(500)
                .header("Content-Type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap();
        }
    };
    let rows = match data_stream.into_first_result().await {
        Ok(r) => r,
        Err(e) => {
            let body = serde_json::json!({"success":false,"message":&format!("读取数据失败: {}", e)}).to_string();
            return axum::response::Response::builder()
                .status(500)
                .header("Content-Type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap();
        }
    };

    if rows.is_empty() {
        let csv = "\u{FEFF}\n".to_string();
        return axum::response::Response::builder()
            .status(200)
            .header("Content-Type", "text/csv; charset=utf-8")
            .header("Content-Disposition", format!("attachment; filename={}_export.csv", params.table))
            .body(axum::body::Body::from(csv))
            .unwrap();
    }

    let columns = rows[0].columns();
    let headers: Vec<String> = columns.iter()
        .filter(|c| c.name() != "_rn")
        .map(|c| c.name().to_string())
        .collect();

    let mut csv = format!("\u{FEFF}{}\n", headers.join(","));

    for row in &rows {
        let vals: Vec<String> = headers.iter().map(|h| {
            let v = try_get_value(row, h);
            match v {
                serde_json::Value::String(s) => {
                    if s.contains(',') || s.contains('"') || s.contains('\n') {
                        format!("\"{}\"", s.replace('"', "\"\""))
                    } else {
                        s
                    }
                }
                serde_json::Value::Null => String::new(),
                other => other.to_string(),
            }
        }).collect();
        csv.push_str(&format!("{}\n", vals.join(",")));
    }

    let resp = axum::response::Response::builder()
        .status(200)
        .header("Content-Type", "text/csv; charset=utf-8")
        .header("Content-Disposition", format!("attachment; filename={}_export.csv", params.table))
        .body(axum::body::Body::from(csv))
        .unwrap();
    resp
}

#[derive(Deserialize)]
pub struct OperLogParams {
    pub module: Option<String>,
    pub record_id: Option<String>,
    pub operation_type: Option<String>,
    pub page: Option<u32>,
    pub page_size: Option<u32>,
}

pub async fn generic_oper_log(
    State(_config): State<Config>,
    Json(params): Json<OperLogParams>,
) -> Json<ApiResponse<Vec<serde_json::Value>>> {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("数据库连接失败: {}", e))),
    };

    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);

    let mut conditions = Vec::new();
    let mut sql_params: Vec<Option<String>> = Vec::new();
    let mut param_idx = 1;

    if let Some(module) = &params.module {
        conditions.push(format!("[Module] = @p{}", param_idx));
        sql_params.push(Some(module.clone()));
        param_idx += 1;
    }

    if let Some(record_id) = &params.record_id {
        conditions.push(format!("[RecordID] = @p{}", param_idx));
        sql_params.push(Some(record_id.clone()));
        param_idx += 1;
    }

    if let Some(op_type) = &params.operation_type {
        conditions.push(format!("[OperationType] = @p{}", param_idx));
        sql_params.push(Some(op_type.clone()));
        param_idx += 1;
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conditions.join(" AND "))
    };

    let count_sql = format!("SELECT COUNT(*) as cnt FROM [tSys_OperHis]{}", where_clause);
    let param_refs: Vec<&dyn tiberius::ToSql> = sql_params.iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();

    let mut total: i32 = 0;
    let count_stream = match conn.query(&count_sql, &param_refs).await {
        Ok(s) => s,
        Err(e) => return Json(ApiResponse::err(&format!("查询操作日志失败: {}", e))),
    };
    match count_stream.into_row().await {
        Ok(Some(row)) => {
            let v = try_get_value(&row, "cnt");
            total = match v {
                serde_json::Value::Number(n) => n.as_i64().unwrap_or(0) as i32,
                serde_json::Value::String(s) => s.parse::<i32>().unwrap_or(0),
                _ => 0,
            };
        }
        Ok(None) => {}
        Err(e) => return Json(ApiResponse::err(&format!("读取日志数量失败: {}", e))),
    }

    let offset = (page - 1) * page_size;
    let data_sql = format!(
        "SELECT * FROM [tSys_OperHis]{} ORDER BY [OperDate] DESC OFFSET {} ROWS FETCH NEXT {} ROWS ONLY",
        where_clause, offset, page_size
    );

    let data_stream = match conn.query(&data_sql, &param_refs).await {
        Ok(s) => s,
        Err(e) => return Json(ApiResponse::err(&format!("查询操作日志数据失败: {}", e))),
    };
    let rows = match data_stream.into_first_result().await {
        Ok(r) => r,
        Err(e) => return Json(ApiResponse::err(&format!("读取日志数据失败: {}", e))),
    };
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Json(ApiResponse::ok_paginated(data, total as u64, page, page_size))
}
