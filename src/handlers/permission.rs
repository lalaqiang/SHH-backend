use axum::{
    extract::{Extension, Multipart, State},
    Json,
};
use serde::Deserialize;
use uuid::Uuid;
use tiberius::Row;
use crate::config::Config;
use crate::db::get_pool;
use crate::error::Result;
use crate::utils::ApiResponse;
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
pub struct RoleIdParams {
    pub RuleID: String,
}

pub async fn get_permissions(
    State(_config): State<Config>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let sql = "SELECT SYM_ID, SYM_PID, SYM_CAPTION, SYM_NO, MDCallName, Used, Flg FROM tSys_Menus WHERE Used = 'Y' ORDER BY SYM_NO";
    let stream = conn.query(sql, &[]).await?;
    let rows: Vec<Row> = stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
    Ok(Json(ApiResponse::ok(data)))
}

pub async fn get_role_permissions(
    State(_config): State<Config>,
    Json(params): Json<RoleIdParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let sql = r#"SELECT rm.RuleMenuID, rm.RuleID, rm.MenuID, rm.CanRead, rm.CanCreate,
                 rm.CanUpdate, rm.CanDelete, rm.CanAudit, rm.CanPrint, rm.LUTime,
                 m.SYM_CAPTION AS MenuName
                 FROM tSys_RuleMenu rm
                 LEFT JOIN tSys_Menus m ON rm.MenuID = m.SYM_ID
                 WHERE rm.RuleID = @p1"#;
    let stream = conn.query(sql, &[&params.RuleID]).await?;
    let rows: Vec<Row> = stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
    Ok(Json(ApiResponse::ok(data)))
}

#[derive(Deserialize)]
pub struct AssignRolePermissionsParams {
    pub RuleID: String,
    pub permissions: Vec<RolePermissionItem>,
}

#[derive(Deserialize)]
pub struct RolePermissionItem {
    pub MenuID: String,
    pub CanRead: Option<String>,
    pub CanCreate: Option<String>,
    pub CanUpdate: Option<String>,
    pub CanDelete: Option<String>,
    pub CanAudit: Option<String>,
    pub CanPrint: Option<String>,
}

pub async fn assign_role_permissions(
    State(_config): State<Config>,
    Json(params): Json<AssignRolePermissionsParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().naive_local();

    let del_sql = "DELETE FROM tSys_RuleMenu WHERE RuleID = @p1";
    conn.execute(del_sql, &[&params.RuleID]).await?;

    for perm in &params.permissions {
        let can_read = perm.CanRead.as_deref().unwrap_or("N");
        let can_create = perm.CanCreate.as_deref().unwrap_or("N");
        let can_update = perm.CanUpdate.as_deref().unwrap_or("N");
        let can_delete = perm.CanDelete.as_deref().unwrap_or("N");
        let can_audit = perm.CanAudit.as_deref().unwrap_or("N");
        let can_print = perm.CanPrint.as_deref().unwrap_or("N");

        let ins_sql = r#"INSERT INTO tSys_RuleMenu (RuleMenuID, RuleID, MenuID, CanRead, CanCreate, CanUpdate, CanDelete, CanAudit, CanPrint, LUTime)
                         VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9)"#;
        conn.execute(ins_sql, &[
            &params.RuleID,
            &perm.MenuID,
            &can_read,
            &can_create,
            &can_update,
            &can_delete,
            &can_audit,
            &can_print,
            &now,
        ]).await?;
    }

    Ok(Json(ApiResponse::msg("权限分配成功")))
}

#[derive(Deserialize)]
pub struct EmpIdParams {
    pub EmpID: String,
}

pub async fn get_user_permissions(
    State(_config): State<Config>,
    Json(params): Json<EmpIdParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let sql = r#"SELECT rm.RuleMenuID, rm.RuleID, rm.MenuID, rm.CanRead, rm.CanCreate,
                 rm.CanUpdate, rm.CanDelete, rm.CanAudit, rm.CanPrint,
                 m.SYM_CAPTION AS MenuName
                 FROM tSys_UserRule ur
                 INNER JOIN tSys_RuleMenu rm ON ur.RuleID = rm.RuleID
                 LEFT JOIN tSys_Menus m ON rm.MenuID = m.SYM_ID
                 WHERE ur.EmpID = @p1"#;
    let stream = conn.query(sql, &[&params.EmpID]).await?;
    let rows: Vec<Row> = stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
    Ok(Json(ApiResponse::ok(data)))
}

