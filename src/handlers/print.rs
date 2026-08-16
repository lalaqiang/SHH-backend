use axum::{
    extract::State,
    Extension,
    Json,
};
use serde::Deserialize;
use tiberius::Row;
use crate::config::Config;
use crate::db::get_pool;
use crate::error::{AppError, Result};
use crate::utils::{ApiResponse, build_pagination_sql_with_sort};
use crate::handlers::base_data::row_to_json;
use crate::middleware::auth::Claims;

/// 零 UUID，用于 EUser 等 uniqueidentifier 字段的默认值
const ZERO_UUID: &str = "00000000-0000-0000-0000-000000000000";

#[derive(Deserialize)]
pub struct GetPrintTemplatesParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
    pub doc_type: Option<String>,
    pub kind: Option<String>,
    pub sort_prop: Option<String>,
    pub sort_order: Option<String>,
    /// 是否只显示已删除（State='D'）的模板
    /// - true:  WHERE t.State = 'D'  （显示已删除）
    /// - false/None: WHERE t.State <> 'D'  （默认，不显示已删除）
    #[serde(default)]
    pub only_deleted: bool,
}

pub async fn get_print_templates(
    State(_config): State<Config>,
    Json(params): Json<GetPrintTemplatesParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(50), 1000);

    // ★ 根据 only_deleted 参数决定查询条件：
    //   - only_deleted=true: 只查已删除的模板（State='D'）
    //   - only_deleted=false: 只查未删除的模板（State<>'D'）
    let state_filter = if params.only_deleted {
        "t.State = 'D'"
    } else {
        "t.State <> 'D'"
    };
    let mut base_query = format!("SELECT t.RptID, t.RptTitleID, t.RptDesc AS TemplateName, t.RptCode AS DocType, \
        t.ToolsType, t.FlgTerm, t.RptFormat AS Content, t.Note AS Remark, t.State, t.Kind, t.ShareAll, t.GridID, t.TermID, \
        t.RptHistory, t.ExecSQL, t.ExecFields, t.SaveTables, t.SaveTableName, t.SaveTableKeyFields, \
        t.AllowAdd, t.DefValueSQL, t.ChangeFrmFlg, t.LUTime, t.EDate, t.EUser, t.AUser, t.ADate, \
        t.SUser, t.SDate, eu.[EmpName] AS [EUserName], au.[EmpName] AS [AUserName], su.[EmpName] AS [SUserName] \
        FROM tSys_Rpt t \
        LEFT JOIN tBas_Emp eu ON t.[EUser] = eu.[EmpID] \
        LEFT JOIN tBas_Emp au ON t.[AUser] = au.[EmpID] \
        LEFT JOIN tBas_Emp su ON t.[SUser] = su.[EmpID] \
        WHERE {}", state_filter);
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

    // Kind 过滤：'D' = 单据打印模板（新设计器创建），'R' = 旧统计报表，'B' = 条码等
    // 不传或空字符串表示不过滤（返回全部）
    if let Some(k) = &params.kind {
        if !k.is_empty() {
            base_query.push_str(&format!(" AND t.Kind = @p{}", pidx));
            query_params.push(Some(k.clone()));
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

    let sql = "SELECT t.RptID, t.RptTitleID, t.RptDesc AS TemplateName, t.RptCode AS DocType, t.ToolsType, \
        t.FlgTerm, t.RptFormat AS Content, t.Note AS Remark, t.State, t.Kind, t.ShareAll, t.GridID, t.TermID, \
        t.RptHistory, t.ExecSQL, t.ExecFields, t.SaveTables, t.SaveTableName, t.SaveTableKeyFields, \
        t.AllowAdd, t.DefValueSQL, t.ChangeFrmFlg, t.LUTime, t.EDate, t.EUser, t.AUser, t.ADate, t.SUser, t.SDate, \
        eu.[EmpName] AS [EUserName], au.[EmpName] AS [AUserName], su.[EmpName] AS [SUserName] \
        FROM tSys_Rpt t \
        LEFT JOIN tBas_Emp eu ON t.[EUser] = eu.[EmpID] \
        LEFT JOIN tBas_Emp au ON t.[AUser] = au.[EmpID] \
        LEFT JOIN tBas_Emp su ON t.[SUser] = su.[EmpID] \
        WHERE t.RptID = @p1";
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
    pub RptTitleID: Option<String>,
    pub ToolsType: Option<String>,
    pub FlgTerm: Option<String>,
    pub Kind: Option<String>,
    pub ShareAll: Option<String>,
    pub GridID: Option<String>,
    pub TermID: Option<String>,
    pub RptHistory: Option<String>,
    pub ExecSQL: Option<String>,
    pub ExecFields: Option<String>,
    pub SaveTables: Option<String>,
    pub SaveTableName: Option<String>,
    pub SaveTableKeyFields: Option<String>,
    pub AllowAdd: Option<String>,
    pub DefValueSQL: Option<String>,
    pub ChangeFrmFlg: Option<String>,
}

pub async fn create_print_template(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<CreatePrintTemplateParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    // EUser 是 uniqueidentifier，必须用 UUID 字符串，不能用 user_code
    let user_uuid = crate::handlers::generic::cached_lookup_user_uuid(&claims.user_code).await
        .unwrap_or_else(|| ZERO_UUID.to_string());

    let template_name = body.TemplateName.as_deref().unwrap_or("");
    let doc_type = body.DocType.as_deref().unwrap_or("");
    let content_str = body.Content.as_deref().unwrap_or("");
    let content_bytes = content_str.as_bytes();
    let remark = body.Remark.as_deref().unwrap_or("");
    let rpt_title_id = body.RptTitleID.as_deref().unwrap_or("");
    let tools_type = body.ToolsType.as_deref().unwrap_or("R");
    let flg_term = body.FlgTerm.as_deref().unwrap_or("");
    // Kind='D' 标识为新设计器创建的单据打印模板，区别于旧 FastReport 报表（'R'）
    let kind = body.Kind.as_deref().unwrap_or("D");
    let share_all = body.ShareAll.as_deref().unwrap_or("");
    let grid_id = body.GridID.as_deref().unwrap_or("");
    let term_id = body.TermID.as_deref().unwrap_or("");
    let rpt_history = body.RptHistory.as_deref().unwrap_or("");
    let exec_sql = body.ExecSQL.as_deref().unwrap_or("");
    let exec_fields = body.ExecFields.as_deref().unwrap_or("");
    let save_tables = body.SaveTables.as_deref().unwrap_or("");
    let save_table_name = body.SaveTableName.as_deref().unwrap_or("");
    let save_table_key_fields = body.SaveTableKeyFields.as_deref().unwrap_or("");
    let allow_add = body.AllowAdd.as_deref().unwrap_or("N");
    let def_value_sql = body.DefValueSQL.as_deref().unwrap_or("");
    let change_frm_flg = body.ChangeFrmFlg.as_deref().unwrap_or("");

    let sql = r#"INSERT INTO tSys_Rpt (RptID, RptTitleID, RptDesc, RptCode, ToolsType, RptFormat,
        FlgTerm, Note, State, Kind, ShareAll, GridID, TermID, RptHistory, ExecSQL, ExecFields,
        SaveTables, SaveTableName, SaveTableKeyFields, AllowAdd, DefValueSQL, ChangeFrmFlg,
        LUTime, EDate, EUser)
        VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7, 'A', @p8, @p9, @p10, @p11, @p12,
        @p13, @p14, @p15, @p16, @p17, @p18, @p19, @p20, @p21, @p22, @p23)"#;

    // uniqueidentifier 字段不允许空字符串，必须传 NULL（None）
    // RptTitleID(@p1)、GridID(@p10)、TermID(@p11)、EUser(@p23) 是 uniqueidentifier
    let opt_rpt_title_id: Option<&str> = if rpt_title_id.is_empty() { None } else { Some(rpt_title_id) };
    let opt_grid_id: Option<&str> = if grid_id.is_empty() { None } else { Some(grid_id) };
    let opt_term_id: Option<&str> = if term_id.is_empty() { None } else { Some(term_id) };

    conn.execute(sql, &[
        &opt_rpt_title_id,
        &template_name,
        &doc_type,
        &tools_type,
        &content_bytes,
        &flg_term,
        &remark,
        &kind,
        &share_all,
        &opt_grid_id,
        &opt_term_id,
        &rpt_history,
        &exec_sql,
        &exec_fields,
        &save_tables,
        &save_table_name,
        &save_table_key_fields,
        &allow_add,
        &def_value_sql,
        &change_frm_flg,
        &now,
        &now,
        &user_uuid.as_str(),
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
    pub RptTitleID: Option<String>,
    pub ToolsType: Option<String>,
    pub FlgTerm: Option<String>,
    pub Kind: Option<String>,
    pub ShareAll: Option<String>,
    pub GridID: Option<String>,
    pub TermID: Option<String>,
    pub RptHistory: Option<String>,
    pub ExecSQL: Option<String>,
    pub ExecFields: Option<String>,
    pub SaveTables: Option<String>,
    pub SaveTableName: Option<String>,
    pub SaveTableKeyFields: Option<String>,
    pub AllowAdd: Option<String>,
    pub DefValueSQL: Option<String>,
    pub ChangeFrmFlg: Option<String>,
    pub State: Option<String>,
}

pub async fn update_print_template(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<UpdatePrintTemplateParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let user_uuid = crate::handlers::generic::cached_lookup_user_uuid(&claims.user_code).await
        .unwrap_or_else(|| ZERO_UUID.to_string());

    let template_name = body.TemplateName.as_deref().unwrap_or("");
    let doc_type = body.DocType.as_deref().unwrap_or("");
    let content_str = body.Content.as_deref().unwrap_or("");
    let content_bytes = content_str.as_bytes();
    let remark = body.Remark.as_deref().unwrap_or("");
    let rpt_title_id = body.RptTitleID.as_deref().unwrap_or("");
    let tools_type = body.ToolsType.as_deref().unwrap_or("R");
    let flg_term = body.FlgTerm.as_deref().unwrap_or("");
    // 更新时保留原 Kind（不强制覆盖），默认值为 'D'（单据打印模板）
    let kind = body.Kind.as_deref().unwrap_or("D");
    let share_all = body.ShareAll.as_deref().unwrap_or("");
    let grid_id = body.GridID.as_deref().unwrap_or("");
    let term_id = body.TermID.as_deref().unwrap_or("");
    let rpt_history = body.RptHistory.as_deref().unwrap_or("");
    let exec_sql = body.ExecSQL.as_deref().unwrap_or("");
    let exec_fields = body.ExecFields.as_deref().unwrap_or("");
    let save_tables = body.SaveTables.as_deref().unwrap_or("");
    let save_table_name = body.SaveTableName.as_deref().unwrap_or("");
    let save_table_key_fields = body.SaveTableKeyFields.as_deref().unwrap_or("");
    let allow_add = body.AllowAdd.as_deref().unwrap_or("N");
    let def_value_sql = body.DefValueSQL.as_deref().unwrap_or("");
    let change_frm_flg = body.ChangeFrmFlg.as_deref().unwrap_or("");
    let state = body.State.as_deref().unwrap_or("A");

    let sql = r#"UPDATE tSys_Rpt SET
        RptTitleID = @p1, RptDesc = @p2, RptCode = @p3, ToolsType = @p4, RptFormat = @p5,
        FlgTerm = @p6, Note = @p7, State = @p8, Kind = @p9, ShareAll = @p10, GridID = @p11,
        TermID = @p12, RptHistory = @p13, ExecSQL = @p14, ExecFields = @p15, SaveTables = @p16,
        SaveTableName = @p17, SaveTableKeyFields = @p18, AllowAdd = @p19, DefValueSQL = @p20,
        ChangeFrmFlg = @p21, LUTime = @p22, EDate = @p23, EUser = @p24
        WHERE RptID = @p25"#;

    // uniqueidentifier 字段空值传 NULL（None）
    // RptTitleID(@p1)、GridID(@p11)、TermID(@p12)、EUser(@p24) 是 uniqueidentifier
    let opt_rpt_title_id: Option<&str> = if rpt_title_id.is_empty() { None } else { Some(rpt_title_id) };
    let opt_grid_id: Option<&str> = if grid_id.is_empty() { None } else { Some(grid_id) };
    let opt_term_id: Option<&str> = if term_id.is_empty() { None } else { Some(term_id) };

    conn.execute(sql, &[
        &opt_rpt_title_id,
        &template_name,
        &doc_type,
        &tools_type,
        &content_bytes,
        &flg_term,
        &remark,
        &state,
        &kind,
        &share_all,
        &opt_grid_id,
        &opt_term_id,
        &rpt_history,
        &exec_sql,
        &exec_fields,
        &save_tables,
        &save_table_name,
        &save_table_key_fields,
        &allow_add,
        &def_value_sql,
        &change_frm_flg,
        &now,
        &now,
        &user_uuid.as_str(),
        &body.TemplateID.as_str(),
    ]).await?;

    Ok(Json(ApiResponse::msg("打印模板更新成功")))
}

#[derive(Deserialize)]
pub struct DeletePrintTemplateParams {
    pub ids: Vec<String>,
    /// 是否物理删除（true=DELETE FROM，false=软删除 State='D'）
    #[serde(default)]
    pub permanent: bool,
}

pub async fn delete_print_template(
    State(_config): State<Config>,
    Json(body): Json<DeletePrintTemplateParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    if body.ids.is_empty() {
        return Ok(Json(ApiResponse::err("请选择要删除的模板")));
    }

    if body.permanent {
        // 物理删除：DELETE FROM，不可恢复
        for id in &body.ids {
            let sql = "DELETE FROM tSys_Rpt WHERE RptID = @p1";
            conn.execute(sql, &[&id.as_str()]).await?;
        }
        Ok(Json(ApiResponse::msg(&format!("成功永久删除{}个模板", body.ids.len()))))
    } else {
        // 软删除：UPDATE State='D'
        for id in &body.ids {
            let sql = "UPDATE tSys_Rpt SET State = 'D' WHERE RptID = @p1";
            conn.execute(sql, &[&id.as_str()]).await?;
        }
        Ok(Json(ApiResponse::msg(&format!("成功删除{}个模板", body.ids.len()))))
    }
}

#[derive(Deserialize)]
pub struct RestorePrintTemplateParams {
    pub ids: Vec<String>,
}

/// 恢复打印模板（从软删除 State='D' 恢复为 State='A'）
pub async fn restore_print_template(
    State(_config): State<Config>,
    Json(body): Json<RestorePrintTemplateParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    if body.ids.is_empty() {
        return Ok(Json(ApiResponse::err("请选择要恢复的模板")));
    }

    for id in &body.ids {
        let sql = "UPDATE tSys_Rpt SET State = 'A' WHERE RptID = @p1";
        conn.execute(sql, &[&id.as_str()]).await?;
    }

    Ok(Json(ApiResponse::msg(&format!("成功恢复{}个模板", body.ids.len()))))
}

#[derive(Deserialize)]
pub struct GetPrintLogsParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
    pub doc_type: Option<String>,
    pub print_emp_id: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub sort_prop: Option<String>,
    pub sort_order: Option<String>,
}

