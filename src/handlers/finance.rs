use axum::extract::{State, Json, Query};
use serde::Deserialize;
use tiberius::Row;
use crate::config::Config;
use crate::db::get_pool;
use crate::error::Result;
use crate::utils::{ApiResponse, build_pagination_sql_with_sort, row_get_f64};
use crate::handlers::base_data::try_get_value;

#[derive(Deserialize)]
pub struct PaginationParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
    pub sort_prop: Option<String>,
    pub sort_order: Option<String>,
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

pub async fn get_receivable_list(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);

    let mut base_query = "SELECT * FROM tArd_AR WHERE State <> 'D'".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(&format!(
                " AND (ARNo LIKE @p{} OR CustName LIKE @p{})",
                pidx, pidx + 1
            ));
            pidx += 2;
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
        }
    }

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(
        &base_query,
        page,
        page_size,
        params.sort_prop.as_deref(),
        params.sort_order.as_deref(),
    );
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params
        .iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();

    let mut total: i32 = 0;
    let count_stream = conn.query(&count_sql, &param_refs).await?;
    if let Some(row) = count_stream.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }

    let data_stream = conn.query(&paginated_sql, &param_refs).await?;
    let rows: Vec<Row> = data_stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok_paginated(
        data,
        total as u64,
        page,
        page_size,
    )))
}

pub async fn get_payable_list(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);

    let mut base_query = "SELECT * FROM tArd_PD WHERE State <> 'D'".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(&format!(
                " AND (PDNo LIKE @p{} OR SuppName LIKE @p{})",
                pidx, pidx + 1
            ));
            pidx += 2;
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
        }
    }

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(
        &base_query,
        page,
        page_size,
        params.sort_prop.as_deref(),
        params.sort_order.as_deref(),
    );
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params
        .iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();

    let mut total: i32 = 0;
    let count_stream = conn.query(&count_sql, &param_refs).await?;
    if let Some(row) = count_stream.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }

    let data_stream = conn.query(&paginated_sql, &param_refs).await?;
    let rows: Vec<Row> = data_stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok_paginated(
        data,
        total as u64,
        page,
        page_size,
    )))
}

pub async fn get_receipt_list(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);

    let mut base_query = "SELECT * FROM tAcc_PayIn WHERE State <> 'D'".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            // tAcc_PayIn 实际只有 PayInNo/Note/EmpID 字段，无客户名
            base_query.push_str(&format!(
                " AND (PayInNo LIKE @p{} OR Note LIKE @p{})",
                pidx, pidx + 1
            ));
            pidx += 2;
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
        }
    }

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(
        &base_query,
        page,
        page_size,
        params.sort_prop.as_deref(),
        params.sort_order.as_deref(),
    );
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params
        .iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();

    let mut total: i32 = 0;
    let count_stream = conn.query(&count_sql, &param_refs).await?;
    if let Some(row) = count_stream.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }

    let data_stream = conn.query(&paginated_sql, &param_refs).await?;
    let rows: Vec<Row> = data_stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok_paginated(
        data,
        total as u64,
        page,
        page_size,
    )))
}

#[derive(Deserialize)]
pub struct CreateReceiptParams {
    // 实际表 tAcc_PayIn 不含客户/单据/银行账号名称字段
    // 真实字段：PayInID, PayInNo, PayInDate, Amt, EmpID, StkID, BankAccNoID, Note, State, EDate, EUser
    pub BankAccNoID: Option<String>,
    pub EmpID: Option<String>,
    pub StkID: Option<String>,
    pub Amount: Option<f64>,           // 实际字段是 Amt（money）
    pub Note: Option<String>,
}

pub async fn create_receipt(
    State(_config): State<Config>,
    Json(body): Json<CreateReceiptParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now();
    let date_str = now.format("%Y%m%d").to_string();
    let now_naive = now.naive_local();

    let prefix = format!("RCT{}", date_str);
    let like_pattern = format!("{}%", prefix);
    let seq_sql = "SELECT ISNULL(MAX(CAST(RIGHT(PayInNo, 4) AS INT)), 0) FROM [tAcc_PayIn] WHERE PayInNo LIKE @p1";
    let seq_stream = conn.query(seq_sql, &[&like_pattern.as_str()]).await?;
    let mut seq: i32 = 1;
    if let Some(row) = seq_stream.into_row().await? {
        seq = row.get::<i32, _>(0).unwrap_or(0) + 1;
    }
    let rcpt_no = format!("{}{:04}", prefix, seq);

    let bank_acc_id = body.BankAccNoID.as_deref().unwrap_or("");
    let emp_id = body.EmpID.as_deref().unwrap_or("");
    let stk_id = body.StkID.as_deref().unwrap_or("");
    let amount = body.Amount.unwrap_or(0.0);
    let note = body.Note.as_deref().unwrap_or("");

    // ⚠️ 财务子表 tArd_AR/PD 在 DB 中是订阅表（非财务 AR/AP），不维护 ReceivableID/PayableID
    //    此处仅写入 tAcc_PayIn，不做 AR 累加
    let sql = r#"INSERT INTO [tAcc_PayIn] (PayInID, PayInNo, PayInDate, Amt, EmpID, StkID, BankAccNoID, Note, State, EDate, EUser)
                 VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10)"#;

    conn.execute(sql, &[
        &rcpt_no.as_str(),
        &now_naive,
        &amount,
        &emp_id,
        &stk_id,
        &bank_acc_id,
        &note,
        &"N",
        &now_naive,
        &"system",
    ]).await?;

    Ok(Json(ApiResponse::msg("收款单创建成功")))
}

