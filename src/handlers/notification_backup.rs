use crate::config::Config;
use crate::db::get_pool;
use crate::error::Result;
use crate::handlers::base_data::row_to_json;
use crate::middleware::auth::Claims;
use crate::utils::{ApiResponse, build_pagination_sql_with_sort, row_get_f64};
use axum::{Extension, Json, extract::State};
use serde::Deserialize;
use tiberius::Row;

#[derive(Deserialize)]
pub struct GetNotificationsParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
    pub notify_type: Option<String>,
    pub is_read: Option<String>,
    pub sort_prop: Option<String>,
    pub sort_order: Option<String>,
}

pub async fn get_notifications(
    Extension(_claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<GetNotificationsParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(50), 1000);

    let mut base_query = r#"SELECT m.MsgID, m.TEmpID, m.FEmpID, m.Msg AS Content, m.MsgType,
        m.State AS IsRead, m.SDate AS CreateDate, m.RDate AS ReadDate,
        m.ProcName, m.MenuID, m.DocID, m.MsgLevel, m.ForceMsg,
        e.EmpName AS ToUserName, fe.EmpName AS FromUserName
        FROM tSys_Msg m
        LEFT JOIN tBas_Emp e ON m.TEmpID = e.EmpID
        LEFT JOIN tBas_Emp fe ON m.FEmpID = fe.EmpID
        WHERE 1=1"#
        .to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    if let Some(ir) = &params.is_read {
        if !ir.is_empty() {
            base_query.push_str(&format!(" AND m.State = @p{}", pidx));
            pidx += 1;
            query_params.push(Some(ir.clone()));
        }
    }

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(&format!(" AND (m.Msg LIKE @p{})", pidx));
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
pub struct CreateNotificationParams {
    pub TEmpID: String,
    pub Msg: String,
    pub MsgType: Option<i32>,
    pub FEmpID: Option<String>,
    pub ProcName: Option<String>,
    pub MenuID: Option<String>,
    pub DocID: Option<String>,
    pub ForceMsg: Option<String>,
    pub MsgLevel: Option<i32>,
}

pub async fn create_notification(
    State(_config): State<Config>,
    Json(body): Json<CreateNotificationParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let msg_type = body.MsgType.unwrap_or(0);
    let f_emp_id = body.FEmpID.as_deref().unwrap_or("");
    let proc_name = body.ProcName.as_deref().unwrap_or("");
    let menu_id = body.MenuID.as_deref().unwrap_or("");
    let doc_id = body.DocID.as_deref().unwrap_or("");
    let force_msg = body.ForceMsg.as_deref().unwrap_or("N");
    let msg_level = body.MsgLevel.unwrap_or(0);

    let sql = r#"INSERT INTO tSys_Msg (MsgID, TEmpID, FEmpID, Msg, MsgType, State, SDate, ProcName, MenuID, DocID, ForceMsg, MsgLevel)
        VALUES (NEWID(), @p1, @p2, @p3, @p4, 'N', @p5, @p6, @p7, @p8, @p9, @p10)"#;

    conn.execute(
        sql,
        &[
            &body.TEmpID.as_str(),
            &f_emp_id,
            &body.Msg.as_str(),
            &msg_type,
            &now,
            &proc_name,
            &menu_id,
            &doc_id,
            &force_msg,
            &msg_level,
        ],
    )
    .await?;

    Ok(Json(ApiResponse::msg("通知创建成功")))
}

#[derive(Deserialize)]
pub struct MarkNotificationReadParams {
    pub notify_ids: Vec<String>,
}

pub async fn mark_notification_read(
    Extension(_claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<MarkNotificationReadParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    if body.notify_ids.is_empty() {
        return Ok(Json(ApiResponse::err("请选择要标记的通知")));
    }

    for id in &body.notify_ids {
        let sql = "UPDATE tSys_Msg SET State = 'Y', RDate = @p1 WHERE MsgID = @p2";
        conn.execute(sql, &[&now, &id.as_str()]).await?;
    }

    Ok(Json(ApiResponse::msg(&format!(
        "成功标记{}条通知为已读",
        body.notify_ids.len()
    ))))
}

pub async fn get_unread_count(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let sql = "SELECT COUNT(*) as cnt FROM tSys_Msg WHERE TEmpID IN (SELECT EmpID FROM tBas_Emp WHERE EmpNo = @p1) AND State = 'N'";
    let stream = conn.query(sql, &[&claims.user_code.as_str()]).await?;

    let mut count: i32 = 0;
    if let Some(row) = stream.into_row().await? {
        count = row.get::<i32, _>("cnt").unwrap_or(0);
    }

    Ok(Json(ApiResponse::ok(
        serde_json::json!({ "unread_count": count }),
    )))
}

#[derive(Deserialize)]
pub struct GetSystemConfigParams {
    pub config_key: Option<String>,
    pub p_kind: Option<String>,
}

pub async fn get_system_config(
    State(_config): State<Config>,
    Json(params): Json<GetSystemConfigParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;

    let mut sql = "SELECT ParametersID, PCode, PName, PKind, PHelp, PValue, PTerm, EUser, EDate FROM tSys_Parameters WHERE 1=1".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    if let Some(ck) = &params.config_key {
        if !ck.is_empty() {
            sql.push_str(&format!(" AND PCode = @p{}", pidx));
            pidx += 1;
            query_params.push(Some(ck.clone()));
        }
    }

    if let Some(pk) = &params.p_kind {
        if !pk.is_empty() {
            sql.push_str(&format!(" AND PKind = @p{}", pidx));
            query_params.push(Some(pk.clone()));
        }
    }

    sql.push_str(" ORDER BY PKind, PCode");

    let param_refs: Vec<&dyn tiberius::ToSql> = query_params
        .iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();
    let stream = conn.query(&sql, &param_refs).await?;
    let rows: Vec<Row> = stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok(data)))
}