/// 查询打印日志列表
/// 通过 JOIN tSys_Rpt（模板）和 tBas_Emp（员工）返回完整的日志信息：
/// - 模板名称（TemplateName）
/// - 单据类型（DocType）
/// - 打印人姓名（PrintEmpName）
/// - 累计打印次数（PrintNum）
pub async fn get_print_logs(
    State(_config): State<Config>,
    Json(params): Json<GetPrintLogsParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(50), 1000);

    // JOIN tSys_Rpt 获取模板名/单据类型，JOIN tBas_Emp 获取打印人姓名
    let mut base_query = r#"SELECT h.DocID, h.PrintDate, h.PrintRptID, h.PrintEmpID, h.PrintComName,
        r.RptDesc AS TemplateName, r.RptCode AS DocType,
        e.EmpName AS PrintEmpName,
        p.PrintNum, p.LastPrintDate, p.LastPrintEmpID, p.LastPrintComName
        FROM tSys_RptPrintHis h
        LEFT JOIN tSys_Rpt r ON h.PrintRptID = r.RptID
        LEFT JOIN tBas_Emp e ON h.PrintEmpID = e.EmpID
        LEFT JOIN tSys_RptPrintNum p ON h.DocID = p.DocID
        WHERE 1=1"#.to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            // ★ P1-8 修复：关键字搜索增加 PrintComName（计算机名）
            base_query.push_str(&format!(
                " AND (CAST(h.DocID AS NVARCHAR(50)) LIKE @p{} OR r.RptDesc LIKE @p{} OR r.RptCode LIKE @p{} OR e.EmpName LIKE @p{} OR h.PrintComName LIKE @p{})",
                pidx, pidx + 1, pidx + 2, pidx + 3, pidx + 4
            ));
            pidx += 5;
            let kw_pat = format!("%{}%", kw);
            query_params.push(Some(kw_pat.clone()));
            query_params.push(Some(kw_pat.clone()));
            query_params.push(Some(kw_pat.clone()));
            query_params.push(Some(kw_pat.clone()));
            query_params.push(Some(kw_pat));
        }
    }

    if let Some(dt) = &params.doc_type {
        if !dt.is_empty() {
            base_query.push_str(&format!(" AND r.RptCode = @p{}", pidx));
            pidx += 1;
            query_params.push(Some(dt.clone()));
        }
    }

    // ★ 新增：按打印人过滤
    if let Some(eid) = &params.print_emp_id {
        if !eid.is_empty() {
            base_query.push_str(&format!(" AND h.PrintEmpID = @p{}", pidx));
            pidx += 1;
            query_params.push(Some(eid.clone()));
        }
    }

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
    /// 可选：手动指定打印人 EmpID；不传则自动取当前登录用户
    pub PrintEmpID: Option<String>,
    pub PrintComName: Option<String>,
}

