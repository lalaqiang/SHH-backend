use axum::{
    extract::State,
    Json,
    Extension,
};
use serde::{Deserialize, Serialize};
use crate::config::Config;
use crate::db::get_pool;
use crate::utils::ApiResponse;
use crate::utils::password::{hash_password, verify_password, needs_upgrade};
use crate::utils::jwt::{create_token, make_claims};
use crate::utils::error_codes::*;
use crate::middleware::auth::Claims;
use crate::services::inventory_ledger::record_oper;

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub user: UserInfo,
}

#[derive(Serialize, Clone)]
pub struct UserInfo {
    /// 数据库主键（uniqueidentifier 转字符串），用于列设置等需要按用户保存的场景
    pub emp_id: String,
    pub id: String,
    pub code: String,
    pub name: String,
}

pub async fn login(
    State(config): State<Config>,
    Json(body): Json<LoginRequest>,
) -> Json<ApiResponse<LoginResponse>> {
    if body.username.is_empty() {
        return Json(ApiResponse::<LoginResponse>::err_with_code("请输入工号", VALIDATION_FIELD_REQUIRED));
    }

    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::<LoginResponse>::err_with_code(&format!("数据库连接失败: {}", e), SYS_DB_UNAVAILABLE)),
    };

    // 合并查询：EmpID 是 uniqueidentifier，必须 CAST 成 nvarchar 才能用 row.get::<&str,_> 读出
    // PassWordStr 是普通 nvarchar，可与 EmpID 同行查询，无需第二次 RTT
    // 支持工号(EmpNo)或手机号(Tel)登录
    // 登录白名单：AllowLogin='Y' AND State<>'D' AND WorkState<>'3'(非离职)
    let sql = "SELECT TOP 1 EmpNo, EmpName, CAST(EmpID AS NVARCHAR(64)) AS EmpID, PassWordStr, AllowLogin, State, WorkState \
               FROM tBas_Emp \
               WHERE (EmpNo = @p1 OR Tel = @p1) \
               AND ISNULL(AllowLogin, 'N') = 'Y' AND ISNULL(State, 'Y') <> 'D' AND ISNULL(WorkState, '1') <> '3' \
               ORDER BY CASE WHEN EmpNo = @p1 THEN 0 ELSE 1 END";
    let stream = match conn.query(sql, &[&body.username.as_str()]).await {
        Ok(s) => s,
        Err(e) => return Json(ApiResponse::<LoginResponse>::err_with_code(&format!("查询用户失败: {}", e), SYS_DB_ERROR)),
    };

    let row = match stream.into_row().await {
        Ok(Some(r)) => r,
        Ok(None) => return Json(ApiResponse::<LoginResponse>::err_with_code("账号不存在或已停用/离职", AUTH_USER_NOT_FOUND)),
        Err(e) => return Json(ApiResponse::<LoginResponse>::err_with_code(&format!("读取用户数据失败: {}", e), SYS_DB_ERROR)),
    };

    let emp_no: &str = row.get::<&str, _>("EmpNo").unwrap_or("");
    let emp_name: &str = row.get::<&str, _>("EmpName").unwrap_or("");
    let emp_id: &str = row.get::<&str, _>("EmpID").unwrap_or("");
    let stored_password: String = row.get::<&str, _>("PassWordStr").unwrap_or("").to_string();

    // 密码为空，可能是NULL或字段不存在
    if stored_password.is_empty() || !verify_password(&body.password, &stored_password) {
        return Json(ApiResponse::<LoginResponse>::err_with_code("密码错误", AUTH_PASSWORD_WRONG));
    }

    // 密码验证成功 — 自动升级旧格式（SHA256/XOR）为 bcrypt
    // 透明升级：用户无感知，下次登录即为 bcrypt 格式
    if needs_upgrade(&stored_password) {
        if let Some(new_hash) = hash_password(&body.password) {
            let _ = conn
                .execute(
                    "UPDATE tBas_Emp SET PassWordStr = @p1 WHERE EmpNo = @p2",
                    &[&new_hash.as_str(), &emp_no],
                )
                .await;
            tracing::info!("用户 {} 密码已自动升级为 bcrypt", emp_no);
        } else {
            tracing::warn!("用户 {} 密码升级失败（hash_password 返回 None）", emp_no);
        }
    }

    let claims = make_claims(emp_no, emp_name, emp_id);

    let token = match create_token(&config.jwt_secret, &claims) {
        Ok(t) => t,
        Err(e) => return Json(ApiResponse::<LoginResponse>::err(&format!("生成Token失败: {}", e))),
    };

    let resp = LoginResponse {
        token,
        user: UserInfo {
            emp_id: emp_id.to_string(),
            id: emp_no.to_string(),
            code: emp_no.to_string(),
            name: emp_name.to_string(),
        },
    };

    // 记录登录审计日志
    let _ = record_oper(&mut conn, "LOGIN", "tBas_Emp", emp_id, emp_no, None, Some(&format!("用户登录：{}", emp_name))).await;

    Json(ApiResponse::ok(resp))
}