#[derive(Deserialize)]
pub struct GetRolesParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
}

pub async fn get_roles(
    State(_config): State<Config>,
    Json(params): Json<GetRolesParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);

    let mut base_query = r#"SELECT r.RuleID, r.RuleName, r.Note, r.Flg, r.State,
                            (SELECT COUNT(*) FROM tSys_RuleMenu rm WHERE rm.RuleID = r.RuleID) AS MenuCount,
                            (SELECT COUNT(*) FROM tSys_UserRule ur WHERE ur.RuleID = r.RuleID) AS UserCount
                            FROM tSys_Rule r WHERE r.State <> 'D'"#.to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(&format!(
                " AND (r.RuleName LIKE @p{} OR r.Note LIKE @p{})",
                pidx, pidx + 1
            ));
            pidx += 2;
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
        }
    }

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let offset = (page - 1) * page_size;
    let top = offset + page_size;
    let paginated_sql = format!(
        "SELECT * FROM (SELECT TOP ({top}) ROW_NUMBER() OVER (ORDER BY (SELECT NULL)) as _rn, * FROM ({base_query}) t) p WHERE _rn > {offset}",
        top = top, base_query = base_query, offset = offset
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
pub struct CreateRoleParams {
    pub RuleName: String,
    pub Note: Option<String>,
    pub Flg: Option<String>,
    pub State: Option<String>,
}

pub async fn create_role(
    State(_config): State<Config>,
    Json(body): Json<CreateRoleParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let note = body.Note.as_deref().unwrap_or("");
    let flg = body.Flg.as_deref().unwrap_or("");
    let state = body.State.as_deref().unwrap_or("Y");

    let sql = r#"INSERT INTO tSys_Rule (RuleID, RuleName, Note, Flg, State)
                 VALUES (NEWID(), @p1, @p2, @p3, @p4)"#;
    conn.execute(sql, &[&body.RuleName, &note, &flg, &state]).await?;

    Ok(Json(ApiResponse::msg("角色创建成功")))
}

#[derive(Deserialize)]
pub struct UpdateRoleParams {
    pub RuleID: String,
    pub RuleName: Option<String>,
    pub Note: Option<String>,
    pub Flg: Option<String>,
    pub State: Option<String>,
}

pub async fn update_role(
    State(_config): State<Config>,
    Json(body): Json<UpdateRoleParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let rule_name = body.RuleName.as_deref().unwrap_or("");
    let note = body.Note.as_deref().unwrap_or("");
    let flg = body.Flg.as_deref().unwrap_or("");
    let state = body.State.as_deref().unwrap_or("Y");

    let sql = r#"UPDATE tSys_Rule SET RuleName = @p1, Note = @p2, Flg = @p3, State = @p4
                 WHERE RuleID = @p5"#;
    conn.execute(sql, &[&rule_name, &note, &flg, &state, &body.RuleID]).await?;

    Ok(Json(ApiResponse::msg("角色更新成功")))
}

#[derive(Deserialize)]
pub struct DeleteRoleParams {
    pub RuleID: String,
}

pub async fn delete_role(
    State(_config): State<Config>,
    Json(body): Json<DeleteRoleParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let del_rule_menu = "DELETE FROM tSys_RuleMenu WHERE RuleID = @p1";
    conn.execute(del_rule_menu, &[&body.RuleID]).await?;

    let del_user_rule = "DELETE FROM tSys_UserRule WHERE RuleID = @p1";
    conn.execute(del_user_rule, &[&body.RuleID]).await?;

    let del_role = "DELETE FROM tSys_Rule WHERE RuleID = @p1";
    conn.execute(del_role, &[&body.RuleID]).await?;

    Ok(Json(ApiResponse::msg("角色删除成功")))
}

#[derive(Deserialize)]
pub struct AssignUserRolesParams {
    pub EmpID: String,
    pub RuleIDs: Vec<String>,
}

pub async fn assign_user_roles(
    State(_config): State<Config>,
    Json(params): Json<AssignUserRolesParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().naive_local();

    let del_sql = "DELETE FROM tSys_UserRule WHERE EmpID = @p1";
    conn.execute(del_sql, &[&params.EmpID]).await?;

    for rule_id in &params.RuleIDs {
        let ins_sql = r#"INSERT INTO tSys_UserRule (UserRuleID, EmpID, RuleID, LUTime)
                         VALUES (NEWID(), @p1, @p2, @p3)"#;
        conn.execute(ins_sql, &[&params.EmpID, rule_id, &now]).await?;
    }

    Ok(Json(ApiResponse::msg("用户角色分配成功")))
}

#[derive(Deserialize)]
pub struct SaveTableColumnConfigParams {
    pub EmpID: String,
    pub TableName: String,
    pub ConfigData: String,
}

pub async fn save_table_column_config(
    State(_config): State<Config>,
    Json(params): Json<SaveTableColumnConfigParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    eprintln!("[save_table_column_config] 进入 EmpID={:?} TableName={:?} ConfigData.len={}",
        params.EmpID, params.TableName, params.ConfigData.len());

    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[save_table_column_config] 获取连接失败: {}", e);
            return Ok(Json(ApiResponse::err(&format!("连接数据库失败: {}", e))));
        }
    };
    eprintln!("[save_table_column_config] 已拿到连接");

    let now = chrono::Local::now().naive_local();

    let emp_uuid_str = match Uuid::parse_str(params.EmpID.trim()) {
        Ok(u) => u.to_string(),
        Err(e) => {
            eprintln!("[save_table_column_config] UUID 解析失败: {}", e);
            return Ok(Json(ApiResponse::err(&format!("EmpID 不是有效 UUID: {}", e))));
        }
    };
    eprintln!("[save_table_column_config] emp_uuid_str={}", emp_uuid_str);

    let check_sql = "SELECT ColumnConfigID FROM tSys_TableColumnConfig WHERE EmpID = CAST(@p1 AS uniqueidentifier) AND TableName = @p2";
    let stream = match conn.query(check_sql, &[&emp_uuid_str, &params.TableName]).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[save_table_column_config] check 查询失败: {}", e);
            return Ok(Json(ApiResponse::err(&format!("SQL 查询失败: {}", e))));
        }
    };
    let existing = match stream.into_row().await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[save_table_column_config] check 取行失败: {}", e);
            return Ok(Json(ApiResponse::err(&format!("取行失败: {}", e))));
        }
    };
    eprintln!("[save_table_column_config] 已存在?={}", existing.is_some());

    if existing.is_some() {
        let upd_sql = r#"UPDATE tSys_TableColumnConfig
                         SET ConfigData = @p1, LUTime = @p2
                         WHERE EmpID = CAST(@p3 AS uniqueidentifier) AND TableName = @p4"#;
        match conn.execute(upd_sql, &[&params.ConfigData, &now, &emp_uuid_str, &params.TableName]).await {
            Ok(_) => {
                eprintln!("[save_table_column_config] UPDATE 成功 TableName={}", params.TableName);
            }
            Err(e) => {
                eprintln!("[save_table_column_config] UPDATE 失败: {}", e);
                return Ok(Json(ApiResponse::err(&format!("UPDATE 失败: {}", e))));
            }
        }
    } else {
        let ins_sql = r#"INSERT INTO tSys_TableColumnConfig (ColumnConfigID, EmpID, TableName, ConfigData, LUTime)
                         VALUES (NEWID(), CAST(@p1 AS uniqueidentifier), @p2, @p3, @p4)"#;
        match conn.execute(ins_sql, &[&emp_uuid_str, &params.TableName, &params.ConfigData, &now]).await {
            Ok(_) => {
                eprintln!("[save_table_column_config] INSERT 成功 TableName={}", params.TableName);
            }
            Err(e) => {
                eprintln!("[save_table_column_config] INSERT 失败: {}", e);
                return Ok(Json(ApiResponse::err(&format!("INSERT 失败: {}", e))));
            }
        }
    }

    Ok(Json(ApiResponse::msg("列配置保存成功")))
}