#[derive(Deserialize)]
pub struct SaveSystemConfigParams {
    pub PCode: String,
    pub PValue: String,
    pub PName: Option<String>,
    pub PKind: Option<String>,
    pub PHelp: Option<String>,
}

pub async fn save_system_config(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<SaveSystemConfigParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let check_sql = "SELECT COUNT(*) as cnt FROM tSys_Parameters WHERE PCode = @p1";
    let stream = conn.query(check_sql, &[&body.PCode.as_str()]).await?;
    let mut exists = false;
    if let Some(row) = stream.into_row().await? {
        let cnt: i32 = row.get::<i32, _>("cnt").unwrap_or(0);
        exists = cnt > 0;
    }

    if exists {
        let sql = "UPDATE tSys_Parameters SET PValue = @p1, PName = @p2, PKind = @p3, PHelp = @p4, EDate = @p5 WHERE PCode = @p6";
        let p_name = body.PName.as_deref().unwrap_or("");
        let p_kind = body.PKind.as_deref().unwrap_or("");
        let p_help = body.PHelp.as_deref().unwrap_or("");
        conn.execute(
            sql,
            &[
                &body.PValue.as_str(),
                &p_name,
                &p_kind,
                &p_help,
                &now,
                &body.PCode.as_str(),
            ],
        )
        .await?;
    } else {
        let sql = r#"INSERT INTO tSys_Parameters (ParametersID, PCode, PName, PKind, PHelp, PValue, EUser, EDate)
            VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7)"#;
        let p_name = body.PName.as_deref().unwrap_or(&body.PCode);
        let p_kind = body.PKind.as_deref().unwrap_or("system");
        let p_help = body.PHelp.as_deref().unwrap_or("");
        conn.execute(
            sql,
            &[
                &body.PCode.as_str(),
                &p_name,
                &p_kind,
                &p_help,
                &body.PValue.as_str(),
                &claims.user_code.as_str(),
                &now,
            ],
        )
        .await?;
    }

    Ok(Json(ApiResponse::msg("系统配置保存成功")))
}

#[derive(Deserialize)]
pub struct GetDashboardStatsParams {
    pub period: Option<String>,
}

