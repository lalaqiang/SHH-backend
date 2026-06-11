use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use bb8::PooledConnection;
use bb8_tiberius::ConnectionManager;
use crate::config::Config;
use crate::db::get_pool;
use crate::error::Result;
use crate::utils::{ApiResponse, row_get_f64};

type Conn = PooledConnection<'static, ConnectionManager>;

// ============================================================
// 订单流程 P1 增强接口
//   - 库存可用量查询
//   - 源单明细查询（含未执行量）
// ============================================================

/// 库存可用量查询参数
#[derive(Deserialize)]
pub struct AvailableQuery {
    pub items: Vec<AvailableItem>,
    /// 当不指定仓库时，汇总所有仓库可用量
    pub aggregate_by_gds: Option<bool>,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct AvailableItem {
    pub gdsid: String,
    pub stkid: String,
}

#[derive(Serialize)]
pub struct AvailableRow {
    pub gdsid: String,
    pub stkid: String,
    pub qty: f64,
    pub reserved: f64,
    pub available: f64,
}

/// 库存可用量查询
/// POST /api/inventory/available
/// body: { items: [{gdsid, stkid}, ...], aggregate_by_gds?: bool }
pub async fn query_available(
    State(_config): State<Config>,
    Json(params): Json<AvailableQuery>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let mut rows: Vec<AvailableRow> = Vec::new();

    if params.items.is_empty() {
        return Ok(Json(ApiResponse::ok(serde_json::json!({ "list": rows }))));
    }

    if params.aggregate_by_gds.unwrap_or(false) {
        for item in &params.items {
            if item.gdsid.is_empty() {
                continue;
            }
            let p: Vec<&dyn tiberius::ToSql> = vec![&item.gdsid];
            let qty: f64 = match conn.query(
                "SELECT ISNULL(SUM(ISNULL(Qty,0)),0) AS Q FROM tStk_Stock WHERE GDSID = @p1",
                &p,
            ).await {
                Ok(stream) => match stream.into_row().await {
                    Ok(Some(row)) => row_get_f64(&row, "Q"),
                    _ => 0.0,
                },
                _ => 0.0,
            };
            let reserved: f64 = match conn.query(
                "SELECT ISNULL(SUM(ISNULL(Qty,0) - ISNULL(ReleasedQty,0)),0) AS R \
                 FROM tStk_Reserve WHERE GDSID = @p1 AND State = 'A'",
                &p,
            ).await {
                Ok(stream) => match stream.into_row().await {
                    Ok(Some(row)) => row_get_f64(&row, "R"),
                    _ => 0.0,
                },
                _ => 0.0,
            };
            rows.push(AvailableRow {
                gdsid: item.gdsid.clone(),
                stkid: String::new(),
                qty,
                reserved,
                available: qty - reserved,
            });
        }
    } else {
        for item in &params.items {
            if item.gdsid.is_empty() || item.stkid.is_empty() {
                continue;
            }
            let p: Vec<&dyn tiberius::ToSql> = vec![&item.gdsid, &item.stkid];
            let qty: f64 = match conn.query(
                "SELECT ISNULL(Qty,0) AS Q FROM tStk_Stock WHERE GDSID = @p1 AND StkID = @p2",
                &p,
            ).await {
                Ok(stream) => match stream.into_row().await {
                    Ok(Some(row)) => row_get_f64(&row, "Q"),
                    _ => 0.0,
                },
                _ => 0.0,
            };
            let reserved: f64 = match conn.query(
                "SELECT ISNULL(SUM(ISNULL(Qty,0) - ISNULL(ReleasedQty,0)),0) AS R \
                 FROM tStk_Reserve WHERE GDSID = @p1 AND StkID = @p2 AND State = 'A'",
                &p,
            ).await {
                Ok(stream) => match stream.into_row().await {
                    Ok(Some(row)) => row_get_f64(&row, "R"),
                    _ => 0.0,
                },
                _ => 0.0,
            };
            rows.push(AvailableRow {
                gdsid: item.gdsid.clone(),
                stkid: item.stkid.clone(),
                qty,
                reserved,
                available: qty - reserved,
            });
        }
    }

    Ok(Json(ApiResponse::ok(serde_json::json!({ "list": rows }))))
}

// ============= 源单明细查询 =============

/// 源单类型 → 主表/明细表/未执行量来源配置
///
/// 设计说明：
/// - source: 源单（被参照的单据）
/// - target: 下游单据（参照源单生成的新单据）
/// - 累计已执行量的方式：找出所有 target 类型的单据明细，
///   它们通过 child_link_fk 字段引用到 source 主表。
#[derive(Debug, Clone, Copy)]
struct SourceDocMeta {
    /// 源单主表
    source_table: &'static str,
    /// 源单明细表
    source_detail: &'static str,
    /// 源单明细主键
    source_detail_pk: &'static str,
    /// 源单明细上指向源单主表的外键（通常 = 源单主表主键）
    source_detail_fk: &'static str,
    /// 下游单据主表
    target_table: &'static str,
    /// 下游单据明细表
    target_detail: &'static str,
    /// 下游明细上指向源单主表的外键字段
    /// - 当下游直接引用源单主键时 = 源单主表主键
    /// - 当下游引用源单明细主键时 = 源单明细主键
    target_detail_link_fk: &'static str,
    /// 下游主表主键（与 target_detail 关联）
    target_master_pk: &'static str,
    /// 下游单据 Kind 过滤（None=不过滤）
    target_kind: Option<&'static str>,
}

fn resolve_source_meta(source_type: &str) -> Option<SourceDocMeta> {
    match source_type {
        "sales_quote" => Some(SourceDocMeta {
            source_table: "tSal_Quote",
            source_detail: "tSal_QuoteDetail",
            source_detail_pk: "SQDetailID",
            source_detail_fk: "SQID",
            target_table: "tSal_Order",
            target_detail: "tSal_OrderDetail",
            target_detail_link_fk: "SQDetailID",  // tSal_OrderDetail.SQDetailID → tSal_QuoteDetail.SQDetailID
            target_master_pk: "SOID",
            target_kind: None,
        }),
        "sales_order" => Some(SourceDocMeta {
            source_table: "tSal_Order",
            source_detail: "tSal_OrderDetail",
            source_detail_pk: "SODetailID",
            source_detail_fk: "SOID",
            target_table: "tSal_Inv",
            target_detail: "tSal_InvDetail",
            target_detail_link_fk: "SOID",  // tSal_InvDetail.SOID → tSal_Order.SOID
            target_master_pk: "SIID",
            target_kind: Some("SD"),
        }),
        "sales_outbound" => Some(SourceDocMeta {
            source_table: "tSal_Inv",
            source_detail: "tSal_InvDetail",
            source_detail_pk: "SIDetailID",
            source_detail_fk: "SIID",
            target_table: "tSal_Inv",
            target_detail: "tSal_InvDetail",
            target_detail_link_fk: "SIID",  // tSal_InvDetail.SIID → tSal_Inv.SIID (self-ref: 出库→退货)
            target_master_pk: "SIID",
            target_kind: Some("SR"),
        }),
        "purchase_order" => Some(SourceDocMeta {
            source_table: "tPur_Order",
            source_detail: "tPur_OrderDetail",
            source_detail_pk: "PODetailID",
            source_detail_fk: "POID",
            target_table: "tStk_IO",
            target_detail: "tStk_IODetail",
            target_detail_link_fk: "POID",  // tStk_IODetail.POID → tPur_Order.POID
            target_master_pk: "IOID",
            target_kind: Some("PD"),
        }),
        "purchase_receipt" => Some(SourceDocMeta {
            source_table: "tStk_IO",
            source_detail: "tStk_IODetail",
            source_detail_pk: "IODetailID",
            source_detail_fk: "IOID",
            target_table: "tStk_IO",
            target_detail: "tStk_IODetail",
            target_detail_link_fk: "IOID",
            target_master_pk: "IOID",
            target_kind: Some("RI"),
        }),
        "purchase_inbound" => Some(SourceDocMeta {
            source_table: "tStk_IO",
            source_detail: "tStk_IODetail",
            source_detail_pk: "IODetailID",
            source_detail_fk: "IOID",
            target_table: "tStk_IO",
            target_detail: "tStk_IODetail",
            target_detail_link_fk: "IOID",
            target_master_pk: "IOID",
            target_kind: Some("TH"),
        }),
        _ => None,
    }
}

#[derive(Deserialize)]
pub struct SourceDetailQuery {
    pub source_type: String,
    pub id: String,
}

#[derive(Serialize)]
pub struct SourceDetailRow {
    pub detail_id: String,
    pub gdsid: String,
    pub stkid: String,
    pub qty: f64,
    pub fulfilled_qty: f64,
    pub pending_qty: f64,
    pub price: f64,
    pub unit_no: String,
    pub remark: String,
}

/// 查询源单明细 + 未执行量
/// POST /api/order/source-detail
/// body: { source_type: "sales_quote"|..., id: "源单主键" }
pub async fn query_source_detail(
    State(_config): State<Config>,
    Json(params): Json<SourceDetailQuery>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let meta = match resolve_source_meta(&params.source_type) {
        Some(m) => m,
        None => {
            return Ok(Json(ApiResponse::err(&format!(
                "不支持的源单类型: {}",
                params.source_type
            ))));
        }
    };

    let mut conn = get_pool().get().await?;

    // 1) 拉源单明细
    let detail_sql = format!(
        "SELECT {} AS DetailID, GDSID, ISNULL(StkID,'') AS StkID, ISNULL(Qty,0) AS Qty, \
         ISNULL(Price,0) AS Price, ISNULL(UnitNO,'') AS UnitNO, ISNULL(Remark,'') AS Remark \
         FROM [{}] WHERE {} = @p1",
        meta.source_detail_pk, meta.source_detail, meta.source_detail_fk
    );
    let p1: Vec<&dyn tiberius::ToSql> = vec![&params.id];
    let detail_rows = match conn.query(&detail_sql, &p1).await {
        Ok(stream) => stream.into_first_result().await.unwrap_or_default(),
        Err(_) => Vec::new(),
    };

    // 2) 下游累计（按 (GDSID, StkID) 聚合）
    // - 普通情形：target_detail.[link_fk] = target_master.[master_pk]，
    //             target_master.[link_fk] = @p1（@p1 = 源单主键）
    // - 特殊情形（link_fk == source_detail_pk，如 sales_quote 用 SQDetailID）：
    //             需要先用子查询取出源单所有明细主键，再让 target_master.[link_fk] IN (...)
    let kind_filter = meta
        .target_kind
        .map(|k| format!("AND M.Kind = '{}'", k))
        .unwrap_or_default();

    let uses_detail_link = meta.target_detail_link_fk == meta.source_detail_pk;
    let agg_sql = if uses_detail_link {
        // 链接到源单明细主键（@p1 是源单主键，需要先取明细主键列表）
        format!(
            "SELECT D.GDSID, ISNULL(D.StkID,'') AS StkID, \
             ISNULL(SUM(D.Qty),0) AS F \
             FROM [{}] D \
             INNER JOIN [{}] M ON M.{} = D.{} \
             WHERE M.{} IN (SELECT {} FROM [{}] WHERE {} = @p1) \
             {} \
             AND (M.State = 'S' OR M.State = 'A') \
             GROUP BY D.GDSID, D.StkID",
            meta.target_detail,
            meta.target_table,
            meta.target_master_pk,
            meta.target_detail_link_fk,
            meta.target_detail_link_fk,
            meta.source_detail_pk,
            meta.source_detail,
            meta.source_detail_fk,
            kind_filter
        )
    } else {
        // 链接到源单主键
        format!(
            "SELECT D.GDSID, ISNULL(D.StkID,'') AS StkID, \
             ISNULL(SUM(D.Qty),0) AS F \
             FROM [{}] D \
             INNER JOIN [{}] M ON M.{} = D.{} \
             WHERE M.{} = @p1 \
             {} \
             AND (M.State = 'S' OR M.State = 'A') \
             GROUP BY D.GDSID, D.StkID",
            meta.target_detail,
            meta.target_table,
            meta.target_master_pk,
            meta.target_detail_link_fk,
            meta.target_detail_link_fk,
            kind_filter
        )
    };

    let mut fulfilled_map: std::collections::HashMap<(String, String), f64> =
        std::collections::HashMap::new();
    if let Ok(stream) = conn.query(&agg_sql, &p1).await {
        if let Ok(rows) = stream.into_first_result().await {
            for r in rows {
                let gdsid: String = r.get::<&str, _>("GDSID").unwrap_or("").to_string();
                let stkid: String = r.get::<&str, _>("StkID").unwrap_or("").to_string();
                let f: f64 = row_get_f64(&r, "F");
                fulfilled_map.insert((gdsid, stkid), f);
            }
        }
    }

    // 3) 组装结果
    let mut out: Vec<SourceDetailRow> = Vec::with_capacity(detail_rows.len());
    for row in &detail_rows {
        let detail_id: String = row.get::<&str, _>("DetailID").unwrap_or("").to_string();
        let gdsid: String = row.get::<&str, _>("GDSID").unwrap_or("").to_string();
        let stkid: String = row.get::<&str, _>("StkID").unwrap_or("").to_string();
        let qty: f64 = row_get_f64(&row, "Qty");
        let price: f64 = row_get_f64(&row, "Price");
        let unit_no: String = row.get::<&str, _>("UnitNO").unwrap_or("").to_string();
        let remark: String = row.get::<&str, _>("Remark").unwrap_or("").to_string();
        let fulfilled = *fulfilled_map.get(&(gdsid.clone(), stkid.clone())).unwrap_or(&0.0);
        let pending = (qty - fulfilled).max(0.0);
        out.push(SourceDetailRow {
            detail_id,
            gdsid,
            stkid,
            qty,
            fulfilled_qty: fulfilled,
            pending_qty: pending,
            price,
            unit_no,
            remark,
        });
    }

    Ok(Json(ApiResponse::ok(serde_json::json!({ "list": out }))))
}
