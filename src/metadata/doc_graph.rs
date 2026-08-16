//! 业务单据表关联元数据
//!
//! 单一事实源：所有业务单据的主表/明细表/上下游单据/Kind 方向/必填字段/状态机
//! 都通过 `DOC_GRAPH` 静态结构描述。前端通过 `/api/doc/graph` 拉取同样数据。
//!
//! 修改后务必同步更新 `client/src/config/docGraph.js` 与 `DOC_GRAPH_VERSION`。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 元数据版本号，与前端 `docGraph.version` 对账
pub const DOC_GRAPH_VERSION: &str = "2026.07.14.001";

/// 库存方向常量
pub const DIR_INBOUND: f64 = 1.0;   // 入库
pub const DIR_OUTBOUND: f64 = -1.0; // 出库
pub const DIR_TRANSFER: f64 = 0.0;   // 调拨（双边）

/// 状态机映射条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateEntry {
    pub code: String,
    pub text: String,
}

/// 业务单据主表元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocMeta {
    pub table: String,
    pub primary_key: String,
    pub no_field: String,
    pub date_field: String,
    pub kind_field: String,
    pub detail_table: String,
    pub detail_primary_key: String,
    pub detail_foreign_key: String,
    pub warehouse_field: String,
    pub source_field: String,
    pub upstream: Vec<String>,
    pub downstream: Vec<String>,
    pub title: String,
    pub doc_no_prefix: String,
    pub biz_type: String,
    pub required_fields: Vec<String>,
    pub state_map: Vec<StateEntry>,
    pub soft_delete_field: String,
    pub soft_delete_value: String,
    pub affects_stock: bool,
    pub detail_requires_gds: bool,
    pub detail_unique_gds: bool,
}

const DEFAULT_STATE_MAP: &[(&str, &str)] = &[
    ("D", "删除"),
    ("E", "编辑中"),
    ("N", "新建"),
    ("S", "已审核"),
    ("Y", "已确认"),
    ("C", "已作废"),
];

fn sm() -> Vec<StateEntry> {
    DEFAULT_STATE_MAP.iter().map(|(k, v)| StateEntry { code: k.to_string(), text: v.to_string() }).collect()
}

fn upstream(v: &[&str]) -> Vec<String> { v.iter().map(|x| x.to_string()).collect() }
fn downstream(v: &[&str]) -> Vec<String> { v.iter().map(|x| x.to_string()).collect() }
fn req(v: &[&str]) -> Vec<String> { v.iter().map(|x| x.to_string()).collect() }

/// 构造函数：把字面量参数打包成 DocMeta
#[allow(clippy::too_many_arguments)]
pub fn build_doc(
    table: &str, pk: &str, no: &str, date: &str, kind: &str,
    dt: &str, dpk: &str, dfk: &str, wh: &str, src: &str,
    up: &[&str], down: &[&str], title: &str, prefix: &str, biz: &str,
    required: &[&str], affects_stock: bool, drg: bool, dug: bool,
) -> DocMeta {
    DocMeta {
        table: table.to_string(),
        primary_key: pk.to_string(),
        no_field: no.to_string(),
        date_field: date.to_string(),
        kind_field: kind.to_string(),
        detail_table: dt.to_string(),
        detail_primary_key: dpk.to_string(),
        detail_foreign_key: dfk.to_string(),
        warehouse_field: wh.to_string(),
        source_field: src.to_string(),
        upstream: upstream(up),
        downstream: downstream(down),
        title: title.to_string(),
        doc_no_prefix: prefix.to_string(),
        biz_type: biz.to_string(),
        required_fields: req(required),
        state_map: sm(),
        soft_delete_field: "State".to_string(),
        soft_delete_value: "D".to_string(),
        affects_stock,
        detail_requires_gds: drg,
        detail_unique_gds: dug,
    }
}

