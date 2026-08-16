use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use crate::config::Config;
use crate::db::get_pool;
use crate::error::Result;
use crate::utils::{ApiResponse, row_get_f64};

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
    /// 下游主表主键（与 target_detail 关联）
    target_master_pk: &'static str,
    /// 下游明细 → 下游主表 的外键字段（如 tStk_IODetail.IOID）
    target_detail_fk: &'static str,
    /// 下游主表 → 源单主表 的外键字段（如 tStk_IO.POID）；空字符串表示不适用
    target_master_link_fk: &'static str,
    /// 下游明细 → 源单明细主键 的外键字段（如 tStk_IODetail.SouID）；空字符串表示不适用
    target_detail_link_fk: &'static str,
    /// 下游单据 Kind 过滤（None=不过滤）
    target_kind: Option<&'static str>,
}

fn resolve_source_meta(source_type: &str) -> Option<SourceDocMeta> {
    match source_type {
        "sales_quote" => Some(SourceDocMeta {
            source_detail: "tSal_QuoteDetail",
            source_detail_pk: "SQDetailID",
            source_detail_fk: "SQID",
            target_table: "tSal_Order",
            target_detail: "tSal_OrderDetail",
            target_master_pk: "SOID",
            target_detail_fk: "SOID",
            target_master_link_fk: "",
            target_detail_link_fk: "SQDetailID",  // tSal_OrderDetail.SQDetailID → tSal_QuoteDetail.SQDetailID
            target_kind: None,
        }),
        "sales_order" => Some(SourceDocMeta {
            source_detail: "tSal_OrderDetail",
            source_detail_pk: "SODetailID",
            source_detail_fk: "SOID",
            // 销售出库实际存储在 tStk_IO (Kind=SD)
            target_table: "tStk_IO",
            target_detail: "tStk_IODetail",
            target_master_pk: "IOID",
            target_detail_fk: "IOID",
            target_master_link_fk: "SOID",  // tStk_IO.SOID → tSal_Order.SOID
            target_detail_link_fk: "",
            target_kind: Some("SD"),
        }),
        "sales_outbound" => Some(SourceDocMeta {
            source_detail: "tStk_IODetail",
            source_detail_pk: "IODetailID",
            source_detail_fk: "IOID",
            target_table: "tStk_IO",
            target_detail: "tStk_IODetail",
            target_master_pk: "IOID",
            target_detail_fk: "IOID",
            target_master_link_fk: "",
            target_detail_link_fk: "SouID",  // tStk_IODetail.SouID → 源 tStk_IODetail.IODetailID
            target_kind: Some("SR"),
        }),
        "purchase_order" => Some(SourceDocMeta {
            source_detail: "tPur_OrderDetail",
            source_detail_pk: "PODetailID",
            source_detail_fk: "POID",
            target_table: "tStk_IO",
            target_detail: "tStk_IODetail",
            target_master_pk: "IOID",
            target_detail_fk: "IOID",
            target_master_link_fk: "POID",  // tStk_IO.POID → tPur_Order.POID
            target_detail_link_fk: "",
            target_kind: Some("PD"),
        }),
        "purchase_receipt" => Some(SourceDocMeta {
            source_detail: "tStk_IODetail",
            source_detail_pk: "IODetailID",
            source_detail_fk: "IOID",
            target_table: "tStk_IO",
            target_detail: "tStk_IODetail",
            target_master_pk: "IOID",
            target_detail_fk: "IOID",
            target_master_link_fk: "POID",  // tStk_IO.POID → 源 tStk_IO.IOID
            target_detail_link_fk: "",
            target_kind: Some("RI"),
        }),
        "purchase_inbound" => Some(SourceDocMeta {
            source_detail: "tStk_IODetail",
            source_detail_pk: "IODetailID",
            source_detail_fk: "IOID",
            target_table: "tStk_IO",
            target_detail: "tStk_IODetail",
            target_master_pk: "IOID",
            target_detail_fk: "IOID",
            target_master_link_fk: "",
            target_detail_link_fk: "SouID",  // tStk_IODetail.SouID → 源 tStk_IODetail.IODetailID
            target_kind: Some("TH"),
        }),
        // ===== 批发链路 =====
        "wholesale_quote" => Some(SourceDocMeta {
            source_detail: "tSal_QuoteDetail",
            source_detail_pk: "SQDetailID",
            source_detail_fk: "SQID",
            target_table: "tSal_Order",
            target_detail: "tSal_OrderDetail",
            target_master_pk: "SOID",
            target_detail_fk: "SOID",
            target_master_link_fk: "",
            target_detail_link_fk: "SQDetailID",  // tSal_OrderDetail.SQDetailID → tSal_QuoteDetail.SQDetailID
            target_kind: None,
        }),
        "wholesale_order" => Some(SourceDocMeta {
            source_detail: "tSal_OrderDetail",
            source_detail_pk: "SODetailID",
            source_detail_fk: "SOID",
            // 批发出库实际存储在 tStk_IO (Kind=SD, BTPID=WHOLESALE)
            target_table: "tStk_IO",
            target_detail: "tStk_IODetail",
            target_master_pk: "IOID",
            target_detail_fk: "IOID",
            target_master_link_fk: "SOID",  // tStk_IO.SOID → tSal_Order.SOID
            target_detail_link_fk: "",
            target_kind: Some("SD"),
        }),
        // 批发出库 → 批发退货（与 sales_outbound 同表结构，通过 SouID 明细级关联）
        "wholesale_outbound" => Some(SourceDocMeta {
            source_detail: "tStk_IODetail",
            source_detail_pk: "IODetailID",
            source_detail_fk: "IOID",
            target_table: "tStk_IO",
            target_detail: "tStk_IODetail",
            target_master_pk: "IOID",
            target_detail_fk: "IOID",
            target_master_link_fk: "",
            target_detail_link_fk: "SouID",  // tStk_IODetail.SouID → 源 tStk_IODetail.IODetailID
            target_kind: Some("SR"),
        }),
        // 采购报价 → 采购订单（无外键关联，仅参照带入数据，不跟踪累计执行量）
        "purchase_quote" => Some(SourceDocMeta {
            source_detail: "tPur_QuoteDetail",
            source_detail_pk: "PQDetailID",
            source_detail_fk: "PQID",
            target_table: "tPur_Order",
            target_detail: "tPur_OrderDetail",
            target_master_pk: "POID",
            target_detail_fk: "POID",
            target_master_link_fk: "",  // tPur_Order 无 PQID 字段
            target_detail_link_fk: "",  // tPur_OrderDetail 无 PQDetailID 字段
            target_kind: None,
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
    pub gdsno: String,
    pub gds_desc: String,
    pub barcode: String,
    pub stkid: String,
    pub qty: f64,
    pub fulfilled_qty: f64,
    pub pending_qty: f64,
    pub price: f64,
    pub unit_no: String,
    pub remark: String,
    /// 包装换算量（来自 tBas_Goods.PackCnvQty），用于前端显示"包装量"列和计算"件数"
    pub pack_cnv_qty: f64,
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

    // 1) 拉源单明细（含商品编码/名称/条码/包装量，便于前端带入）
    // 注：所有明细表的主键、GDSID、StkID 都是 uniqueidentifier，必须 CAST 为 NVARCHAR 才能用 &str 读取；
    // 备注字段实际列名是 Note（不是 Remark），UnitNO 也是大写 NO
    // ★ PackCnvQty 通过 LEFT JOIN tBas_Goods 获取（tStk_IODetail/tStk_MoveDetail 等明细表本身无此字段）
    //   tPur_OrderDetail 虽然本身有 PackCnvQty/PackQty，但用 tBas_Goods 取最新值更可靠（商品资料可能被修改）
    let detail_sql = format!(
        "SELECT CAST(d.{} AS NVARCHAR(40)) AS DetailID, CAST(d.GDSID AS NVARCHAR(40)) AS GDSID, \
         ISNULL(d.GDSNO,'') AS GDSNO, ISNULL(d.GDSDesc,'') AS GDSDesc, ISNULL(d.BarCode,'') AS BarCode, \
         ISNULL(CAST(d.StkID AS NVARCHAR(40)),'') AS StkID, ISNULL(d.Qty,0) AS Qty, \
         ISNULL(d.Price,0) AS Price, ISNULL(d.UnitNO,'') AS UnitNO, ISNULL(d.Note,'') AS Remark, \
         ISNULL(g.PackCnvQty, 0) AS PackCnvQty \
         FROM [{}] d LEFT JOIN tBas_Goods g ON g.GDSID = d.GDSID \
         WHERE d.{} = @p1",
        meta.source_detail_pk, meta.source_detail, meta.source_detail_fk
    );
    let p1: Vec<&dyn tiberius::ToSql> = vec![&params.id];
    let detail_rows = match conn.query(&detail_sql, &p1).await {
        Ok(stream) => match stream.into_first_result().await {
            Ok(rs) => rs,
            Err(e) => {
                tracing::warn!("[query_source_detail] 读取明细行失败: {} | sql={}", e, detail_sql);
                Vec::new()
            }
        },
        Err(e) => {
            tracing::warn!("[query_source_detail] SQL 错误: {} | sql={}", e, detail_sql);
            Vec::new()
        }
    };

    // 2) 下游累计（按 (GDSID, StkID) 聚合）
    // 两种关联模式：
    //   A) 主表引用源单主键：target_master.[master_link_fk] = @p1（如 tStk_IO.POID = 源 POID）
    //   B) 明细引用源单明细主键：target_detail.[detail_link_fk] IN (SELECT 源明细PK FROM 源明细 WHERE 源明细FK = @p1)
    // JOIN 条件统一为：target_master.[master_pk] = target_detail.[detail_fk]
    let kind_filter = meta
        .target_kind
        .map(|k| format!("AND M.Kind = '{}'", k))
        .unwrap_or_default();

    let uses_detail_link = !meta.target_detail_link_fk.is_empty();
    let uses_master_link = !meta.target_master_link_fk.is_empty();
    // 当两种关联都不存在时（如采购报价→采购订单，无外键），跳过累计计算
    let has_link = uses_detail_link || uses_master_link;
    let agg_sql = if !has_link {
        None
    } else if uses_detail_link {
        // 模式 B：下游明细引用源单明细主键
        // 注：GDSID/StkID 是 uniqueidentifier，必须 CAST 为 NVARCHAR 才能用作字符串聚合键；
        // 参数 @p1 是字符串，与 uniqueidentifier 列比较时 SQL Server 会自动转换，无需显式 CAST
        Some(format!(
            "SELECT CAST(D.GDSID AS NVARCHAR(40)) AS GDSID, \
             ISNULL(CAST(D.StkID AS NVARCHAR(40)),'') AS StkID, \
             ISNULL(SUM(D.Qty),0) AS F \
             FROM [{}] D \
             INNER JOIN [{}] M ON M.{} = D.{} \
             WHERE D.{} IN (SELECT {} FROM [{}] WHERE {} = @p1) \
             {} \
             AND M.State IN ('S','Y') \
             GROUP BY D.GDSID, D.StkID",
            meta.target_detail,
            meta.target_table,
            meta.target_master_pk,
            meta.target_detail_fk,
            meta.target_detail_link_fk,
            meta.source_detail_pk,
            meta.source_detail,
            meta.source_detail_fk,
            kind_filter
        ))
    } else {
        // 模式 A：下游主表引用源单主键
        Some(format!(
            "SELECT CAST(D.GDSID AS NVARCHAR(40)) AS GDSID, \
             ISNULL(CAST(D.StkID AS NVARCHAR(40)),'') AS StkID, \
             ISNULL(SUM(D.Qty),0) AS F \
             FROM [{}] D \
             INNER JOIN [{}] M ON M.{} = D.{} \
             WHERE M.{} = @p1 \
             {} \
             AND M.State IN ('S','Y') \
             GROUP BY D.GDSID, D.StkID",
            meta.target_detail,
            meta.target_table,
            meta.target_master_pk,
            meta.target_detail_fk,
            meta.target_master_link_fk,
            kind_filter
        ))
    };

    let mut fulfilled_map: std::collections::HashMap<(String, String), f64> =
        std::collections::HashMap::new();
    if let Some(sql) = agg_sql {
        match conn.query(&sql, &p1).await {
            Ok(stream) => match stream.into_first_result().await {
                Ok(rows) => {
                    for r in rows {
                        // GDSID/StkID 已 CAST 为 NVARCHAR，可以安全用 &str 读取
                        let gdsid: String = r.get::<&str, _>("GDSID").unwrap_or("").to_string();
                        let stkid: String = r.get::<&str, _>("StkID").unwrap_or("").to_string();
                        let f: f64 = row_get_f64(&r, "F");
                        fulfilled_map.insert((gdsid, stkid), f);
                    }
                }
                Err(e) => tracing::warn!("[query_source_detail] 聚合 SQL 读取失败: {} | sql={}", e, sql),
            },
            Err(e) => tracing::warn!("[query_source_detail] 聚合 SQL 错误: {} | sql={}", e, sql),
        }
    }

    // 3) 组装结果
    let mut out: Vec<SourceDetailRow> = Vec::with_capacity(detail_rows.len());
    for row in &detail_rows {
        let detail_id: String = row.get::<&str, _>("DetailID").unwrap_or("").to_string();
        let gdsid: String = row.get::<&str, _>("GDSID").unwrap_or("").to_string();
        let gdsno: String = row.get::<&str, _>("GDSNO").unwrap_or("").to_string();
        let gds_desc: String = row.get::<&str, _>("GDSDesc").unwrap_or("").to_string();
        let barcode: String = row.get::<&str, _>("BarCode").unwrap_or("").to_string();
        let stkid: String = row.get::<&str, _>("StkID").unwrap_or("").to_string();
        let qty: f64 = row_get_f64(&row, "Qty");
        let price: f64 = row_get_f64(&row, "Price");
        let unit_no: String = row.get::<&str, _>("UnitNO").unwrap_or("").to_string();
        let remark: String = row.get::<&str, _>("Remark").unwrap_or("").to_string();
        let pack_cnv_qty: f64 = row_get_f64(&row, "PackCnvQty");
        let fulfilled = *fulfilled_map.get(&(gdsid.clone(), stkid.clone())).unwrap_or(&0.0);
        let pending = (qty - fulfilled).max(0.0);
        out.push(SourceDetailRow {
            detail_id,
            gdsid,
            gdsno,
            gds_desc,
            barcode,
            stkid,
            qty,
            fulfilled_qty: fulfilled,
            pending_qty: pending,
            price,
            unit_no,
            remark,
            pack_cnv_qty,
        });
    }

    Ok(Json(ApiResponse::ok(serde_json::json!({ "list": out }))))
}