#[derive(Deserialize)]
pub struct GetTableColumnConfigParams {
    pub EmpID: String,
    pub TableName: String,
}

pub async fn get_table_column_config(
    State(_config): State<Config>,
    Json(params): Json<GetTableColumnConfigParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    eprintln!("[get_table_column_config] 进入 EmpID={:?} TableName={:?}", params.EmpID, params.TableName);
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[get_table_column_config] 获取连接失败: {}", e);
            return Ok(Json(ApiResponse::err(&format!("连接数据库失败: {}", e))));
        }
    };
    eprintln!("[get_table_column_config] 已拿到连接");
    let emp_uuid_str = match Uuid::parse_str(params.EmpID.trim()) {
        Ok(u) => u.to_string(),
        Err(e) => {
            eprintln!("[get_table_column_config] EmpID UUID 解析失败: {}", e);
            return Ok(Json(ApiResponse::err(&format!("EmpID 不是有效 UUID: {}", e))));
        }
    };
    eprintln!("[get_table_column_config] 准备执行 SQL emp_uuid_str={}", emp_uuid_str);
    let sql = "SELECT ColumnConfigID, EmpID, TableName, ConfigData, LUTime FROM tSys_TableColumnConfig WHERE EmpID = CAST(@p1 AS uniqueidentifier) AND TableName = @p2";
    let stream = match conn.query(sql, &[&emp_uuid_str, &params.TableName]).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[get_table_column_config] conn.query 失败: {}", e);
            return Ok(Json(ApiResponse::err(&format!("SQL 查询失败: {}", e))));
        }
    };
    eprintln!("[get_table_column_config] SQL 已执行, 准备 into_row");
    let row = match stream.into_row().await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[get_table_column_config] into_row 失败: {}", e);
            return Ok(Json(ApiResponse::err(&format!("取行失败: {}", e))));
        }
    };
    eprintln!("[get_table_column_config] into_row 完成, row.is_some={}", row.is_some());

    match row {
        Some(r) => Ok(Json(ApiResponse::ok(row_to_json(&r)))),
        None => Ok(Json(ApiResponse::ok(serde_json::Value::Null))),
    }
}

