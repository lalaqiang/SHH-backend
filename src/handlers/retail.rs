use crate::config::Config;
use crate::db::get_pool;
use crate::error::Result;
use crate::middleware::auth::Claims;
use crate::services::inventory_ledger;
use crate::utils::{ApiResponse, row_get_f64};
use axum::extract::{Extension, Json, State};
use serde::Deserialize;
use tiberius::ToSql;

#[derive(Deserialize)]
pub struct RetailSaleRequest {
    pub CustID: Option<String>,
    pub StkID: String,
    pub details: Vec<RetailSaleDetailItem>,
    pub TotalAmt: f64,
    pub PayMethod: Option<String>,
    pub Remark: Option<String>,
}

#[derive(Deserialize)]
pub struct RetailSaleDetailItem {
    pub GDSID: String,
    pub Qty: f64,
    pub Price: f64,
    pub Amt: f64,
    pub Discount: Option<f64>,
}

const ZERO_UUID: &str = "00000000-0000-0000-0000-000000000000";

/// 原子分配零售单据号（并发安全）
///
/// 格式：LS{YYYYMMDD}-{seq:03}（按日重置序号）
/// 算法：
///   1. UPDATE-OUTPUT 原子自增 tSys_DocNoSeq（DocTypeID='LS', PeriodKey=YYYYMMDD）
///   2. 记录不存在时，查 tSal_Inv MAX 序号初始化后 INSERT
///   3. 并发初始化冲突时重试 UPDATE
///
/// 注意：单据号分配在主事务外独立提交（auto-commit），即使主事务回滚，
/// 已分配的序号也不回收（跳号是并发安全的必要代价）。
async fn generate_retail_no(conn: &mut inventory_ledger::Conn) -> Result<String> {
    let today = chrono::Local::now().format("%Y%m%d").to_string();
    let prefix = "LS";

    for _attempt in 0..5 {
        // 步骤 1：原子递增（记录存在时直接分配）
        let update_sql = "UPDATE tSys_DocNoSeq SET CurrentSeq = CurrentSeq + 1, LUTime = GETDATE() \
                          OUTPUT INSERTED.CurrentSeq \
                          WHERE DocTypeID = @p1 AND PeriodKey = @p2";
        let p: Vec<&dyn ToSql> = vec![&prefix, &today];
        if let Ok(stream) = conn.query(update_sql, &p).await {
            if let Ok(Some(row)) = stream.into_row().await {
                if let Some(seq) = row.get::<i64, _>(0) {
                    return Ok(format!("LS{}-{:03}", today, seq));
                }
            }
        }

        // 步骤 2：首次初始化 — 查 tSal_Inv 实际 MAX 序号
        let pattern = format!("LS{}-%", today);
        let max_sql = "SELECT MAX([SINo]) as max_no FROM [tSal_Inv] WHERE [SINo] LIKE @p1";
        let mut init_seq: i64 = 1;
        if let Ok(stream) = conn.query(max_sql, &[&pattern]).await {
            if let Ok(Some(row)) = stream.into_row().await {
                if let Some(max) = row.get::<&str, _>("max_no") {
                    if let Some(seq_part) = max.rsplit('-').next() {
                        init_seq = seq_part.parse::<i64>().unwrap_or(0) + 1;
                    }
                }
            }
        }

        // 尝试 INSERT（CurrentSeq 直接设为 init_seq）
        let insert_sql = "INSERT INTO tSys_DocNoSeq (DocTypeID, PeriodKey, CurrentSeq, LUTime) \
                          VALUES (@p1, @p2, @p3, GETDATE())";
        let p: Vec<&dyn ToSql> = vec![&prefix, &today, &init_seq];
        match conn.execute(insert_sql, &p).await {
            Ok(r) => {
                let affected = r.rows_affected().iter().sum::<u64>();
                if affected > 0 {
                    return Ok(format!("LS{}-{:03}", today, init_seq));
                }
                // affected=0：异常，重试
            }
            Err(_) => {
                // 主键冲突：并发已有人插入，重试 UPDATE
                continue;
            }
        }
    }

    Err(crate::error::AppError::Internal(
        "零售单据号生成失败：连续 5 次重试均失败".to_string(),
    ))
}

