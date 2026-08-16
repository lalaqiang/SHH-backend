use crate::config::Config;
use crate::db::get_pool;
use crate::error::Result;
use crate::handlers::base_data::{row_to_json, try_get_value};
use crate::middleware::auth::Claims;
use crate::utils::ApiResponse;
use axum::{
    Json,
    extract::{Extension, Multipart, State},
};
use serde::Deserialize;
use tiberius::Row;
use uuid::Uuid;

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
                 rm.CanUpdate, rm.CanDelete, rm.CanAudit, rm.CanPrint, rm.CanExport, rm.LUTime,
                 m.SYM_CAPTION AS MenuName, m.SYM_NO AS MenuCode
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
    pub CanRead: Option<i32>,
    pub CanCreate: Option<i32>,
    pub CanUpdate: Option<i32>,
    pub CanDelete: Option<i32>,
    pub CanAudit: Option<i32>,
    pub CanPrint: Option<i32>,
    pub CanExport: Option<i32>,
}

/// 将权限标志归一化为 i32 (1/0)
/// 兼容前端可能传来的 bool/null/数字/字符串
fn norm_flag(v: Option<i32>) -> i32 {
    v.unwrap_or(0)
}

pub async fn assign_role_permissions(
    State(_config): State<Config>,
    Extension(claims): Extension<Claims>,
    Json(params): Json<AssignRolePermissionsParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    // 事务包裹：DELETE 旧权限 + INSERT 新权限 原子化
    // 避免删除后写入失败导致该角色权限被全部清空
    let tx_result: std::result::Result<(), String> = async {
        crate::services::inventory_ledger::begin_tran(&mut conn).await.map_err(|e| e.to_string())?;

        let del_sql = "DELETE FROM tSys_RuleMenu WHERE RuleID = @p1";
        conn.execute(del_sql, &[&params.RuleID]).await.map_err(|e| e.to_string())?;

        for perm in &params.permissions {
            let can_read = norm_flag(perm.CanRead);
            let can_create = norm_flag(perm.CanCreate);
            let can_update = norm_flag(perm.CanUpdate);
            let can_delete = norm_flag(perm.CanDelete);
            let can_audit = norm_flag(perm.CanAudit);
            let can_print = norm_flag(perm.CanPrint);
            let can_export = norm_flag(perm.CanExport);

            let ins_sql = r#"INSERT INTO tSys_RuleMenu (RuleMenuID, RuleID, MenuID, CanRead, CanCreate, CanUpdate, CanDelete, CanAudit, CanPrint, CanExport, LUTime)
                             VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10)"#;
            conn.execute(ins_sql, &[
                &params.RuleID,
                &perm.MenuID,
                &can_read,
                &can_create,
                &can_update,
                &can_delete,
                &can_audit,
                &can_print,
                &can_export,
                &now,
            ]).await.map_err(|e| e.to_string())?;
        }

        crate::services::inventory_ledger::commit_tran(&mut conn).await.map_err(|e| e.to_string())?;
        Ok(())
    }.await;
    if let Err(e) = tx_result {
        crate::services::inventory_ledger::rollback_tran(&mut conn).await;
        return Ok(Json(ApiResponse::err(&crate::utils::db_err(
            "权限分配失败: {}",
            &e,
        ))));
    }

    // 清除所有用户的权限缓存（角色权限变更可能影响多个用户）
    crate::middleware::permission::invalidate_all_permission_cache();

    // 审计日志：记录权限分配操作
    let audit_remark = format!("分配角色权限：共 {} 项菜单权限", params.permissions.len());
    crate::handlers::audit_log::log_perm_action(
        &mut conn,
        crate::handlers::audit_log::OPER_ASSIGN_PERM,
        "tSys_RuleMenu",
        &params.RuleID,
        &claims,
        &audit_remark,
    )
    .await;

    Ok(Json(ApiResponse::msg("权限分配成功")))
}

#[derive(Deserialize)]
pub struct EmpIdParams {
    pub EmpID: String,
}