pub async fn get_cash_flow_list(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);

    // ⚠️ tFin_CashFlow 在 DB 中不存在 → 改为 UNION ALL tAcc_PayIn/Out
    // 真实字段：tAcc_PayIn(Amt, PayInDate), tAcc_PayOut(Amt, PayOutDate)
    let mut base_query = r#"
        SELECT CONVERT(varchar(10), PayInDate, 120) as FlowDate,
               'IN' as FlowType,
               ISNULL(SUM(Amt), 0) as TotalAmt,
               COUNT(*) as FlowCount
        FROM tAcc_PayIn
        WHERE State <> 'D'
        GROUP BY CONVERT(varchar(10), PayInDate, 120)
        UNION ALL
        SELECT CONVERT(varchar(10), PayOutDate, 120) as FlowDate,
               'OUT' as FlowType,
               ISNULL(SUM(Amt), 0) as TotalAmt,
               COUNT(*) as FlowCount
        FROM tAcc_PayOut
        WHERE State <> 'D'
        GROUP BY CONVERT(varchar(10), PayOutDate, 120)
    "#
    .to_string();

    let mut query_params: Vec<Option<String>> = Vec::new();

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(
        &base_query,
        page,
        page_size,
        params.sort_prop.as_deref(),
        params.sort_order.as_deref(),
    );
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params
        .iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();

    let mut total: i32 = 0;
    let count_stream = conn.query(&count_sql, &param_refs).await?;
    if let Some(row) = count_stream.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }

    let data_stream = conn.query(&paginated_sql, &param_refs).await?;
    let rows: Vec<Row> = data_stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok_paginated(
        data,
        total as u64,
        page,
        page_size,
    )))
}

pub async fn get_payment_list(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);

    let mut base_query = "SELECT * FROM tAcc_PayOut WHERE State <> 'D'".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            // tAcc_PayOut 不含 SuppName 字段（无供应商关联）
            base_query.push_str(&format!(
                " AND (PayOutNo LIKE @p{} OR Note LIKE @p{})",
                pidx, pidx + 1
            ));
            pidx += 2;
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
        }
    }

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(
        &base_query,
        page,
        page_size,
        params.sort_prop.as_deref(),
        params.sort_order.as_deref(),
    );
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params
        .iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();

    let mut total: i32 = 0;
    let count_stream = conn.query(&count_sql, &param_refs).await?;
    if let Some(row) = count_stream.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }

    let data_stream = conn.query(&paginated_sql, &param_refs).await?;
    let rows: Vec<Row> = data_stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok_paginated(
        data,
        total as u64,
        page,
        page_size,
    )))
}

#[derive(Deserialize)]
pub struct CreatePaymentParams {
    // 实际表 tAcc_PayOut 不含供应商/单据/银行账号名称字段
    // 真实字段：PayOutID, PayOutNo, PayOutDate, Amt, EmpID, StkID, BankAccNoID, Note, State, EDate, EUser
    pub BankAccNoID: Option<String>,
    pub EmpID: Option<String>,
    pub StkID: Option<String>,
    pub Amount: Option<f64>,           // 实际字段是 Amt（money）
    pub Note: Option<String>,
}

