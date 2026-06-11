use axum::{
    extract::State,
    Extension,
    Json,
};
use serde::Deserialize;
use tiberius::Row;
use crate::config::Config;
use crate::db::get_pool;
use crate::error::Result;
use crate::utils::{ApiResponse, build_pagination_sql_with_sort};
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
pub struct GetPrintTemplatesParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
    pub doc_type: Option<String>,
    pub sort_prop: Option<String>,
    pub sort_order: Option<String>,
}

pub async fn get_print_templates(
    State(_config): State<Config>,
    Json(params): Json<GetPrintTemplatesParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(50), 200);

    let mut base_query = "SELECT t.RptID, t.RptDesc AS TemplateName, t.RptCode AS DocType, t.State, t.EDate, t.EUser, t.Note AS Remark FROM tSys_Rpt t WHERE t.State <> 'D'".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(&format!(" AND (t.RptDesc LIKE @p{} OR t.RptCode LIKE @p{})", pidx, pidx + 1));
            pidx += 2;
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
        }
    }

    if let Some(dt) = &params.doc_type {
        if !dt.is_empty() {
            base_query.push_str(&format!(" AND t.RptCode = @p{}", pidx));
            pidx += 1;
            query_params.push(Some(dt.clone()));
        }
    }

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, params.sort_prop.as_deref(), params.sort_order.as_deref());
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();

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

#[derive(Deserialize)]
pub struct GetPrintTemplateParams {
    pub template_id: String,
}

pub async fn get_print_template(
    State(_config): State<Config>,
    Json(params): Json<GetPrintTemplateParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let sql = "SELECT RptID, RptDesc AS TemplateName, RptCode AS DocType, RptFormat AS Content, State, EDate, EUser, Note AS Remark FROM tSys_Rpt WHERE RptID = @p1";
    let stream = conn.query(sql, &[&params.template_id.as_str()]).await?;

    if let Some(row) = stream.into_row().await? {
        Ok(Json(ApiResponse::ok(row_to_json(&row))))
    } else {
        Ok(Json(ApiResponse::err("模板不存在")))
    }
}

#[derive(Deserialize)]
pub struct CreatePrintTemplateParams {
    pub TemplateName: Option<String>,
    pub DocType: Option<String>,
    pub Content: Option<String>,
    pub Remark: Option<String>,
}

pub async fn create_print_template(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<CreatePrintTemplateParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    // 用字符串格式而非 NaiveDateTime，规避 tiberius chrono 绑定兼容性问题
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let template_name = body.TemplateName.as_deref().unwrap_or("");
    let doc_type = body.DocType.as_deref().unwrap_or("");
    let content_str = body.Content.as_deref().unwrap_or("");
    let content_bytes = content_str.as_bytes();
    let remark = body.Remark.as_deref().unwrap_or("");

    let sql = r#"INSERT INTO tSys_Rpt (RptID, RptDesc, RptCode, ToolsType, RptFormat, Note, State, EDate, EUser)
        VALUES (NEWID(), @p1, @p2, 'R', @p3, @p4, 'A', @p5, @p6)"#;

    conn.execute(sql, &[
        &template_name,
        &doc_type,
        &content_bytes,
        &remark,
        &now,
        &claims.user_code.as_str(),
    ]).await?;

    Ok(Json(ApiResponse::msg("打印模板创建成功")))
}

#[derive(Deserialize)]
pub struct UpdatePrintTemplateParams {
    pub TemplateID: String,
    pub TemplateName: Option<String>,
    pub DocType: Option<String>,
    pub Content: Option<String>,
    pub Remark: Option<String>,
}

