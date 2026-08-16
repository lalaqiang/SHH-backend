//! 统一 JWT 令牌生成与验证模块
//!
//! 设计目标：
//!   - 消除 `handlers/auth.rs`、`handlers/mobile.rs` 中重复的 token 生成逻辑
//!   - 消除 `middleware/auth.rs` 中独立的 token 验证逻辑
//!   - 统一 Claims 结构定义、过期时间计算（24 小时）、算法（HS256）
//!
//! 使用方式：
//!   ```ignore
//!   use crate::utils::jwt::{create_token, verify_token, make_claims};
//!
//!   // 生成 token
//!   let claims = make_claims("admin", "张三", "emp-id-xxx");
//!   let token = create_token(&config.jwt_secret, &claims)?;
//!
//!   // 验证 token
//!   let claims = verify_token(&config.jwt_secret, &token)?;
//!   ```
//!
//! 兼容性：
//!   - `Claims` 结构体在此模块定义，`middleware::auth` 通过 `pub use` 重导出，
//!     所有既有的 `use crate::middleware::auth::Claims` 无需修改。

use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Mutex;

/// token 有效期（小时）
const TOKEN_TTL_HOURS: i64 = 24;

/// JWT Claims 载荷
///
/// 字段说明：
///   - `sub`: 主题（用户工号，兼容旧 JWT 标准字段）
///   - `user_code`: 用户工号
///   - `user_name`: 用户姓名
///   - `exp`: 过期时间（Unix 时间戳，秒）
///   - `emp_id`: EmpID（GUID 字符串），移动端 handler 用它写入 EUser/EmpID 字段
///   - `jti`: token 唯一 ID（用于黑名单吊销）
///
/// `#[serde(default)]` 兼容旧 token（无 emp_id/jti 字段时反序列化为空字符串）
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String,
    pub user_code: String,
    pub user_name: String,
    pub exp: usize,
    #[serde(default)]
    pub emp_id: String,
    #[serde(default)]
    pub jti: String,
}

/// 构造标准 Claims（24 小时过期）
///
/// 统一过期时间计算逻辑，避免各 handler 自行 `chrono::Utc::now() + Duration::hours(24)` 重复代码。
pub fn make_claims(user_code: &str, user_name: &str, emp_id: &str) -> Claims {
    Claims {
        sub: user_code.to_string(),
        user_code: user_code.to_string(),
        user_name: user_name.to_string(),
        exp: (chrono::Utc::now() + chrono::Duration::hours(TOKEN_TTL_HOURS)).timestamp() as usize,
        emp_id: emp_id.to_string(),
        // P2-17 修复：新增 jti 字段用于 token 黑名单
        //   使用时间戳 + 随机数构造，保证唯一性（不需要密码学强度）
        jti: format!(
            "{}-{:x}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
            rand_u64()
        ),
    }
}

/// 简易随机数（不依赖 rand crate，用 std + 纳秒时间戳混合）
fn rand_u64() -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::Hasher;
    let mut h = DefaultHasher::new();
    h.write_u64(chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64);
    h.write_usize(std::process::id() as usize);
    // 额外加一个自增计数器（atomic），保证同一纳秒并发也产生不同哈希
    use std::sync::atomic::{AtomicU64, Ordering};
    static CTR: AtomicU64 = AtomicU64::new(0);
    h.write_u64(CTR.fetch_add(1, Ordering::Relaxed));
    h.finish()
}

/// 生成 JWT token
///
/// 算法：HS256（与 `verify_token` 保持一致）
pub fn create_token(secret: &str, claims: &Claims) -> Result<String, jsonwebtoken::errors::Error> {
    encode(
        &Header::default(),
        claims,
        &EncodingKey::from_secret(secret.as_ref()),
    )
}

/// 验证 JWT token，返回 Claims
///
/// 算法：HS256（与 `create_token` 保持一致）
pub fn verify_token(secret: &str, token: &str) -> Result<Claims, jsonwebtoken::errors::Error> {
    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_ref()),
        &Validation::new(Algorithm::HS256),
    )?;
    Ok(token_data.claims)
}

// ============================================================
// P2-17 修复：JWT token 黑名单（简化版，基于内存 + 自动清理）
//
// 设计权衡：
//   - 不引入 Redis 等外部依赖，保持部署简单（ERP 通常单实例）
//   - 黑名单存 jti + exp，过期后自动清理（避免无限增长）
//   - 多实例部署时黑名单不共享，但 token 24 小时自然过期可接受
//   - 如需多实例共享，可后续替换为 Redis 后端
// ============================================================

static BLACKLIST: Lazy<Mutex<HashSet<(String, usize)>>> = Lazy::new(|| Mutex::new(HashSet::new()));