pub async fn create_payment(
    State(_config): State<Config>,
    Json(body): Json<CreatePaymentParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now();
    let date_str = now.format("%Y%m%d").to_string();
    let now_naive = now.naive_local();

    let prefix = format!("PAY{}", date_str);
    let like_pattern = format!("{}%", prefix);
    let seq_sql = "SELECT ISNULL(MAX(CAST(RIGHT(PayOutNo, 4) AS INT)), 0) FROM [tAcc_PayOut] WHERE PayOutNo LIKE @p1";
    let seq_stream = conn.query(seq_sql, &[&like_pattern.as_str()]).await?;
    let mut seq: i32 = 1;
    if let Some(row) = seq_stream.into_row().await? {
        seq = row.get::<i32, _>(0).unwrap_or(0) + 1;
    }
    let pay_no = format!("{}{:04}", prefix, seq);

    let bank_acc_id = body.BankAccNoID.as_deref().unwrap_or("");
    let emp_id = body.EmpID.as_deref().unwrap_or("");
    let stk_id = body.StkID.as_deref().unwrap_or("");
    let amount = body.Amount.unwrap_or(0.0);
    let note = body.Note.as_deref().unwrap_or("");

    // ⚠️ tArd_PD 是订阅表（非财务 AP），不维护 PayableID
    let sql = r#"INSERT INTO [tAcc_PayOut] (PayOutID, PayOutNo, PayOutDate, Amt, EmpID, StkID, BankAccNoID, Note, State, EDate, EUser)
                 VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10)"#;

    conn.execute(sql, &[
        &pay_no.as_str(),
        &now_naive,
        &amount,
        &emp_id,
        &stk_id,
        &bank_acc_id,
        &note,
        &"N",
        &now_naive,
        &"system",
    ]).await?;

    Ok(Json(ApiResponse::msg("付款单创建成功")))
}

#[derive(Deserialize)]
pub struct UpdatePaymentParams {
    pub PayID: String,
    pub PayDate: Option<String>,
    pub SuppID: Option<String>,
    pub DeptID: Option<String>,
    pub EmpID: Option<String>,
    pub StkID: Option<String>,
    pub Amount: Option<f64>,
    pub PayMethod: Option<String>,
    pub BankName: Option<String>,
    pub BankAccount: Option<String>,
    pub DocID: Option<String>,
    pub DocNo: Option<String>,
    pub Note: Option<String>,
}

pub async fn update_payment(
    State(_config): State<Config>,
    Json(body): Json<UpdatePaymentParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now_naive = chrono::Local::now().naive_local();

    let pay_id = body.PayID.as_str();
    let supp_id = body.SuppID.as_deref().unwrap_or("");
    let dept_id = body.DeptID.as_deref().unwrap_or("");
    let emp_id = body.EmpID.as_deref().unwrap_or("");
    let stk_id = body.StkID.as_deref().unwrap_or("");
    let amount = body.Amount.unwrap_or(0.0);
    let pay_method = body.PayMethod.as_deref().unwrap_or("bank");
    let bank_name = body.BankName.as_deref().unwrap_or("");
    let bank_account = body.BankAccount.as_deref().unwrap_or("");
    let doc_id = body.DocID.as_deref().unwrap_or("");
    let doc_no = body.DocNo.as_deref().unwrap_or("");
    let note = body.Note.as_deref().unwrap_or("");

    let sql = r#"UPDATE [tAcc_PayOut] SET SuppID=@p1, DeptID=@p2, EmpID=@p3, StkID=@p4,
                 Amount=@p5, PayOutMethod=@p6, BankName=@p7, BankAccount=@p8, DocID=@p9, DocNo=@p10,
                 Note=@p11, EDate=@p12, EUser=@p13 WHERE PayOutID=@p14"#;

    conn.execute(sql, &[
        &supp_id,
        &dept_id,
        &emp_id,
        &stk_id,
        &amount,
        &pay_method,
        &bank_name,
        &bank_account,
        &doc_id,
        &doc_no,
        &note,
        &now_naive,
        &"system",
        &pay_id,
    ]).await?;

    Ok(Json(ApiResponse::msg("付款单更新成功")))
}

#[derive(Deserialize)]
pub struct DeletePaymentParams {
    pub PayID: String,
}

pub async fn delete_payment(
    State(_config): State<Config>,
    Json(body): Json<DeletePaymentParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let pay_id = body.PayID.as_str();

    let query_sql = "SELECT Amount, DocID FROM [tAcc_PayOut] WHERE PayOutID = @p1";
    let stream = conn.query(query_sql, &[&pay_id]).await?;
    let row = stream.into_row().await?;

    if let Some(row) = row {
        let amount: f64 = row_get_f64(&row, "Amount");
        let doc_id: Option<&str> = row.get::<&str, _>("DocID");

        conn.execute("BEGIN TRANSACTION", &[]).await?;

        if let Some(doc_id_val) = doc_id {
            if !doc_id_val.is_empty() {
                let reverse_sql = r#"UPDATE [tArd_PD] SET PaidAmt = PaidAmt - @p1, RemainAmt = RemainAmt + @p1,
                                     Status = CASE WHEN PaidAmt - @p1 <= 0 THEN 'unpaid' ELSE 'partial' END
                                     WHERE PDID = @p2"#;
                conn.execute(reverse_sql, &[
                    &amount,
                    &doc_id_val,
                ]).await?;
            }
        }

        let del_sql = "UPDATE [tAcc_PayOut] SET State = 'D' WHERE PayOutID = @p1";
        conn.execute(del_sql, &[&pay_id]).await?;

        conn.execute("COMMIT TRANSACTION", &[]).await?;
    }

    Ok(Json(ApiResponse::msg("付款单删除成功")))
}