pub async fn update_print_template(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<UpdatePrintTemplateParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    // 用字符串格式而非 NaiveDateTime，规避 tiberius chrono 绑定兼容性问题
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let template_name = body.TemplateName.as_deref().unwrap_or("");
    let doc_type = body.DocType.as_deref().unwrap_or("");
    let content_str = body.Content.as_deref().unwrap_or("");
    let content_bytes = content_str.as_bytes();
    let remark = body.Remark.as_deref().unwrap_or("");

    let sql = r#"UPDATE tSys_Rpt SET
        RptDesc = @p1, RptCode = @p2, RptFormat = @p3, Note = @p4,
        EDate = @p5, EUser = @p6
        WHERE RptID = @p7"#;

    conn.execute(sql, &[
        &template_name,
        &doc_type,
        &content_bytes,
        &remark,
        &now,
        &claims.user_code.as_str(),
        &body.TemplateID.as_str(),
    ]).await?;

    Ok(Json(ApiResponse::msg("打印模板更新成功")))
}

#[derive(Deserialize)]
pub struct DeletePrintTemplateParams {
    pub ids: Vec<String>,
}

pub async fn delete_print_template(
    State(_config): State<Config>,
    Json(body): Json<DeletePrintTemplateParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    if body.ids.is_empty() {
        return Ok(Json(ApiResponse::err("请选择要删除的模板")));
    }

    for id in &body.ids {
        let sql = "UPDATE tSys_Rpt SET State = 'D' WHERE RptID = @p1";
        conn.execute(sql, &[&id.as_str()]).await?;
    }

    Ok(Json(ApiResponse::msg(&format!("成功删除{}个模板", body.ids.len()))))
}

#[derive(Deserialize)]
pub struct GetPrintLogsParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
    pub doc_type: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub sort_prop: Option<String>,
    pub sort_order: Option<String>,
}

pub async fn get_print_logs(
    State(_config): State<Config>,
    Json(params): Json<GetPrintLogsParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(50), 200);

    let mut base_query = r#"SELECT h.DocID, h.PrintDate, h.PrintRptID, h.PrintEmpID, h.PrintComName,
        p.PrintNum, p.LastPrintDate, p.LastPrintEmpID, p.LastPrintComName
        FROM tSys_RptPrintHis h
        LEFT JOIN tSys_RptPrintNum p ON h.DocID = p.DocID
        WHERE 1=1"#.to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    if let Some(sd) = &params.start_date {
        if !sd.is_empty() {
            base_query.push_str(&format!(" AND h.PrintDate >= @p{}", pidx));
            pidx += 1;
            query_params.push(Some(sd.clone()));
        }
    }

    if let Some(ed) = &params.end_date {
        if !ed.is_empty() {
            base_query.push_str(&format!(" AND h.PrintDate <= @p{}", pidx));
            pidx += 1;
            query_params.push(Some(ed.clone()));
        }
    }

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, params.sort_prop.as_deref(), params.sort_order.as_deref());
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();

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

#[derive(Deserialize)]
pub struct CreatePrintLogParams {
    pub DocID: String,
    pub PrintRptID: Option<String>,
    pub PrintEmpID: Option<String>,
    pub PrintComName: Option<String>,
}