pub async fn get_user_permissions(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<EmpIdParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    // P1-8 安全校验：仅 admin 或查询自己的权限允许，避免越权查他人权限
    let is_admin = claims.user_code.eq_ignore_ascii_case("admin");
    let is_self = !claims.emp_id.is_empty() && claims.emp_id == params.EmpID;
    if !is_admin && !is_self {
        return Ok(Json(ApiResponse::err_with_code(
            "无权限查询其他用户的权限信息",
            "PERMISSION_DENIED",
        )));
    }

    let mut conn = get_pool().get().await?;
    let sql = r#"SELECT rm.RuleMenuID, rm.RuleID, rm.MenuID, rm.CanRead, rm.CanCreate,
                 rm.CanUpdate, rm.CanDelete, rm.CanAudit, rm.CanPrint, rm.CanExport,
                 m.SYM_CAPTION AS MenuName, m.SYM_NO AS MenuCode
                 FROM tSys_UserRule ur
                 INNER JOIN tSys_RuleMenu rm ON ur.RuleID = rm.RuleID
                 LEFT JOIN tSys_Menus m ON rm.MenuID = m.SYM_ID
                 WHERE ur.EmpID = @p1"#;
    let stream = conn.query(sql, &[&params.EmpID]).await?;
    let rows: Vec<Row> = stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
    Ok(Json(ApiResponse::ok(data)))
}

