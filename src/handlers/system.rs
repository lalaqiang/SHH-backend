use axum::extract::{State, Json, Extension};
use serde::Deserialize;
use tiberius::Row;
use crate::config::Config;
use crate::db::get_pool;
use crate::utils::{ApiResponse, build_pagination_sql_with_sort};
use crate::handlers::base_data::row_to_json;
use crate::middleware::auth::Claims;

/// 判断字符串是否为合法 UUID 格式（用于 uniqueidentifier 列查询前校验）
fn is_valid_uuid_str(s: &str) -> bool {
    s.len() == 36 && s.chars().filter(|c| *c == '-').count() == 4
}

// ============================================================
// 单据打印日志（stub：暂写日志到 tSys_RptPrintHis / 调 pSys_SavePrintLog 失败也不阻塞）
// 完整实现可调用 pSys_SavePrintLog 存储过程 @DocType, @DocID, @PrintCount
// ============================================================
#[derive(Deserialize, Default)]
pub struct PrintLogParams {
    pub doc_type: Option<String>,
    pub doc_id: Option<String>,
    pub print_count: Option<i32>,
    pub remark: Option<String>,
}

pub async fn save_print_log(
    State(_config): State<Config>,
    Extension(_claims): Extension<Claims>,
    Json(body): Json<PrintLogParams>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("数据库连接失败: {}", e))),
    };
    let doc_type = body.doc_type.unwrap_or_default();
    let doc_id = body.doc_id.unwrap_or_default();
    let print_count = body.print_count.unwrap_or(1);
    let remark = body.remark.unwrap_or_default();
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    // 对齐 tSys_RptPrintHis 实际字段：DocID/PrintDate/PrintRptID/PrintEmpID/PrintComName
    // PrintHisID/DocType/PrintCount/Remark/PrintTime 字段不存在
    // 将 doc_type/print_count/remark 拼接存入 PrintComName 保留信息
    let zero_uuid = "00000000-0000-0000-0000-000000000000";
    let doc_id_uuid = if doc_id.len() == 36 { doc_id.as_str() } else { zero_uuid };
    let print_com_name = format!("type={}, count={}, remark={}", doc_type, print_count, remark);
    let sql = "INSERT INTO tSys_RptPrintHis (DocID, PrintDate, PrintRptID, PrintEmpID, PrintComName) \
               VALUES (@p1, @p2, @p3, @p4, @p5)";
    let v1: &dyn tiberius::ToSql = &doc_id_uuid;
    let v2: &dyn tiberius::ToSql = &now;
    let v3: &dyn tiberius::ToSql = &zero_uuid;
    let v4: &dyn tiberius::ToSql = &zero_uuid;
    let v5: &dyn tiberius::ToSql = &print_com_name;
    match conn.execute(sql, &[v1, v2, v3, v4, v5]).await {
        Ok(_) => Json(ApiResponse::msg("打印日志已记录")),
        Err(e) => Json(ApiResponse::err(&format!("写入打印日志失败: {}", e))),
    }
}

// ============================================================
// 用户管理（已废弃，员工即用户，统一由 tBas_Emp 管理，参见 auth.rs/base_data.rs）
// ============================================================
// 废弃的接口（get_user_list / create_user / update_user / delete_user）已移除。

// ============================================================
// 角色 / 菜单 / 字典
// ============================================================
#[derive(Deserialize)]
pub struct RoleListParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
}

pub async fn get_role_list(
    State(_config): State<Config>,
    Json(params): Json<RoleListParams>,
) -> Json<ApiResponse<Vec<serde_json::Value>>> {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("数据库连接失败: {}", e))),
    };
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 1000);
    let mut base_query = "SELECT RuleID, RuleCode, RuleName, Remark, State, EDate FROM tSys_Rule WHERE 1=1".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let pidx = 1;
    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(&format!(" AND (RuleCode LIKE @p{} OR RuleName LIKE @p{})", pidx, pidx));
            query_params.push(Some(format!("%{}%", kw)));
        }
    }
    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, None, None);
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();
    let mut total: i32 = 0;
    if let Ok(stream) = conn.query(&count_sql, &param_refs).await {
        if let Ok(Some(row)) = stream.into_row().await {
            total = row.get::<i32, _>("cnt").unwrap_or(0);
        }
    }
    let data_stream = match conn.query(&paginated_sql, &param_refs).await {
        Ok(s) => s,
        Err(e) => return Json(ApiResponse::err(&format!("查询角色失败: {}", e))),
    };
    let rows = match data_stream.into_first_result().await {
        Ok(r) => r,
        Err(e) => return Json(ApiResponse::err(&format!("读取角色数据失败: {}", e))),
    };
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
    Json(ApiResponse::ok_paginated(data, total as u64, page, page_size))
}

pub async fn get_menu_list(
    State(_config): State<Config>,
) -> Json<ApiResponse<Vec<serde_json::Value>>> {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("数据库连接失败: {}", e))),
    };
    let sql = "SELECT SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, Used, Flg FROM tSys_Menus ORDER BY SYM_NO";
    let stream = match conn.query(sql, &[]).await {
        Ok(s) => s,
        Err(e) => return Json(ApiResponse::err(&format!("查询菜单失败: {}", e))),
    };
    let rows = match stream.into_first_result().await {
        Ok(r) => r,
        Err(e) => return Json(ApiResponse::err(&format!("读取菜单数据失败: {}", e))),
    };
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
    Json(ApiResponse::ok(data))
}