pub async fn retail_sale(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<RetailSaleRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    if body.details.is_empty() {
        return Ok(Json(ApiResponse::err("销售明细不能为空")));
    }
    if body.StkID.is_empty() {
        return Ok(Json(ApiResponse::err("StkID 不能为空")));
    }

    let mut conn = get_pool().get().await?;

    // 1. 原子分配单据号（独立 auto-commit，主事务回滚不影响序号）
    let inv_no = generate_retail_no(&mut conn).await?;
    let si_id = format!("{}", uuid::Uuid::new_v4());
    let now_str = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let cust_id = body.CustID.as_deref().unwrap_or(ZERO_UUID);
    let emp_id = if claims.emp_id.is_empty() {
        ZERO_UUID.to_string()
    } else {
        claims.emp_id.clone()
    };
    let remark = body.Remark.as_deref().unwrap_or("");

    // 2. 主事务包裹：主表+明细+库存过账，任一失败回滚
    let result: std::result::Result<(), String> = async {
        inventory_ledger::begin_tran(&mut conn).await?;

        // 写主表 tSal_Inv（State='S' 已审核，因为 POS 即时销售）
        let header_sql = r#"INSERT INTO [tSal_Inv] ([SIID], [SINo], [SIDate], [CustID], [StkID], [SumAmt], [State], [Note], [EDate], [EUser], [LUTime])
            VALUES (@p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10, @p9)"#;
        let header_params: Vec<&dyn ToSql> = vec![
            &si_id, &inv_no, &now_str, &cust_id, &body.StkID, &body.TotalAmt,
            &"S", &remark, &now_str, &emp_id,
        ];
        conn.execute(header_sql, &header_params).await
            .map_err(|e| format!("保存主表失败: {}", e))?;

        // 写明细 + 库存过账
        for (i, detail) in body.details.iter().enumerate() {
            let row_no = format!("{:03}", i + 1);
            let dis_rate = detail.Discount.unwrap_or(1.0) * 100.0;
            let detail_id = format!("{}", uuid::Uuid::new_v4());

            let detail_sql = r#"INSERT INTO [tSal_InvDetail] ([SIID], [SIDetailID], [RowNO], [GDSID], [StkID], [Qty], [Price], [DisRate], [Amt])
                VALUES (@p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9)"#;
            let detail_params: Vec<&dyn ToSql> = vec![
                &si_id, &detail_id, &row_no, &detail.GDSID, &body.StkID, &detail.Qty,
                &detail.Price, &dis_rate, &detail.Amt,
            ];
            conn.execute(detail_sql, &detail_params).await
                .map_err(|e| format!("保存明细(行{})失败: {}", i + 1, e))?;

            // 库存过账：销售出库 direction = -1（写三件套 tStk_Stock + tStk_StockTranHis + tStk_StockYM + tStk_Qty）
            // 失败（库存不足）时返回 ok=false，由外层回滚整张单据
            let (_, ok) = inventory_ledger::post_ledger(
                &mut conn,
                &detail.GDSID,
                &body.StkID,
                detail.Qty,
                -1.0,
                &si_id,
                &detail_id,
            ).await;
            if !ok {
                return Err(format!(
                    "库存不足，无法完成销售：商品ID={} 仓库ID={} 数量={}",
                    detail.GDSID, body.StkID, detail.Qty
                ));
            }
        }

        inventory_ledger::commit_tran(&mut conn).await?;
        Ok(())
    }.await;

    if let Err(e) = result {
        // 回滚主事务（仅回滚数据写入，不回滚已分配的单据号）
        inventory_ledger::rollback_tran(&mut conn).await;
        return Ok(Json(ApiResponse::err(&crate::utils::db_err(
            "销售失败: {}",
            &e,
        ))));
    }

    // ★ POS 零售保存成功后自动重算提成（对齐 88 项目，不依赖前端调用）
    // 提成计算失败不影响销售主流程，仅记录 warn 日志
    if let Err(e) =
        crate::services::commission_service::recalc_invoice_commission(&mut conn, &si_id).await
    {
        tracing::warn!(
            "[retail_sale] POS 零售 {} 提成重算失败（不影响销售）: {}",
            si_id,
            e
        );
    }

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "InvNo": inv_no,
        "SIID": si_id
    }))))
}