pub async fn get_dashboard_stats(
    State(_config): State<Config>,
    Json(_params): Json<GetDashboardStatsParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let mut stats = serde_json::Map::new();

    let product_count_sql = "SELECT COUNT(*) as cnt FROM tBas_Goods WHERE State <> 'D'";
    if let Some(row) = conn.query(product_count_sql, &[]).await?.into_row().await? {
        stats.insert(
            "product_count".to_string(),
            serde_json::Value::Number(serde_json::Number::from(
                row.get::<i32, _>("cnt").unwrap_or(0),
            )),
        );
    }

    let customer_count_sql = "SELECT COUNT(*) as cnt FROM tBas_Cust WHERE State <> 'D'";
    if let Some(row) = conn
        .query(customer_count_sql, &[])
        .await?
        .into_row()
        .await?
    {
        stats.insert(
            "customer_count".to_string(),
            serde_json::Value::Number(serde_json::Number::from(
                row.get::<i32, _>("cnt").unwrap_or(0),
            )),
        );
    }

    let supplier_count_sql = "SELECT COUNT(*) as cnt FROM tBas_Supp WHERE State <> 'D'";
    if let Some(row) = conn
        .query(supplier_count_sql, &[])
        .await?
        .into_row()
        .await?
    {
        stats.insert(
            "supplier_count".to_string(),
            serde_json::Value::Number(serde_json::Number::from(
                row.get::<i32, _>("cnt").unwrap_or(0),
            )),
        );
    }

    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let this_month_start = chrono::Local::now().format("%Y-%m-01").to_string();

    let today_sales_sql = "SELECT ISNULL(SUM(SumAmt), 0) as total FROM tSal_Inv WHERE State <> 'D' AND CONVERT(varchar(10), EDate, 120) = @p1";
    if let Some(row) = conn
        .query(today_sales_sql, &[&today.as_str()])
        .await?
        .into_row()
        .await?
    {
        let total: f64 = row_get_f64(&row, "total");
        stats.insert(
            "today_sales".to_string(),
            serde_json::Number::from_f64(total)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
        );
    }

    let month_sales_sql = "SELECT ISNULL(SUM(SumAmt), 0) as total FROM tSal_Inv WHERE State <> 'D' AND CONVERT(varchar(10), EDate, 120) >= @p1";
    if let Some(row) = conn
        .query(month_sales_sql, &[&this_month_start.as_str()])
        .await?
        .into_row()
        .await?
    {
        let total: f64 = row_get_f64(&row, "total");
        stats.insert(
            "month_sales".to_string(),
            serde_json::Number::from_f64(total)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
        );
    }

    let pending_order_sql = "SELECT COUNT(*) as cnt FROM tPur_Order WHERE State = 'N'";
    if let Some(row) = conn.query(pending_order_sql, &[]).await?.into_row().await? {
        stats.insert(
            "pending_order_count".to_string(),
            serde_json::Value::Number(serde_json::Number::from(
                row.get::<i32, _>("cnt").unwrap_or(0),
            )),
        );
    }

    let unread_msg_sql = "SELECT COUNT(*) as cnt FROM tSys_Msg WHERE State = 'N'";
    if let Some(row) = conn.query(unread_msg_sql, &[]).await?.into_row().await? {
        stats.insert(
            "unread_msg_count".to_string(),
            serde_json::Value::Number(serde_json::Number::from(
                row.get::<i32, _>("cnt").unwrap_or(0),
            )),
        );
    }

    let sales_trend_sql = r#"SELECT CONVERT(varchar(10), EDate, 120) as SaleDate, ISNULL(SUM(SumAmt), 0) as SumAmt
        FROM tSal_Inv WHERE State <> 'D' AND EDate >= DATEADD(day, -30, GETDATE())
        GROUP BY CONVERT(varchar(10), EDate, 120) ORDER BY SaleDate"#;
    let trend_stream = conn.query(sales_trend_sql, &[]).await?;
    let trend_rows: Vec<Row> = trend_stream.into_first_result().await?;
    let sales_trend: Vec<serde_json::Value> = trend_rows
        .iter()
        .map(|r| {
            let date = r.get::<&str, _>("SaleDate").unwrap_or("").to_string();
            let amt = row_get_f64(&r, "SumAmt");
            serde_json::json!({ "date": date, "amount": amt })
        })
        .collect();
    stats.insert(
        "sales_trend".to_string(),
        serde_json::Value::Array(sales_trend),
    );

    Ok(Json(ApiResponse::ok(serde_json::Value::Object(stats))))
}