pub async fn create_print_log(
    Extension(_claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<CreatePrintLogParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().naive_local();

    let print_rpt_id = body.PrintRptID.as_deref().unwrap_or("");
    let print_emp_id = body.PrintEmpID.as_deref().unwrap_or("");
    let print_com_name = body.PrintComName.as_deref().unwrap_or("");

    let his_sql = r#"INSERT INTO tSys_RptPrintHis (DocID, PrintDate, PrintRptID, PrintEmpID, PrintComName)
        VALUES (@p1, @p2, @p3, @p4, @p5)"#;
    conn.execute(his_sql, &[
        &body.DocID.as_str(),
        &now,
        &print_rpt_id,
        &print_emp_id,
        &print_com_name,
    ]).await?;

    let num_check = "SELECT COUNT(*) as cnt FROM tSys_RptPrintNum WHERE DocID = @p1";
    let stream = conn.query(num_check, &[&body.DocID.as_str()]).await?;
    let mut exists = false;
    if let Some(row) = stream.into_row().await? {
        exists = row.get::<i32, _>("cnt").unwrap_or(0) > 0;
    }

    if exists {
        let num_sql = "UPDATE tSys_RptPrintNum SET PrintNum = PrintNum + 1, LastPrintDate = @p1, LastPrintEmpID = @p2, LastPrintComName = @p3 WHERE DocID = @p4";
        conn.execute(num_sql, &[&now, &print_emp_id, &print_com_name, &body.DocID.as_str()]).await?;
    } else {
        let num_sql = r#"INSERT INTO tSys_RptPrintNum (DocID, PrintNum, LastPrintDate, LastPrintEmpID, LastPrintComName)
            VALUES (@p1, 1, @p2, @p3, @p4)"#;
        conn.execute(num_sql, &[&body.DocID.as_str(), &now, &print_emp_id, &print_com_name]).await?;
    }

    Ok(Json(ApiResponse::msg("打印记录已保存")))
}

pub async fn get_print_config(
    State(_config): State<Config>,
    Json(_params): Json<serde_json::Value>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let sql = "SELECT RptID, RptDesc, RptCode, State FROM tSys_Rpt WHERE State <> 'D' AND ToolsType = 'R'";
    let stream = conn.query(sql, &[]).await?;
    let rows: Vec<Row> = stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
    Ok(Json(ApiResponse::ok(data)))
}

pub async fn save_print_config(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let user_code = claims.user_code.as_str();

    if let Some(configs) = body.get("configs").and_then(|v| v.as_array()) {
        for cfg in configs {
            let rpt_id = cfg.get("RptID").and_then(|v| v.as_str()).unwrap_or("");
            let state = cfg.get("State").and_then(|v| v.as_str()).unwrap_or("A");
            if rpt_id.is_empty() { continue; }
            let sql = "UPDATE tSys_Rpt SET State = @p1, EDate = @p2, EUser = @p3 WHERE RptID = @p4";
            conn.execute(sql, &[&state, &now, &user_code, &rpt_id]).await?;
        }
    }

    Ok(Json(ApiResponse::msg("打印配置已保存")))
}

pub async fn get_print_versions(
    State(_config): State<Config>,
    Json(params): Json<serde_json::Value>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let rpt_id = params.get("template_id").and_then(|v| v.as_str()).unwrap_or("");
    if rpt_id.is_empty() {
        return Ok(Json(ApiResponse::ok(vec![])));
    }

    let sql = r#"SELECT VersionID, RptID, VersionNo, RptDesc, RptCode, Note, EDate, EUser, SnapshotName
        FROM tSys_RptVersion WHERE RptID = @p1 ORDER BY VersionNo DESC"#;
    let stream = conn.query(sql, &[&rpt_id]).await?;
    let rows: Vec<Row> = stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
    Ok(Json(ApiResponse::ok(data)))
}

pub async fn create_print_version(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let user_code = claims.user_code.as_str();

    let rpt_id = body.get("template_id").and_then(|v| v.as_str()).unwrap_or("");
    let snapshot_name = body.get("snapshot_name").and_then(|v| v.as_str()).unwrap_or("");
    if rpt_id.is_empty() {
        return Ok(Json(ApiResponse::err("模板ID不能为空")));
    }

    // Get current template data
    let src_sql = "SELECT RptDesc, RptCode, RptFormat, Note FROM tSys_Rpt WHERE RptID = @p1";
    let stream = conn.query(src_sql, &[&rpt_id]).await?;
    let row = match stream.into_row().await? {
        Some(r) => r,
        None => return Ok(Json(ApiResponse::err("模板不存在"))),
    };

    let rpt_desc: Option<&str> = row.try_get("RptDesc").unwrap_or(None);
    let rpt_code: Option<&str> = row.try_get("RptCode").unwrap_or(None);
    let rpt_format: Option<&[u8]> = row.try_get("RptFormat").unwrap_or(None);
    let note: Option<&str> = row.try_get("Note").unwrap_or(None);

    // Get next version number
    let ver_sql = "SELECT ISNULL(MAX(VersionNo), 0) + 1 AS NextVer FROM tSys_RptVersion WHERE RptID = @p1";
    let ver_stream = conn.query(ver_sql, &[&rpt_id]).await?;
    let next_ver: i32 = match ver_stream.into_row().await? {
        Some(r) => r.get::<i32, _>("NextVer").unwrap_or(1),
        None => 1,
    };

    let insert_sql = r#"INSERT INTO tSys_RptVersion (VersionID, RptID, VersionNo, RptDesc, RptCode, RptFormat, Note, EDate, EUser, SnapshotName)
        VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9)"#;
    conn.execute(insert_sql, &[
        &rpt_id,
        &next_ver,
        &rpt_desc,
        &rpt_code,
        &rpt_format,
        &note,
        &now,
        &user_code,
        &snapshot_name,
    ]).await?;

    Ok(Json(ApiResponse::msg(&format!("版本 v{} 已创建", next_ver))))
}

