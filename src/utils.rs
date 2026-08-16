pub mod doc_no;
pub mod error_codes;
pub mod jwt;
pub mod password;

use serde::Serialize;
use tiberius::Row;

/// 安全地从 tiberius Row 获取 f64 值，兼容 SQL Server 的 decimal/numeric/money 类型
pub fn row_get_f64(row: &Row, col_name: &str) -> f64 {
    // 先尝试直接取 f64
    if let Ok(Some(v)) = row.try_get::<f64, _>(col_name) {
        return v;
    }
    // 再尝试取 Numeric（SQL Server decimal 类型）
    if let Ok(Some(n)) = row.try_get::<tiberius::numeric::Numeric, _>(col_name) {
        let scale = n.scale() as i32;
        return n.value() as f64 / 10f64.powi(scale);
    }
    0.0
}

/// 安全地从 tiberius Row 获取 uniqueidentifier 字符串
/// 兼容 &str / [u8;16] 两种返回类型，避免类型不匹配 panic
pub fn row_get_uuid_str(row: &Row, col_name: &str) -> String {
    // 先尝试取 &str（CONVERT(varchar) 后的列）
    if let Ok(Some(v)) = row.try_get::<&str, _>(col_name) {
        return v.to_string();
    }
    // 再尝试取 [u8;16]（原始 uniqueidentifier 列，tiberius 0.11 返回字节序列）
    if let Ok(Some(bytes)) = row.try_get::<&[u8], _>(col_name) {
        if bytes.len() == 16 {
            // SQL Server uniqueidentifier 字节序与标准 UUID 不同，需调整
            let mut b = [0u8; 16];
            b.copy_from_slice(bytes);
            return format_guid_bytes(&b);
        }
        return String::from_utf8_lossy(bytes).to_string();
    }
    String::new()
}

/// 将 SQL Server uniqueidentifier 的 16 字节序列化为标准 UUID 字符串
fn format_guid_bytes(b: &[u8; 16]) -> String {
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[3],
        b[2],
        b[1],
        b[0],
        b[5],
        b[4],
        b[7],
        b[6],
        b[8],
        b[9],
        b[10],
        b[11],
        b[12],
        b[13],
        b[14],
        b[15]
    )
}

/// P1 修复：错误细节不再回显给客户端。
///
/// 原先全库约 190 处 `ApiResponse::err(&format!("上下文: {}", e))` 会把 tiberius/DB 的
/// 底层错误（含表结构、SQL 片段、连接串上下文）直接透给前端。
/// 统一改走本函数：服务端 tracing 记录完整错误，客户端只收到上下文消息本身。
pub fn db_err(context: &str, e: &impl std::fmt::Display) -> String {
    tracing::error!(context = context, error = %e, "数据库/后端操作失败");
    context.to_string()
}

#[derive(Serialize)]
pub struct ApiResponse<T: Serialize> {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// 业务错误码（如 "STOCK_INSUFFICIENT"），成功时为 None。
    /// 前端可据此精确分类错误，避免依赖 message 字符串匹配。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_size: Option<u32>,
}

impl<T: Serialize> ApiResponse<T> {
    pub fn ok(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            message: None,
            code: None,
            total: None,
            page: None,
            page_size: None,
        }
    }

    pub fn ok_paginated(data: T, total: u64, page: u32, page_size: u32) -> Self {
        Self {
            success: true,
            data: Some(data),
            message: None,
            code: None,
            total: Some(total),
            page: Some(page),
            page_size: Some(page_size),
        }
    }

    pub fn msg(msg: &str) -> Self {
        Self {
            success: true,
            data: None,
            message: Some(msg.to_string()),
            code: None,
            total: None,
            page: None,
            page_size: None,
        }
    }

    pub fn err(msg: &str) -> Self {
        Self {
            success: false,
            data: None,
            message: Some(msg.to_string()),
            code: None,
            total: None,
            page: None,
            page_size: None,
        }
    }

    /// 带业务错误码的错误响应。
    ///
    /// `code` 命名规范：`MODULE_ACTION_REASON`（大写下划线），如：
    /// - `STOCK_INSUFFICIENT` 库存不足
    /// - `BIZ_DOC_ALREADY_APPROVED` 单据已审核
    /// - `VALIDATION_FIELD_REQUIRED` 必填字段缺失
    /// 详见前端 `client/src/config/errorCodes.js`。
    pub fn err_with_code(msg: &str, code: &'static str) -> Self {
        Self {
            success: false,
            data: None,
            message: Some(msg.to_string()),
            code: Some(code),
            total: None,
            page: None,
            page_size: None,
        }
    }

    /// 带业务错误码 + 结构化数据的错误响应。
    ///
    /// 用于需要在前端展示结构化错误明细的场景（如库存不足明细表格）。
    /// 前端通过 `code` 判断错误类型，通过 `data` 获取明细列表渲染表格。
    pub fn err_with_data(msg: &str, code: &'static str, data: T) -> Self {
        Self {
            success: false,
            data: Some(data),
            message: Some(msg.to_string()),
            code: Some(code),
            total: None,
            page: None,
            page_size: None,
        }
    }
}

pub fn build_pagination_sql(base_query: &str, page: u32, page_size: u32) -> String {
    let offset = (page - 1) * page_size;
    let top = offset + page_size;
    format!(
        "SELECT * FROM (SELECT TOP ({top}) ROW_NUMBER() OVER (ORDER BY (SELECT NULL)) as _rn, * FROM ({base_query}) t) p WHERE _rn > {offset}",
        top = top,
        base_query = base_query,
        offset = offset
    )
}

pub fn build_pagination_sql_with_sort(
    base_query: &str,
    page: u32,
    page_size: u32,
    sort_prop: Option<&str>,
    sort_order: Option<&str>,
) -> String {
    let offset = (page - 1) * page_size;
    // 解析排序字段
    let order_clause = match (sort_prop, sort_order) {
        (Some(prop), Some(order)) if !prop.is_empty() => {
            // 允许字母、数字、下划线和点号（表别名.字段名场景，如 h.OperDate）
            let safe_prop: String = prop
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
                .collect();
            if safe_prop.is_empty() {
                String::new()
            } else {
                let direction = if order.eq_ignore_ascii_case("desc") {
                    "DESC"
                } else {
                    "ASC"
                };
                // 含点号的标识符（如 h.OperDate）不加方括号，直接使用；否则加方括号防注入
                if safe_prop.contains('.') {
                    format!("{} {}", safe_prop, direction)
                } else {
                    format!("[{}] {}", safe_prop, direction)
                }
            }
        }
        _ => String::new(),
    };

    // 性能优化：有排序字段时使用 SQL Server 2012+ 的 OFFSET/FETCH NEXT
    // 比旧版 TOP + ROW_NUMBER 三层嵌套更快，深分页不需要排序丢弃前 N 行
    if !order_clause.is_empty() {
        return format!(
            "SELECT * FROM ({base_query}) t ORDER BY {order_clause} OFFSET {offset} ROWS FETCH NEXT {page_size} ROWS ONLY",
            base_query = base_query,
            order_clause = order_clause,
            offset = offset,
            page_size = page_size
        );
    }

    // 无排序字段时回退到旧模式（不保证顺序，但兼容未传 sort_prop 的调用方）
    let top = offset + page_size;
    format!(
        "SELECT * FROM (SELECT TOP ({top}) ROW_NUMBER() OVER (ORDER BY (SELECT NULL)) as _rn, * FROM ({base_query}) t) p WHERE _rn > {offset}",
        top = top,
        base_query = base_query,
        offset = offset
    )
}