/// 获取当前登录用户的权限码列表（供前端路由守卫校验）+ 完整动态菜单树
///
/// 从 JWT Claims 提取 emp_id，查询用户有权限的菜单 SYM_NO（权限码）列表，
/// 并返回完整菜单字段（图标/排序/路径/可见性），构造为树形结构供前端侧边栏直接渲染。
///
/// **权限码生成规则**（按钮级权限）：
///   对每个菜单，根据 CanRead/CanCreate/CanUpdate/CanDelete/CanAudit/CanPrint/CanExport
///   生成 `${base_code}.${action}` 形式的权限码，如 `system.user.create`。
///   base_code 优先取 SYM_NO，其次 MDCallName，最后 SYM_ID。
///
/// **admin 超级权限**：工号为 admin 的用户返回 `["*"]`，前端 hasPermission 对 `*` 直接放行。
///
/// 返回格式：
///   ```json
///   {
///     "success": true,
///     "data": {
///       "permissions": ["system.user.read", "system.user.create", ...],
///       "menus": [ { "id": "1", "pid": "", "label": "基础资料", ... } ]
///     }
///   }
///   ```
///
/// 向后兼容：如果用户无任何角色分配（tSys_UserRule 无记录），返回空列表，
/// 前端 `hasPermission` 对空列表全放行，前端 stores/app.js 检测到 menus 为空时
/// 回退到 hardcoded menuData，确保不锁死系统。
pub async fn get_my_permissions(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let emp_id = claims.emp_id.clone();
    if emp_id.is_empty() {
        return Ok(Json(ApiResponse::ok(serde_json::json!({
            "permissions": [],
            "menus": [],
        }))));
    }

    // admin 超级权限：工号 admin 直接返回 ["*"]，拥有所有权限
    // 同时仍返回完整菜单树供前端渲染（查全部启用菜单）
    let is_admin = claims.user_code.eq_ignore_ascii_case("admin");
    if is_admin {
        // 注意：tSys_Menus 表没有 SYM_Order/SYM_Visible 字段，使用 SYM_NO 排序
        // PermCode 字段存储语义化权限码（如 base.goods / purchase.order）
        // MDCallName 字段存储前端路由路径（如 /base/product）
        let sql = r#"SELECT m.SYM_ID, m.SYM_PID, m.SYM_CAPTION, m.SYM_NO, m.MDCallName,
                     m.SYM_PPT, m.Used, m.PermCode
                     FROM tSys_Menus m
                     WHERE ISNULL(m.Used, 'Y') = 'Y'
                     ORDER BY m.SYM_NO"#;
        let stream = conn.query(sql, &[]).await?;
        let rows: Vec<Row> = stream.into_first_result().await?;
        let mut flat_menus: Vec<serde_json::Value> = Vec::new();
        for r in &rows {
            let sym_no: String = get_str_col(r, "SYM_NO");
            let md_call: String = get_str_col(r, "MDCallName");
            let sym_id: String = get_str_col(r, "SYM_ID");
            let sym_pid: String = get_str_col(r, "SYM_PID");
            let caption: String = get_str_col(r, "SYM_CAPTION");
            let perm_code: String = get_str_col(r, "PermCode");
            // 权限码：优先用 PermCode（语义化），其次 MDCallName，最后 SYM_NO
            let code = if !perm_code.is_empty() {
                perm_code.clone()
            } else if !md_call.is_empty() {
                md_call.clone()
            } else if !sym_no.is_empty() {
                sym_no.clone()
            } else {
                sym_id.clone()
            };
            // 路径直接用 MDCallName（存储前端路由路径如 /base/product）
            let path = md_call.clone();
            flat_menus.push(serde_json::json!({
                "id": sym_id,
                "pid": sym_pid,
                "label": caption,
                "code": code,
                "path": path,
                "icon": "",
                "order": 0,
                "visible": "Y",
                "canRead": "1", "canCreate": "1", "canUpdate": "1",
                "canDelete": "1", "canAudit": "1", "canPrint": "1", "canExport": "1",
            }));
        }
        let menu_tree = build_menu_tree(&flat_menus);
        return Ok(Json(ApiResponse::ok(serde_json::json!({
            "permissions": vec!["*".to_string()],
            "menus": menu_tree,
            "isAdmin": true,
        }))));
    }

    // 查询当前用户有 CanRead 权限的菜单列表
    // 注意：tSys_Menus 表没有 SYM_Order/SYM_Visible 字段
    // MDCallName 字段存储前端路由路径（如 /base/product）
    let sql = r#"SELECT m.SYM_ID, m.SYM_PID, m.SYM_CAPTION, m.SYM_NO, m.MDCallName,
                 m.SYM_PPT, m.Used, m.PermCode,
                 rm.CanRead, rm.CanCreate, rm.CanUpdate, rm.CanDelete, rm.CanAudit, rm.CanPrint, rm.CanExport
                 FROM tSys_UserRule ur
                 INNER JOIN tSys_RuleMenu rm ON ur.RuleID = rm.RuleID
                 LEFT JOIN tSys_Menus m ON rm.MenuID = m.SYM_ID
                 WHERE ur.EmpID = @p1 AND ISNULL(m.Used, 'Y') = 'Y'"#;
    let stream = conn.query(sql, &[&emp_id]).await?;
    let rows: Vec<Row> = stream.into_first_result().await?;

    let mut permissions: Vec<String> = Vec::new();
    let mut flat_menus: Vec<serde_json::Value> = Vec::new();
    for r in &rows {
        let sym_no: String = get_str_col(r, "SYM_NO");
        let md_call: String = get_str_col(r, "MDCallName");
        let sym_id: String = get_str_col(r, "SYM_ID");
        let sym_pid: String = get_str_col(r, "SYM_PID");
        let caption: String = get_str_col(r, "SYM_CAPTION");
        let perm_code: String = get_str_col(r, "PermCode");
        let order: i32 = 0;
        let visible: String = "Y".to_string();
        // 权限码：与 admin 分支保持一致
        let code = if !perm_code.is_empty() {
            perm_code.clone()
        } else if !md_call.is_empty() {
            md_call.clone()
        } else if !sym_no.is_empty() {
            sym_no.clone()
        } else {
            sym_id.clone()
        };
        // 路径直接用 MDCallName（存储前端路由路径如 /base/product）
        let path = md_call.clone();

        // 读取 7 个动作权限位（int 类型，可能为 0/1/null）
        let can_read = read_perm_flag(r, "CanRead");
        let can_create = read_perm_flag(r, "CanCreate");
        let can_update = read_perm_flag(r, "CanUpdate");
        let can_delete = read_perm_flag(r, "CanDelete");
        let can_audit = read_perm_flag(r, "CanAudit");
        let can_print = read_perm_flag(r, "CanPrint");
        let can_export = read_perm_flag(r, "CanExport");

        // 生成按钮级权限码：${base_code}.${action}
        // 只有 CanRead=1 的菜单才生成其他动作权限码（无读权限的菜单不应出现在列表中）
        if !code.is_empty() {
            if can_read {
                permissions.push(format!("{}.read", code));
            }
            if can_create {
                permissions.push(format!("{}.create", code));
            }
            if can_update {
                permissions.push(format!("{}.update", code));
            }
            if can_delete {
                permissions.push(format!("{}.delete", code));
            }
            if can_audit {
                permissions.push(format!("{}.audit", code));
            }
            if can_print {
                permissions.push(format!("{}.print", code));
            }
            if can_export {
                permissions.push(format!("{}.export", code));
            }
        }

        // 路径直接用 MDCallName（存储前端路由路径如 /base/product）
        flat_menus.push(serde_json::json!({
            "id": sym_id,
            "pid": sym_pid,
            "label": caption,
            "code": code,
            "path": path,
            "icon": "",
            "order": order,
            "visible": visible,
            "canRead": can_read as i32,
            "canCreate": can_create as i32,
            "canUpdate": can_update as i32,
            "canDelete": can_delete as i32,
            "canAudit": can_audit as i32,
            "canPrint": can_print as i32,
            "canExport": can_export as i32,
        }));
    }

    // 构造树形结构：按 SYM_PID 关联，根节点为 SYM_PID 为空/'0'/null 的菜单，按 order 升序
    let menu_tree = build_menu_tree(&flat_menus);

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "permissions": permissions,
        "menus": menu_tree,
        "isAdmin": false,
    }))))
}