pub async fn user_info(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
) -> Json<ApiResponse<serde_json::Value>> {
    // 顺手查出 EmpID，方便前端做按用户保存的列设置
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(_) => {
            return Json(ApiResponse::ok(serde_json::json!({
                "user_code": claims.user_code,
                "user_name": claims.user_name,
                "emp_id": "",
            })))
        }
    };
    let stream = conn
        .query("SELECT TOP 1 CAST(EmpID AS NVARCHAR(64)) AS EmpID FROM tBas_Emp WHERE EmpNo = @p1",
               &[&claims.user_code.as_str()])
        .await;
    let emp_id = match stream {
        Ok(s) => match s.into_row().await {
            Ok(Some(r)) => r.get::<&str, _>("EmpID").unwrap_or("").to_string(),
            _ => String::new(),
        },
        Err(_) => String::new(),
    };
    Json(ApiResponse::ok(serde_json::json!({
        "user_code": claims.user_code,
        "user_name": claims.user_name,
        "emp_id": emp_id,
    })))
}

pub async fn logout(
    Extension(claims): Extension<Claims>,
) -> Json<ApiResponse<serde_json::Value>> {
    // P2-17 修复：将 token 加入黑名单，登出后立即失效（原仅记录审计日志，token 仍可使用 24h）
    crate::utils::jwt::revoke_token(&claims);
    // 记录登出审计日志
    if let Ok(mut conn) = get_pool().get().await {
        let _ = record_oper(&mut conn, "LOGOUT", "tBas_Emp", &claims.emp_id, &claims.user_code, None, Some("用户登出")).await;
    }
    Json(ApiResponse::msg("登出成功"))
}

#[derive(Deserialize)]
pub struct ChangePasswordParams {
    pub old_password: String,
    pub new_password: String,
}

pub async fn change_password(
    Extension(claims): Extension<Claims>,
    Json(params): Json<ChangePasswordParams>,
) -> Json<ApiResponse<serde_json::Value>> {
    // P1-12 修复：移除密码长度限制
    //   用户偏好明确："No length restrictions and allow empty passwords for internal system ease of use"
    //   ERP 是内部系统，密码策略以易用性为先；如需限制可在 tSys_Config 表配置（后续扩展）
    //   bcrypt 仍会拒绝超过 72 字节的密码（在 hash_password 中处理）

    // 从JWT token中获取emp_no
    let emp_no = &claims.sub;

    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err_with_code(&format!("数据库连接失败: {}", e), SYS_DB_UNAVAILABLE)),
    };

    let sql = "SELECT TOP 1 EmpNo, PassWordStr FROM tBas_Emp WHERE EmpNo = @p1";
    let stream = match conn.query(sql, &[&emp_no.as_str()]).await {
        Ok(s) => s,
        Err(e) => return Json(ApiResponse::err_with_code(&format!("查询用户失败: {}", e), SYS_DB_ERROR)),
    };

    match stream.into_row().await {
        Ok(Some(row)) => {
            let stored_password = match row.try_get::<&str, _>("PassWordStr") {
                Ok(Some(p)) => p,
                Ok(None) => "",
                Err(_) => "",
            };
            if !verify_password(&params.old_password, stored_password) {
                return Json(ApiResponse::err_with_code("旧密码错误", AUTH_PASSWORD_WRONG));
            }
            let hashed = match hash_password(&params.new_password) {
                Some(h) => h,
                None => {
                    // P1-7 修复：原用 AUTH_PASSWORD_WRONG 与"密码错误"语义混淆
                    //   改用 SYS_HASH_FAILED 表示系统级哈希失败（如密码超长 >72 字节）
                    return Json(ApiResponse::err_with_code(
                        "密码哈希失败，密码过长（>72 字节），请缩短密码后重试",
                        SYS_HASH_FAILED,
                    ));
                }
            };
            let update_sql = "UPDATE tBas_Emp SET PassWordStr = @p1 WHERE EmpNo = @p2";
            match conn.execute(update_sql, &[&hashed.as_str(), &emp_no.as_str()]).await {
                Ok(_) => {
                    // 记录改密审计日志
                    let _ = record_oper(&mut conn, "PWD", "tBas_Emp", &claims.emp_id, emp_no, None, Some("修改密码")).await;
                    Json(ApiResponse::msg("密码修改成功"))
                },
                Err(e) => Json(ApiResponse::err_with_code(&format!("更新密码失败: {}", e), SYS_DB_ERROR)),
            }
        }
        Ok(None) => Json(ApiResponse::err_with_code("未找到该用户", AUTH_USER_NOT_FOUND)),
        Err(e) => Json(ApiResponse::err_with_code(&format!("读取用户数据失败: {}", e), SYS_DB_ERROR)),
    }
}

