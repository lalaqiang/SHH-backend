use axum::extract::{State, Json, Extension};
use serde::Deserialize;
use tiberius::Row;
use crate::config::Config;
use crate::db::get_pool;
use crate::error::Result;
use crate::utils::{ApiResponse, row_get_f64};
use crate::handlers::base_data::try_get_value;
use crate::middleware::auth::Claims;

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

pub async fn retail_sale(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<RetailSaleRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    if body.details.is_empty() {
        return Ok(Json(ApiResponse::err("销售明细不能为空")));
    }

    let mut conn = get_pool().get().await?;
    conn.execute("BEGIN TRANSACTION", &[]).await?;

    let today = chrono::Local::now().format("%Y%m%d").to_string();
    let prefix_pattern = format!("LS{}-%", today);

    let seq_sql = "SELECT MAX([InvNo]) as max_no FROM [tSal_Inv] WHERE [InvNo] LIKE @p1";
    let seq_stream = conn.query(seq_sql, &[&prefix_pattern.as_str()]).await?;
    let seq_row = seq_stream.into_row().await?;

    let next_seq = if let Some(row) = seq_row {
        let max_no: Option<&str> = row.get("max_no");
        if let Some(max) = max_no {
            if let Some(seq_part) = max.rsplit('-').next() {
                seq_part.parse::<u32>().unwrap_or(0) + 1
            } else {
                1
            }
        } else {
            1
        }
    } else {
        1
    };

    let inv_no = format!("LS{}-{:03}", today, next_seq);
    let now = chrono::Local::now().naive_local();
    let cust_id = body.CustID.as_deref().unwrap_or("");
    let pay_method = body.PayMethod.as_deref().unwrap_or("");
    let remark = body.Remark.as_deref().unwrap_or("");

    let header_sql = r#"INSERT INTO [tSal_Inv] ([InvNo], [InvDate], [CustID], [StkID], [TotalAmt], [PayMethod], [State], [Remark], [EDate], [EUser])
        VALUES (@p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10)"#;
    let header_params: Vec<&dyn tiberius::ToSql> = vec![
        &inv_no, &now, &cust_id, &body.StkID, &body.TotalAmt,
        &pay_method, &"S", &remark, &now, &claims.user_code,
    ];
    conn.execute(header_sql, &header_params).await?;

    for (i, detail) in body.details.iter().enumerate() {
        let line_no = (i + 1) as i32;
        let discount = detail.Discount.unwrap_or(1.0);

        let detail_sql = r#"INSERT INTO [tSal_InvDetail] ([InvNo], [LineNo], [GDSID], [Qty], [Price], [Amt], [Discount])
            VALUES (@p1, @p2, @p3, @p4, @p5, @p6, @p7)"#;
        let detail_params: Vec<&dyn tiberius::ToSql> = vec![
            &inv_no, &line_no, &detail.GDSID, &detail.Qty,
            &detail.Price, &detail.Amt, &discount,
        ];
        conn.execute(detail_sql, &detail_params).await?;
    }

    conn.execute("COMMIT TRANSACTION", &[]).await?;

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "InvNo": inv_no
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
    let summary_sql = r#"SELECT COUNT(*) as todaySalesCount, ISNULL(SUM([TotalAmt]), 0) as todaySalesAmt
        FROM [tSal_Inv]
        WHERE [State] <> 'D' AND [EUser] = @p1 AND CONVERT(varchar(10), [InvDate], 120) = @p2"#;

    let mut today_sales_count: i32 = 0;
    let mut today_sales_amt: f64 = 0.0;

    if !stk_id.is_empty() {
        let summary_with_stk_sql = r#"SELECT COUNT(*) as todaySalesCount, ISNULL(SUM([TotalAmt]), 0) as todaySalesAmt
            FROM [tSal_Inv]
            WHERE [State] <> 'D' AND [StkID] = @p1 AND CONVERT(varchar(10), [InvDate], 120) = @p2"#;
        let summary_stream = conn.query(summary_with_stk_sql, &[&stk_id.as_str(), &today.as_str()]).await?;
        if let Some(row) = summary_stream.into_row().await? {
            today_sales_count = row.get::<i32, _>("todaySalesCount").unwrap_or(0);
            today_sales_amt = row_get_f64(&row, "todaySalesAmt");
        }
    } else {
        let summary_stream = conn.query(summary_sql, &[&claims.user_code.as_str(), &today.as_str()]).await?;
        if let Some(row) = summary_stream.into_row().await? {
            today_sales_count = row.get::<i32, _>("todaySalesCount").unwrap_or(0);
            today_sales_amt = row_get_f64(&row, "todaySalesAmt");
        }
    }

    // 查询最后一笔销售单号
    let mut last_inv_no = String::new();
    if !stk_id.is_empty() {
        let last_sql = r#"SELECT TOP 1 [InvNo] FROM [tSal_Inv]
            WHERE [State] <> 'D' AND [StkID] = @p1
            ORDER BY [EDate] DESC"#;
        let last_stream = conn.query(last_sql, &[&stk_id.as_str()]).await?;
        if let Some(row) = last_stream.into_row().await? {
            last_inv_no = row.get::<&str, _>("InvNo").unwrap_or("").to_string();
        }
    } else {
        let last_sql = r#"SELECT TOP 1 [InvNo] FROM [tSal_Inv]
            WHERE [State] <> 'D' AND [EUser] = @p1
            ORDER BY [EDate] DESC"#;
        let last_stream = conn.query(last_sql, &[&claims.user_code.as_str()]).await?;
        if let Some(row) = last_stream.into_row().await? {
            last_inv_no = row.get::<&str, _>("InvNo").unwrap_or("").to_string();
        }
    }

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "todaySalesCount": today_sales_count,
        "todaySalesAmt": today_sales_amt,
        "lastInvNo": last_inv_no
    }))))
}
