//! 统一密码哈希与验证模块
//!
//! 安全策略：
//!   - 新密码一律使用 bcrypt（cost=12）加盐哈希，存储格式 `BCRYPT:$2b$12$...`
//!   - 兼容旧 SHA256+静态盐格式（`SHA256:` 前缀），登录验证成功后自动升级为 bcrypt
//!   - 兼容老 ERP XOR 加密（16 位 hex），登录验证成功后自动升级为 bcrypt
//!   - 明文密码一律拒绝登录
//!
//! 升级路径：
//!   1. `hash_password` → 始终输出 bcrypt
//!   2. `verify_password` → 支持 bcrypt / SHA256 / XOR 三种格式
//!   3. `needs_upgrade` → 非 bcrypt 格式均需升级
//!   4. 登录成功且 `needs_upgrade` 为 true 时，由 handler 调用 `hash_password` 重写数据库

use bcrypt::{hash as bcrypt_hash, verify as bcrypt_verify};

/// bcrypt cost factor（12 = 约 250ms/次，兼顾安全与性能）
const BCRYPT_COST: u32 = 12;
/// bcrypt 哈希前缀，用于区分格式
pub const BCRYPT_PREFIX: &str = "BCRYPT:";

/// 防枚举等时校验用的固定 bcrypt 哈希（cost 与真实密码一致 = 12，明文为无意义占位串）。
/// 仅用于"账号不存在"分支调用 verify_password 消耗与真实校验相同的时间，返回值丢弃。
pub const DUMMY_BCRYPT_FOR_TIMING: &str =
    "BCRYPT:$2b$12$RDfMsRa4Pl3Ov5ImxlRAMu5UcQ1TrXO4y4mshRmt5/6sSvLaiWZ9y";

/// 旧 SHA256+静态盐前缀（仅用于验证旧密码，不再用于新哈希）
const SHA256_PREFIX: &str = "SHA256:";
/// 旧静态盐（仅用于验证旧 SHA256 密码）
const PASSWORD_SALT: &str = "erp_shenhuihui_2024";

/// 老 ERP XOR 加密密钥（8 字节，仅用于验证旧密码）
const LEGACY_XOR_KEY: [u8; 8] = [0xFC, 0xAA, 0x62, 0xA0, 0x30, 0x9C, 0xF1, 0xD4];

/// 哈希密码（bcrypt），返回 `BCRYPT:$2b$12$...` 格式
///
/// 返回 None 的情况：密码超过 72 字节（bcrypt 限制）或内部错误
///
/// 注：bcrypt 0.15 对超长密码会静默截断到 72 字节而非报错，
/// 此处显式检查以避免静默截断导致的安全风险。
pub fn hash_password(password: &str) -> Option<String> {
    if password.len() > 72 {
        tracing::warn!(
            "密码长度 {} 字节超过 bcrypt 72 字节限制，拒绝哈希",
            password.len()
        );
        return None;
    }
    match bcrypt_hash(password, BCRYPT_COST) {
        Ok(h) => Some(format!("{}{}", BCRYPT_PREFIX, h)),
        Err(e) => {
            tracing::error!(
                "bcrypt 哈希失败: {:?}（密码长度 {} 字节）",
                e,
                password.len()
            );
            None
        }
    }
}

/// 验证密码
///
/// 支持三种存储格式（按优先级）：
/// 1. `BCRYPT:` 前缀 → bcrypt 验证
/// 2. `SHA256:` 前缀 → SHA256+静态盐验证（旧格式，需升级）
/// 3. 16 位 hex → XOR 解密比较（老 ERP，需升级）
/// 4. 其他 → 拒绝（防止明文密码）
pub fn verify_password(password: &str, stored: &str) -> bool {
    // 方式1: bcrypt
    if stored.starts_with(BCRYPT_PREFIX) {
        let hash_part = &stored[BCRYPT_PREFIX.len()..];
        return match bcrypt_verify(password, hash_part) {
            Ok(ok) => ok,
            Err(e) => {
                tracing::warn!("bcrypt 验证异常: {:?}", e);
                false
            }
        };
    }

    // 方式2: SHA256+静态盐（旧格式）
    if stored.starts_with(SHA256_PREFIX) {
        return verify_sha256_legacy(password, stored);
    }

    // 方式3: XOR 加密（16 位 hex，老 ERP）
    // 兼容老 ERP：自动把字母 O/o 替换为数字 0
    let normalized_stored = stored.replace('O', "0").replace('o', "0");
    if is_legacy_encrypted_password(&normalized_stored) {
        if let Some(decrypted) = decrypt_legacy_password(&normalized_stored) {
            return password == decrypted;
        }
    }

    // 方式4: 空密码或无法识别的格式 → 拒绝
    if stored.is_empty() {
        return false;
    }

    // 明文比较已移除（安全隐患）：存储值非 bcrypt/SHA256/XOR 一律拒绝
    tracing::warn!("拒绝登录：密码存储格式无法识别（非 bcrypt/SHA256/旧 XOR），可能为非法明文");
    false
}

