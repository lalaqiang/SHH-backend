use axum::extract::{State, Json, Extension};
use serde::Deserialize;
use tiberius::Row;
use crate::config::Config;
use crate::db::get_pool;
use crate::utils::{ApiResponse, build_pagination_sql_with_sort};
use crate::handlers::base_data::try_get_value;
use crate::middleware::auth::Claims;

fn json_str(v: &serde_json::Value, key: &str) -> String {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("").to_string()
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

async fn lookup_user_uuid(
    conn: &mut bb8::PooledConnection<'static, bb8_tiberius::ConnectionManager>,
    user_code: &str,
) -> Option<String> {
    if user_code.is_empty() {
        return None;
    }
    let sql = "SELECT TOP 1 CAST([EmpID] AS nvarchar(64)) AS EmpID FROM [dbo].[tBas_Emp] WHERE [EmpNo] = @p1";
    let v: &dyn tiberius::ToSql = &user_code;
    if let Ok(stream) = conn.query(sql, &[v]).await {
        if let Ok(rows) = stream.into_first_result().await {
            if let Some(row) = rows.first() {
                if let Some(s) = row.get::<&str, _>("EmpID") {
                    if !s.is_empty() {
                        return Some(s.to_string());
                    }
                }
            }
        }
    }
    None
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
    let now = chrono::Local::now().naive_local();
    let sql = "INSERT INTO tSys_RptPrintHis (PrintHisID, DocType, DocID, PrintCount, Remark, PrintTime) \
               VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5)";
    let v1: &dyn tiberius::ToSql = &doc_type;
    let v2: &dyn tiberius::ToSql = &doc_id;
    let v3: &dyn tiberius::ToSql = &print_count;
    let v4: &dyn tiberius::ToSql = &remark;
    let v5: &dyn tiberius::ToSql = &now;
    match conn.execute(sql, &[v1, v2, v3, v4, v5]).await {
        Ok(_) => Json(ApiResponse::msg("打印日志已记录")),
        Err(e) => Json(ApiResponse::err(&format!("写入打印日志失败: {}", e))),
    }
}

// ============================================================
// 用户管理
// ============================================================
#[derive(Deserialize)]
pub struct UserListParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
    pub include_deleted: Option<bool>,
}

pub async fn get_user_list(
    State(_config): State<Config>,
    Json(params): Json<UserListParams>,
) -> Json<ApiResponse<Vec<serde_json::Value>>> {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("数据库连接失败: {}", e))),
    };
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);
    let mut base_query = "SELECT UserID, UserCode, UserName, RuleID, EmpID, StkID, RealName, Phone, Email, Remark, Used, EDate FROM tSys_Users WHERE 1=1".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;
    if let Some(include_deleted) = params.include_deleted {
        if !include_deleted {
            base_query.push_str(" AND (Used = 'Y' OR Used IS NULL)");
        }
    } else {
        base_query.push_str(" AND (Used = 'Y' OR Used IS NULL)");
    }
    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(&format!(" AND (UserCode LIKE @p{} OR UserName LIKE @p{})", pidx, pidx));
            query_params.push(Some(format!("%{}%", kw)));
            pidx += 1;
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
        Err(e) => return Json(ApiResponse::err(&format!("查询用户失败: {}", e))),
    };
    let rows = match data_stream.into_first_result().await {
        Ok(r) => r,
        Err(e) => return Json(ApiResponse::err(&format!("读取用户数据失败: {}", e))),
    };
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
    Json(ApiResponse::ok_paginated(data, total as u64, page, page_size))
}

#[derive(Deserialize)]
pub struct CreateUserParams {
    pub UserCode: Option<String>,
    pub UserName: Option<String>,
    pub PassWordStr: Option<String>,
    pub RuleID: Option<String>,
    pub EmpID: Option<String>,
    pub StkID: Option<String>,
    pub RealName: Option<String>,
    pub Phone: Option<String>,
    pub Email: Option<String>,
    pub Remark: Option<String>,
}