/// 全量单据元数据（运行时初始化，避免 const fn 限制）
pub fn all_docs() -> Vec<DocMeta> {
    vec![
        build_doc("tPur_Order", "POID", "PoNo", "PoDate", "",
                  "tPur_OrderDetail", "PODetailID", "POID", "StkID", "",
                  // 下游：tPur_Inv(采购入库)、tStk_IO:PR(采购退货)；TH 已迁移到 tStk_Move 不再是 PO 下游
                  &[], &["tPur_Inv", "tStk_IO:PR"],
                  "采购订单", "PO", "purchase",
                  &["POID", "PoNo", "PoDate", "StkID", "SuppID"], false, true, false),
        build_doc("tPur_Inv", "PIID", "PiNo", "RecvDate", "",
                  "tPur_InvDetail", "PIDetailID", "PIID", "StkID", "POID",
                  // 下游：tStk_IO:PR(采购退货)；TH 已迁移到 tStk_Move 不再是 PI 下游
                  &["tPur_Order"], &["tStk_IO:PR"],
                  "采购入库", "PI", "purchase",
                  &["PIID", "PiNo", "RecvDate", "StkID", "SuppID"], true, true, false),
        build_doc("tPur_Return", "PRID", "PrNo", "RetDate", "",
                  "tPur_ReturnDetail", "PRDetailID", "PRID", "StkID", "FromRID",
                  &["tPur_Inv", "tPur_Order"], &[],
                  "采购退货", "PR", "purchase",
                  &["PRID", "PrNo", "RetDate", "StkID", "SuppID"], true, true, false),
        build_doc("tSal_Order", "SOID", "SoNo", "SoDate", "",
                  "tSal_OrderDetail", "SODetailID", "SOID", "StkID", "",
                  &[], &["tSal_Inv", "tStk_IO:SR"],
                  "销售订单", "SO", "sales",
                  // affects_stock=true：销售订单审核会写 QQty 预占和 tStk_Reserve，需要事务保护和期间检查
                  &["SOID", "SoNo", "SoDate", "StkID", "CustID"], true, true, false),
        build_doc("tSal_Inv", "SIID", "SINo", "SIDate", "",
                  "tSal_InvDetail", "SIDetailID", "SIID", "StkID", "SOID",
                  &["tSal_Order"], &["tStk_IO:SR"],
                  "销售出库", "SI", "sales",
                  &["SIID", "SINo", "SIDate", "StkID", "CustID"], true, true, false),
        build_doc("tStk_IO", "IOID", "IONo", "IoDate", "Kind",
                  "tStk_IODetail", "IODetailID", "IOID", "StkID", "POID",
                  &["tPur_Order", "tPur_Inv", "tSal_Order", "tSal_Inv"], &[],
                  "入出库单", "IO", "stock",
                  &["IOID", "IONo", "IoDate", "Kind", "StkID"], true, true, false),
        build_doc("tStk_Move", "MoveID", "MoveNO", "MoveDate", "Kind",
                  "tStk_MoveDetail", "MoveDetailID", "MoveID", "FromStkID,ToStkID", "",
                  &[], &[],
                  "调拨单", "MV", "stock",
                  &["MoveID", "MoveNO", "MoveDate", "Kind"], true, true, false),
        build_doc("tStk_Tran", "TranID", "TranNo", "TranDate", "BTPID",
                  "tStk_TranDetail", "TranDetailID", "TranID", "StkID", "",
                  &[], &[],
                  "盘点单", "TR", "stock",
                  &["TranID", "TranNo", "TranDate"], true, true, true),
        build_doc("tStk_ReplenishApply", "ReplenishApplyID", "ReplenishApplyNo", "ReplenishApplyDate", "Kind",
                  "tStk_ReplenishApplyDtl", "ReplenishApplyDtlID", "ReplenishApplyID", "StkID", "",
                  &[], &["tStk_IO:PD"],
                  "补货申请", "RPA", "stock",
                  &["ReplenishApplyID", "ReplenishApplyDate", "EndDate"], false, true, false),
        build_doc("tFin_Receipt", "RecID", "RecNO", "RecDate", "",
                  "tFin_ReceiptDtl", "ReceiptDtlID", "RecID", "", "SourceDocID",
                  &["tStk_IO"], &[],
                  "收款单", "RCV", "finance",
                  &["RecID", "RecNO", "RecDate", "CustID"], false, false, false),
        build_doc("tFin_Payment", "PayID", "PayNO", "PayDate", "",
                  "tFin_PaymentDtl", "PaymentDtlID", "PayID", "", "SourceDocID",
                  &["tStk_IO"], &[],
                  "付款单", "PAY", "finance",
                  &["PayID", "PayNO", "PayDate", "SuppID"], false, false, false),
        // 销售报价
        build_doc("tSal_Quote", "SQID", "SQNo", "SQDate", "",
                  "tSal_QuoteDetail", "SQDetailID", "SQID", "StkID", "",
                  &[], &["tSal_Order"],
                  "销售报价", "SRQ", "sales",
                  &["SQID", "SQNo", "SQDate", "CustID"], false, true, false),
        // 采购报价
        build_doc("tPur_Quote", "PQID", "PqNo", "PqDate", "",
                  "tPur_QuoteDetail", "PQDetailID", "PQID", "", "",
                  &[], &["tPur_Order"],
                  "采购报价", "PRQ", "purchase",
                  &["PQID", "PqNo", "PqDate", "SuppID"], false, true, false),
        // 采购调价
        build_doc("tPur_AdjPrice", "PAPID", "PAPNo", "PAPDate", "",
                  "tPur_AdjPriceDetail", "PAPDetailID", "PAPID", "", "",
                  &[], &[],
                  "采购调价", "PAP", "purchase",
                  &["PAPID", "PAPNo", "PAPDate"], false, true, false),
        // 周期盘点
        build_doc("tStk_StockCycle", "CycleID", "CycleNo", "CycleDate", "",
                  "tStk_StockCycleDetail", "CycleDetailID", "CycleID", "StkID", "",
                  &[], &[],
                  "周期盘点", "CYC", "stock",
                  &["CycleID", "CycleNo", "CycleDate", "StkID"], true, true, false),
        // 员工销量录入（扁平表，无明细）
        build_doc("tSal_EmpSales", "ID", "", "SaleDate", "",
                  "", "", "", "", "",
                  &[], &[],
                  "员工销量录入", "", "sales",
                  &["ID", "EmpID", "GDSID", "Qty", "SaleDate"], false, false, false),
        // 现金流量（扁平表，无明细）
        build_doc("tFin_CashFlow", "CFID", "CFNO", "CFDate", "",
                  "", "", "", "", "",
                  &[], &[],
                  "现金流量", "CF", "finance",
                  &["CFID", "CFNO", "CFDate", "CFType", "CFAmt"], false, false, false),
    ]
}