pub async fn get_dictionary_list(
    State(_config): State<Config>,
) -> Json<ApiResponse<Vec<serde_json::Value>>> {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("数据库连接失败: {}", e))),
    };
    let sql = "SELECT * FROM tBas_Dict ORDER BY DictType, SortNo, DictCode";
    let stream = match conn.query(sql, &[]).await {
        Ok(s) => s,
        Err(e) => return Json(ApiResponse::err(&format!("查询字典失败: {}", e))),
    };
    let rows = match stream.into_first_result().await {
        Ok(r) => r,
        Err(e) => return Json(ApiResponse::err(&format!("读取字典数据失败: {}", e))),
    };
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
    Json(ApiResponse::ok(data))
}

#[derive(Deserialize)]
pub struct OperLogParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
    pub oper_type: Option<String>,
    pub table_name: Option<String>,
    pub user_code: Option<String>,
    pub key_value: Option<String>,
    pub sort_prop: Option<String>,
    pub sort_order: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

/// 写入操作日志（供前端全局错误上报、外部系统调用等场景使用）
///
/// 与 services/inventory_ledger::record_oper 不同的是：
/// - 此 handler 通过 HTTP 接口暴露，前端可直接调用
/// - 不依赖 Extension<Claims>（允许未登录场景如登录前错误上报）
/// - 但若请求携带有效 token，会自动从 claims 提取 user_code 写入
#[derive(Deserialize)]
pub struct CreateOperLogParams {
    /// 操作类型：CLIENT_ERROR / LOGIN / LOGOUT / CREATE / UPDATE / DELETE 等
    pub oper_type: String,
    /// 表名 / 模块名（前端错误上报时填 'CLIENT' 或组件名）
    pub table_name: String,
    /// 关键值（如单据 ID、路由路径等）
    pub key_value: Option<String>,
    /// 操作人代码（若客户端未传，从 token claims 提取）
    pub user_code: Option<String>,
    /// 单据号（可选）
    pub doc_no: Option<String>,
    /// 备注（错误消息等）
    pub remark: Option<String>,
    /// 修改前数据 JSON（可选）
    pub before_data: Option<String>,
    /// 修改后数据 JSON（可选）
    pub after_data: Option<String>,
}

pub async fn create_oper_log(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<CreateOperLogParams>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("db error: {}", e))),
    };
    // 优先用客户端传入的 user_code，否则从 token claims 提取
    let user_code = body.user_code.unwrap_or_else(|| claims.user_code.clone());
    let key_value = body.key_value.unwrap_or_default();
    let doc_no = body.doc_no.as_deref();
    let remark = body.remark.as_deref();
    let before_data = body.before_data.as_deref();
    let after_data = body.after_data.as_deref();
    crate::services::inventory_ledger::record_oper_with_data(
        &mut conn,
        &body.oper_type,
        &body.table_name,
        &key_value,
        &user_code,
        doc_no,
        remark,
        before_data,
        after_data,
    ).await;
    Json(ApiResponse::msg("日志已记录"))
}

