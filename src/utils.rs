pub mod doc_no;

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

#[derive(Serialize)]
pub struct ApiResponse<T: Serialize> {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_size: Option<u32>,
}

impl<T: Serialize> ApiResponse<T> {
    pub fn ok(data: T) -> Self {
        Self { success: true, data: Some(data), message: None, total: None, page: None, page_size: None }
    }

    pub fn ok_paginated(data: T, total: u64, page: u32, page_size: u32) -> Self {
        Self { success: true, data: Some(data), message: None, total: Some(total), page: Some(page), page_size: Some(page_size) }
    }

    pub fn msg(msg: &str) -> Self {
        Self { success: true, data: None, message: Some(msg.to_string()), total: None, page: None, page_size: None }
    }

    pub fn err(msg: &str) -> Self {
        Self { success: false, data: None, message: Some(msg.to_string()), total: None, page: None, page_size: None }
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

pub fn build_pagination_sql_with_sort(base_query: &str, page: u32, page_size: u32, sort_prop: Option<&str>, sort_order: Option<&str>) -> String {
    let offset = (page - 1) * page_size;
    let top = offset + page_size;
    let order_clause = match (sort_prop, sort_order) {
        (Some(prop), Some(order)) if !prop.is_empty() => {
            let safe_prop: String = prop.chars().filter(|c| c.is_alphanumeric() || *c == '_').collect();
            if safe_prop.is_empty() {
                "(SELECT NULL)".to_string()
            } else {
                let direction = if order.eq_ignore_ascii_case("desc") { "DESC" } else { "ASC" };
                format!("[{}] {}", safe_prop, direction)
            }
        }
        _ => "(SELECT NULL)".to_string(),
    };
    format!(
        "SELECT * FROM (SELECT TOP ({top}) ROW_NUMBER() OVER (ORDER BY {order_clause}) as _rn, * FROM ({base_query}) t) p WHERE _rn > {offset}",
        top = top,
        base_query = base_query,
        offset = offset,
        order_clause = order_clause
    )
}