/// 写入打印日志的公共函数（供 create_print_log handler 和 approval::print_log 复用）
/// - 写入 tSys_RptPrintHis 历史记录
/// - 更新/创建 tSys_RptPrintNum 累计打印次数
pub async fn write_print_log_internal(
    conn: &mut bb8::PooledConnection<'static, bb8_tiberius::ConnectionManager>,
    doc_id: &str,
    print_rpt_id: &str,
    print_emp_id: &str,
    print_com_name: &str,
) -> Result<()> {
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let his_sql = r#"INSERT INTO tSys_RptPrintHis (DocID, PrintDate, PrintRptID, PrintEmpID, PrintComName)
        VALUES (@p1, @p2, @p3, @p4, @p5)"#;
    conn.execute(his_sql, &[
        &doc_id,
        &now,
        &print_rpt_id,
        &print_emp_id,
        &print_com_name,
    ]).await.map_err(|e| AppError::Internal(format!("写入打印历史失败: {}", e)))?;

    let num_check = "SELECT COUNT(*) as cnt FROM tSys_RptPrintNum WHERE DocID = @p1";
    let stream = conn.query(num_check, &[&doc_id]).await.map_err(|e| AppError::Internal(format!("查询打印次数失败: {}", e)))?;
    let mut exists = false;
    if let Some(row) = stream.into_row().await.map_err(|e| AppError::Internal(format!("读取打印次数失败: {}", e)))? {
        exists = row.get::<i32, _>("cnt").unwrap_or(0) > 0;
    }

    if exists {
        let num_sql = "UPDATE tSys_RptPrintNum SET PrintNum = PrintNum + 1, LastPrintDate = @p1, LastPrintEmpID = @p2, LastPrintComName = @p3 WHERE DocID = @p4";
        conn.execute(num_sql, &[&now, &print_emp_id, &print_com_name, &doc_id]).await
            .map_err(|e| AppError::Internal(format!("更新打印次数失败: {}", e)))?;
    } else {
        let num_sql = r#"INSERT INTO tSys_RptPrintNum (DocID, PrintNum, LastPrintDate, LastPrintEmpID, LastPrintComName)
            VALUES (@p1, 1, @p2, @p3, @p4)"#;
        conn.execute(num_sql, &[&doc_id, &now, &print_emp_id, &print_com_name]).await
            .map_err(|e| AppError::Internal(format!("创建打印次数记录失败: {}", e)))?;
    }
    Ok(())
}