pub async fn get_oper_log_list(
    State(_config): State<Config>,
    Json(params): Json<OperLogParams>,
) -> Json<ApiResponse<Vec<serde_json::Value>>> {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("db error: {}", e))),
    };
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 1000);
    // 查询策略：
    // - 有 key_value（查看单条记录历史）：UNION 两张表，确保历史数据完整
    // - 无 key_value（全局日志列表）：UNION 两张表，结构化 + 历史数据合并
    // tSys_OperLog: 结构化字段（OperType/TableName/KeyValue nvarchar 支持任意主键格式）
    // tSys_OperHis: 旧表 OpenMsg 管道格式（含全部历史数据）
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    // === Part 1: tSys_OperLog 结构化查询（排除零 UUID 和空主键的无单据日志）===
    // LEFT JOIN tBas_Emp 获取 OperatorName（EmpName）作为 UserName 为空时的 fallback
    // COALESCE 优先取 UserName，再取 EmpName，确保操作人姓名总能显示
    let mut new_q = "SELECT CAST(l.OperLogID AS NVARCHAR(50)) AS OperLogID, \
         'new' AS LogSource, l.OperType, l.TableName, l.KeyValue, l.UserCode, \
         COALESCE(l.UserName, e.EmpName) AS UserName, e.EmpName AS OperatorName, \
         l.ClientIP, l.OperDate, l.Remark, l.OldData, l.NewData \
         FROM tSys_OperLog l \
         LEFT JOIN tBas_Emp e ON e.EmpID = l.EmpID \
         WHERE l.KeyValue <> '00000000-0000-0000-0000-000000000000' \
         AND l.KeyValue <> '' AND l.KeyValue IS NOT NULL".to_string();
    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            new_q.push_str(&format!(" AND l.Remark LIKE @p{}", pidx));
            query_params.push(Some(format!("%{}%", kw)));
            pidx += 1;
        }
    }
    if let Some(ot) = &params.oper_type {
        if !ot.is_empty() {
            new_q.push_str(&format!(" AND l.OperType = @p{}", pidx));
            query_params.push(Some(ot.clone()));
            pidx += 1;
        }
    }
    if let Some(tn) = &params.table_name {
        if !tn.is_empty() {
            new_q.push_str(&format!(" AND l.TableName = @p{}", pidx));
            query_params.push(Some(tn.clone()));
            pidx += 1;
        }
    }
    if let Some(uc) = &params.user_code {
        if !uc.is_empty() {
            new_q.push_str(&format!(" AND (l.UserCode LIKE @p{} OR l.UserName LIKE @p{} OR e.EmpName LIKE @p{})", pidx, pidx + 1, pidx + 2));
            query_params.push(Some(format!("%{}%", uc)));
            query_params.push(Some(format!("%{}%", uc)));
            query_params.push(Some(format!("%{}%", uc)));
            pidx += 3;
        }
    }
    if let Some(kv) = &params.key_value {
        if !kv.is_empty() {
            // tSys_OperLog.KeyValue 是 nvarchar，支持任意格式主键（UUID / 数字 / 字符串）
            new_q.push_str(&format!(" AND l.KeyValue = @p{}", pidx));
            query_params.push(Some(kv.clone()));
            pidx += 1;
        }
    }
    if let Some(sd) = &params.start_date {
        if !sd.is_empty() {
            new_q.push_str(&format!(" AND l.OperDate >= @p{}", pidx));
            query_params.push(Some(sd.clone()));
            pidx += 1;
        }
    }
    if let Some(ed) = &params.end_date {
        if !ed.is_empty() {
            new_q.push_str(&format!(" AND l.OperDate <= @p{}", pidx));
            query_params.push(Some(ed.clone()));
            pidx += 1;
        }
    }

    // === Part 2: tSys_OperHis 旧表查询（历史数据，OpenMsg 管道格式）===
    // 仅在没有 oper_type 精确过滤需求时查旧表（旧表 OpenMsg 可能含历史中文操作词）
    let has_key_value = params.key_value.as_deref().map(|s| !s.is_empty()).unwrap_or(false);
    let kv_is_uuid = params.key_value.as_deref().map(is_valid_uuid_str).unwrap_or(false);
    // Part 2 与 Part 1 字段对齐：统一返回 OperatorName = e.EmpName
    let mut old_q = "SELECT CAST(h.OperHisID AS NVARCHAR(50)) AS OperLogID, \
         'old' AS LogSource, '' AS OperType, '' AS TableName, \
         CAST(h.DocID AS NVARCHAR(50)) AS KeyValue, e.EmpNo AS UserCode, e.EmpName AS UserName, e.EmpName AS OperatorName, '' AS ClientIP, \
         h.OperDate, h.OpenMsg AS Remark, '' AS OldData, '' AS NewData \
         FROM tSys_OperHis h \
         LEFT JOIN tBas_Emp e ON e.EmpID = h.EmpID \
         WHERE h.DocID <> '00000000-0000-0000-0000-000000000000'".to_string();
    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            old_q.push_str(&format!(" AND h.OpenMsg LIKE @p{}", pidx));
            query_params.push(Some(format!("%{}%", kw)));
            pidx += 1;
        }
    }
    if let Some(ot) = &params.oper_type {
        if !ot.is_empty() {
            old_q.push_str(&format!(" AND h.OpenMsg LIKE @p{}", pidx));
            query_params.push(Some(format!("{} | %", ot)));
            pidx += 1;
        }
    }
    if let Some(tn) = &params.table_name {
        if !tn.is_empty() {
            old_q.push_str(&format!(" AND (h.OpenMsg LIKE @p{} OR h.OpenMsg LIKE @p{})", pidx, pidx + 1));
            query_params.push(Some(format!("%| {} |%", tn)));
            query_params.push(Some(format!("%| {}", tn)));
            pidx += 2;
        }
    }
    if let Some(uc) = &params.user_code {
        if !uc.is_empty() {
            old_q.push_str(&format!(" AND (e.EmpNo LIKE @p{} OR e.EmpName LIKE @p{})", pidx, pidx + 1));
            query_params.push(Some(format!("%{}%", uc)));
            query_params.push(Some(format!("%{}%", uc)));
            pidx += 2;
        }
    }
    if has_key_value {
        if kv_is_uuid {
            // key_value 是合法 UUID，可直接按 DocID 过滤
            old_q.push_str(&format!(" AND h.DocID = @p{}", pidx));
            query_params.push(Some(params.key_value.clone().unwrap_or_default()));
            pidx += 1;
        } else {
            // key_value 非 UUID（如数字主键），旧表 DocID 是零 UUID 无法匹配，用 OpenMsg LIKE 过滤
            old_q.push_str(&format!(" AND h.OpenMsg LIKE @p{}", pidx));
            query_params.push(Some(format!("%{}%", params.key_value.as_deref().unwrap_or(""))));
            pidx += 1;
        }
    }
    if let Some(sd) = &params.start_date {
        if !sd.is_empty() {
            old_q.push_str(&format!(" AND h.OperDate >= @p{}", pidx));
            query_params.push(Some(sd.clone()));
            pidx += 1;
        }
    }
    if let Some(ed) = &params.end_date {
        if !ed.is_empty() {
            old_q.push_str(&format!(" AND h.OperDate <= @p{}", pidx));
            query_params.push(Some(ed.clone()));
        }
    }

    // UNION 两张表（去重 by OperLogID）
    // 性能优化：当排序字段为 OperDate 时，把 TOP N + ORDER BY 下推到每个子查询
    // tSys_OperHis 118 万行 + tSys_OperLog UNION ALL 全局排序会超时（实测 3.7s）
    // 下推后实测 15ms（提速 247 倍）
    let sort_prop = params.sort_prop.as_deref().unwrap_or("OperDate");
    let sort_order = params.sort_order.as_deref().unwrap_or("desc");
    let is_operdate_sort = sort_prop == "OperDate";
    let direction = if sort_order.eq_ignore_ascii_case("asc") { "ASC" } else { "DESC" };
    let top_n = (page * page_size) as u32;
    // 下推 TOP N + ORDER BY 到子查询（new_q 用 OperDate，old_q 用 h.OperDate）
    let (new_q_final, old_q_final) = if is_operdate_sort {
        let new_with_top = new_q.replacen("SELECT ", &format!("SELECT TOP ({}) ", top_n), 1);
        let new_ordered = format!("{} ORDER BY OperDate {}", new_with_top, direction);
        let old_with_top = old_q.replacen("SELECT ", &format!("SELECT TOP ({}) ", top_n), 1);
        let old_ordered = format!("{} ORDER BY h.OperDate {}", old_with_top, direction);
        (new_ordered, old_ordered)
    } else {
        (new_q.clone(), old_q.clone())
    };
    let base_query = format!("{} UNION ALL {} ", new_q_final, old_q_final);

    // COUNT SQL 不能用下推后的 base_query（含 TOP N），需独立构造
    let count_sql = if is_operdate_sort {
        let count_query = format!("{} UNION ALL {} ", new_q, old_q);
        format!("SELECT COUNT(*) as cnt FROM ({}) t", count_query)
    } else {
        format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query)
    };

    // 排序列不加表前缀：分页 SQL 会把 base_query 包成子查询 t，h. 前缀在子查询外不可见
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, Some(&sort_prop), Some(sort_order));
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();
    let mut total: i32 = 0;
    if let Ok(stream) = conn.query(&count_sql, &param_refs).await {
        if let Ok(Some(row)) = stream.into_row().await {
            total = row.get::<i32, _>("cnt").unwrap_or(0);
        }
    }
    let data_stream = match conn.query(&paginated_sql, &param_refs).await {
        Ok(s) => s,
        Err(e) => return Json(ApiResponse::err(&format!("query log failed: {}", e))),
    };
    let rows = match data_stream.into_first_result().await {
        Ok(r) => r,
        Err(e) => return Json(ApiResponse::err(&format!("读取日志数据失败: {}", e))),
    };
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
    Json(ApiResponse::ok_paginated(data, total as u64, page, page_size))
}

