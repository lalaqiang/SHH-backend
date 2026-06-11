use axum::{extract::State, Json};
use bb8::PooledConnection;
use bb8_tiberius::ConnectionManager;
use serde::Deserialize;
use tiberius::ToSql;
use crate::config::Config;
use crate::db::get_pool;
use crate::error::Result;
use crate::utils::ApiResponse;

type Conn = PooledConnection<'static, ConnectionManager>;

// ============================================================
// 单据编号自增模块
// 真实 DB 结构（tBas_BillType + fGetNewCode 标量函数）：
//   - tBas_BillType.BTPID     : 单据类型主键（uniqueidentifier）
//   - tBas_BillType.BTPCode   : 单据类型编码（001/002/01/02/0）
//   - tBas_BillType.BTPName   : 单据类型名称
//   - tBas_BillType.Kind      : 业务方向（SD/PD/OT/PR/SR/RI/TH/ZP/SO/DB）
//   - tBas_BillType.CodePreFix: 编号前缀
//   - tBas_BillType.CodeRule  : 编号规则（如 YYMM####）
//   - tBas_BillType.MaxCode   : 当前期号的最大单据号
//   - tBas_BillType.LUTime    : 上次使用时间
//   - fGetNewCode(@aBTPID)    : 标量函数，返回新单据号
// ============================================================

/// 生成单据号请求参数
///   doc_type:  必填, 业务类型 Kind 编码, 如 SD/PD/OT/SO/PO/RI/TH/...
///   prefix:    可选, 覆盖配置中的前缀
///   date:      可选, 自定义日期(YYYY-MM-DD), 默认今天（fGetNewCode 实际以服务端 GetDate 为准）
#[derive(Deserialize)]
pub struct GenerateNoParams {
    pub doc_type: String,
    #[serde(default)]
    pub prefix: Option<String>,
    #[serde(default)]
    pub date: Option<String>,
}

/// 序号重置请求
#[derive(Deserialize)]
pub struct ResetSeqParams {
    pub doc_type: String,
    #[serde(default)]
    pub period_key: Option<String>,
}

/// 列出所有单据类型配置 (供前端维护页面) - 列 tBas_BillType
pub async fn list_doc_types(
    State(_config): State<Config>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let sql = "SELECT CAST(BTPID AS NVARCHAR(50)) AS BTPID, BTPCode, BTPName, Kind, \
               Flg, CodePreFix, CodeRule, MaxCode, State, LUTime, Note, ShareAll, btpSD \
               FROM tBas_BillType ORDER BY Kind, BTPCode";
    let stream = conn.query(sql, &[]).await?;
    let rows = stream.into_first_result().await?;
    let list: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            serde_json::json!({
                "doc_type":    r.get::<&str, _>("Kind").unwrap_or(""),
                "btp_code":    r.get::<&str, _>("BTPCode").unwrap_or(""),
                "doc_name":    r.get::<&str, _>("BTPName").unwrap_or(""),
                "btp_id":      r.get::<&str, _>("BTPID").unwrap_or(""),
                "prefix":      r.get::<&str, _>("CodePreFix").unwrap_or(""),
                "code_rule":   r.get::<&str, _>("CodeRule").unwrap_or("YYMM####"),
                "max_code":    r.get::<&str, _>("MaxCode").unwrap_or(""),
                "state":       r.get::<&str, _>("State").unwrap_or("Y"),
                "lu_time":     r.try_get::<chrono::NaiveDateTime, _>("LUTime").ok().flatten().map(|d| d.and_utc().to_rfc3339()).unwrap_or_default(),
                "share_all":   r.get::<&str, _>("ShareAll").unwrap_or(""),
                "btp_sd":      r.get::<i32, _>("btpSD").unwrap_or(0),
            })
        })
        .collect();
    Ok(Json(ApiResponse::ok(serde_json::json!(list))))
}

/// 重置单据序号 - 清空 MaxCode + LUTime 设为 1900-01-01
pub async fn reset_doc_seq(
    State(_config): State<Config>,
    Json(params): Json<ResetSeqParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let btps = resolve_bill_types(&mut conn, &params.doc_type).await?;
    if btps.is_empty() {
        return Ok(Json(ApiResponse::err(&format!(
            "单据业务类型 [{}] 在 tBas_BillType 中未配置", params.doc_type
        ))));
    }
    let mut updated = 0;
    for btp in &btps {
        let btp_id_s = btp.btp_id.as_str();
        let sql = "UPDATE tBas_BillType SET MaxCode = '', LUTime = '1900-01-01' WHERE BTPID = @p1";
        let p: Vec<&dyn ToSql> = vec![&btp_id_s];
        conn.execute(sql, &p).await?;
        updated += 1;
    }
    Ok(Json(ApiResponse::msg(&format!(
        "已重置单据 {} (匹配 {} 条) 的序号", params.doc_type, updated
    ))))
}