pub fn get_doc_meta(table: &str) -> Option<DocMeta> {
    all_docs().into_iter().find(|m| m.table == table)
}

pub fn has_kind_field(table: &str) -> bool {
    get_doc_meta(table).map(|m| !m.kind_field.is_empty()).unwrap_or(false)
}

/// Kind → 库存方向映射（仅对 tStk_IO 与 tStk_Move 有效）
/// 与前端 `client/src/config/docGraph.js` 的 KIND_DIRECTION 严格一致
pub fn kind_direction(kind: &str) -> f64 {
    match kind {
        "PD" | "SR" | "OTI" | "DBI" => DIR_INBOUND,
        "SD" | "SI" | "POS" | "PR" | "OTO" | "RI" | "ADJ" | "O" | "REQ" | "DBO" => DIR_OUTBOUND,
        "DB" | "ZP" | "TH" | "OT" => DIR_TRANSFER,
        _ => 0.0,
    }
}

pub fn diff_qty_direction(diff_qty: f64) -> f64 {
    if diff_qty > 0.0 {
        DIR_INBOUND
    } else if diff_qty < 0.0 {
        DIR_OUTBOUND
    } else {
        0.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocGraphResponse {
    pub version: String,
    pub docs: Vec<DocGraphNode>,
    pub edges: Vec<DocGraphEdge>,
    pub kind_map: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocGraphNode {
    pub table: String,
    pub title: String,
    pub biz_type: String,
    pub doc_no_prefix: String,
    pub affects_stock: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocGraphEdge {
    pub from: String,
    pub to: String,
    pub kind: Option<String>,
}

pub fn build_graph_response() -> DocGraphResponse {
    let graph = all_docs();
    let docs: Vec<DocGraphNode> = graph.iter()
        .map(|m| DocGraphNode {
            table: m.table.clone(),
            title: m.title.clone(),
            biz_type: m.biz_type.clone(),
            doc_no_prefix: m.doc_no_prefix.clone(),
            affects_stock: m.affects_stock,
        })
        .collect();

    let mut edges: Vec<DocGraphEdge> = Vec::new();
    for m in graph.iter() {
        for up in &m.upstream {
            edges.push(DocGraphEdge {
                from: up.clone(),
                to: m.table.clone(),
                kind: None,
            });
        }
        for down in &m.downstream {
            edges.push(DocGraphEdge {
                from: m.table.clone(),
                to: down.clone(),
                kind: None,
            });
        }
    }

    let mut kind_map: HashMap<String, String> = HashMap::new();
    kind_map.insert("PD".into(), "采购入库 (+)".into());
    kind_map.insert("SR".into(), "销售退货 (+)".into());
    kind_map.insert("OTI".into(), "零散入库 (+)".into());
    kind_map.insert("DBI".into(), "调拨入库 (+)".into());
    kind_map.insert("SD".into(), "销售出库 (-)".into());
    kind_map.insert("SI".into(), "门店销售 (-)".into());
    kind_map.insert("POS".into(), "POS收银 (-)".into());
    kind_map.insert("PR".into(), "采购退货 (-)".into());
    kind_map.insert("OTO".into(), "零散出库 (-)".into());
    kind_map.insert("RI".into(), "领用单 (-)".into());
    kind_map.insert("ADJ".into(), "库存调整 (-)".into());
    kind_map.insert("DB".into(), "内部调拨 (双边)".into());
    kind_map.insert("ZP".into(), "门店直配 (双边)".into());
    kind_map.insert("TH".into(), "门店退货 (双边)".into());
    kind_map.insert("OT".into(), "零散出入库 (按符号)".into());

    DocGraphResponse {
        version: DOC_GRAPH_VERSION.to_string(),
        docs,
        edges,
        kind_map,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kind_direction_inbound() {
        assert_eq!(kind_direction("PD"), 1.0);
        assert_eq!(kind_direction("SR"), 1.0);
        assert_eq!(kind_direction("OTI"), 1.0);
    }

    #[test]
    fn test_kind_direction_outbound() {
        assert_eq!(kind_direction("SD"), -1.0);
        assert_eq!(kind_direction("POS"), -1.0);
        assert_eq!(kind_direction("PR"), -1.0);
        assert_eq!(kind_direction("OTO"), -1.0);
        assert_eq!(kind_direction("RI"), -1.0);
        assert_eq!(kind_direction("O"), -1.0);
        assert_eq!(kind_direction("REQ"), -1.0);
        assert_eq!(kind_direction("DBO"), -1.0);
    }

    #[test]
    fn test_kind_direction_transfer() {
        assert_eq!(kind_direction("DB"), 0.0);
        assert_eq!(kind_direction("ZP"), 0.0);
        assert_eq!(kind_direction("TH"), 0.0);
    }

    #[test]
    fn test_diff_qty_direction() {
        assert_eq!(diff_qty_direction(5.0), 1.0);
        assert_eq!(diff_qty_direction(-3.0), -1.0);
        assert_eq!(diff_qty_direction(0.0), 0.0);
    }

    #[test]
    fn test_get_doc_meta() {
        assert!(get_doc_meta("tPur_Order").is_some());
        assert!(get_doc_meta("tStk_IO").is_some());
        assert!(get_doc_meta("tNonExistent").is_none());
    }

    #[test]
    fn test_all_docs_have_required() {
        for m in all_docs() {
            // 核心字段：所有单据都必须有
            assert!(!m.table.is_empty(), "table name required");
            assert!(!m.primary_key.is_empty(), "primary_key required for {}", m.table);
            // 扁平表（如 tSal_EmpSales、tFin_CashFlow）无单据号、无明细，
            // no_field/detail_table 等允许为空，仅当存在明细表时才校验明细字段一致性
            if !m.detail_table.is_empty() {
                assert!(!m.detail_primary_key.is_empty(), "detail_primary_key required when detail_table set for {}", m.table);
                assert!(!m.detail_foreign_key.is_empty(), "detail_foreign_key required when detail_table set for {}", m.table);
            }
            // 业务标题与模块必须存在
            assert!(!m.title.is_empty(), "title required for {}", m.table);
            assert!(!m.biz_type.is_empty(), "biz_type required for {}", m.table);
        }
    }

    #[test]
    fn test_graph_response_built() {
        let resp = build_graph_response();
        assert!(!resp.docs.is_empty());
        assert!(!resp.edges.is_empty());
        assert!(!resp.version.is_empty());
    }
}