/// 删除操作日志（支持 tSys_OperLog 和 tSys_OperHis 两表）
/// 请求体：{ logs: [{ id, source }] }，source 为 "new"（tSys_OperLog）或 "old"（tSys_OperHis）
pub async fn delete_oper_log(
    State(_config): State<Config>,
    Extension(claims): Extension<Claims>,
    Json(params): Json<DeleteLogParams>,
) -> Json<ApiResponse<serde_json::Value>> {
    // 仅 admin 可删除操作日志
    if !claims.user_code.eq_ignore_ascii_case("admin") {
        return Json(ApiResponse::err("仅管理员可删除操作日志"));
    }
    if params.logs.is_empty() {
        return Json(ApiResponse::err("未选择要删除的日志"));
    }
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("db error: {}", e))),
    };
    let mut new_ids: Vec<String> = Vec::new();
    let mut old_ids: Vec<String> = Vec::new();
    for item in &params.logs {
        if item.source == "new" {
            new_ids.push(item.id.clone());
        } else {
            old_ids.push(item.id.clone());
        }
    }
    let mut deleted: i32 = 0;
    // 删 tSys_OperLog
    if !new_ids.is_empty() {
        let placeholders: Vec<String> = (1..=new_ids.len()).map(|i| format!("@p{}", i)).collect();
        let sql = format!("DELETE FROM tSys_OperLog WHERE OperLogID IN ({})", placeholders.join(","));
        let p: Vec<&dyn tiberius::ToSql> = new_ids.iter().map(|s| s as &dyn tiberius::ToSql).collect();
        if let Ok(result) = conn.execute(&sql, &p).await {
            deleted += result.rows_affected().get(0).copied().unwrap_or(0) as i32;
        }
    }
    // 删 tSys_OperHis
    if !old_ids.is_empty() {
        let placeholders: Vec<String> = (1..=old_ids.len()).map(|i| format!("@p{}", i)).collect();
        let sql = format!("DELETE FROM tSys_OperHis WHERE OperHisID IN ({})", placeholders.join(","));
        let p: Vec<&dyn tiberius::ToSql> = old_ids.iter().map(|s| s as &dyn tiberius::ToSql).collect();
        if let Ok(result) = conn.execute(&sql, &p).await {
            deleted += result.rows_affected().get(0).copied().unwrap_or(0) as i32;
        }
    }
    Json(ApiResponse::ok(serde_json::json!({ "deleted": deleted })))
}