#[derive(Deserialize)]
pub struct AuditPaymentParams {
    pub PayID: String,
}

pub async fn audit_payment(
    State(_config): State<Config>,
    Json(body): Json<AuditPaymentParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now_naive = chrono::Local::now().naive_local();

    let pay_id = body.PayID.as_str();

    let sql = r#"UPDATE [tAcc_PayOut] SET State = 'S', EDate = @p1, EUser = @p2 WHERE PayOutID = @p3 AND State = 'N'"#;
    conn.execute(sql, &[
        &now_naive,
        &"system",
        &pay_id,
    ]).await?;

    Ok(Json(ApiResponse::msg("付款单审核成功")))
}

#[derive(Deserialize)]
pub struct UpdateReceiptParams {
    pub RcptID: String,
    pub CustID: Option<String>,
    pub DeptID: Option<String>,
    pub EmpID: Option<String>,
    pub StkID: Option<String>,
    pub Amount: Option<f64>,
    pub RcptMethod: Option<String>,
    pub BankName: Option<String>,
    pub BankAccount: Option<String>,
    pub DocID: Option<String>,
    pub DocNo: Option<String>,
    pub Note: Option<String>,
}

pub async fn update_receipt(
    State(_config): State<Config>,
    Json(body): Json<UpdateReceiptParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now_naive = chrono::Local::now().naive_local();

    let rcpt_id = body.RcptID.as_str();
    let cust_id = body.CustID.as_deref().unwrap_or("");
    let dept_id = body.DeptID.as_deref().unwrap_or("");
    let emp_id = body.EmpID.as_deref().unwrap_or("");
    let stk_id = body.StkID.as_deref().unwrap_or("");
    let amount = body.Amount.unwrap_or(0.0);
    let rcpt_method = body.RcptMethod.as_deref().unwrap_or("cash");
    let bank_name = body.BankName.as_deref().unwrap_or("");
    let bank_account = body.BankAccount.as_deref().unwrap_or("");
    let doc_id = body.DocID.as_deref().unwrap_or("");
    let doc_no = body.DocNo.as_deref().unwrap_or("");
    let note = body.Note.as_deref().unwrap_or("");

    let sql = r#"UPDATE [tAcc_PayIn] SET CustID=@p1, DeptID=@p2, EmpID=@p3, StkID=@p4,
                 Amount=@p5, PayInMethod=@p6, BankName=@p7, BankAccount=@p8, DocID=@p9, DocNo=@p10,
                 Note=@p11, EDate=@p12, EUser=@p13 WHERE PayInID=@p14"#;

    conn.execute(sql, &[
        &cust_id,
        &dept_id,
        &emp_id,
        &stk_id,
        &amount,
        &rcpt_method,
        &bank_name,
        &bank_account,
        &doc_id,
        &doc_no,
        &note,
        &now_naive,
        &"system",
        &rcpt_id,
    ]).await?;

    Ok(Json(ApiResponse::msg("收款单更新成功")))
}

#[derive(Deserialize)]
pub struct DeleteReceiptParams {
    pub RcptID: String,
}

pub async fn delete_receipt(
    State(_config): State<Config>,
    Json(body): Json<DeleteReceiptParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let rcpt_id = body.RcptID.as_str();

    let query_sql = "SELECT Amount, DocID FROM [tAcc_PayIn] WHERE PayInID = @p1";
    let stream = conn.query(query_sql, &[&rcpt_id]).await?;
    let row = stream.into_row().await?;

    if let Some(row) = row {
        let amount: f64 = row_get_f64(&row, "Amount");
        let doc_id: Option<&str> = row.get::<&str, _>("DocID");

        conn.execute("BEGIN TRANSACTION", &[]).await?;

        if let Some(doc_id_val) = doc_id {
            if !doc_id_val.is_empty() {
                let reverse_sql = r#"UPDATE [tArd_AR] SET ReceivedAmt = ReceivedAmt - @p1, RemainAmt = RemainAmt + @p1,
                                     Status = CASE WHEN ReceivedAmt - @p1 <= 0 THEN 'unpaid' ELSE 'partial' END
                                     WHERE ARID = @p2"#;
                conn.execute(reverse_sql, &[
                    &amount,
                    &doc_id_val,
                ]).await?;
            }
        }

        let del_sql = "UPDATE [tAcc_PayIn] SET State = 'D' WHERE PayInID = @p1";
        conn.execute(del_sql, &[&rcpt_id]).await?;

        conn.execute("COMMIT TRANSACTION", &[]).await?;
    }

    Ok(Json(ApiResponse::msg("收款单删除成功")))
}