pub async fn create_user(
    State(_config): State<Config>,
    Json(body): Json<CreateUserParams>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("数据库连接失败: {}", e))),
    };
    let user_code = body.UserCode.unwrap_or_default();
    let user_name = body.UserName.unwrap_or_default();
    let password = body.PassWordStr.unwrap_or_else(|| "123456".to_string());
    let rule_id = body.RuleID.unwrap_or_default();
    let emp_id = body.EmpID.unwrap_or_default();
    let stk_id = body.StkID.unwrap_or_default();
    let real_name = body.RealName.unwrap_or_default();
    let phone = body.Phone.unwrap_or_default();
    let email = body.Email.unwrap_or_default();
    let remark = body.Remark.unwrap_or_default();
    let now = chrono::Local::now().naive_local();
    let sql = "INSERT INTO tSys_Users (UserID, UserCode, UserName, PassWordStr, RuleID, EmpID, StkID, RealName, Phone, Email, Remark, Used, EDate) \
               VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10, 'Y', @p11)";
    let v1: &dyn tiberius::ToSql = &user_code;
    let v2: &dyn tiberius::ToSql = &user_name;
    let v3: &dyn tiberius::ToSql = &password;
    let v4: &dyn tiberius::ToSql = &rule_id;
    let v5: &dyn tiberius::ToSql = &emp_id;
    let v6: &dyn tiberius::ToSql = &stk_id;
    let v7: &dyn tiberius::ToSql = &real_name;
    let v8: &dyn tiberius::ToSql = &phone;
    let v9: &dyn tiberius::ToSql = &email;
    let v10: &dyn tiberius::ToSql = &remark;
    let v11: &dyn tiberius::ToSql = &now;
    match conn.execute(sql, &[v1, v2, v3, v4, v5, v6, v7, v8, v9, v10, v11]).await {
        Ok(_) => Json(ApiResponse::msg("用户创建成功")),
        Err(e) => Json(ApiResponse::err(&format!("创建用户失败: {}", e))),
    }
}

#[derive(Deserialize)]
pub struct UpdateUserParams {
    pub UserID: String,
    pub UserName: Option<String>,
    pub PassWordStr: Option<String>,
    pub RuleID: Option<String>,
    pub EmpID: Option<String>,
    pub StkID: Option<String>,
    pub RealName: Option<String>,
    pub Phone: Option<String>,
    pub Email: Option<String>,
    pub Remark: Option<String>,
}

pub async fn update_user(
    State(_config): State<Config>,
    Json(body): Json<UpdateUserParams>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("数据库连接失败: {}", e))),
    };
    let now = chrono::Local::now().naive_local();
    let sql = "UPDATE tSys_Users SET UserName = @p1, RuleID = @p2, EmpID = @p3, StkID = @p4, RealName = @p5, \
               Phone = @p6, Email = @p7, Remark = @p8, LUTime = @p9 WHERE UserID = @p10";
    let user_name = body.UserName.unwrap_or_default();
    let rule_id = body.RuleID.unwrap_or_default();
    let emp_id = body.EmpID.unwrap_or_default();
    let stk_id = body.StkID.unwrap_or_default();
    let real_name = body.RealName.unwrap_or_default();
    let phone = body.Phone.unwrap_or_default();
    let email = body.Email.unwrap_or_default();
    let remark = body.Remark.unwrap_or_default();
    let v1: &dyn tiberius::ToSql = &user_name;
    let v2: &dyn tiberius::ToSql = &rule_id;
    let v3: &dyn tiberius::ToSql = &emp_id;
    let v4: &dyn tiberius::ToSql = &stk_id;
    let v5: &dyn tiberius::ToSql = &real_name;
    let v6: &dyn tiberius::ToSql = &phone;
    let v7: &dyn tiberius::ToSql = &email;
    let v8: &dyn tiberius::ToSql = &remark;
    let v9: &dyn tiberius::ToSql = &now;
    let v10: &dyn tiberius::ToSql = &body.UserID;
    match conn.execute(sql, &[v1, v2, v3, v4, v5, v6, v7, v8, v9, v10]).await {
        Ok(_) => {
            // 如果提供新密码则单独更新
            if let Some(p) = body.PassWordStr {
                if !p.is_empty() {
                    let up = "UPDATE tSys_Users SET PassWordStr = @p1, LUTime = @p2 WHERE UserID = @p3";
                    let a: &dyn tiberius::ToSql = &p;
                    let b: &dyn tiberius::ToSql = &now;
                    let c: &dyn tiberius::ToSql = &body.UserID;
                    let _ = conn.execute(up, &[a, b, c]).await;
                }
            }
            Json(ApiResponse::msg("用户更新成功"))
        }
        Err(e) => Json(ApiResponse::err(&format!("更新用户失败: {}", e))),
    }
}

#[derive(Deserialize)]
pub struct DeleteUserParams {
    pub UserID: String,
}