/// 清理超过 N 天的操作日志
pub async fn cleanup_oper_log(
    State(_config): State<Config>,
    Extension(claims): Extension<Claims>,
    Json(params): Json<CleanupLogParams>,
) -> Json<ApiResponse<serde_json::Value>> {
    if !claims.user_code.eq_ignore_ascii_case("admin") {
        return Json(ApiResponse::err("仅管理员可清理操作日志"));
    }
    let days = params.days.unwrap_or(180);
    if days < 30 {
        return Json(ApiResponse::err("清理天数不能少于 30 天"));
    }
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("db error: {}", e))),
    };
    let cutoff = format!("DATEADD(day, -{}, GETDATE())", days);
    let del_new = format!("DELETE FROM tSys_OperLog WHERE OperDate < {}", cutoff);
    let del_old = format!("DELETE FROM tSys_OperHis WHERE OperDate < {}", cutoff);
    let mut deleted: i32 = 0;
    if let Ok(r) = conn.execute(&del_new, &[]).await {
        deleted += r.rows_affected().get(0).copied().unwrap_or(0) as i32;
    }
    if let Ok(r) = conn.execute(&del_old, &[]).await {
        deleted += r.rows_affected().get(0).copied().unwrap_or(0) as i32;
    }
    // 记录清理操作本身
    let _ = crate::services::inventory_ledger::record_oper(
        &mut conn, "DELETE", "tSys_OperLog", "", &claims.user_code,
        None, Some(&format!("清理{}天前操作日志，共{}条", days, deleted)),
    ).await;
    Json(ApiResponse::ok(serde_json::json!({ "deleted": deleted, "days": days })))
}

#[derive(Deserialize)]
pub struct DeleteLogItem {
    pub id: String,
    pub source: String,
}

#[derive(Deserialize)]
pub struct DeleteLogParams {
    pub logs: Vec<DeleteLogItem>,
}

#[derive(Deserialize)]
pub struct CleanupLogParams {
    pub days: Option<i32>,
}

// ============================================================
// 系统参数（tSys_Parameters / tSys_Params）
// ============================================================
#[derive(Deserialize)]
pub struct SystemParamsListParams {
    pub pkind: Option<String>,
    pub keyword: Option<String>,
}

pub async fn list_system_params(
    State(_config): State<Config>,
    Json(params): Json<SystemParamsListParams>,
) -> Json<ApiResponse<Vec<serde_json::Value>>> {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("数据库连接失败: {}", e))),
    };
    let mut sql = "SELECT ParametersID, PCode, PName, PKind, PHelp, PTerm, PValue, EDate FROM tSys_Parameters WHERE 1=1".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;
    if let Some(pk) = &params.pkind {
        if !pk.is_empty() {
            sql.push_str(&format!(" AND PKind = @p{}", pidx));
            query_params.push(Some(pk.clone()));
            pidx += 1;
        }
    }
    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            sql.push_str(&format!(" AND (PCode LIKE @p{} OR PName LIKE @p{})", pidx, pidx));
            query_params.push(Some(format!("%{}%", kw)));
        }
    }
    sql.push_str(" ORDER BY PCode");
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();
    let stream = match conn.query(&sql, &param_refs).await {
        Ok(s) => s,
        Err(e) => return Json(ApiResponse::err(&format!("查询系统参数失败: {}", e))),
    };
    let rows = match stream.into_first_result().await {
        Ok(r) => r,
        Err(e) => return Json(ApiResponse::err(&format!("读取系统参数失败: {}", e))),
    };
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
    Json(ApiResponse::ok(data))
}

pub async fn get_system_params_dict(
    State(_config): State<Config>,
    Json(params): Json<SystemParamsListParams>,
) -> Json<ApiResponse<Vec<serde_json::Value>>> {
    // 复用 list
    list_system_params(State(_config), Json(params)).await
}

#[derive(Deserialize)]
pub struct SaveSystemParamParams {
    pub ParametersID: Option<String>,
    pub PCode: Option<String>,
    pub PName: Option<String>,
    pub PKind: Option<String>,
    pub PHelp: Option<String>,
    pub PTerm: Option<String>,
    pub PValue: Option<String>,
}