#[derive(Deserialize)]
pub struct AuditReceiptParams {
    pub RcptID: String,
}

pub async fn audit_receipt(
    State(_config): State<Config>,
    Json(body): Json<AuditReceiptParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now_naive = chrono::Local::now().naive_local();

    let rcpt_id = body.RcptID.as_str();

    // tFin_Receipt 表不存在 → 实际是 tAcc_PayIn
    let sql = r#"UPDATE [tAcc_PayIn] SET State = 'S', EDate = @p1, EUser = @p2 WHERE PayInID = @p3 AND State = 'N'"#;
    conn.execute(sql, &[
        &now_naive,
        &"system",
        &rcpt_id,
    ]).await?;

    Ok(Json(ApiResponse::msg("收款单审核成功")))
}

#[derive(Deserialize)]
pub struct ProcessPayablePaymentParams {
    pub PayableID: String,
    pub Amount: Option<f64>,
}

pub async fn process_payable_payment(
    State(_config): State<Config>,
    Json(body): Json<ProcessPayablePaymentParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let payable_id = body.PayableID.as_str();
    let amount = body.Amount.unwrap_or(0.0);

    let sql = r#"UPDATE [tArd_PD] SET PaidAmt = PaidAmt + @p1, RemainAmt = RemainAmt - @p1,
                 Status = CASE WHEN RemainAmt - @p1 <= 0 THEN 'paid' ELSE 'partial' END
                 WHERE PDID = @p2"#;
    conn.execute(sql, &[
        &amount,
        &payable_id,
    ]).await?;

    Ok(Json(ApiResponse::msg("应付付款处理成功")))
}

#[derive(Deserialize)]
pub struct WriteoffPayableParams {
    pub PayableID: String,
}

pub async fn writeoff_payable(
    State(_config): State<Config>,
    Json(body): Json<WriteoffPayableParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let payable_id = body.PayableID.as_str();

    let sql = r#"UPDATE [tArd_PD] SET RemainAmt = 0, PaidAmt = TotalAmt, Status = 'paid' WHERE PDID = @p1"#;
    conn.execute(sql, &[&payable_id]).await?;

    Ok(Json(ApiResponse::msg("应付核销成功")))
}

#[derive(Deserialize)]
pub struct AdjustPayableParams {
    pub PayableID: String,
    pub AdjustAmt: Option<f64>,
    pub Note: Option<String>,
}

pub async fn adjust_payable(
    State(_config): State<Config>,
    Json(body): Json<AdjustPayableParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now_naive = chrono::Local::now().naive_local();

    let payable_id = body.PayableID.as_str();
    let adjust_amt = body.AdjustAmt.unwrap_or(0.0);
    let note = body.Note.as_deref().unwrap_or("");

    let sql = r#"UPDATE [tArd_PD] SET RemainAmt = RemainAmt + @p1,
                 Status = CASE WHEN RemainAmt + @p1 <= 0 THEN 'paid'
                           WHEN PaidAmt > 0 THEN 'partial'
                           ELSE 'unpaid' END,
                 Note = CASE WHEN @p2 <> '' THEN @p2 ELSE Note END,
                 EDate = @p3
                 WHERE PDID = @p4"#;
    conn.execute(sql, &[
        &adjust_amt,
        &note,
        &now_naive,
        &payable_id,
    ]).await?;

    Ok(Json(ApiResponse::msg("应付调整成功")))
}

#[derive(Deserialize)]
pub struct ProcessReceivableRefundParams {
    pub ReceivableID: String,
    pub Amount: Option<f64>,
}

pub async fn process_receivable_refund(
    State(_config): State<Config>,
    Json(body): Json<ProcessReceivableRefundParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let receivable_id = body.ReceivableID.as_str();
    let amount = body.Amount.unwrap_or(0.0);

    let sql = r#"UPDATE [tArd_AR] SET ReceivedAmt = ReceivedAmt - @p1, RemainAmt = RemainAmt + @p1,
                 Status = CASE WHEN ReceivedAmt - @p1 <= 0 THEN 'unpaid' ELSE 'partial' END
                 WHERE ARID = @p2"#;
    conn.execute(sql, &[
        &amount,
        &receivable_id,
    ]).await?;

    Ok(Json(ApiResponse::msg("应收退款处理成功")))
}