/// 读取权限标志位，兼容 int / 字符串 "Y"/"N" / null
fn read_perm_flag(row: &Row, col: &str) -> bool {
    // 优先按 i32 读取
    if let Ok(Some(v)) = row.try_get::<i32, _>(col) {
        return v != 0;
    }
    // 兜底：按字符串读取（历史数据可能是 "Y"/"N"）
    if let Ok(Some(s)) = row.try_get::<&str, _>(col) {
        return s.eq_ignore_ascii_case("Y") || s == "1";
    }
    false
}

/// 将扁平菜单列表构造为树形结构
/// 根节点判定：pid 为空、'0'、'00000000-0000-0000-0000-000000000000' 或 pid 在列表中找不到父节点
fn build_menu_tree(flat: &[serde_json::Value]) -> Vec<serde_json::Value> {
    use std::collections::HashMap;

    // 收集所有 id，用于判断根节点
    let mut id_set: HashMap<String, bool> = HashMap::new();
    for m in flat {
        if let Some(id) = m.get("id").and_then(|v| v.as_str()) {
            id_set.insert(id.to_string(), true);
        }
    }

    // 分组：pid → 子节点列表
    let mut children_map: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
    let mut roots: Vec<serde_json::Value> = Vec::new();

    for m in flat {
        let pid = m
            .get("pid")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let is_root = pid.is_empty()
            || pid == "0"
            || pid == "00000000-0000-0000-0000-000000000000"
            || !id_set.contains_key(&pid);
        if is_root {
            roots.push(m.clone());
        } else {
            children_map.entry(pid).or_default().push(m.clone());
        }
    }

    // 递归挂载 children，并按 order 升序排序
    fn attach_children(
        node: &mut serde_json::Value,
        children_map: &HashMap<String, Vec<serde_json::Value>>,
    ) {
        if let Some(obj) = node.as_object_mut() {
            let id = obj
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if let Some(mut children) = children_map.get(&id).cloned() {
                for child in children.iter_mut() {
                    attach_children(child, children_map);
                }
                children.sort_by_key(|v| v.get("order").and_then(|o| o.as_i64()).unwrap_or(0));
                obj.insert("children".to_string(), serde_json::Value::Array(children));
            }
        }
    }

    roots.sort_by_key(|v| v.get("order").and_then(|o| o.as_i64()).unwrap_or(0));
    for root in roots.iter_mut() {
        attach_children(root, &children_map);
    }
    roots
}

/// 兼容 uniqueidentifier 类型字段的字符串读取
fn get_str_col(row: &Row, col: &str) -> String {
    if let Ok(Some(s)) = row.try_get::<&str, _>(col) {
        return s.to_string();
    }
    // 兜底：通过 try_get_value 处理 uniqueidentifier 等类型
    let v = try_get_value(row, col);
    match v {
        serde_json::Value::String(s) => s,
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
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
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 1000);

    let mut base_query = r#"SELECT r.RuleID, r.RuleName, r.Note, r.Flg, r.State,
                            (SELECT COUNT(*) FROM tSys_RuleMenu rm WHERE rm.RuleID = r.RuleID) AS MenuCount,
                            (SELECT COUNT(*) FROM tSys_UserRule ur WHERE ur.RuleID = r.RuleID) AS UserCount
                            FROM tSys_Rule r WHERE r.State <> 'D'"#.to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let pidx = 1;

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(&format!(
                " AND (r.RuleName LIKE @p{} OR r.Note LIKE @p{})",
                pidx,
                pidx + 1
            ));
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
        }
    }

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let offset = (page - 1) * page_size;
    let top = offset + page_size;
    let paginated_sql = format!(
        "SELECT * FROM (SELECT TOP ({top}) ROW_NUMBER() OVER (ORDER BY (SELECT NULL)) as _rn, * FROM ({base_query}) t) p WHERE _rn > {offset}",
        top = top,
        base_query = base_query,
        offset = offset
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
    Extension(claims): Extension<Claims>,
    Json(body): Json<CreateRoleParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let note = body.Note.as_deref().unwrap_or("");
    let flg = body.Flg.as_deref().unwrap_or("");
    let state = body.State.as_deref().unwrap_or("Y");

    let sql = r#"INSERT INTO tSys_Rule (RuleID, RuleName, Note, Flg, State)
                 VALUES (NEWID(), @p1, @p2, @p3, @p4)"#;
    conn.execute(sql, &[&body.RuleName, &note, &flg, &state])
        .await?;

    // 审计日志
    let audit_remark = format!("新建角色：{}", body.RuleName);
    crate::handlers::audit_log::log_perm_action(
        &mut conn,
        crate::handlers::audit_log::OPER_CREATE,
        "tSys_Rule",
        "",
        &claims,
        &audit_remark,
    )
    .await;

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
    Extension(claims): Extension<Claims>,
    Json(body): Json<UpdateRoleParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let rule_name = body.RuleName.as_deref().unwrap_or("");
    let note = body.Note.as_deref().unwrap_or("");
    let flg = body.Flg.as_deref().unwrap_or("");
    let state = body.State.as_deref().unwrap_or("Y");

    let sql = r#"UPDATE tSys_Rule SET RuleName = @p1, Note = @p2, Flg = @p3, State = @p4
                 WHERE RuleID = @p5"#;
    conn.execute(sql, &[&rule_name, &note, &flg, &state, &body.RuleID])
        .await?;

    // 角色状态/信息变更可能影响所有关联该角色的用户，清除全部权限缓存
    crate::middleware::permission::invalidate_all_permission_cache();

    // 审计日志
    let audit_remark = format!("修改角色：{}", rule_name);
    crate::handlers::audit_log::log_perm_action(
        &mut conn,
        crate::handlers::audit_log::OPER_UPDATE,
        "tSys_Rule",
        &body.RuleID,
        &claims,
        &audit_remark,
    )
    .await;

    Ok(Json(ApiResponse::msg("角色更新成功")))
}