pub async fn delete_user(
    State(_config): State<Config>,
    Json(body): Json<DeleteUserParams>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("数据库连接失败: {}", e))),
    };
    let sql = "UPDATE tSys_Users SET Used = 'N' WHERE UserID = @p1";
    let v: &dyn tiberius::ToSql = &body.UserID;
    match conn.execute(sql, &[v]).await {
        Ok(_) => Json(ApiResponse::msg("用户已停用")),
        Err(e) => Json(ApiResponse::err(&format!("停用用户失败: {}", e))),
    }
}

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
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);
    let mut base_query = "SELECT RuleID, RuleCode, RuleName, Remark, State, EDate FROM tSys_Rule WHERE 1=1".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;
    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(&format!(" AND (RuleCode LIKE @p{} OR RuleName LIKE @p{})", pidx, pidx));
            query_params.push(Some(format!("%{}%", kw)));
            pidx += 1;
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
    let sql = "SELECT * FROM tSys_Dictionary ORDER BY DictType, DCode";
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
    pub sort_prop: Option<String>,
    pub sort_order: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

pub async fn get_oper_log_list(
    State(_config): State<Config>,
    Json(params): Json<OperLogParams>,
) -> Json<ApiResponse<Vec<serde_json::Value>>> {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("数据库连接失败: {}", e))),
    };
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);
    let mut base_query = "SELECT * FROM tSys_OperHis WHERE 1=1".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;
    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(&format!(" AND (OperUser LIKE @p{} OR OperDesc LIKE @p{})", pidx, pidx));
            query_params.push(Some(format!("%{}%", kw)));
            pidx += 1;
        }
    }
    if let Some(sd) = &params.start_date {
        if !sd.is_empty() {
            base_query.push_str(&format!(" AND OperDate >= @p{}", pidx));
            query_params.push(Some(sd.clone()));
            pidx += 1;
        }
    }
    if let Some(ed) = &params.end_date {
        if !ed.is_empty() {
            base_query.push_str(&format!(" AND OperDate <= @p{}", pidx));
            query_params.push(Some(ed.clone()));
            pidx += 1;
        }
    }
    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, params.sort_prop.as_deref(), params.sort_order.as_deref());
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();
    let mut total: i32 = 0;
    if let Ok(stream) = conn.query(&count_sql, &param_refs).await {
        if let Ok(Some(row)) = stream.into_row().await {
            total = row.get::<i32, _>("cnt").unwrap_or(0);
        }
    }
    let data_stream = match conn.query(&paginated_sql, &param_refs).await {
        Ok(s) => s,
        Err(e) => return Json(ApiResponse::err(&format!("查询日志列表失败: {}", e))),
    };
    let rows = match data_stream.into_first_result().await {
        Ok(r) => r,
        Err(e) => return Json(ApiResponse::err(&format!("读取日志数据失败: {}", e))),
    };
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
    Json(ApiResponse::ok_paginated(data, total as u64, page, page_size))
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
            pidx += 1;
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
    let now = chrono::Local::now().naive_local();
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
    let sql = "INSERT INTO tSys_Parameters (ParametersID, PCode, PName, PKind, PHelp, PTerm, PValue, EDate) VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7)";
    let v1: &dyn tiberius::ToSql = &pcode;
    let v2: &dyn tiberius::ToSql = &pname;
    let v3: &dyn tiberius::ToSql = &pkind;
    let v4: &dyn tiberius::ToSql = &phelp;
    let v5: &dyn tiberius::ToSql = &pterm;
    let v6: &dyn tiberius::ToSql = &pvalue;
    let v7: &dyn tiberius::ToSql = &now;
    match conn.execute(sql, &[v1, v2, v3, v4, v5, v6, v7]).await {
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
    let now = chrono::Local::now().naive_local();
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
        let sql = "INSERT INTO tSys_Parameters (ParametersID, PCode, PName, PKind, PValue, EDate) VALUES (NEWID(), @p1, @p2, 'custom', @p3, @p4)";
        let v1: &dyn tiberius::ToSql = &body.key;
        let v2: &dyn tiberius::ToSql = &body.key;
        let v3: &dyn tiberius::ToSql = &val;
        let v4: &dyn tiberius::ToSql = &now;
        match conn.execute(sql, &[v1, v2, v3, v4]).await {
            Ok(_) => Json(ApiResponse::msg("参数已保存")),
            Err(e) => Json(ApiResponse::err(&format!("保存参数失败: {}", e))),
        }
    }
}