/// 单据号生成主函数
pub async fn generate_doc_no(
    State(_config): State<Config>,
    Json(params): Json<GenerateNoParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    // 1) 解析业务类型 → BTPID 列表
    let btps = resolve_bill_types(&mut conn, &params.doc_type).await?;
    if btps.is_empty() {
        return Ok(Json(ApiResponse::err(&format!(
            "单据业务类型 [{}] 在 tBas_BillType 中未配置", params.doc_type
        ))));
    }

    // 2) 校验 State, 取第一条 Y 状态的
    let active: Vec<&BillTypeRow> = btps.iter().filter(|b| b.state == "Y" || b.state == "S").collect();
    if active.is_empty() {
        return Ok(Json(ApiResponse::err(&format!(
            "单据业务类型 [{}] 全部已停用", params.doc_type
        ))));
    }
    let btp = &active[0];

    // 3) 调 fGetNewCode(BTPID) 标量函数
    let btp_id_s = btp.btp_id.as_str();
    let sql = "SELECT dbo.fGetNewCode(@p1) AS NewCode";
    let p: Vec<&dyn ToSql> = vec![&btp_id_s];
    let stream = conn.query(sql, &p).await?;
    let row = match stream.into_row().await? {
        Some(r) => r,
        None => return Ok(Json(ApiResponse::err("fGetNewCode 返回空"))),
    };
    let doc_no = row.get::<&str, _>("NewCode").unwrap_or("").to_string();
    if doc_no.is_empty() {
        return Ok(Json(ApiResponse::err("fGetNewCode 返回空字符串")));
    }

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "doc_no":     doc_no,
        "doc_type":   params.doc_type,
        "btp_id":     btp.btp_id,
        "btp_name":   btp.btp_name,
        "prefix":     params.prefix.unwrap_or_else(|| btp.prefix.clone()),
        "code_rule":  btp.code_rule,
    }))))
}

// ============================================================
// 内部数据结构
// ============================================================

#[derive(Debug, Clone)]
struct BillTypeRow {
    btp_id: String,
    btp_code: String,
    btp_name: String,
    kind: String,
    prefix: String,
    code_rule: String,
    max_code: String,
    state: String,
}

/// 把前端传的 doc_type (如 "SD" / "PD" / "SO" / "PO") 解析成 tBas_BillType 记录列表
///   - 优先按 Kind 字段匹配
///   - 若 Kind 没匹配, 再按 BTPCode 匹配
///   - 仍找不到则按 BTPName LIKE '%doc_type%' 模糊匹配
async fn resolve_bill_types(
    conn: &mut Conn,
    doc_type: &str,
) -> Result<Vec<BillTypeRow>> {
    let sql = "SELECT CAST(BTPID AS NVARCHAR(50)) AS BTPID, BTPCode, BTPName, Kind, \
               CodePreFix, CodeRule, MaxCode, State \
               FROM tBas_BillType \
               WHERE Kind = @p1 OR BTPCode = @p1 OR BTPName LIKE '%' + @p1 + '%' \
               ORDER BY \
                 CASE WHEN Kind = @p1 THEN 0 \
                      WHEN BTPCode = @p1 THEN 1 \
                      ELSE 2 END";
    let p: Vec<&dyn ToSql> = vec![&doc_type];
    let stream = conn.query(sql, &p).await?;
    let rows = stream.into_first_result().await?;
    Ok(rows.iter().map(|r| BillTypeRow {
        btp_id:    r.get::<&str, _>("BTPID").unwrap_or("").to_string(),
        btp_code:  r.get::<&str, _>("BTPCode").unwrap_or("").to_string(),
        btp_name:  r.get::<&str, _>("BTPName").unwrap_or("").to_string(),
        kind:      r.get::<&str, _>("Kind").unwrap_or("").to_string(),
        prefix:    r.get::<&str, _>("CodePreFix").unwrap_or("").to_string(),
        code_rule: r.get::<&str, _>("CodeRule").unwrap_or("YYMM####").to_string(),
        max_code:  r.get::<&str, _>("MaxCode").unwrap_or("").to_string(),
        state:     r.get::<&str, _>("State").unwrap_or("Y").to_string(),
    }).collect())
}

// ============================================================
// 单元测试
// ============================================================
#[cfg(test)]
mod tests {
    #[test]
    fn test_format_dummy() {
        // 占位：DB 相关逻辑需要集成测试
        assert!(true);
    }
}