#[derive(Deserialize)]
pub struct DeleteRoleParams {
    pub RuleID: String,
}

pub async fn delete_role(
    State(_config): State<Config>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<DeleteRoleParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    // 事务包裹：删除角色关联的菜单权限 + 用户角色关联 + 角色本身 原子化
    // 任何一步失败都回滚，避免部分删除造成数据不一致
    let tx_result: std::result::Result<(), String> = async {
        crate::services::inventory_ledger::begin_tran(&mut conn)
            .await
            .map_err(|e| e.to_string())?;

        let del_rule_menu = "DELETE FROM tSys_RuleMenu WHERE RuleID = @p1";
        conn.execute(del_rule_menu, &[&body.RuleID])
            .await
            .map_err(|e| e.to_string())?;

        let del_user_rule = "DELETE FROM tSys_UserRule WHERE RuleID = @p1";
        conn.execute(del_user_rule, &[&body.RuleID])
            .await
            .map_err(|e| e.to_string())?;

        let del_role = "DELETE FROM tSys_Rule WHERE RuleID = @p1";
        conn.execute(del_role, &[&body.RuleID])
            .await
            .map_err(|e| e.to_string())?;

        crate::services::inventory_ledger::commit_tran(&mut conn)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    .await;
    if let Err(e) = tx_result {
        crate::services::inventory_ledger::rollback_tran(&mut conn).await;
        return Ok(Json(ApiResponse::err(&crate::utils::db_err(
            "角色删除失败: {}",
            &e,
        ))));
    }

    // 角色删除后级联清除了 tSys_RuleMenu 和 tSys_UserRule，
    // 受影响用户的缓存需要失效，清除全部权限缓存
    crate::middleware::permission::invalidate_all_permission_cache();

    // 审计日志
    let audit_remark = format!("删除角色");
    crate::handlers::audit_log::log_perm_action(
        &mut conn,
        crate::handlers::audit_log::OPER_DELETE,
        "tSys_Rule",
        &body.RuleID,
        &claims,
        &audit_remark,
    )
    .await;

    Ok(Json(ApiResponse::msg("角色删除成功")))
}

#[derive(Deserialize)]
pub struct AssignUserRolesParams {
    pub EmpID: String,
    pub RuleIDs: Vec<String>,
}

pub async fn assign_user_roles(
    State(_config): State<Config>,
    Extension(claims): Extension<Claims>,
    Json(params): Json<AssignUserRolesParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    // 事务包裹：DELETE 旧用户角色 + INSERT 新用户角色 原子化
    let tx_result: std::result::Result<(), String> = async {
        crate::services::inventory_ledger::begin_tran(&mut conn)
            .await
            .map_err(|e| e.to_string())?;

        let del_sql = "DELETE FROM tSys_UserRule WHERE EmpID = @p1";
        conn.execute(del_sql, &[&params.EmpID])
            .await
            .map_err(|e| e.to_string())?;

        for rule_id in &params.RuleIDs {
            let ins_sql = r#"INSERT INTO tSys_UserRule (UserRuleID, EmpID, RuleID, LUTime)
                             VALUES (NEWID(), @p1, @p2, @p3)"#;
            conn.execute(ins_sql, &[&params.EmpID, rule_id, &now])
                .await
                .map_err(|e| e.to_string())?;
        }

        crate::services::inventory_ledger::commit_tran(&mut conn)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    .await;
    if let Err(e) = tx_result {
        crate::services::inventory_ledger::rollback_tran(&mut conn).await;
        return Ok(Json(ApiResponse::err(&crate::utils::db_err(
            "用户角色分配失败: {}",
            &e,
        ))));
    }

    // 清除该用户的权限缓存
    crate::middleware::permission::invalidate_user_permission_cache(&params.EmpID);

    // 审计日志：记录用户角色分配
    let audit_remark = format!("分配用户角色：共 {} 个角色", params.RuleIDs.len());
    crate::handlers::audit_log::log_perm_action(
        &mut conn,
        crate::handlers::audit_log::OPER_ASSIGN_ROLE,
        "tSys_UserRule",
        &params.EmpID,
        &claims,
        &audit_remark,
    )
    .await;

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
    tracing::debug!(
        "[save_table_column_config] 进入 EmpID={:?} TableName={:?} ConfigData.len={}",
        params.EmpID,
        params.TableName,
        params.ConfigData.len()
    );

    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("[save_table_column_config] 获取连接失败: {}", e);
            return Ok(Json(ApiResponse::err(&crate::utils::db_err(
                "连接数据库失败: {}",
                &e,
            ))));
        }
    };
    tracing::debug!("[save_table_column_config] 已拿到连接");

    // 用字符串格式而非 NaiveDateTime，规避 tiberius chrono 绑定兼容性问题
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let emp_uuid_str = match Uuid::parse_str(params.EmpID.trim()) {
        Ok(u) => u.to_string(),
        Err(e) => {
            tracing::warn!("[save_table_column_config] UUID 解析失败: {}", e);
            return Ok(Json(ApiResponse::err(&format!(
                "EmpID 不是有效 UUID: {}",
                e
            ))));
        }
    };
    tracing::debug!("[save_table_column_config] emp_uuid_str={}", emp_uuid_str);

    let check_sql = "SELECT ColumnConfigID FROM tSys_TableColumnConfig WHERE EmpID = CAST(@p1 AS uniqueidentifier) AND TableName = @p2";
    let stream = match conn
        .query(check_sql, &[&emp_uuid_str, &params.TableName])
        .await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("[save_table_column_config] check 查询失败: {}", e);
            return Ok(Json(ApiResponse::err(&crate::utils::db_err(
                "SQL 查询失败: {}",
                &e,
            ))));
        }
    };
    let existing = match stream.into_row().await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("[save_table_column_config] check 取行失败: {}", e);
            return Ok(Json(ApiResponse::err(&crate::utils::db_err(
                "取行失败: {}",
                &e,
            ))));
        }
    };
    tracing::debug!("[save_table_column_config] 已存在?={}", existing.is_some());

    if existing.is_some() {
        let upd_sql = r#"UPDATE tSys_TableColumnConfig
                         SET ConfigData = @p1, LUTime = @p2
                         WHERE EmpID = CAST(@p3 AS uniqueidentifier) AND TableName = @p4"#;
        match conn
            .execute(
                upd_sql,
                &[&params.ConfigData, &now, &emp_uuid_str, &params.TableName],
            )
            .await
        {
            Ok(_) => {
                tracing::debug!(
                    "[save_table_column_config] UPDATE 成功 TableName={}",
                    params.TableName
                );
            }
            Err(e) => {
                tracing::warn!("[save_table_column_config] UPDATE 失败: {}", e);
                return Ok(Json(ApiResponse::err(&crate::utils::db_err(
                    "UPDATE 失败: {}",
                    &e,
                ))));
            }
        }
    } else {
        let ins_sql = r#"INSERT INTO tSys_TableColumnConfig (ColumnConfigID, EmpID, TableName, ConfigData, LUTime)
                         VALUES (NEWID(), CAST(@p1 AS uniqueidentifier), @p2, @p3, @p4)"#;
        match conn
            .execute(
                ins_sql,
                &[&emp_uuid_str, &params.TableName, &params.ConfigData, &now],
            )
            .await
        {
            Ok(_) => {
                tracing::debug!(
                    "[save_table_column_config] INSERT 成功 TableName={}",
                    params.TableName
                );
            }
            Err(e) => {
                tracing::warn!("[save_table_column_config] INSERT 失败: {}", e);
                return Ok(Json(ApiResponse::err(&crate::utils::db_err(
                    "INSERT 失败: {}",
                    &e,
                ))));
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
    tracing::debug!(
        "[get_table_column_config] 进入 EmpID={:?} TableName={:?}",
        params.EmpID,
        params.TableName
    );
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("[get_table_column_config] 获取连接失败: {}", e);
            return Ok(Json(ApiResponse::err(&crate::utils::db_err(
                "连接数据库失败: {}",
                &e,
            ))));
        }
    };
    tracing::debug!("[get_table_column_config] 已拿到连接");
    let emp_uuid_str = match Uuid::parse_str(params.EmpID.trim()) {
        Ok(u) => u.to_string(),
        Err(e) => {
            tracing::warn!("[get_table_column_config] EmpID UUID 解析失败: {}", e);
            return Ok(Json(ApiResponse::err(&format!(
                "EmpID 不是有效 UUID: {}",
                e
            ))));
        }
    };
    tracing::debug!(
        "[get_table_column_config] 准备执行 SQL emp_uuid_str={}",
        emp_uuid_str
    );
    let sql = "SELECT ColumnConfigID, EmpID, TableName, ConfigData, LUTime FROM tSys_TableColumnConfig WHERE EmpID = CAST(@p1 AS uniqueidentifier) AND TableName = @p2";
    let stream = match conn.query(sql, &[&emp_uuid_str, &params.TableName]).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("[get_table_column_config] conn.query 失败: {}", e);
            return Ok(Json(ApiResponse::err(&crate::utils::db_err(
                "SQL 查询失败: {}",
                &e,
            ))));
        }
    };
    tracing::debug!("[get_table_column_config] SQL 已执行, 准备收集行");

    // 用 rows() 收集所有行，返回数组（前端 useColumnConfig 期望 res.data 是 Array）
    let rows_json: Vec<serde_json::Value> = match stream.into_results().await {
        Ok(rows) => {
            let mut arr = Vec::with_capacity(rows.len());
            for r in rows {
                if let Some(row) = r.into_iter().next() {
                    arr.push(row_to_json(&row));
                }
            }
            arr
        }
        Err(e) => {
            tracing::warn!("[get_table_column_config] into_results 失败: {}", e);
            return Ok(Json(ApiResponse::err(&crate::utils::db_err(
                "取行失败: {}",
                &e,
            ))));
        }
    };
    tracing::debug!(
        "[get_table_column_config] 收集完成, 共 {} 条",
        rows_json.len()
    );

    Ok(Json(ApiResponse::ok(serde_json::Value::Array(rows_json))))
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
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let is_default = params.IsDefault.unwrap_or(false);

    let emp_uuid_str = match Uuid::parse_str(params.EmpID.trim()) {
        Ok(u) => u.to_string(),
        Err(e) => {
            return Ok(Json(ApiResponse::err(&format!(
                "EmpID 不是有效 UUID: {}",
                e
            ))));
        }
    };

    if is_default {
        let reset_sql = r#"UPDATE tSys_ColumnPreset SET IsDefault = 0
                          WHERE EmpID = CAST(@p1 AS uniqueidentifier) AND TableName = @p2 AND IsDefault = 1"#;
        conn.execute(reset_sql, &[&emp_uuid_str, &params.TableName])
            .await?;
    }

    let ins_sql = r#"INSERT INTO tSys_ColumnPreset (PresetID, EmpID, TableName, PresetName, ConfigData, IsDefault, LUTime)
                     OUTPUT INSERTED.PresetID
                     VALUES (NEWID(), CAST(@p1 AS uniqueidentifier), @p2, @p3, @p4, @p5, @p6)"#;
    let stream = conn
        .query(
            ins_sql,
            &[
                &emp_uuid_str,
                &params.TableName,
                &params.PresetName,
                &params.ConfigData,
                &is_default,
                &now,
            ],
        )
        .await?;

    let row = stream.into_row().await?;
    let preset_id = row
        .and_then(|r| {
            r.try_get::<uuid::Uuid, _>("PresetID")
                .ok()
                .flatten()
                .map(|u| u.to_string())
        })
        .unwrap_or_default();

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
            return Ok(Json(ApiResponse::err(&format!(
                "EmpID 不是有效 UUID: {}",
                e
            ))));
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