#[derive(Deserialize)]
pub struct WriteoffReceivableParams {
    pub ReceivableID: String,
}

pub async fn writeoff_receivable(
    State(_config): State<Config>,
    Json(body): Json<WriteoffReceivableParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let receivable_id = body.ReceivableID.as_str();

    let sql = r#"UPDATE [tArd_AR] SET RemainAmt = 0, ReceivedAmt = TotalAmt, Status = 'paid' WHERE ARID = @p1"#;
    conn.execute(sql, &[&receivable_id]).await?;

    Ok(Json(ApiResponse::msg("应收核销成功")))
}

#[derive(Deserialize)]
pub struct AdjustReceivableParams {
    pub ReceivableID: String,
    pub AdjustAmt: Option<f64>,
    pub Note: Option<String>,
}

pub async fn adjust_receivable(
    State(_config): State<Config>,
    Json(body): Json<AdjustReceivableParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now_naive = chrono::Local::now().naive_local();

    let receivable_id = body.ReceivableID.as_str();
    let adjust_amt = body.AdjustAmt.unwrap_or(0.0);
    let note = body.Note.as_deref().unwrap_or("");

    let sql = r#"UPDATE [tArd_AR] SET RemainAmt = RemainAmt + @p1,
                 Status = CASE WHEN RemainAmt + @p1 <= 0 THEN 'paid'
                           WHEN ReceivedAmt > 0 THEN 'partial'
                           ELSE 'unpaid' END,
                 Note = CASE WHEN @p2 <> '' THEN @p2 ELSE Note END,
                 EDate = @p3
                 WHERE ARID = @p4"#;
    conn.execute(sql, &[
        &adjust_amt,
        &note,
        &now_naive,
        &receivable_id,
    ]).await?;

    Ok(Json(ApiResponse::msg("应收调整成功")))
}

#[derive(Deserialize)]
pub struct OverdueAccountsQuery {
    pub kind: Option<String>,
}

pub async fn get_overdue_accounts(
    State(_config): State<Config>,
    Query(params): Query<OverdueAccountsQuery>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    // ⚠️ tArd_AR/PD 是订阅表（TelCode/ProvidersName/SubscriberId），不是财务 AR/AP
    // 改用派生 AR/AP：从 tStk_IO 按 Kind 汇总（无需新建表）
    let mut conn = get_pool().get().await?;
    let kind = params.kind.as_deref().unwrap_or("");

    let mut data: Vec<serde_json::Value> = Vec::new();

    if kind.is_empty() || kind == "receivable" {
        // 派生 AR：所有 SD/SI/POS 出库金额（按客户）减去 SR 退货金额
        // 已审过的单据（State='S'）且未作废（State<>'D'）
        let sql = r#"
            SELECT
                io.CustID,
                c.CustName,
                ISNULL(SUM(CASE WHEN io.Kind IN ('SD','SI','POS') THEN io.SumAmt ELSE 0 END), 0) AS TotalAmt,
                ISNULL(SUM(CASE WHEN io.Kind = 'SR' THEN io.SumAmt ELSE 0 END), 0) AS ReturnedAmt,
                ISNULL(SUM(CASE WHEN io.Kind IN ('SD','SI','POS') THEN io.SumAmt ELSE 0 END), 0) -
                ISNULL(SUM(CASE WHEN io.Kind = 'SR' THEN io.SumAmt ELSE 0 END), 0) AS OpenAR,
                MAX(io.IoDate) AS LastDate
            FROM tStk_IO io
            LEFT JOIN tBas_Cust c ON c.CustID = io.CustID
            WHERE io.State = 'S'
              AND io.CustID IS NOT NULL
              AND io.Kind IN ('SD','SI','POS','SR')
              AND io.IoDate < DATEADD(DAY, -30, GETDATE())
            GROUP BY io.CustID, c.CustName
            HAVING (ISNULL(SUM(CASE WHEN io.Kind IN ('SD','SI','POS') THEN io.SumAmt ELSE 0 END), 0) -
                    ISNULL(SUM(CASE WHEN io.Kind = 'SR' THEN io.SumAmt ELSE 0 END), 0)) > 0
            ORDER BY OpenAR DESC
        "#;
        let stream = conn.query(sql, &[]).await?;
        let rows: Vec<Row> = stream.into_first_result().await?;
        for row in &rows {
            data.push(row_to_json(row));
        }
    }

    if kind.is_empty() || kind == "payable" {
        // 派生 AP：所有 PD/RI 入库金额（按供应商）减去 PR/TH 退货金额
        let sql = r#"
            SELECT
                io.SuppID,
                s.SuppName,
                ISNULL(SUM(CASE WHEN io.Kind IN ('PD','RI') THEN io.SumAmt ELSE 0 END), 0) AS TotalAmt,
                ISNULL(SUM(CASE WHEN io.Kind IN ('PR','TH') THEN io.SumAmt ELSE 0 END), 0) AS ReturnedAmt,
                ISNULL(SUM(CASE WHEN io.Kind IN ('PD','RI') THEN io.SumAmt ELSE 0 END), 0) -
                ISNULL(SUM(CASE WHEN io.Kind IN ('PR','TH') THEN io.SumAmt ELSE 0 END), 0) AS OpenAP,
                MAX(io.IoDate) AS LastDate
            FROM tStk_IO io
            LEFT JOIN tBas_Supp s ON s.SuppID = io.SuppID
            WHERE io.State = 'S'
              AND io.SuppID IS NOT NULL
              AND io.Kind IN ('PD','RI','PR','TH')
              AND io.IoDate < DATEADD(DAY, -30, GETDATE())
            GROUP BY io.SuppID, s.SuppName
            HAVING (ISNULL(SUM(CASE WHEN io.Kind IN ('PD','RI') THEN io.SumAmt ELSE 0 END), 0) -
                    ISNULL(SUM(CASE WHEN io.Kind IN ('PR','TH') THEN io.SumAmt ELSE 0 END), 0)) > 0
            ORDER BY OpenAP DESC
        "#;
        let stream = conn.query(sql, &[]).await?;
        let rows: Vec<Row> = stream.into_first_result().await?;
        for row in &rows {
            data.push(row_to_json(row));
        }
    }

    Ok(Json(ApiResponse::ok(data)))
}