/// 创建打印日志
/// - 自动填充当前登录用户为打印人（PrintEmpID）
/// - 自动获取客户端计算机名（PrintComName，若未传则用 "web"）
/// - 同时更新/创建 tSys_RptPrintNum 累计打印次数
pub async fn create_print_log(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<CreatePrintLogParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let print_rpt_id = body.PrintRptID.as_deref().unwrap_or("");
    // 打印人：优先用客户端传入，否则取当前登录用户的 EmpID
    let print_emp_id = if let Some(id) = body.PrintEmpID.as_deref() {
        if !id.is_empty() { id.to_string() }
        else {
            crate::handlers::generic::cached_lookup_user_uuid(&claims.user_code).await
                .unwrap_or_else(|| ZERO_UUID.to_string())
        }
    } else {
        crate::handlers::generic::cached_lookup_user_uuid(&claims.user_code).await
            .unwrap_or_else(|| ZERO_UUID.to_string())
    };
    let print_com_name = body.PrintComName.as_deref().unwrap_or("web");

    write_print_log_internal(&mut conn, &body.DocID, print_rpt_id, &print_emp_id, print_com_name).await?;

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
    let user_uuid = crate::handlers::generic::cached_lookup_user_uuid(&claims.user_code).await
        .unwrap_or_else(|| ZERO_UUID.to_string());

    if let Some(configs) = body.get("configs").and_then(|v| v.as_array()) {
        for cfg in configs {
            let rpt_id = cfg.get("RptID").and_then(|v| v.as_str()).unwrap_or("");
            let state = cfg.get("State").and_then(|v| v.as_str()).unwrap_or("A");
            if rpt_id.is_empty() { continue; }
            let sql = "UPDATE tSys_Rpt SET State = @p1, EDate = @p2, EUser = @p3 WHERE RptID = @p4";
            conn.execute(sql, &[&state, &now, &user_uuid.as_str(), &rpt_id]).await?;
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

    let sql = r#"SELECT VersionID, RptID, VersionNo, RptDesc, RptCode, RptFormat AS Content, Note, EDate, EUser, SnapshotName
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
    let user_uuid = crate::handlers::generic::cached_lookup_user_uuid(&claims.user_code).await
        .unwrap_or_else(|| ZERO_UUID.to_string());

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
        &user_uuid.as_str(),
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
    let user_uuid = crate::handlers::generic::cached_lookup_user_uuid(&claims.user_code).await
        .unwrap_or_else(|| ZERO_UUID.to_string());

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
            &rpt_id, &next_ver, &cur_desc, &cur_code, &cur_fmt, &cur_note, &now, &user_uuid.as_str(), &backup_name,
        ]).await?;
    }

    // Restore version to current template
    let update_sql = r#"UPDATE tSys_Rpt SET RptDesc = @p1, RptCode = @p2, RptFormat = @p3, Note = @p4, EDate = @p5, EUser = @p6
        WHERE RptID = @p7"#;
    conn.execute(update_sql, &[
        &rpt_desc, &rpt_code, &rpt_format, &note, &now, &user_uuid.as_str(), &rpt_id,
    ]).await?;

    Ok(Json(ApiResponse::msg("已回滚到指定版本")))
}