#[derive(Deserialize)]
pub struct DeleteTableColumnConfigParams {
    pub ColumnConfigID: String,
}

pub async fn delete_table_column_config(
    State(_config): State<Config>,
    Json(params): Json<DeleteTableColumnConfigParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let sql = "DELETE FROM tSys_TableColumnConfig WHERE ColumnConfigID = @p1";
    conn.execute(sql, &[&params.ColumnConfigID]).await?;
    Ok(Json(ApiResponse::msg("列配置删除成功")))
}

#[derive(Deserialize)]
pub struct SaveColumnPresetParams {
    pub EmpID: String,
    pub TableName: String,
    pub PresetName: String,
    pub ConfigData: String,
    pub IsDefault: Option<bool>,
}

pub async fn save_column_preset(
    State(_config): State<Config>,
    Json(params): Json<SaveColumnPresetParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().naive_local();
    let is_default = params.IsDefault.unwrap_or(false);

    let emp_uuid_str = match Uuid::parse_str(params.EmpID.trim()) {
        Ok(u) => u.to_string(),
        Err(e) => {
            return Ok(Json(ApiResponse::err(&format!("EmpID 不是有效 UUID: {}", e))));
        }
    };

    if is_default {
        let reset_sql = r#"UPDATE tSys_ColumnPreset SET IsDefault = 0
                          WHERE EmpID = CAST(@p1 AS uniqueidentifier) AND TableName = @p2 AND IsDefault = 1"#;
        conn.execute(reset_sql, &[&emp_uuid_str, &params.TableName]).await?;
    }

    let ins_sql = r#"INSERT INTO tSys_ColumnPreset (PresetID, EmpID, TableName, PresetName, ConfigData, IsDefault, LUTime)
                     OUTPUT INSERTED.PresetID
                     VALUES (NEWID(), CAST(@p1 AS uniqueidentifier), @p2, @p3, @p4, @p5, @p6)"#;
    let stream = conn.query(ins_sql, &[
        &emp_uuid_str,
        &params.TableName,
        &params.PresetName,
        &params.ConfigData,
        &is_default,
        &now,
    ]).await?;

    let row = stream.into_row().await?;
    let preset_id = row.and_then(|r| {
        r.try_get::<uuid::Uuid, _>("PresetID")
            .ok()
            .flatten()
            .map(|u| u.to_string())
    }).unwrap_or_default();

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "PresetID": preset_id
    }))))
}

#[derive(Deserialize)]
pub struct ListColumnPresetsParams {
    pub EmpID: String,
    pub TableName: String,
}

