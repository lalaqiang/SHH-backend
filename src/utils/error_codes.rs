//! 业务错误码集中定义
//!
//! 所有错误码在此定义常量，handler 通过 `ApiResponse::err_with_code(msg, CODE)` 引用，
//! 避免在代码中散落字符串字面量，便于重构与排查。
//!
//! 命名规范：`MODULE_ACTION_REASON`（大写下划线），与前端 `config/errorCodes.js` 一一对应。
//!
//! 使用方式：
//!   ```ignore
//!   use crate::utils::error_codes::*;
//!   return Json(ApiResponse::err_with_code("密码错误", AUTH_PASSWORD_WRONG));
//!   ```

// ============================================================
// 认证 / 权限
// ============================================================
/// token 缺失（未携带 Authorization 头）
pub const AUTH_TOKEN_MISSING: &str = "AUTH_TOKEN_MISSING";
/// token 格式无效（非 Bearer 前缀等）
pub const AUTH_TOKEN_INVALID: &str = "AUTH_TOKEN_INVALID";
/// token 过期或签名错误
pub const AUTH_TOKEN_EXPIRED: &str = "AUTH_TOKEN_EXPIRED";
/// 用户名不存在
pub const AUTH_USER_NOT_FOUND: &str = "AUTH_USER_NOT_FOUND";
/// 密码错误
pub const AUTH_PASSWORD_WRONG: &str = "AUTH_PASSWORD_WRONG";
/// 密码长度不足（P1-12：已废弃，密码长度限制已移除以符合内部易用性偏好；保留常量仅供向后兼容）
pub const AUTH_PASSWORD_TOO_SHORT: &str = "AUTH_PASSWORD_TOO_SHORT";
/// 新旧密码相同
pub const AUTH_PASSWORD_SAME: &str = "AUTH_PASSWORD_SAME";
/// 无操作权限
pub const PERM_DENIED: &str = "PERM_DENIED";
/// P0-S2：访问敏感系统表被拒绝（黑名单校验）
///   用于通用 CRUD 接口的 is_table_blacklisted 校验失败时返回
pub const PERMISSION_DENIED_TABLE: &str = "PERMISSION_DENIED_TABLE";
/// P0-S4：记录级越权（用户尝试更新/删除非自己创建的记录）
pub const PERMISSION_DENIED_RECORD: &str = "PERMISSION_DENIED_RECORD";
/// P1-7：密码哈希失败（bcrypt 错误，如密码超长 >72 字节）
///   原代码复用 AUTH_PASSWORD_WRONG 与"密码错误"语义混淆
pub const SYS_HASH_FAILED: &str = "SYS_HASH_FAILED";

// ============================================================
// 限流
// ============================================================
/// 请求频率超限
pub const RATE_LIMITED: &str = "RATE_LIMITED";

// ============================================================
// 库存
// ============================================================
/// 库存不足
pub const STOCK_INSUFFICIENT: &str = "STOCK_INSUFFICIENT";

// ============================================================
// 单据业务规则
// ============================================================
/// 单据状态不允许此操作（如审核非草稿状态单据）
pub const BIZ_DOC_STATE_INVALID: &str = "BIZ_DOC_STATE_INVALID";
/// 单据已审核，无需重复审核
pub const BIZ_DOC_ALREADY_APPROVED: &str = "BIZ_DOC_ALREADY_APPROVED";
/// 单据反审失败
pub const BIZ_DOC_REVERSE_FAILED: &str = "BIZ_DOC_REVERSE_FAILED";
/// 单据不存在
pub const BIZ_DOC_NOT_FOUND: &str = "BIZ_DOC_NOT_FOUND";
/// 单据明细为空
pub const BIZ_DOC_DETAIL_EMPTY: &str = "BIZ_DOC_DETAIL_EMPTY";
/// 单据号生成失败
pub const BIZ_DOC_NO_GEN_FAILED: &str = "BIZ_DOC_NO_GEN_FAILED";

// ============================================================
// 数据验证
// ============================================================
/// 必填字段缺失
pub const VALIDATION_FIELD_REQUIRED: &str = "VALIDATION_FIELD_REQUIRED";
/// 字段格式不正确
pub const VALIDATION_FIELD_FORMAT: &str = "VALIDATION_FIELD_FORMAT";
/// 表名无效（通用 CRUD 接口校验）
pub const VALIDATION_TABLE_INVALID: &str = "VALIDATION_TABLE_INVALID";
/// 字段名无效（含非法字符）
pub const VALIDATION_FIELD_INVALID: &str = "VALIDATION_FIELD_INVALID";
/// 主键缺失
pub const VALIDATION_PK_MISSING: &str = "VALIDATION_PK_MISSING";