pub async fn rollback_print_version(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let user_code = claims.user_code.as_str();

    let rpt_id = body.get("template_id").and_then(|v| v.as_str()).unwrap_or("");
    let version_id = body.get("version_id").and_then(|v| v.as_str()).unwrap_or("");
    if rpt_id.is_empty() || version_id.is_empty() {
        return Ok(Json(ApiResponse::err("参数不完整")));
    }

    // Get version snapshot
    let ver_sql = "SELECT RptDesc, RptCode, RptFormat, Note FROM tSys_RptVersion WHERE VersionID = @p1 AND RptID = @p2";
    let stream = conn.query(ver_sql, &[&version_id, &rpt_id]).await?;
    let row = match stream.into_row().await? {
        Some(r) => r,
        None => return Ok(Json(ApiResponse::err("版本不存在"))),
    };

    let rpt_desc: Option<&str> = row.try_get("RptDesc").unwrap_or(None);
    let rpt_code: Option<&str> = row.try_get("RptCode").unwrap_or(None);
    let rpt_format: Option<&[u8]> = row.try_get("RptFormat").unwrap_or(None);
    let note: Option<&str> = row.try_get("Note").unwrap_or(None);

    // Auto-create a version snapshot before rollback
    let ver_num_sql = "SELECT ISNULL(MAX(VersionNo), 0) + 1 AS NextVer FROM tSys_RptVersion WHERE RptID = @p1";
    let ver_stream = conn.query(ver_num_sql, &[&rpt_id]).await?;
    let next_ver: i32 = match ver_stream.into_row().await? {
        Some(r) => r.get::<i32, _>("NextVer").unwrap_or(1),
        None => 1,
    };

    // Get current template data for backup
    let src_sql = "SELECT RptDesc, RptCode, RptFormat, Note FROM tSys_Rpt WHERE RptID = @p1";
    let src_stream = conn.query(src_sql, &[&rpt_id]).await?;
    if let Some(src_row) = src_stream.into_row().await? {
        let cur_desc: Option<&str> = src_row.try_get("RptDesc").unwrap_or(None);
        let cur_code: Option<&str> = src_row.try_get("RptCode").unwrap_or(None);
        let cur_fmt: Option<&[u8]> = src_row.try_get("RptFormat").unwrap_or(None);
        let cur_note: Option<&str> = src_row.try_get("Note").unwrap_or(None);
        let backup_name = format!("回滚前自动备份 v{}", next_ver);
        let bak_sql = r#"INSERT INTO tSys_RptVersion (VersionID, RptID, VersionNo, RptDesc, RptCode, RptFormat, Note, EDate, EUser, SnapshotName)
            VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9)"#;
        conn.execute(bak_sql, &[
            &rpt_id, &next_ver, &cur_desc, &cur_code, &cur_fmt, &cur_note, &now, &user_code, &backup_name,
        ]).await?;
    }

    // Restore version to current template
    let update_sql = r#"UPDATE tSys_Rpt SET RptDesc = @p1, RptCode = @p2, RptFormat = @p3, Note = @p4, EDate = @p5, EUser = @p6
        WHERE RptID = @p7"#;
    conn.execute(update_sql, &[
        &rpt_desc, &rpt_code, &rpt_format, &note, &now, &user_code, &rpt_id,
    ]).await?;

    Ok(Json(ApiResponse::msg("已回滚到指定版本")))
}
