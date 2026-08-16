//! 单据状态机工具
//!
//! 统一所有业务单据的状态码（与项目规则一致）：
//! - N 新建/草稿 (New)     - 已保存可修改，未生效（新建单据初始状态）
//! - E 编辑中 (Editing)     - 保留位
//! - S 已审核 (Reviewed)    - 审核通过，触发库存/财务等下游影响
//! - Y 已确认 (Confirmed)   - 终态
//! - D 删除/作废 (Deleted)  - 软删除状态，不可编辑/审核
//! - C 已作废 (Cancelled)   - 显式作废
//!
//! 表覆盖与字段映射严格以 doc_graph.rs 为单一事实源。

pub const STATE_DRAFT: &str = "D";        // D=删除/作废（保留常量名向后兼容，但语义为"已删除"）
pub const STATE_EDITING: &str = "E";
pub const STATE_REVIEWED: &str = "S";
pub const STATE_CONFIRMED: &str = "Y";
pub const STATE_NEW: &str = "N";          // 新建单据初始状态
pub const STATE_CANCELLED: &str = "C";

/// 判断状态是否为"已生效"（影响库存/财务）
pub fn is_effective(state: &str) -> bool {
    matches!(state, STATE_REVIEWED | STATE_CONFIRMED)
}

/// 判断状态是否可以编辑（D=已删除不可编辑）
pub fn is_editable(state: &str) -> bool {
    matches!(state, "" | STATE_NEW | STATE_EDITING)
}

/// 判断状态是否可以审核（仅 N/E 可审核，D=已删除不可审核）
pub fn can_review(state: &str) -> bool {
    matches!(state, STATE_NEW | STATE_EDITING)
}

/// 判断状态是否可以反审
pub fn can_unreview(state: &str) -> bool {
    matches!(state, STATE_REVIEWED)
}

/// 判断状态是否可以作废（软删除）
pub fn can_cancel(state: &str) -> bool {
    !matches!(state, STATE_CANCELLED | STATE_DRAFT)
}

/// 状态码 → 中文标签
pub fn label(state: &str) -> &'static str {
    match state {
        STATE_DRAFT => "删除",
        STATE_EDITING => "编辑中",
        STATE_REVIEWED => "已审核",
        STATE_CONFIRMED => "已确认",
        STATE_NEW => "新建",
        STATE_CANCELLED => "作废",
        _ => "未知",
    }
}

/// 单据表名 → 详情表 / 详情主键 / 详情外键 到主表的字段
/// 返回 (detail_table, detail_pk, detail_fk_to_master)
/// 严格对齐 doc_graph.rs 的 detail_table/detail_primary_key/detail_foreign_key
pub fn detail_meta(table: &str) -> Option<(&'static str, &'static str, &'static str)> {
    match table {
        "tSal_Order" => Some(("tSal_OrderDetail", "SODetailID", "SOID")),
        "tSal_Inv" => Some(("tSal_InvDetail", "SIDetailID", "SIID")),
        "tPur_Order" => Some(("tPur_OrderDetail", "PODetailID", "POID")),
        "tPur_Inv" => Some(("tPur_InvDetail", "PIDetailID", "PIID")),
        "tPur_Return" => Some(("tPur_ReturnDetail", "PRDetailID", "PRID")),
        "tSal_Quote" => Some(("tSal_QuoteDetail", "SQDetailID", "SQID")),
        "tPur_Quote" => Some(("tPur_QuoteDetail", "PQDetailID", "PQID")),
        "tPur_AdjPrice" => Some(("tPur_AdjPriceDetail", "PAPDetailID", "PAPID")),
        "tStk_IO" => Some(("tStk_IODetail", "IODetailID", "IOID")),
        "tStk_Move" => Some(("tStk_MoveDetail", "MoveDetailID", "MoveID")),
        "tStk_StockCycle" => Some(("tStk_StockCycleDetail", "CycleDetailID", "CycleID")),
        "tStk_Tran" => Some(("tStk_TranDetail", "TranDetailID", "TranID")),
        "tStk_ReplenishApply" => Some(("tStk_ReplenishApplyDtl", "ReplenishApplyDtlID", "ReplenishApplyID")),
        "tFin_Receipt" => Some(("tFin_ReceiptDtl", "ReceiptDtlID", "RecID")),
        "tFin_Payment" => Some(("tFin_PaymentDtl", "PaymentDtlID", "PayID")),
        // tSal_EmpSales / tFin_CashFlow 无明细表
        _ => None,
    }
}

