//! 单据状态机工具
//!
//! 统一所有业务单据的状态码：
//! - D 草稿 (Draft)         - 已保存可修改，未生效
//! - E 编辑中 (Editing)     - 保留位
//! - S 已审核 (Reviewed)    - 审核通过，触发库存/财务等下游影响
//! - Y 已确认 (Confirmed)   - 终态
//! - N 新建 (New)           - 老系统兼容
//! - C 作废 (Cancelled)    - 软删除状态

pub const STATE_DRAFT: &str = "D";
pub const STATE_EDITING: &str = "E";
pub const STATE_REVIEWED: &str = "S";
pub const STATE_CONFIRMED: &str = "Y";
pub const STATE_NEW: &str = "N";
pub const STATE_CANCELLED: &str = "C";

/// 判断状态是否为"已生效"（影响库存/财务）
pub fn is_effective(state: &str) -> bool {
    matches!(state, STATE_REVIEWED | STATE_CONFIRMED)
}

/// 判断状态是否可以编辑
pub fn is_editable(state: &str) -> bool {
    matches!(state, "" | STATE_DRAFT | STATE_NEW | STATE_EDITING)
}

/// 判断状态是否可以审核
pub fn can_review(state: &str) -> bool {
    matches!(state, STATE_DRAFT | STATE_NEW)
}

/// 判断状态是否可以反审
pub fn can_unreview(state: &str) -> bool {
    matches!(state, STATE_REVIEWED)
}

/// 判断状态是否可以作废（软删除）
pub fn can_cancel(state: &str) -> bool {
    !matches!(state, STATE_CANCELLED)
}

/// 状态码 → 中文标签
pub fn label(state: &str) -> &'static str {
    match state {
        STATE_DRAFT => "草稿",
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
pub fn detail_meta(table: &str) -> Option<(&'static str, &'static str, &'static str)> {
    match table {
        "tSal_Order" => Some(("tSal_OrderDetail", "SODetailID", "SOID")),
        "tSal_Inv" => Some(("tSal_InvDetail", "SIDetailID", "SIID")),
        "tSal_Return" => Some(("tSal_ReturnDetail", "SRDetailID", "SRID")),
        "tPur_Order" => Some(("tPur_OrderDetail", "PODetailID", "POID")),
        "tPur_Inv" => Some(("tPur_InvDetail", "PIDetailID", "PIID")),
        "tPur_Return" => Some(("tPur_ReturnDetail", "PRDetailID", "PRID")),
        "tSal_Quote" => Some(("tSal_QuoteDetail", "SQDetailID", "SQID")),
        "tPur_Quote" => Some(("tPur_QuoteDetail", "PQDetailID", "PQID")),
        "tStk_IO" => Some(("tStk_IODetail", "IODetailID", "IOID")),
        "tStk_Move" => Some(("tStk_MoveDetail", "MoveDetailID", "MoveID")),
        "tStk_StockCycle" => Some(("tStk_StockCycleDetail", "CycleDetailID", "CycleID")),
        "tStk_Tran" => Some(("tStk_TranDetail", "TranDetailID", "TranID")),
        "tStk_ReplenishApply" => Some(("tStk_ReplenishApplyDetail", "ApplyDetailID", "ApplyID")),
        _ => None,
    }
}

/// 单据表主键字段
pub fn master_pk(table: &str) -> Option<&'static str> {
    match table {
        "tSal_Order" => Some("SOID"),
        "tSal_Inv" => Some("SIID"),
        "tSal_Return" => Some("SRID"),
        "tPur_Order" => Some("POID"),
        "tPur_Inv" => Some("PIID"),
        "tPur_Return" => Some("PRID"),
        "tSal_Quote" => Some("SQID"),
        "tPur_Quote" => Some("PQID"),
        "tStk_IO" => Some("IOID"),
        "tStk_Move" => Some("MoveID"),
        "tStk_StockCycle" => Some("CycleID"),
        "tStk_Tran" => Some("TranID"),
        "tStk_ReplenishApply" => Some("ApplyID"),
        _ => None,
    }
}

/// 单据表业务单号字段
pub fn master_no(table: &str) -> Option<&'static str> {
    match table {
        "tSal_Order" => Some("OrderNo"),
        "tSal_Inv" => Some("InvNo"),
        "tSal_Return" => Some("ReturnNo"),
        "tPur_Order" => Some("PoNo"),
        "tPur_Inv" => Some("PiNo"),
        "tPur_Return" => Some("PrNo"),
        "tSal_Quote" => Some("QuoteNo"),
        "tPur_Quote" => Some("PqNo"),
        "tStk_IO" => Some("IONo"),
        "tStk_Move" => Some("MoveNo"),
        "tStk_StockCycle" => Some("CycleNo"),
        "tStk_Tran" => Some("TranNo"),
        "tStk_ReplenishApply" => Some("ApplyNo"),
        "tAcc_PayIn" => Some("PayInNo"),
        "tAcc_PayOut" => Some("PayOutNo"),
        _ => None,
    }
}

/// 哪些单据表支持审核
pub fn is_reviewable_table(table: &str) -> bool {
    matches!(
        table,
        "tSal_Quote" | "tPur_Quote"
        | "tSal_Order" | "tPur_Order"
        | "tSal_Inv" | "tPur_Inv"
        | "tSal_Return" | "tPur_Return"
        | "tStk_IO" | "tStk_Move"
        | "tStk_StockCycle"
        | "tStk_Tran"
        | "tStk_ReplenishApply"
        | "tAcc_PayIn" | "tAcc_PayOut"
    )
}

/// 哪些单据影响库存（用于审核/反审时决定是否更新 tStk_Stock）
pub fn affects_stock(table: &str) -> bool {
    matches!(
        table,
        "tStk_IO" | "tStk_Move" | "tStk_StockCycle" | "tStk_Tran"
    )
}