#[derive(Deserialize)]
pub struct SetDefaultPresetParams {
    pub PresetID: String,
    /// true=设为默认，false=取消默认
    pub IsDefault: bool,
}

/// 设置/取消默认预设
/// 设为默认时，先取消同 EmpID + TableName 下的其他默认预设，再将目标预设设为默认
/// 取消默认时，直接将目标预设 IsDefault 置 0
pub async fn set_default_preset(
    State(_config): State<Config>,
    Json(params): Json<SetDefaultPresetParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    // 校验 PresetID 是否为有效 UUID
    let preset_uuid_str = match Uuid::parse_str(params.PresetID.trim()) {
        Ok(u) => u.to_string(),
        Err(e) => {
            return Ok(Json(ApiResponse::err(&format!(
                "PresetID 不是有效 UUID: {}",
                e
            ))));
        }
    };

    // 查出该预设对应的 EmpID + TableName（用于取消其他默认预设）
    let lookup_sql = "SELECT EmpID, TableName FROM tSys_ColumnPreset WHERE PresetID = @p1";
    let stream = conn.query(lookup_sql, &[&preset_uuid_str]).await?;
    let row = stream.into_row().await?;
    let (emp_uuid_str, table_name) = match row {
        Some(r) => {
            let emp_id: uuid::Uuid = r.get::<uuid::Uuid, _>("EmpID").unwrap_or_default();
            // tiberius 字符串列用 &str 获取（String 不实现 FromSql）
            let tbl: String = r.get::<&str, _>("TableName").unwrap_or("").to_string();
            (emp_id.to_string(), tbl)
        }
        None => return Ok(Json(ApiResponse::err("预设不存在"))),
    };

    if params.IsDefault {
        // 取消同 EmpID + TableName 下的其他默认预设
        let reset_sql = r#"UPDATE tSys_ColumnPreset SET IsDefault = 0, LUTime = @p1
                          WHERE EmpID = CAST(@p2 AS uniqueidentifier) AND TableName = @p3 AND IsDefault = 1"#;
        conn.execute(reset_sql, &[&now, &emp_uuid_str, &table_name])
            .await?;
        // 将目标预设设为默认
        let set_sql =
            r#"UPDATE tSys_ColumnPreset SET IsDefault = 1, LUTime = @p1 WHERE PresetID = @p2"#;
        conn.execute(set_sql, &[&now, &preset_uuid_str]).await?;
    } else {
        // 取消默认
        let clear_sql =
            r#"UPDATE tSys_ColumnPreset SET IsDefault = 0, LUTime = @p1 WHERE PresetID = @p2"#;
        conn.execute(clear_sql, &[&now, &preset_uuid_str]).await?;
    }

    Ok(Json(ApiResponse::msg("预设默认状态已更新")))
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

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| crate::error::AppError::BadRequest(format!("读取上传字段失败: {}", e)))?
    {
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

                let dir_name = if biz_type.is_empty() {
                    "default".to_string()
                } else {
                    biz_type.clone()
                };
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
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let relative_path = format!("uploads/{}/{}_{}", biz_type, file_id, original_name);

    // 对齐 tSys_UploadFile 实际字段：FileID/FileName/FilePath/FileSize/FileType/BizType/BizID/State/EUser/EDate/LUTime
    // UploadUser/UploadTime 不存在，改用 EUser/EDate；State 默认 'A'，LUTime 由数据库默认值填充
    let sql = r#"INSERT INTO tSys_UploadFile (FileID, BizType, BizID, FileName, FilePath, FileSize, State, EUser, EDate)
                 VALUES (@p1, @p2, @p3, @p4, @p5, @p6, 'A', @p7, @p8)"#;
    conn.execute(
        sql,
        &[
            &file_id,
            &biz_type,
            &biz_id,
            &original_name,
            &relative_path,
            &file_size,
            &claims.user_code.as_str(),
            &now,
        ],
    )
    .await?;

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
    // 对齐 tSys_UploadFile 实际字段：UploadUser/UploadTime 不存在，改用 EUser/EDate
    let sql = r#"SELECT FileID, BizType, BizID, FileName, FilePath, FileSize, State, EUser, EDate, LUTime
                 FROM tSys_UploadFile WHERE BizType = @p1 AND BizID = @p2 ORDER BY EDate DESC"#;
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
    // 合并员工/用户后，用户数 = 允许登录的在职员工数（与登录白名单一致）
    let user_sql = "SELECT COUNT(*) as cnt FROM tBas_Emp WHERE ISNULL(AllowLogin, 'N') = 'Y' AND ISNULL(State, 'Y') <> 'D' AND ISNULL(WorkState, '1') <> '3'";
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
    let sql = "SELECT StkID, StkName, StkCode FROM tBas_Stock WHERE Used = 'Y' AND State <> 'D' ORDER BY StkCode";
    let stream = conn.query(sql, &[]).await?;
    let rows: Vec<Row> = stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
    Ok(Json(ApiResponse::ok(data)))
}