// ============================================================================
// 派生 AR/AP 实时查询（方案 B）
//   - 不维护 tFin_Receivable/Payable 表
//   - 直接从 tStk_IO（已审核单据）按 Kind 维度计算
//   - 实时准确但有性能成本（大表需走索引）
// ============================================================================

/// 单客户应收汇总
pub async fn get_customer_ar(
    State(_config): State<Config>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);

    let mut base_query = r#"
        SELECT
            io.CustID,
            c.CustName,
            ISNULL(SUM(CASE WHEN io.Kind IN ('SD','SI','POS') THEN io.SumAmt ELSE 0 END), 0) AS SalesAmt,
            ISNULL(SUM(CASE WHEN io.Kind = 'SR' THEN io.SumAmt ELSE 0 END), 0) AS ReturnAmt,
            ISNULL(SUM(CASE WHEN io.Kind IN ('SD','SI','POS') THEN io.SumAmt ELSE 0 END), 0) -
            ISNULL(SUM(CASE WHEN io.Kind = 'SR' THEN io.SumAmt ELSE 0 END), 0) AS OpenAR,
            COUNT(DISTINCT CASE WHEN io.Kind IN ('SD','SI','POS') THEN io.IOID END) AS DocCount,
            MAX(io.IoDate) AS LastSaleDate
        FROM tStk_IO io
        LEFT JOIN tBas_Cust c ON c.CustID = io.CustID
        WHERE io.State = 'S'
          AND io.CustID IS NOT NULL
          AND io.Kind IN ('SD','SI','POS','SR')
    "#
    .to_string();

    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(&format!(
                " AND (c.CustName LIKE @p{} OR io.CustID = CAST(@p{} AS uniqueidentifier))",
                pidx, pidx + 1
            ));
            pidx += 2;
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(kw.clone()));
        }
    }

    base_query.push_str(
        " GROUP BY io.CustID, c.CustName \
          HAVING (ISNULL(SUM(CASE WHEN io.Kind IN ('SD','SI','POS') THEN io.SumAmt ELSE 0 END), 0) - \
                  ISNULL(SUM(CASE WHEN io.Kind = 'SR' THEN io.SumAmt ELSE 0 END), 0)) <> 0 \
          ORDER BY OpenAR DESC"
    );

    let count_sql = format!("SELECT COUNT(*) AS cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(
        &base_query,
        page,
        page_size,
        params.sort_prop.as_deref(),
        params.sort_order.as_deref(),
    );
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params
        .iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();

    let mut total: i32 = 0;
    let count_stream = conn.query(&count_sql, &param_refs).await?;
    if let Some(row) = count_stream.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }

    let data_stream = conn.query(&paginated_sql, &param_refs).await?;
    let rows: Vec<Row> = data_stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok_paginated(data, total as u64, page, page_size)))
}