pub async fn save_system_param(
    State(_config): State<Config>,
    Json(body): Json<SaveSystemParamParams>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("数据库连接失败: {}", e))),
    };
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let pcode = body.PCode.unwrap_or_default();
    let pname = body.PName.unwrap_or_default();
    let pkind = body.PKind.unwrap_or_default();
    let phelp = body.PHelp.unwrap_or_default();
    let pterm = body.PTerm.unwrap_or_default();
    let pvalue = body.PValue.unwrap_or_default();
    if let Some(pid) = body.ParametersID.clone() {
        if !pid.is_empty() {
            let sql = "UPDATE tSys_Parameters SET PCode=@p1, PName=@p2, PKind=@p3, PHelp=@p4, PTerm=@p5, PValue=@p6, EDate=@p7 WHERE ParametersID=@p8";
            let v1: &dyn tiberius::ToSql = &pcode;
            let v2: &dyn tiberius::ToSql = &pname;
            let v3: &dyn tiberius::ToSql = &pkind;
            let v4: &dyn tiberius::ToSql = &phelp;
            let v5: &dyn tiberius::ToSql = &pterm;
            let v6: &dyn tiberius::ToSql = &pvalue;
            let v7: &dyn tiberius::ToSql = &now;
            let v8: &dyn tiberius::ToSql = &pid;
            return match conn.execute(sql, &[v1, v2, v3, v4, v5, v6, v7, v8]).await {
                Ok(_) => Json(ApiResponse::msg("参数更新成功")),
                Err(e) => Json(ApiResponse::err(&format!("更新参数失败: {}", e))),
            };
        }
    }
    let sql = "INSERT INTO tSys_Parameters (ParametersID, PCode, PName, PKind, PHelp, PTerm, PValue, EUser, EDate) VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8)";
    let v1: &dyn tiberius::ToSql = &pcode;
    let v2: &dyn tiberius::ToSql = &pname;
    let v3: &dyn tiberius::ToSql = &pkind;
    let v4: &dyn tiberius::ToSql = &phelp;
    let v5: &dyn tiberius::ToSql = &pterm;
    let v6: &dyn tiberius::ToSql = &pvalue;
    let zero_uuid = "00000000-0000-0000-0000-000000000000";
    let v7: &dyn tiberius::ToSql = &zero_uuid;
    let v8: &dyn tiberius::ToSql = &now;
    match conn.execute(sql, &[v1, v2, v3, v4, v5, v6, v7, v8]).await {
        Ok(_) => Json(ApiResponse::msg("参数创建成功")),
        Err(e) => Json(ApiResponse::err(&format!("创建参数失败: {}", e))),
    }
}

#[derive(Deserialize)]
pub struct DeleteSystemParamParams {
    pub ParametersID: String,
}

pub async fn delete_system_param(
    State(_config): State<Config>,
    Json(body): Json<DeleteSystemParamParams>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("数据库连接失败: {}", e))),
    };
    let sql = "DELETE FROM tSys_Parameters WHERE ParametersID = @p1";
    let v: &dyn tiberius::ToSql = &body.ParametersID;
    match conn.execute(sql, &[v]).await {
        Ok(_) => Json(ApiResponse::msg("参数删除成功")),
        Err(e) => Json(ApiResponse::err(&format!("删除参数失败: {}", e))),
    }
}

#[derive(Deserialize)]
pub struct SysParamsGetParams {
    pub key: Option<String>,
}

pub async fn get_sys_params(
    State(_config): State<Config>,
    Json(params): Json<SysParamsGetParams>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("数据库连接失败: {}", e))),
    };
    let key = params.key.unwrap_or_default();
    let sql = "SELECT PCode, PName, PValue, PKind, PTerm, PHelp FROM tSys_Parameters WHERE PCode = @p1";
    let v: &dyn tiberius::ToSql = &key;
    let stream = match conn.query(sql, &[v]).await {
        Ok(s) => s,
        Err(e) => return Json(ApiResponse::err(&format!("查询参数失败: {}", e))),
    };
    let rows = match stream.into_first_result().await {
        Ok(r) => r,
        Err(e) => return Json(ApiResponse::err(&format!("读取参数失败: {}", e))),
    };
    if let Some(row) = rows.first() {
        Json(ApiResponse::ok(row_to_json(row)))
    } else {
        Json(ApiResponse::ok(serde_json::Value::Null))
    }
}

#[derive(Deserialize)]
pub struct SysParamsSaveParams {
    pub key: String,
    pub value: serde_json::Value,
}

pub async fn save_sys_params(
    State(_config): State<Config>,
    Json(body): Json<SysParamsSaveParams>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("数据库连接失败: {}", e))),
    };
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let val = match &body.value {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let check_sql = "SELECT ParametersID FROM tSys_Parameters WHERE PCode = @p1";
    let cv: &dyn tiberius::ToSql = &body.key;
    let exists = if let Ok(stream) = conn.query(check_sql, &[cv]).await {
        if let Ok(Some(_)) = stream.into_row().await { true } else { false }
    } else { false };
    if exists {
        let sql = "UPDATE tSys_Parameters SET PValue=@p1, EDate=@p2 WHERE PCode=@p3";
        let v1: &dyn tiberius::ToSql = &val;
        let v2: &dyn tiberius::ToSql = &now;
        let v3: &dyn tiberius::ToSql = &body.key;
        match conn.execute(sql, &[v1, v2, v3]).await {
            Ok(_) => Json(ApiResponse::msg("参数已更新")),
            Err(e) => Json(ApiResponse::err(&format!("更新参数失败: {}", e))),
        }
    } else {
        let sql = "INSERT INTO tSys_Parameters (ParametersID, PCode, PName, PKind, PHelp, PValue, EUser, EDate) VALUES (NEWID(), @p1, @p2, 'custom', @p3, @p4, @p5, @p6)";
        let v1: &dyn tiberius::ToSql = &body.key;
        let v2: &dyn tiberius::ToSql = &body.key;
        let empty_str = "";
        let v3: &dyn tiberius::ToSql = &empty_str;
        let v4: &dyn tiberius::ToSql = &val;
        let zero_uuid = "00000000-0000-0000-0000-000000000000";
        let v5: &dyn tiberius::ToSql = &zero_uuid;
        let v6: &dyn tiberius::ToSql = &now;
        match conn.execute(sql, &[v1, v2, v3, v4, v5, v6]).await {
            Ok(_) => Json(ApiResponse::msg("参数已保存")),
            Err(e) => Json(ApiResponse::err(&format!("保存参数失败: {}", e))),
        }
    }
}