pub async fn get_cashier_info(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    // 查询当前用户的仓库ID
    let emp_sql = "SELECT TOP 1 [StkID] FROM [tBas_Emp] WHERE [EmpNo] = @p1";
    let emp_stream = conn.query(emp_sql, &[&claims.user_code.as_str()]).await?;
    let emp_row = emp_stream.into_row().await?;

    let stk_id: String = if let Some(row) = emp_row {
        row.get::<&str, _>("StkID").unwrap_or("").to_string()
    } else {
        "".to_string()
    };

    let today = chrono::Local::now().format("%Y-%m-%d").to_string();

    // 查询今日销售汇总
    let summary_sql = r#"SELECT COUNT(*) as todaySalesCount, ISNULL(SUM([SumAmt]), 0) as todaySalesAmt
        FROM [tSal_Inv]
        WHERE [State] <> 'D' AND [EUser] = @p1 AND CONVERT(varchar(10), [SIDate], 120) = @p2"#;

    let mut today_sales_count: i32 = 0;
    let mut today_sales_amt: f64 = 0.0;

    if !stk_id.is_empty() {
        let summary_with_stk_sql = r#"SELECT COUNT(*) as todaySalesCount, ISNULL(SUM([SumAmt]), 0) as todaySalesAmt
            FROM [tSal_Inv]
            WHERE [State] <> 'D' AND [StkID] = @p1 AND CONVERT(varchar(10), [SIDate], 120) = @p2"#;
        let summary_stream = conn
            .query(summary_with_stk_sql, &[&stk_id.as_str(), &today.as_str()])
            .await?;
        if let Some(row) = summary_stream.into_row().await? {
            today_sales_count = row.get::<i32, _>("todaySalesCount").unwrap_or(0);
            today_sales_amt = row_get_f64(&row, "todaySalesAmt");
        }
    } else {
        let summary_stream = conn
            .query(summary_sql, &[&claims.user_code.as_str(), &today.as_str()])
            .await?;
        if let Some(row) = summary_stream.into_row().await? {
            today_sales_count = row.get::<i32, _>("todaySalesCount").unwrap_or(0);
            today_sales_amt = row_get_f64(&row, "todaySalesAmt");
        }
    }

    // 查询最后一笔销售单号
    let mut last_inv_no = String::new();
    if !stk_id.is_empty() {
        let last_sql = r#"SELECT TOP 1 [SINo] FROM [tSal_Inv]
            WHERE [State] <> 'D' AND [StkID] = @p1
            ORDER BY [EDate] DESC"#;
        let last_stream = conn.query(last_sql, &[&stk_id.as_str()]).await?;
        if let Some(row) = last_stream.into_row().await? {
            last_inv_no = row.get::<&str, _>("SINo").unwrap_or("").to_string();
        }
    } else {
        let last_sql = r#"SELECT TOP 1 [SINo] FROM [tSal_Inv]
            WHERE [State] <> 'D' AND [EUser] = @p1
            ORDER BY [EDate] DESC"#;
        let last_stream = conn.query(last_sql, &[&claims.user_code.as_str()]).await?;
        if let Some(row) = last_stream.into_row().await? {
            last_inv_no = row.get::<&str, _>("SINo").unwrap_or("").to_string();
        }
    }

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "todaySalesCount": today_sales_count,
        "todaySalesAmt": today_sales_amt,
        "lastInvNo": last_inv_no
    }))))
}