pub async fn list_column_presets(
    State(_config): State<Config>,
    Json(params): Json<ListColumnPresetsParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let emp_uuid_str = match Uuid::parse_str(params.EmpID.trim()) {
        Ok(u) => u.to_string(),
        Err(e) => {
            return Ok(Json(ApiResponse::err(&format!("EmpID 不是有效 UUID: {}", e))));
        }
    };

    let sql = r#"SELECT PresetID, PresetName, IsDefault, LUTime
                 FROM tSys_ColumnPreset
                 WHERE EmpID = CAST(@p1 AS uniqueidentifier) AND TableName = @p2
                 ORDER BY IsDefault DESC, LUTime DESC"#;
    let stream = conn.query(sql, &[&emp_uuid_str, &params.TableName]).await?;
    let rows: Vec<Row> = stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
    Ok(Json(ApiResponse::ok(data)))
}

#[derive(Deserialize)]
pub struct DeleteColumnPresetParams {
    pub PresetID: String,
}

pub async fn delete_column_preset(
    State(_config): State<Config>,
    Json(params): Json<DeleteColumnPresetParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let sql = "DELETE FROM tSys_ColumnPreset WHERE PresetID = @p1";
    conn.execute(sql, &[&params.PresetID]).await?;
    Ok(Json(ApiResponse::msg("预设删除成功")))
}

#[derive(Deserialize)]
pub struct ApplyColumnPresetParams {
    pub PresetID: String,
}

pub async fn apply_column_preset(
    State(_config): State<Config>,
    Json(params): Json<ApplyColumnPresetParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let sql = "SELECT ConfigData FROM tSys_ColumnPreset WHERE PresetID = @p1";
    let stream = conn.query(sql, &[&params.PresetID]).await?;
    let row = stream.into_row().await?;

    match row {
        Some(r) => {
            let config_data = r.get::<&str, _>("ConfigData").unwrap_or("[]").to_string();
            Ok(Json(ApiResponse::ok(serde_json::json!(config_data))))
        }
        None => Ok(Json(ApiResponse::err("预设不存在"))),
    }
}

pub async fn upload_file(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    mut multipart: Multipart,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut file_id = String::new();
    let mut biz_type = String::new();
    let mut biz_id = String::new();
    let mut original_name = String::new();
    let mut file_size: i64 = 0;

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        crate::error::AppError::BadRequest(format!("读取上传字段失败: {}", e))
    })? {
        let field_name = field.name().unwrap_or("").to_string();

        match field_name.as_str() {
            "biz_type" => {
                biz_type = field.text().await.map_err(|e| {
                    crate::error::AppError::BadRequest(format!("读取biz_type失败: {}", e))
                })?;
            }
            "biz_id" => {
                biz_id = field.text().await.map_err(|e| {
                    crate::error::AppError::BadRequest(format!("读取biz_id失败: {}", e))
                })?;
            }
            "file" => {
                original_name = field.file_name().unwrap_or("unknown").to_string();
                let data = field.bytes().await.map_err(|e| {
                    crate::error::AppError::BadRequest(format!("读取文件数据失败: {}", e))
                })?;
                file_size = data.len() as i64;

                file_id = format!("{}", uuid::Uuid::new_v4());

                let dir_name = if biz_type.is_empty() { "default".to_string() } else { biz_type.clone() };
                let dir_path = format!("./uploads/{}", dir_name);
                std::fs::create_dir_all(&dir_path).map_err(|e| {
                    crate::error::AppError::Internal(format!("创建上传目录失败: {}", e))
                })?;

                let ext = std::path::Path::new(&original_name)
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| format!(".{}", e))
                    .unwrap_or_default();
                let file_name = format!("{}_{}{}", file_id, original_name.replace('.', "_"), ext);
                let full_path = format!("{}/{}", dir_path, file_name);

                std::fs::write(&full_path, &data).map_err(|e| {
                    crate::error::AppError::Internal(format!("保存文件失败: {}", e))
                })?;
            }
            _ => {}
        }
    }

    if file_id.is_empty() {
        return Ok(Json(ApiResponse::err("未找到上传文件")));
    }

    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().naive_local();
    let relative_path = format!("uploads/{}/{}_{}", biz_type, file_id, original_name);

    let sql = r#"INSERT INTO tSys_UploadFile (FileID, BizType, BizID, FileName, FilePath, FileSize, UploadUser, UploadTime)
                 VALUES (@p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8)"#;
    conn.execute(sql, &[
        &file_id,
        &biz_type,
        &biz_id,
        &original_name,
        &relative_path,
        &file_size,
        &claims.user_code.as_str(),
        &now,
    ]).await?;

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "FileID": file_id,
        "FileName": original_name,
        "FilePath": relative_path,
        "FileSize": file_size
    }))))
}

