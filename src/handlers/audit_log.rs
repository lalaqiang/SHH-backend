//! 权限审计日志模块
//!
//! 为权限相关敏感操作（用户 CRUD、角色 CRUD、权限分配、菜单变更）记录结构化审计日志。
//! 日志双写：优先写入 tSys_OperLog（含 OperType/TableName/UserName/ClientIP 等结构化字段），
//! 失败时 fallback 到 tSys_OperHis（旧表，仅 OpenMsg 拼接字符串）。
//!
//! 表结构（tSys_OperLog）：
//! - OperLogID: 主键 (NEWID)
//! - OperType: 操作类型 (CREATE/UPDATE/DELETE/ASSIGN_PERM/ASSIGN_ROLE/ENABLE/DISABLE)
//! - TableName: 受影响的表名 (tBas_Emp / tSys_Rule / tSys_RuleMenu / tSys_UserRule / tSys_Menus)
//! - KeyValue: 受影响记录的主键值
//! - UserCode: 操作者的工号
//! - EmpID: 操作者的 EmpID (uniqueidentifier)
//! - UserName: 操作者的姓名
//! - ClientIP: 客户端 IP
//! - OperDate: 操作时间
//! - OldData: 操作前的数据 (JSON 字符串，可选)
//! - NewData: 操作后的数据 (JSON 字符串，可选)
//! - Remark: 备注

use bb8::PooledConnection;
use bb8_tiberius::ConnectionManager;
use tiberius::ToSql;

pub type Conn = PooledConnection<'static, ConnectionManager>;

use crate::middleware::auth::Claims;

/// 审计日志操作类型
pub const OPER_CREATE: &str = "CREATE";
pub const OPER_UPDATE: &str = "UPDATE";
pub const OPER_DELETE: &str = "DELETE";
pub const OPER_ASSIGN_PERM: &str = "ASSIGN_PERM";
pub const OPER_ASSIGN_ROLE: &str = "ASSIGN_ROLE";
pub const OPER_ENABLE: &str = "ENABLE";
pub const OPER_DISABLE: &str = "DISABLE";

/// 写入权限审计日志（结构化，写入 tSys_OperLog 表）
///
/// # 参数
/// - `conn`: 数据库连接
/// - `oper_type`: 操作类型（CREATE/UPDATE/DELETE/ASSIGN_PERM/ASSIGN_ROLE/ENABLE/DISABLE）
/// - `table_name`: 受影响的表名
/// - `key_value`: 受影响记录的主键值
/// - `claims`: 当前登录用户的 JWT Claims（含 user_code/emp_id/user_name）
/// - `client_ip`: 客户端 IP（可为空）
/// - `new_data`: 操作后的数据（JSON 字符串，可选）
/// - `remark`: 备注
pub async fn write_audit_log(
    conn: &mut Conn,
    oper_type: &str,
    table_name: &str,
    key_value: &str,
    claims: &Claims,
    client_ip: Option<&str>,
    new_data: Option<&str>,
    remark: &str,
) {
    let now = chrono::Local::now().naive_local();
    let user_code = claims.user_code.clone();
    let user_name = claims.user_name.clone();
    let emp_id_str = claims.emp_id.clone();

    // EmpID 是 uniqueidentifier，需要 Option<&str>，空字符串转 NULL
    let emp_id_opt: Option<&str> =
        if emp_id_str.len() == 36 && emp_id_str.chars().filter(|c| *c == '-').count() == 4 {
            Some(emp_id_str.as_str())
        } else {
            None
        };

    let ip_val: Option<&str> = client_ip.filter(|s| !s.is_empty());
    let new_data_val: Option<&str> = new_data.filter(|s| !s.is_empty());

    let sql = "INSERT INTO tSys_OperLog (OperLogID, OperType, TableName, KeyValue, UserCode, EmpID, UserName, ClientIP, OperDate, NewData, Remark) \
               VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10)";

    let p: Vec<&dyn ToSql> = vec![
        &oper_type,
        &table_name,
        &key_value,
        &user_code,
        &emp_id_opt,
        &user_name,
        &ip_val,
        &now,
        &new_data_val,
        &remark,
    ];

    if let Err(e) = conn.execute(sql, &p).await {
        tracing::error!(
            error = %e,
            table = %table_name,
            key = %key_value,
            "[write_audit_log] 写入 tSys_OperLog 失败"
        );
        // 失败时 fallback 到旧表 tSys_OperHis（OpenMsg 格式与 record_oper 一致）
        // 格式：操作类型 | 表名 | 备注 | 操作人:工号
        let mut parts: Vec<String> = vec![oper_type.to_string(), table_name.to_string()];
        if !remark.is_empty() {
            parts.push(remark.to_string());
        }
        if !user_code.is_empty() {
            parts.push(format!("操作人:{}", user_code));
        }
        let open_msg = parts.join(" | ");
        let zero_uuid = "00000000-0000-0000-0000-000000000000".to_string();
        let fallback_sql = "INSERT INTO tSys_OperHis (OperHisID, DocID, EmpID, MenusID, OperDate, OpenMsg) \
                            VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5)";
        let p2: Vec<&dyn ToSql> = vec![&key_value, &emp_id_opt, &zero_uuid, &now, &open_msg];
        if let Err(e2) = conn.execute(fallback_sql, &p2).await {
            tracing::error!(error = %e2, "[write_audit_log] fallback 写入 tSys_OperHis 也失败");
        }
    }
}

/// 简化版审计日志（无 new_data，仅记录操作类型和备注）
pub async fn log_perm_action(
    conn: &mut Conn,
    oper_type: &str,
    table_name: &str,
    key_value: &str,
    claims: &Claims,
    remark: &str,
) {
    write_audit_log(
        conn, oper_type, table_name, key_value, claims, None, None, remark,
    )
    .await;
}