/// 单据表主键字段
/// 严格对齐 doc_graph.rs 的 primary_key
pub fn master_pk(table: &str) -> Option<&'static str> {
    match table {
        "tSal_Order" => Some("SOID"),
        "tSal_Inv" => Some("SIID"),
        "tPur_Order" => Some("POID"),
        "tPur_Inv" => Some("PIID"),
        "tPur_Return" => Some("PRID"),
        "tSal_Quote" => Some("SQID"),
        "tPur_Quote" => Some("PQID"),
        "tPur_AdjPrice" => Some("PAPID"),
        "tStk_IO" => Some("IOID"),
        "tStk_Move" => Some("MoveID"),
        "tStk_StockCycle" => Some("CycleID"),
        "tStk_Tran" => Some("TranID"),
        "tStk_ReplenishApply" => Some("ReplenishApplyID"),
        "tFin_Receipt" => Some("RecID"),
        "tFin_Payment" => Some("PayID"),
        "tFin_CashFlow" => Some("CFID"),
        "tSal_EmpSales" => Some("ID"),
        _ => None,
    }
}

/// 单据表业务单号字段
/// 严格对齐 doc_graph.rs 的 no_field
pub fn master_no(table: &str) -> Option<&'static str> {
    match table {
        "tSal_Order" => Some("SoNo"),
        "tSal_Inv" => Some("SINo"),
        "tPur_Order" => Some("PoNo"),
        "tPur_Inv" => Some("PiNo"),
        "tPur_Return" => Some("PrNo"),
        "tSal_Quote" => Some("SQNo"),
        "tPur_Quote" => Some("PqNo"),
        "tPur_AdjPrice" => Some("PAPNo"),
        "tStk_IO" => Some("IONo"),
        "tStk_Move" => Some("MoveNO"),
        "tStk_StockCycle" => Some("CycleNo"),
        "tStk_Tran" => Some("TranNo"),
        "tStk_ReplenishApply" => Some("ReplenishApplyNo"),
        "tFin_Receipt" => Some("RecNO"),
        "tFin_Payment" => Some("PayNO"),
        "tFin_CashFlow" => Some("CFNO"),
        // tSal_EmpSales 无单据号字段
        _ => None,
    }
}

/// 单据表业务日期字段
/// 严格对齐 doc_graph.rs 的 date_field
pub fn date_field(table: &str) -> Option<&'static str> {
    match table {
        "tSal_Order" => Some("SoDate"),
        "tSal_Inv" => Some("SIDate"),
        "tPur_Order" => Some("PoDate"),
        "tPur_Inv" => Some("RecvDate"),
        "tPur_Return" => Some("RetDate"),
        "tSal_Quote" => Some("SQDate"),
        "tPur_Quote" => Some("PqDate"),
        "tPur_AdjPrice" => Some("PAPDate"),
        "tStk_IO" => Some("IoDate"),
        "tStk_Move" => Some("MoveDate"),
        "tStk_StockCycle" => Some("CycleDate"),
        "tStk_Tran" => Some("TranDate"),
        "tStk_ReplenishApply" => Some("ReplenishApplyDate"),
        "tFin_Receipt" => Some("RecDate"),
        "tFin_Payment" => Some("PayDate"),
        "tFin_CashFlow" => Some("CFDate"),
        "tSal_EmpSales" => Some("SaleDate"),
        _ => None,
    }
}

/// 哪些单据表支持审核
/// 与 doc_graph.rs 的表覆盖保持一致（除 tSal_EmpSales 扁平表外均可审核）
pub fn is_reviewable_table(table: &str) -> bool {
    matches!(
        table,
        "tSal_Quote" | "tPur_Quote" | "tPur_AdjPrice"
        | "tSal_Order" | "tPur_Order"
        | "tSal_Inv" | "tPur_Inv" | "tPur_Return"
        | "tStk_IO" | "tStk_Move"
        | "tStk_StockCycle"
        | "tStk_Tran"
        | "tStk_ReplenishApply"
        | "tFin_Receipt" | "tFin_Payment" | "tFin_CashFlow"
    )
}

/// 哪些单据影响库存（用于审核/反审时决定是否更新 tStk_Stock）
/// 严格对齐 doc_graph.rs 的 affects_stock 字段：
/// - tStk_IO/tStk_Move/tStk_Tran/tStk_StockCycle：直接写库存
/// - tSal_Order：审核写 QQty 预占 + tStk_Reserve
/// - tSal_Inv：出库扣减 Qty
/// - tPur_Inv/tPur_Return：历史表（数据已迁移到 tStk_IO:PD/PR），保留以维持元数据一致
pub fn affects_stock(table: &str) -> bool {
    matches!(
        table,
        "tStk_IO" | "tStk_Move" | "tStk_StockCycle" | "tStk_Tran"
        | "tSal_Order" | "tSal_Inv"
        | "tPur_Inv" | "tPur_Return"
    )
}