#[derive(Deserialize)]
pub struct GetUploadedFilesParams {
    pub BizType: String,
    pub BizID: String,
}

pub async fn get_uploaded_files(
    State(_config): State<Config>,
    Json(params): Json<GetUploadedFilesParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let sql = r#"SELECT FileID, BizType, BizID, FileName, FilePath, FileSize, UploadUser, UploadTime
                 FROM tSys_UploadFile WHERE BizType = @p1 AND BizID = @p2 ORDER BY UploadTime"#;
    let stream = conn.query(sql, &[&params.BizType, &params.BizID]).await?;
    let rows: Vec<Row> = stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
    Ok(Json(ApiResponse::ok(data)))
}

pub async fn get_system_overview(
    State(_config): State<Config>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let user_count: i32;
    let user_sql = "SELECT COUNT(*) as cnt FROM tSys_User WHERE State <> 'D'";
    let stream = conn.query(user_sql, &[]).await?;
    if let Some(row) = stream.into_row().await? {
        user_count = row.get::<i32, _>("cnt").unwrap_or(0);
    } else {
        user_count = 0;
    }

    let role_count: i32;
    let role_sql = "SELECT COUNT(*) as cnt FROM tSys_Rule WHERE State <> 'D'";
    let stream = conn.query(role_sql, &[]).await?;
    if let Some(row) = stream.into_row().await? {
        role_count = row.get::<i32, _>("cnt").unwrap_or(0);
    } else {
        role_count = 0;
    }

    let menu_count: i32;
    let menu_sql = "SELECT COUNT(*) as cnt FROM tSys_Menus WHERE Used = 'Y'";
    let stream = conn.query(menu_sql, &[]).await?;
    if let Some(row) = stream.into_row().await? {
        menu_count = row.get::<i32, _>("cnt").unwrap_or(0);
    } else {
        menu_count = 0;
    }

    let purchase_count: i32;
    let purchase_sql = "SELECT COUNT(*) as cnt FROM tPur_Order WHERE State <> 'D'";
    let stream = conn.query(purchase_sql, &[]).await?;
    if let Some(row) = stream.into_row().await? {
        purchase_count = row.get::<i32, _>("cnt").unwrap_or(0);
    } else {
        purchase_count = 0;
    }

    let sales_count: i32;
    let sales_sql = "SELECT COUNT(*) as cnt FROM tSal_Order WHERE State <> 'D'";
    let stream = conn.query(sales_sql, &[]).await?;
    if let Some(row) = stream.into_row().await? {
        sales_count = row.get::<i32, _>("cnt").unwrap_or(0);
    } else {
        sales_count = 0;
    }

    let goods_count: i32;
    let goods_sql = "SELECT COUNT(*) as cnt FROM tBas_Goods WHERE State <> 'D'";
    let stream = conn.query(goods_sql, &[]).await?;
    if let Some(row) = stream.into_row().await? {
        goods_count = row.get::<i32, _>("cnt").unwrap_or(0);
    } else {
        goods_count = 0;
    }

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "userCount": user_count,
        "roleCount": role_count,
        "menuCount": menu_count,
        "purchaseOrderCount": purchase_count,
        "salesOrderCount": sales_count,
        "goodsCount": goods_count
    }))))
}

pub async fn get_public_company_name(
    State(_config): State<Config>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let sql = "SELECT TOP 1 PValue FROM tSys_Parameters WHERE PCode = 'company_name'";
    let stream = conn.query(sql, &[]).await?;
    let row = stream.into_row().await?;

    let company_name = match row {
        Some(r) => r.get::<&str, _>("PValue").unwrap_or("").to_string(),
        None => String::new(),
    };

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "company_name": company_name
    }))))
}

pub async fn get_public_warehouses(
    State(_config): State<Config>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let sql = "SELECT StkID, StkName, StkNO FROM tBas_Stock WHERE Used = 'Y' AND State <> 'D' ORDER BY StkNO";
    let stream = conn.query(sql, &[]).await?;
    let rows: Vec<Row> = stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
    Ok(Json(ApiResponse::ok(data)))
}
