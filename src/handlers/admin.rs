// ============== 库存安全网触发器 / 约束 状态查询 ==============
// 用于运维检查 DB 端的兜底机制是否安装

use crate::config::Config;
use crate::db::get_pool;
use crate::error::Result;
use crate::utils::ApiResponse;
use axum::{Json, extract::State};
use serde::Serialize;

#[derive(Serialize)]
pub struct TriggerStatus {
    pub name: String,
    pub kind: String, // "TRIGGER" / "CHECK_CONSTRAINT"
    pub is_installed: bool,
    pub details: Option<String>,
}

#[derive(Serialize)]
pub struct TriggerCheckResult {
    pub all_installed: bool,
    pub expected_count: usize,
    pub installed_count: usize,
    pub sql_script: String,
    pub items: Vec<TriggerStatus>,
}

/// GET /api/admin/check_triggers
/// 检查 init_db_triggers.sql 里的 4 个触发器 + 3 个 CHECK 约束是否全部安装
pub async fn check_triggers(
    State(_config): State<Config>,
) -> Result<Json<ApiResponse<TriggerCheckResult>>> {
    let mut conn = get_pool().get().await?;

    // 期望的对象清单（与 init_db_triggers.sql 保持一致）
    let expected: Vec<(&str, &str)> = vec![
        ("trg_IODetail_SafetyStock", "TRIGGER"),
        ("trg_MoveDetail_SafetyStock", "TRIGGER"),
        ("trg_TranDetail_SafetyStock", "TRIGGER"),
        ("trg_Stock_AfterChange", "TRIGGER"),
        ("CK_Stock_Qty_NonNeg", "CHECK_CONSTRAINT"),
        ("CK_Stock_Qty_GE_QQty", "CHECK_CONSTRAINT"),
        ("CK_IODetail_Qty_NotZero", "CHECK_CONSTRAINT"),
    ];

    let mut items: Vec<TriggerStatus> = Vec::new();
    for (name, kind) in &expected {
        let status = match conn.query(
            "SELECT TOP 1 o.name, o.type_desc, \
                    ISNULL(CASE WHEN o.type_desc = 'CHECK_CONSTRAINT' \
                         THEN (SELECT definition FROM sys.check_constraints WHERE object_id = o.object_id) \
                         ELSE NULL END, '') AS Def \
             FROM sys.objects o WHERE o.name = @p1",
            &[name],
        ).await {
            Ok(s) => match s.into_row().await {
                Ok(Some(r)) => {
                    let details = r.get::<&str, _>("Def").unwrap_or("").to_string();
                    TriggerStatus {
                        name: name.to_string(),
                        kind: kind.to_string(),
                        is_installed: true,
                        details: if details.is_empty() { None } else { Some(details) },
                    }
                }
                _ => TriggerStatus {
                    name: name.to_string(),
                    kind: kind.to_string(),
                    is_installed: false,
                    details: None,
                },
            },
            Err(e) => TriggerStatus {
                name: name.to_string(),
                kind: kind.to_string(),
                is_installed: false,
                details: Some(format!("查询失败: {}", e)),
            },
        };
        items.push(status);
    }

    let installed_count = items.iter().filter(|s| s.is_installed).count();
    let result = TriggerCheckResult {
        all_installed: installed_count == expected.len(),
        expected_count: expected.len(),
        installed_count,
        sql_script: "server-rust/scripts/init_db_triggers.sql".to_string(),
        items,
    };
    Ok(Json(ApiResponse::ok(result)))
}