/// 管理员重置/修改任意员工密码
///   - 仅 admin（工号 admin）可调用，permission_middleware 已对 /api/admin/* 强制拒绝非 admin
///   - 不需要旧密码，直接覆盖
///   - new_password 为空表示清空密码（用户偏好：内部系统允许空密码）
#[derive(Deserialize)]
pub struct AdminResetPasswordParams {
    /// 目标员工工号（EmpNo），与登录用户名一致
    pub emp_no: String,
    pub new_password: String,
}

pub async fn admin_reset_password(
    Extension(claims): Extension<Claims>,
    Json(params): Json<AdminResetPasswordParams>,
) -> Json<ApiResponse<serde_json::Value>> {
    if params.emp_no.is_empty() {
        return Json(ApiResponse::err_with_code("请指定目标员工工号", VALIDATION_FIELD_REQUIRED));
    }

    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err_with_code(&format!("数据库连接失败: {}", e), SYS_DB_UNAVAILABLE)),
    };

    // 校验目标员工存在
    let check_sql = "SELECT TOP 1 CAST(EmpID AS NVARCHAR(64)) AS EmpID, EmpName FROM tBas_Emp WHERE EmpNo = @p1";
    let stream = match conn.query(check_sql, &[&params.emp_no.as_str()]).await {
        Ok(s) => s,
        Err(e) => return Json(ApiResponse::err_with_code(&format!("查询用户失败: {}", e), SYS_DB_ERROR)),
    };
    let (target_emp_id, target_emp_name): (String, String) = match stream.into_row().await {
        Ok(Some(r)) => (
            r.get::<&str, _>("EmpID").unwrap_or("").to_string(),
            r.get::<&str, _>("EmpName").unwrap_or("").to_string(),
        ),
        Ok(None) => return Json(ApiResponse::err_with_code("目标员工不存在", AUTH_USER_NOT_FOUND)),
        Err(e) => return Json(ApiResponse::err_with_code(&format!("读取用户数据失败: {}", e), SYS_DB_ERROR)),
    };

    // 空密码直接写空串；非空则哈希
    let hashed = if params.new_password.is_empty() {
        String::new()
    } else {
        match hash_password(&params.new_password) {
            Some(h) => h,
            None => {
                return Json(ApiResponse::err_with_code(
                    "密码哈希失败，密码过长（>72 字节），请缩短密码后重试",
                    SYS_HASH_FAILED,
                ));
            }
        }
    };

    let update_sql = "UPDATE tBas_Emp SET PassWordStr = @p1 WHERE EmpNo = @p2";
    match conn.execute(update_sql, &[&hashed.as_str(), &params.emp_no.as_str()]).await {
        Ok(_) => {
            // 记录审计日志：管理员重置他人密码
            let _ = record_oper(
                &mut conn,
                "PWD",
                "tBas_Emp",
                &target_emp_id,
                &params.emp_no,
                None,
                Some(&format!("管理员 {} 重置员工 {}({}) 的密码", claims.user_code, target_emp_name, params.emp_no)),
            ).await;
            Json(ApiResponse::msg("密码重置成功"))
        },
        Err(e) => Json(ApiResponse::err_with_code(&format!("更新密码失败: {}", e), SYS_DB_ERROR)),
    }
}

// ==================== 用户偏好（跨设备同步：主题、布局等） ====================
//
// 数据表 tSys_UserPref（migration 014_create_user_pref_table 创建）
//   - EmpID + PrefKey 唯一索引，保证一个用户同一偏好只有一行
//   - 设计通用化：未来可保存布局密度、默认仓库等其他偏好，前端按 PrefKey 区分
//
// 失败容忍：表不存在或查询失败时返回空对象/静默失败，前端回退到 localStorage