/// 判断存储格式是否需要升级为 bcrypt
///
/// - `BCRYPT:` 前缀 → false（已是最新）
/// - `SHA256:` 前缀 → true（需升级）
/// - 16 位 hex → true（需升级）
/// - 其他 → true（异常格式，需重置）
pub fn needs_upgrade(stored: &str) -> bool {
    !stored.starts_with(BCRYPT_PREFIX)
}

/// 判断是否为老 ERP XOR 加密格式（16 位 hex）
pub fn is_legacy_encrypted_password(s: &str) -> bool {
    s.len() == 16 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// 解密老 ERP XOR 加密密码
///
/// 16 位 hex → 8 字节 → 逐位异或 LEGACY_XOR_KEY → 遇 0 截断 → UTF-8 解码
pub fn decrypt_legacy_password(stored: &str) -> Option<String> {
    // 必须是偶数长度且能解析出恰好 8 字节
    if stored.len() % 2 != 0 {
        return None;
    }
    let bytes: Vec<u8> = (0..stored.len())
        .step_by(2)
        .filter_map(|i| u8::from_str_radix(&stored[i..i + 2], 16).ok())
        .collect();
    if bytes.len() != 8 {
        return None;
    }
    let decrypted: Vec<u8> = bytes
        .iter()
        .enumerate()
        .map(|(i, &b)| b ^ LEGACY_XOR_KEY[i])
        .collect();
    let trimmed = decrypted
        .iter()
        .take_while(|&&b| b != 0)
        .copied()
        .collect::<Vec<u8>>();
    String::from_utf8(trimmed).ok()
}

/// 旧 SHA256+静态盐验证（仅用于向后兼容）
fn verify_sha256_legacy(password: &str, stored: &str) -> bool {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(format!("{}{}", password, PASSWORD_SALT).as_bytes());
    let result = hasher.finalize();
    let expected = format!("{}{}", SHA256_PREFIX, hex::encode(result));
    expected == stored
}

/// 仅供测试用：生成旧 SHA256 格式哈希（模拟数据库中的旧密码）
#[cfg(test)]
pub fn hash_sha256_legacy_for_test(password: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(format!("{}{}", password, PASSWORD_SALT).as_bytes());
    let result = hasher.finalize();
    format!("{}{}", SHA256_PREFIX, hex::encode(result))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ============================================================
    // hash_password (bcrypt)
    // ============================================================
    #[test]
    fn test_hash_password_format() {
        let h = hash_password("123456").expect("应成功");
        assert!(h.starts_with(BCRYPT_PREFIX), "应以 BCRYPT: 前缀开头");
        let bcrypt_part = &h[BCRYPT_PREFIX.len()..];
        assert!(bcrypt_part.starts_with("$2b$"), "bcrypt 部分应以 $2b$ 开头");
    }

    #[test]
    fn test_hash_password_non_deterministic() {
        // bcrypt 每次哈希使用不同盐，相同密码产生不同输出
        let h1 = hash_password("123456").expect("应成功");
        let h2 = hash_password("123456").expect("应成功");
        assert_ne!(h1, h2, "bcrypt 相同密码应产生不同哈希（不同盐）");
    }

    #[test]
    fn test_hash_password_distinct() {
        let h1 = hash_password("123456").expect("应成功");
        let h2 = hash_password("123457").expect("应成功");
        assert_ne!(h1, h2, "不同密码应产生不同哈希");
    }

    #[test]
    fn test_hash_password_too_long() {
        // bcrypt 限制 72 字节，超过应返回 None
        let long_pwd = "a".repeat(100);
        assert!(hash_password(&long_pwd).is_none(), "超长密码应返回 None");
    }

    #[test]
    fn test_hash_password_empty() {
        // 空密码应能哈希（bcrypt 允许）
        let h = hash_password("").expect("空密码应成功");
        assert!(h.starts_with(BCRYPT_PREFIX));
    }

    // ============================================================
    // verify_password (bcrypt)
    // ============================================================
    #[test]
    fn test_verify_bcrypt_correct() {
        let stored = hash_password("123456").expect("应成功");
        assert!(verify_password("123456", &stored));
    }

    #[test]
    fn test_verify_bcrypt_wrong() {
        let stored = hash_password("123456").expect("应成功");
        assert!(!verify_password("wrong", &stored));
    }

    #[test]
    fn test_verify_bcrypt_case_sensitive() {
        let stored = hash_password("Admin").expect("应成功");
        assert!(verify_password("Admin", &stored));
        assert!(!verify_password("admin", &stored));
    }

    // ============================================================
    // verify_password (SHA256 旧格式兼容)
    // ============================================================
    #[test]
    fn test_verify_sha256_legacy_correct() {
        let stored = hash_sha256_legacy_for_test("123456");
        assert!(verify_password("123456", &stored));
    }

    #[test]
    fn test_verify_sha256_legacy_wrong() {
        let stored = hash_sha256_legacy_for_test("123456");
        assert!(!verify_password("wrong", &stored));
    }

    #[test]
    fn test_verify_sha256_legacy_distinct_from_pure_sha256() {
        // 旧哈希含静态盐，与纯 sha256(password) 不同
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(b"123456");
        let pure_hash = format!("{}{}", SHA256_PREFIX, hex::encode(hasher.finalize()));
        let salted_hash = hash_sha256_legacy_for_test("123456");
        assert_ne!(pure_hash, salted_hash);
    }

    // ============================================================
    // verify_password (XOR 老ERP兼容)
    // ============================================================
    #[test]
    fn test_verify_xor_known_vector_123456() {
        // '1'(0x31) ^ 0xFC = 0xCD, '2'(0x32) ^ 0xAA = 0x98, '3'(0x33) ^ 0x62 = 0x51,
        // '4'(0x34) ^ 0xA0 = 0x94, '5'(0x35) ^ 0x30 = 0x05, '6'(0x36) ^ 0x9C = 0xAA,
        // \0(0x00) ^ 0xF1 = 0xF1, \0(0x00) ^ 0xD4 = 0xD4
        let stored = "CD98519405AAF1D4";
        assert!(verify_password("123456", stored));
    }

    #[test]
    fn test_verify_xor_known_vector_admin() {
        // 'a'(0x61) ^ 0xFC = 0x9D, 'd'(0x64) ^ 0xAA = 0xCE, 'm'(0x6D) ^ 0x62 = 0x0F,
        // 'i'(0x69) ^ 0xA0 = 0xC9, 'n'(0x6E) ^ 0x30 = 0x5E, \0 ^ 0x9C = 0x9C,
        // \0 ^ 0xF1 = 0xF1, \0 ^ 0xD4 = 0xD4
        let stored = "9DCE0FC95E9CF1D4";
        assert!(verify_password("admin", stored));
    }

    #[test]
    fn test_verify_xor_o_replacement() {
        // 字母 O/o 应自动替换为数字 0 后再解密
        // "CD98519405AAF1D4" 中的 O 替换为 0 → "CD98519405AAF1D4"（无 O 不变）
        // 测试带 O 的变体：将 hex 中的 0 替换为 O 应仍能验证
        let stored_with_o = "CD985194O5AAF1D4";
        assert!(verify_password("123456", stored_with_o));
    }

    #[test]
    fn test_verify_xor_wrong() {
        let stored = "CD98519405AAF1D4";
        assert!(!verify_password("wrong", stored));
    }

    // ============================================================
    // verify_password (异常格式拒绝)
    // ============================================================
    #[test]
    fn test_verify_empty_stored_rejected() {
        assert!(!verify_password("123456", ""));
    }

    #[test]
    fn test_verify_plaintext_rejected() {
        // 非法明文存储应拒绝（不执行明文比较）
        assert!(!verify_password("123456", "123456"));
        assert!(!verify_password("123456", "some_random_text"));
    }

    // ============================================================
    // needs_upgrade
    // ============================================================
    #[test]
    fn test_needs_upgrade_bcrypt() {
        let stored = hash_password("123456").expect("应成功");
        assert!(!needs_upgrade(&stored), "bcrypt 格式无需升级");
    }

    #[test]
    fn test_needs_upgrade_sha256() {
        let stored = hash_sha256_legacy_for_test("123456");
        assert!(needs_upgrade(&stored), "SHA256 格式需升级");
    }

    #[test]
    fn test_needs_upgrade_xor() {
        let stored = "CD98519405AAF1D4";
        assert!(needs_upgrade(stored), "XOR 格式需升级");
    }

    #[test]
    fn test_needs_upgrade_unknown() {
        assert!(needs_upgrade("unknown_format"));
        assert!(needs_upgrade(""));
    }

    // ============================================================
    // 辅助函数测试
    // ============================================================
    #[test]
    fn test_is_legacy_encrypted_password() {
        assert!(is_legacy_encrypted_password("CD98519405AAF1D4"));
        assert!(is_legacy_encrypted_password("cd98519405aaf1d4")); // 小写
        assert!(!is_legacy_encrypted_password("CD98519405AAF1D")); // 15 位
        assert!(!is_legacy_encrypted_password("CD98519405AAF1D44")); // 17 位
        assert!(!is_legacy_encrypted_password("CD98519405AAF1DG")); // 含非 hex
        assert!(!is_legacy_encrypted_password(""));
    }

    #[test]
    fn test_decrypt_legacy_password_known_vector() {
        let stored = "CD98519405AAF1D4";
        assert_eq!(decrypt_legacy_password(stored), Some("123456".to_string()));
    }

    #[test]
    fn test_decrypt_legacy_password_admin() {
        let stored = "9DCE0FC95E9CF1D4";
        assert_eq!(decrypt_legacy_password(stored), Some("admin".to_string()));
    }

    #[test]
    fn test_decrypt_legacy_password_wrong_length() {
        assert_eq!(decrypt_legacy_password("CD9851"), None);
        assert_eq!(decrypt_legacy_password("CD98519405AAF1D44"), None);
    }

    #[test]
    fn test_decrypt_legacy_password_zero_truncation() {
        // "123456" 后跟 \0\0 → 解密后 take_while 遇 0 截断
        let stored = "CD98519405AAF1D4";
        let decrypted = decrypt_legacy_password(stored);
        assert_eq!(decrypted, Some("123456".to_string()));
        // 确保没有尾部 \0
        assert!(!decrypted.unwrap().contains('\0'));
    }

    // ============================================================
    // 端到端：哈希 → 验证 → 升级判断
    // ============================================================
    #[test]
    fn test_end_to_end_bcrypt() {
        let stored = hash_password("MySecure@123").expect("应成功");
        assert!(verify_password("MySecure@123", &stored));
        assert!(!verify_password("wrong", &stored));
        assert!(!needs_upgrade(&stored));
    }

    #[test]
    fn test_end_to_end_legacy_sha256_upgrade_path() {
        // 模拟数据库中的旧 SHA256 密码
        let legacy_stored = hash_sha256_legacy_for_test("123456");
        // 旧密码能验证通过
        assert!(verify_password("123456", &legacy_stored));
        // 需要升级
        assert!(needs_upgrade(&legacy_stored));
        // 升级后为新 bcrypt 格式
        let new_stored = hash_password("123456").expect("应成功");
        assert!(!needs_upgrade(&new_stored));
        // 新密码能验证通过
        assert!(verify_password("123456", &new_stored));
    }

    #[test]
    fn test_end_to_end_legacy_xor_upgrade_path() {
        // 模拟数据库中的老 ERP XOR 密码
        let legacy_stored = "CD98519405AAF1D4";
        assert!(verify_password("123456", legacy_stored));
        assert!(needs_upgrade(legacy_stored));
        // 升级
        let new_stored = hash_password("123456").expect("应成功");
        assert!(!needs_upgrade(&new_stored));
        assert!(verify_password("123456", &new_stored));
    }
}
