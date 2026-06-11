use axum::{
    extract::State,
    Json,
    Extension,
};
use serde::{Deserialize, Serialize};
use crate::config::Config;
use crate::db::get_pool;
use crate::utils::ApiResponse;
use crate::middleware::auth::Claims;

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

const PASSWORD_SALT: &str = "erp_shenhuihui_2024";
const HASH_PREFIX: &str = "SHA256:";

fn hash_password(password: &str) -> String {
    use sha2::{Sha256, Digest};
    let mut hasher = Sha256::new();
    hasher.update(format!("{}{}", password, PASSWORD_SALT).as_bytes());
    let result = hasher.finalize();
    format!("{}{}", HASH_PREFIX, hex::encode(result))
}

const LEGACY_XOR_KEY: [u8; 8] = [0xFC, 0xAA, 0x62, 0xA0, 0x30, 0x9C, 0xF1, 0xD4];

fn is_legacy_encrypted_password(s: &str) -> bool {
    s.len() == 16 && s.chars().all(|c| c.is_ascii_hexdigit())
}

fn decrypt_legacy_password(stored: &str) -> Option<String> {
    let bytes: Vec<u8> = (0..stored.len())
        .step_by(2)
        .filter_map(|i| u8::from_str_radix(&stored[i..i + 2], 16).ok())
        .collect();
    if bytes.len() != 8 {
        return None;
    }
    let decrypted: Vec<u8> = bytes.iter()
        .enumerate()
        .map(|(i, &b)| b ^ LEGACY_XOR_KEY[i])
        .collect();
    let trimmed = decrypted.iter()
        .take_while(|&&b| b != 0)
        .copied()
        .collect::<Vec<u8>>();
    String::from_utf8(trimmed).ok()
}

fn verify_password(password: &str, stored: &str) -> bool {
    // 方式1: SHA256加密
    if stored.starts_with(HASH_PREFIX) {
        let hash = hash_password(password);
        return hash == stored;
    }
    
    // 方式2: XOR加密（16位十六进制）
    // 兼容老ERP：自动把字母O/o替换为数字0
    let normalized_stored = stored.replace('O', "0").replace('o', "0");
    
    if normalized_stored.len() == 16 && normalized_stored.chars().all(|c| c.is_ascii_hexdigit()) {
        // 使用规范化后的值进行解密
        if let Some(decrypted) = decrypt_legacy_password(&normalized_stored) {
            return password == decrypted;
        }
    }
    
    // 方式3: 空密码
    if stored.is_empty() {
        return false;
    }
    
    // 方式4: 明文比较
    password == stored
}

fn needs_upgrade(stored: &str) -> bool {
    !stored.starts_with(HASH_PREFIX)
}

pub async fn login(
    State(config): State<Config>,
    Json(body): Json<LoginRequest>,
) -> Json<ApiResponse<LoginResponse>> {
    if body.username.is_empty() {
        return Json(ApiResponse::<LoginResponse>::err("请输入工号"));
    }

    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::<LoginResponse>::err(&format!("数据库连接失败: {}", e))),
    };

    // 注意：EmpID 是 uniqueidentifier，必须 CAST 成 nvarchar 才能用 row.get::<&str,_> 读出
    let sql = "SELECT TOP 1 EmpNo, EmpName, CAST(EmpID AS NVARCHAR(64)) AS EmpID FROM tBas_Emp WHERE EmpNo = @p1";
    let stream = match conn.query(sql, &[&body.username.as_str()]).await {
        Ok(s) => s,
        Err(e) => return Json(ApiResponse::<LoginResponse>::err(&format!("查询用户失败: {}", e))),
    };

    let row = match stream.into_row().await {
        Ok(Some(r)) => r,
        Ok(None) => return Json(ApiResponse::<LoginResponse>::err("未找到该工号")),
        Err(e) => return Json(ApiResponse::<LoginResponse>::err(&format!("读取用户数据失败: {}", e))),
    };

    let emp_no: &str = row.get::<&str, _>("EmpNo").unwrap_or("");
    let emp_name: &str = row.get::<&str, _>("EmpName").unwrap_or("");
    let emp_id: &str = row.get::<&str, _>("EmpID").unwrap_or("");

    // 单独查密码（不在主查询里 SELECT * 是为了避免在 tiberius 下 uniqueidentifier 列读不出来）
    let pwd_stream = conn
        .query("SELECT TOP 1 PassWordStr FROM tBas_Emp WHERE EmpNo = @p1", &[&body.username.as_str()])
        .await;
    let mut stored_password = String::new();
    if let Ok(s) = pwd_stream {
        if let Ok(Some(pr)) = s.into_row().await {
            stored_password = pr.get::<&str, _>("PassWordStr").unwrap_or("").to_string();
        }
    }
    // 密码为空，可能是NULL或字段不存在
    if stored_password.is_empty() || !verify_password(&body.password, &stored_password) {
        return Json(ApiResponse::<LoginResponse>::err("密码错误"));
    }

    // 密码验证成功
    // 注意：暂不自动升级密码为SHA256，以保持与老ERP的兼容性
    // 如果需要升级，请手动在数据库中修改
    
    let claims = Claims {
        sub: emp_no.to_string(),
        user_code: emp_no.to_string(),
        user_name: emp_name.to_string(),
        exp: (chrono::Utc::now() + chrono::Duration::hours(24)).timestamp() as usize,
    };

    let token = match jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(config.jwt_secret.as_ref()),
    ) {
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

pub async fn logout() -> Json<ApiResponse<serde_json::Value>> {
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
    if params.new_password.len() < 6 {
        return Json(ApiResponse::err("新密码长度不能少于6位"));
    }

    // 从JWT token中获取emp_no
    let emp_no = &claims.sub;

    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("数据库连接失败: {}", e))),
    };

    let sql = "SELECT TOP 1 EmpNo, PassWordStr FROM tBas_Emp WHERE EmpNo = @p1";
    let stream = match conn.query(sql, &[&emp_no.as_str()]).await {
        Ok(s) => s,
        Err(e) => return Json(ApiResponse::err(&format!("查询用户失败: {}", e))),
    };

    match stream.into_row().await {
        Ok(Some(row)) => {
            let stored_password = match row.try_get::<&str, _>("PassWordStr") {
                Ok(Some(p)) => p,
                Ok(None) => "",
                Err(_) => "",
            };
            if !verify_password(&params.old_password, stored_password) {
                return Json(ApiResponse::err("旧密码错误"));
            }
            let hashed = hash_password(&params.new_password);
            let update_sql = "UPDATE tBas_Emp SET PassWordStr = @p1 WHERE EmpNo = @p2";
            match conn.execute(update_sql, &[&hashed.as_str(), &emp_no.as_str()]).await {
                Ok(_) => Json(ApiResponse::msg("密码修改成功")),
                Err(e) => Json(ApiResponse::err(&format!("更新密码失败: {}", e))),
            }
        }
        Ok(None) => Json(ApiResponse::err("未找到该用户")),
        Err(e) => Json(ApiResponse::err(&format!("读取用户数据失败: {}", e))),
    }
}