/// GET /api/user/pref — 返回当前登录用户的所有偏好 { key: value, ... }
pub async fn get_user_prefs(
    Extension(claims): Extension<Claims>,
) -> Json<ApiResponse<serde_json::Value>> {
    let emp_id = &claims.emp_id;
    if emp_id.is_empty() {
        return Json(ApiResponse::ok(serde_json::json!({})));
    }

    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("[get_user_prefs] 数据库连接失败: {}", e);
            return Json(ApiResponse::ok(serde_json::json!({})));
        }
    };

    // 表可能不存在（迁移未执行/失败），用 TRY/CATCH 容错：失败时返回空对象
    let sql = "BEGIN TRY \
               SELECT [PrefKey], [PrefValue] FROM [tSys_UserPref] WHERE [EmpID] = @p1 \
               END TRY \
               BEGIN CATCH \
               SELECT NULL AS [PrefKey], NULL AS [PrefValue] WHERE 1=0 \
               END CATCH";
    let stream = match conn.query(sql, &[&emp_id.as_str()]).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("[get_user_prefs] 查询失败: {}", e);
            return Json(ApiResponse::ok(serde_json::json!({})));
        }
    };

    let rows = stream.into_first_result().await.unwrap_or_default();
    let mut prefs = serde_json::Map::new();
    for row in rows {
        if let (Some(k), Some(v)) = (row.get::<&str, _>("PrefKey"), row.get::<&str, _>("PrefValue")) {
            prefs.insert(k.to_string(), serde_json::Value::String(v.to_string()));
        }
    }
    Json(ApiResponse::ok(serde_json::Value::Object(prefs)))
}

#[derive(Deserialize)]
pub struct SetUserPrefParams {
    pub key: String,
    pub value: String,
}

/// PUT /api/user/pref — upsert 当前登录用户的单个偏好（EmpID + PrefKey 存在则 UPDATE，否则 INSERT）
pub async fn set_user_pref(
    Extension(claims): Extension<Claims>,
    Json(params): Json<SetUserPrefParams>,
) -> Json<ApiResponse<serde_json::Value>> {
    let emp_id = &claims.emp_id;
    if emp_id.is_empty() {
        return Json(ApiResponse::err_with_code("未登录或用户身份缺失", AUTH_USER_NOT_FOUND));
    }
    if params.key.is_empty() || params.key.len() > 64 {
        return Json(ApiResponse::err_with_code("偏好 key 非法（1-64 字符）", VALIDATION_FIELD_REQUIRED));
    }
    if params.value.len() > 255 {
        return Json(ApiResponse::err_with_code("偏好 value 过长（>255 字符）", VALIDATION_FIELD_REQUIRED));
    }

    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err_with_code(&format!("数据库连接失败: {}", e), SYS_DB_UNAVAILABLE)),
    };

    // upsert：通过唯一索引 (EmpID, PrefKey) 实现
    //   - 表不存在时 CATCH 捕获错误并返回 0 行受影响（不影响前端主流程，前端会保留 localStorage）
    //   - 使用 MERGE 替代 IF EXISTS，单条 SQL 原子完成 upsert
    let sql = "BEGIN TRY \
               MERGE [tSys_UserPref] AS target \
               USING (SELECT @p1 AS EmpID, @p2 AS PrefKey) AS source \
               ON (target.[EmpID] = source.[EmpID] AND target.[PrefKey] = source.[PrefKey]) \
               WHEN MATCHED THEN \
                 UPDATE SET [PrefValue] = @p3, [LUTime] = GETDATE() \
               WHEN NOT MATCHED THEN \
                 INSERT ([UserPrefID], [EmpID], [PrefKey], [PrefValue], [LUTime]) \
                 VALUES (NEWID(), @p1, @p2, @p3, GETDATE()); \
               SELECT 1 AS ok, CAST(NULL AS NVARCHAR(4000)) AS msg; \
               END TRY \
               BEGIN CATCH \
               SELECT 0 AS ok, ERROR_MESSAGE() AS msg; \
               END CATCH";

    let result: Option<(i32, Option<String>)> = match conn
        .query(sql, &[&emp_id.as_str(), &params.key.as_str(), &params.value.as_str()])
        .await
    {
        Ok(stream) => stream.into_row().await.ok().flatten().map(|row| {
            let ok: i32 = row.get::<i32, _>("ok").unwrap_or(0);
            let msg: Option<String> = row.get::<&str, _>("msg").map(|s| s.to_string());
            (ok, msg)
        }),
        Err(e) => Some((0, Some(e.to_string()))),
    };

    match result {
        Some((1, _)) => Json(ApiResponse::ok(serde_json::json!({ "ok": true }))),
        Some((_, Some(msg))) => {
            // 表不存在或权限不足等：静默返回错误，前端会保留 localStorage 兜底
            tracing::warn!("[set_user_pref] 保存失败 emp_id={} key={} msg={}", emp_id, params.key, msg);
            Json(ApiResponse::err(&format!("保存偏好失败: {}", msg)))
        }
        _ => Json(ApiResponse::err("保存偏好失败：未知错误")),
    }
}