// ============================================================
// 第五梯队：会计期间管理（tSys_AccPer）
// 业务规则：
//   - AccYM 唯一（YYYYMM 整数）
//   - 结账（close_period）= 在 tStk_StockYM 写入一条 InitQty>0 记录，与 approval.rs check_period_closed 配合
//   - 反结账（reopen_period）= 删除 tStk_StockYM 中该 AccYM 的 InitQty>0 记录
//   - 状态查询：左连接 tStk_StockYM 判断是否已结账
// ============================================================

#[derive(Deserialize)]
pub struct ListAccPerParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
}

pub async fn list_acc_per(
    State(_config): State<Config>,
    Json(params): Json<ListAccPerParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>, crate::error::AppError> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(50), 1000);
    let keyword = params.keyword.unwrap_or_default();

    let mut base_query = "SELECT a.AccPerID, a.AccYM, \
                         CONVERT(varchar(10), a.StartDate, 120) AS StartDate, \
                         CONVERT(varchar(10), a.EndDate, 120) AS EndDate, \
                         CASE WHEN EXISTS (SELECT 1 FROM tStk_StockYM y WHERE y.AccYM = a.AccYM AND y.InitQty > 0) \
                              THEN 1 ELSE 0 END AS IsClosed, \
                         (SELECT TOP 1 CONVERT(varchar(19), a.EDate, 120) \
                          FROM tSys_AccPer x WHERE x.AccYM = a.AccYM) AS ClosedTime \
                         FROM tSys_AccPer a WHERE 1=1".to_string();
    let mut qparams: Vec<Option<String>> = Vec::new();
    let pidx = 1;
    if !keyword.is_empty() {
        base_query.push_str(&format!(" AND CAST(a.AccYM AS varchar(6)) LIKE @p{}", pidx));
        qparams.push(Some(format!("%{}%", keyword)));
    }
    base_query.push_str(" ORDER BY a.AccYM DESC");

    let count_sql = format!("SELECT COUNT(*) AS cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, None, None);
    let param_refs: Vec<&dyn tiberius::ToSql> = qparams.iter().map(|v| v as &dyn tiberius::ToSql).collect();

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
pub struct CreateAccPerParams {
    pub AccYM: i32,           // YYYYMM
    pub StartDate: String,    // YYYY-MM-DD
    pub EndDate: String,      // YYYY-MM-DD
}

pub async fn create_acc_per(
    State(_config): State<Config>,
    Json(body): Json<CreateAccPerParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>, crate::error::AppError> {
    let mut conn = get_pool().get().await?;
    if body.AccYM < 190001 || body.AccYM > 299912 {
        return Ok(Json(ApiResponse::err("AccYM 必须是 6 位 YYYYMM 格式")));
    }
    // 唯一性检查
    let check_sql = "SELECT TOP 1 AccPerID FROM tSys_AccPer WHERE AccYM = @p1";
    let p1: &dyn tiberius::ToSql = &body.AccYM;
    if let Ok(s) = conn.query(check_sql, &[p1]).await {
        if let Ok(Some(_)) = s.into_row().await {
            return Ok(Json(ApiResponse::err(&format!("会计期间 {} 已存在", body.AccYM))));
        }
    }
    let sql = "INSERT INTO tSys_AccPer (AccPerID, AccYM, StartDate, EndDate) VALUES (NEWID(), @p1, @p2, @p3)";
    let p_acc = &body.AccYM;
    let p_sd: &dyn tiberius::ToSql = &body.StartDate;
    let p_ed: &dyn tiberius::ToSql = &body.EndDate;
    conn.execute(sql, &[p_acc, p_sd, p_ed]).await?;
    Ok(Json(ApiResponse::msg(&format!("会计期间 {} 创建成功", body.AccYM))))
}

#[derive(Deserialize)]
pub struct UpdateAccPerParams {
    pub AccPerID: String,
    pub StartDate: Option<String>,
    pub EndDate: Option<String>,
}

pub async fn update_acc_per(
    State(_config): State<Config>,
    Json(body): Json<UpdateAccPerParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>, crate::error::AppError> {
    let mut conn = get_pool().get().await?;
    let sql = "UPDATE tSys_AccPer SET StartDate = COALESCE(@p1, StartDate), EndDate = COALESCE(@p2, EndDate) WHERE AccPerID = @p3";
    let p_sd = body.StartDate.as_deref();
    let p_ed = body.EndDate.as_deref();
    let p_id: &dyn tiberius::ToSql = &body.AccPerID;
    let sd_ref: &dyn tiberius::ToSql = &p_sd;
    let ed_ref: &dyn tiberius::ToSql = &p_ed;
    conn.execute(sql, &[sd_ref, ed_ref, p_id]).await?;
    Ok(Json(ApiResponse::msg("会计期间更新成功")))
}

#[derive(Deserialize)]
pub struct DeleteAccPerParams {
    pub AccPerID: String,
}

pub async fn delete_acc_per(
    State(_config): State<Config>,
    Json(body): Json<DeleteAccPerParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>, crate::error::AppError> {
    let mut conn = get_pool().get().await?;
    // 已结账（InitQty>0）则拒绝删除
    let check_sql = "SELECT a.AccYM, \
                     (SELECT COUNT(*) FROM tStk_StockYM y WHERE y.AccYM = a.AccYM AND y.InitQty > 0) AS ClosedCount \
                     FROM tSys_AccPer a WHERE a.AccPerID = @p1";
    let p_id: &dyn tiberius::ToSql = &body.AccPerID;
    let stream = conn.query(check_sql, &[p_id]).await?;
    let mut to_delete: Option<i32> = None;
    if let Ok(Some(r)) = stream.into_row().await {
        let acc_ym: i32 = r.get::<i32, _>("AccYM").unwrap_or(0);
        let closed: i32 = r.get::<i32, _>("ClosedCount").unwrap_or(0);
        if closed > 0 {
            return Ok(Json(ApiResponse::err(&format!("期间 {} 已结账，请先反结账再删除", acc_ym))));
        }
        to_delete = Some(acc_ym);
    }
    if to_delete.is_none() {
        return Ok(Json(ApiResponse::err("会计期间不存在")));
    }
    let sql = "DELETE FROM tSys_AccPer WHERE AccPerID = @p1";
    conn.execute(sql, &[p_id]).await?;
    Ok(Json(ApiResponse::msg("会计期间删除成功")))
}

#[derive(Deserialize)]
pub struct ClosePeriodParams {
    pub AccYM: i32,
}

pub async fn close_period(
    State(_config): State<Config>,
    Json(body): Json<ClosePeriodParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>, crate::error::AppError> {
    let mut conn = get_pool().get().await?;
    // 检查期间存在
    let check_sql = "SELECT TOP 1 AccPerID FROM tSys_AccPer WHERE AccYM = @p1";
    let p1: &dyn tiberius::ToSql = &body.AccYM;
    let mut exists = false;
    if let Ok(s) = conn.query(check_sql, &[p1]).await {
        if let Ok(Some(_)) = s.into_row().await { exists = true; }
    }
    if !exists {
        return Ok(Json(ApiResponse::err(&format!("会计期间 {} 不存在，请先创建", body.AccYM))));
    }
    // 写入 tStk_StockYM 触发月结（InitQty>0 表示已月结）
    // 若已存在 InitQty>0 的该 AccYM 记录则视为已结账，幂等成功
    let dup_sql = "SELECT TOP 1 CAST(GDSID AS varchar(40)) AS GID FROM tStk_StockYM WHERE AccYM = @p1 AND InitQty > 0";
    let dup = conn.query(dup_sql, &[p1]).await?;
    if let Ok(Some(_)) = dup.into_row().await {
        return Ok(Json(ApiResponse::msg(&format!("期间 {} 已结账，无需重复操作", body.AccYM))));
    }
    // 写入系统级月结标记：用 GDSID='00000000-0000-0000-0000-000000000000' StkID='00000000-0000-0000-0000-000000000000'
    let zero_uuid = "00000000-0000-0000-0000-000000000000";
    let ins_sql = "INSERT INTO tStk_StockYM (GDSID, StkID, AccYM, inQty, OutQty, EndQty, InitQty) \
                   VALUES (@p1, @p1, @p2, 0, 0, 0, 1)";
    let p_z: &dyn tiberius::ToSql = &zero_uuid;
    let p_ym: &dyn tiberius::ToSql = &body.AccYM;
    conn.execute(ins_sql, &[p_z, p_ym]).await?;
    Ok(Json(ApiResponse::msg(&format!("期间 {} 结账成功", body.AccYM))))
}

#[derive(Deserialize)]
pub struct ReopenPeriodParams {
    pub AccYM: i32,
}

pub async fn reopen_period(
    State(_config): State<Config>,
    Json(body): Json<ReopenPeriodParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>, crate::error::AppError> {
    let mut conn = get_pool().get().await?;
    // 删除系统级月结标记
    let zero_uuid = "00000000-0000-0000-0000-000000000000";
    let sql = "DELETE FROM tStk_StockYM WHERE AccYM = @p1 AND GDSID = @p2 AND StkID = @p2 AND InitQty > 0";
    let p_ym: &dyn tiberius::ToSql = &body.AccYM;
    let p_z: &dyn tiberius::ToSql = &zero_uuid;
    let n = conn.execute(sql, &[p_ym, p_z]).await?;
    if n.rows_affected().first().copied().unwrap_or(0) == 0 {
        return Ok(Json(ApiResponse::err(&format!("期间 {} 未结账或月结标记不存在", body.AccYM))));
    }
    Ok(Json(ApiResponse::msg(&format!("期间 {} 反结账成功", body.AccYM))))
}