/// 将 token 加入黑名单（登出时调用）
pub fn revoke_token(claims: &Claims) {
    if claims.jti.is_empty() {
        return;
    }
    if let Ok(mut bl) = BLACKLIST.lock() {
        // 顺便清理已过期的条目
        let now = chrono::Utc::now().timestamp() as usize;
        bl.retain(|(_, exp)| *exp > now);
        // 加入新条目
        bl.insert((claims.jti.clone(), claims.exp));
    }
}

/// 检查 token 是否已被吊销（verify_token 后调用）
pub fn is_token_revoked(claims: &Claims) -> bool {
    if claims.jti.is_empty() {
        return false;
    }
    if let Ok(bl) = BLACKLIST.lock() {
        bl.iter().any(|(jti, _)| jti == &claims.jti)
    } else {
        false
    }
}

// ============================================================
// 单元测试
// ============================================================
#[cfg(test)]
mod tests {
    use super::*;

    const TEST_SECRET: &str = "test_secret_key_for_unit_test";

    #[test]
    fn test_make_claims_fields() {
        let c = make_claims("admin", "张三", "emp-001");
        assert_eq!(c.sub, "admin");
        assert_eq!(c.user_code, "admin");
        assert_eq!(c.user_name, "张三");
        assert_eq!(c.emp_id, "emp-001");
    }

    #[test]
    fn test_make_claims_exp_in_future() {
        let c = make_claims("admin", "张三", "");
        let now = chrono::Utc::now().timestamp() as usize;
        // exp 应在 23~25 小时之后（留 1 小时容差）
        assert!(c.exp > now + 23 * 3600);
        assert!(c.exp < now + 25 * 3600);
    }

    #[test]
    fn test_create_and_verify_token_roundtrip() {
        let claims = make_claims("admin", "张三", "emp-001");
        let token = create_token(TEST_SECRET, &claims).expect("应成功生成 token");
        let decoded = verify_token(TEST_SECRET, &token).expect("应成功验证 token");
        assert_eq!(decoded.sub, "admin");
        assert_eq!(decoded.user_code, "admin");
        assert_eq!(decoded.user_name, "张三");
        assert_eq!(decoded.emp_id, "emp-001");
    }

    #[test]
    fn test_verify_token_wrong_secret() {
        let claims = make_claims("admin", "张三", "");
        let token = create_token(TEST_SECRET, &claims).expect("应成功生成 token");
        // 用错误密钥验证应失败
        assert!(verify_token("wrong_secret", &token).is_err());
    }

    #[test]
    fn test_verify_token_invalid_token() {
        // 非法 token 字符串
        assert!(verify_token(TEST_SECRET, "not.a.valid.token").is_err());
        assert!(verify_token(TEST_SECRET, "garbage").is_err());
        assert!(verify_token(TEST_SECRET, "").is_err());
    }

    #[test]
    fn test_verify_token_expired() {
        // 构造已过期的 Claims
        let expired_claims = Claims {
            sub: "admin".into(),
            user_code: "admin".into(),
            user_name: "张三".into(),
            exp: (chrono::Utc::now() - chrono::Duration::hours(1)).timestamp() as usize,
            emp_id: "".into(),
            jti: "test-jti".into(),
        };
        let token = create_token(TEST_SECRET, &expired_claims).expect("应成功生成 token");
        // 过期 token 验证应失败
        assert!(verify_token(TEST_SECRET, &token).is_err());
    }

    #[test]
    fn test_claims_serde_default_emp_id() {
        // 旧 token 无 emp_id 字段，反序列化应为空字符串
        let old_payload = serde_json::json!({
            "sub": "admin",
            "user_code": "admin",
            "user_name": "张三",
            "exp": 9999999999usize,
        });
        let claims: Claims = serde_json::from_value(old_payload).expect("应成功反序列化");
        assert_eq!(claims.emp_id, "");
    }

    #[test]
    fn test_claims_serde_roundtrip() {
        let claims = make_claims("admin", "张三", "emp-001");
        let json = serde_json::to_string(&claims).expect("应成功序列化");
        let decoded: Claims = serde_json::from_str(&json).expect("应成功反序列化");
        assert_eq!(decoded.sub, claims.sub);
        assert_eq!(decoded.emp_id, claims.emp_id);
        assert_eq!(decoded.exp, claims.exp);
    }

    #[test]
    fn test_token_deterministic_for_same_claims() {
        // 相同 Claims + 相同 secret → 相同 token（Header 无随机成分，HS256 是确定性算法）
        let claims = make_claims("admin", "张三", "emp-001");
        let t1 = create_token(TEST_SECRET, &claims).expect("应成功");
        let t2 = create_token(TEST_SECRET, &claims).expect("应成功");
        assert_eq!(t1, t2, "相同 Claims 应生成相同 token");
    }
}