/// 单客户应收明细（单据级）
#[derive(Deserialize)]
pub struct CustomerARDetailQuery {
    pub cust_id: String,
}

pub async fn get_customer_ar_detail(
    State(_config): State<Config>,
    Query(params): Query<CustomerARDetailQuery>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let cust_id = &params.cust_id;

    // 派生：列出该客户所有 SD/SI/POS + SR 单据（已审）
    let sql = r#"
        SELECT
            io.IOID, io.IONo, io.IoDate, io.Kind, io.SumAmt, io.SumQty,
            io.Note, io.State,
            c.CustName
        FROM tStk_IO io
        LEFT JOIN tBas_Cust c ON c.CustID = io.CustID
        WHERE io.CustID = @p1
          AND io.Kind IN ('SD','SI','POS','SR')
          AND io.State = 'S'
        ORDER BY io.IoDate DESC
    "#;
    let stream = conn.query(sql, &[&cust_id.as_str()]).await?;
    let rows: Vec<Row> = stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
    Ok(Json(ApiResponse::ok(data)))
}

/// 单供应商应付汇总
pub async fn get_supplier_ap(
    State(_config): State<Config>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);

    let mut base_query = r#"
        SELECT
            io.SuppID,
            s.SuppName,
            ISNULL(SUM(CASE WHEN io.Kind IN ('PD','RI') THEN io.SumAmt ELSE 0 END), 0) AS PurchaseAmt,
            ISNULL(SUM(CASE WHEN io.Kind IN ('PR','TH') THEN io.SumAmt ELSE 0 END), 0) AS ReturnAmt,
            ISNULL(SUM(CASE WHEN io.Kind IN ('PD','RI') THEN io.SumAmt ELSE 0 END), 0) -
            ISNULL(SUM(CASE WHEN io.Kind IN ('PR','TH') THEN io.SumAmt ELSE 0 END), 0) AS OpenAP,
            COUNT(DISTINCT CASE WHEN io.Kind IN ('PD','RI') THEN io.IOID END) AS DocCount,
            MAX(io.IoDate) AS LastPurchaseDate
        FROM tStk_IO io
        LEFT JOIN tBas_Supp s ON s.SuppID = io.SuppID
        WHERE io.State = 'S'
          AND io.SuppID IS NOT NULL
          AND io.Kind IN ('PD','RI','PR','TH')
    "#
    .to_string();

    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(&format!(
                " AND (s.SuppName LIKE @p{} OR io.SuppID = CAST(@p{} AS uniqueidentifier))",
                pidx, pidx + 1
            ));
            pidx += 2;
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(kw.clone()));
        }
    }

    base_query.push_str(
        " GROUP BY io.SuppID, s.SuppName \
          HAVING (ISNULL(SUM(CASE WHEN io.Kind IN ('PD','RI') THEN io.SumAmt ELSE 0 END), 0) - \
                  ISNULL(SUM(CASE WHEN io.Kind IN ('PR','TH') THEN io.SumAmt ELSE 0 END), 0)) <> 0 \
          ORDER BY OpenAP DESC"
    );

    let count_sql = format!("SELECT COUNT(*) AS cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(
        &base_query,
        page,
        page_size,
        params.sort_prop.as_deref(),
        params.sort_order.as_deref(),
    );
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params
        .iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();

    let mut total: i32 = 0;
    let count_stream = conn.query(&count_sql, &param_refs).await?;
    if let Some(row) = count_stream.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }

    let data_stream = conn.query(&paginated_sql, &param_refs).await?;
    let rows: Vec<Row> = data_stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok_paginated(data, total as u64, page, page_size)))
}

/// 单供应商应付明细
pub async fn get_supplier_ap_detail(
    State(_config): State<Config>,
    Query(params): Query<CustomerARDetailQuery>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let supp_id = &params.cust_id; // 复用 struct 字段

    let sql = r#"
        SELECT
            io.IOID, io.IONo, io.IoDate, io.Kind, io.SumAmt, io.SumQty,
            io.Note, io.State,
            s.SuppName
        FROM tStk_IO io
        LEFT JOIN tBas_Supp s ON s.SuppID = io.SuppID
        WHERE io.SuppID = @p1
          AND io.Kind IN ('PD','RI','PR','TH')
          AND io.State = 'S'
        ORDER BY io.IoDate DESC
    "#;
    let stream = conn.query(sql, &[&supp_id.as_str()]).await?;
    let rows: Vec<Row> = stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
    Ok(Json(ApiResponse::ok(data)))
}