// ============================================================
// 系统级
// ============================================================
/// 数据库错误
pub const SYS_DB_ERROR: &str = "SYS_DB_ERROR";
/// 数据库不可用（连接池耗尽等）
pub const SYS_DB_UNAVAILABLE: &str = "SYS_DB_UNAVAILABLE";
/// 服务器内部错误
pub const SYS_INTERNAL_ERROR: &str = "SYS_INTERNAL_ERROR";
/// 请求参数错误
pub const BAD_REQUEST: &str = "BAD_REQUEST";

// ============================================================
// 单元测试
// ============================================================
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_codes_not_empty() {
        assert!(!AUTH_TOKEN_MISSING.is_empty());
        assert!(!STOCK_INSUFFICIENT.is_empty());
        assert!(!BIZ_DOC_STATE_INVALID.is_empty());
        assert!(!VALIDATION_FIELD_REQUIRED.is_empty());
        assert!(!SYS_DB_ERROR.is_empty());
    }

    #[test]
    fn test_error_codes_naming_convention() {
        // 所有错误码应为大写下划线格式
        let codes = [
            AUTH_TOKEN_MISSING,
            AUTH_TOKEN_INVALID,
            AUTH_TOKEN_EXPIRED,
            AUTH_USER_NOT_FOUND,
            AUTH_PASSWORD_WRONG,
            AUTH_PASSWORD_TOO_SHORT,
            AUTH_PASSWORD_SAME,
            PERM_DENIED,
            PERMISSION_DENIED_TABLE,
            PERMISSION_DENIED_RECORD,
            RATE_LIMITED,
            STOCK_INSUFFICIENT,
            BIZ_DOC_STATE_INVALID,
            BIZ_DOC_ALREADY_APPROVED,
            BIZ_DOC_REVERSE_FAILED,
            BIZ_DOC_NOT_FOUND,
            BIZ_DOC_DETAIL_EMPTY,
            BIZ_DOC_NO_GEN_FAILED,
            VALIDATION_FIELD_REQUIRED,
            VALIDATION_FIELD_FORMAT,
            VALIDATION_TABLE_INVALID,
            VALIDATION_FIELD_INVALID,
            VALIDATION_PK_MISSING,
            SYS_DB_ERROR,
            SYS_DB_UNAVAILABLE,
            SYS_INTERNAL_ERROR,
            BAD_REQUEST,
            SYS_HASH_FAILED,
        ];
        for code in codes {
            assert!(
                code.chars().all(|c| c.is_ascii_uppercase() || c == '_'),
                "错误码 {} 应全为大写下划线格式",
                code
            );
        }
    }

    #[test]
    fn test_error_codes_unique() {
        let codes = [
            AUTH_TOKEN_MISSING,
            AUTH_TOKEN_INVALID,
            AUTH_TOKEN_EXPIRED,
            AUTH_USER_NOT_FOUND,
            AUTH_PASSWORD_WRONG,
            AUTH_PASSWORD_TOO_SHORT,
            AUTH_PASSWORD_SAME,
            PERM_DENIED,
            PERMISSION_DENIED_TABLE,
            PERMISSION_DENIED_RECORD,
            RATE_LIMITED,
            STOCK_INSUFFICIENT,
            BIZ_DOC_STATE_INVALID,
            BIZ_DOC_ALREADY_APPROVED,
            BIZ_DOC_REVERSE_FAILED,
            BIZ_DOC_NOT_FOUND,
            BIZ_DOC_DETAIL_EMPTY,
            BIZ_DOC_NO_GEN_FAILED,
            VALIDATION_FIELD_REQUIRED,
            VALIDATION_FIELD_FORMAT,
            VALIDATION_TABLE_INVALID,
            VALIDATION_FIELD_INVALID,
            VALIDATION_PK_MISSING,
            SYS_DB_ERROR,
            SYS_DB_UNAVAILABLE,
            SYS_INTERNAL_ERROR,
            BAD_REQUEST,
            SYS_HASH_FAILED,
        ];
        let mut sorted = codes.to_vec();
        sorted.sort();
        for i in 1..sorted.len() {
            assert_ne!(sorted[i], sorted[i - 1], "错误码重复: {}", sorted[i]);
        }
    }
}
