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
// 单据编号自增模块（统一路径）
//
// 所有单据统一使用 tSys_DocNoSeq 表生成单据号：
//   格式：prefix + YYMM + 4位序号（如 PO26060001）
//   表：tSys_DocNoSeq (DocTypeID, PeriodKey, CurrentSeq, LUTime)
//   并发安全：UPDATE-OUTPUT 原子操作 + INSERT 失败重试
//
// 前端传 doc_type 作为前缀（如 PO/SO/SD/SR/MV 等）
// 按月重置序号（PeriodKey = YYMM）
// ============================================================

/// 生成单据号请求参数
///   doc_type:  必填, 单据类型前缀, 如 SD/PD/SO/PO/MV/...
///   prefix:    可选, 覆盖 doc_type 作为前缀
///   date:      可选, 自定义日期(YYYY-MM-DD), 默认今天（当前未使用，保留兼容）
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

/// 重置单据序号 - 删除 tSys_DocNoSeq 中对应记录（下次生成时自动从 1 开始）
pub async fn reset_doc_seq(
    State(_config): State<Config>,
    Json(params): Json<ResetSeqParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let prefix = params.doc_type.as_str();

    let deleted = if let Some(pk) = &params.period_key {
        // 重置指定期号
        let pk_str = pk.as_str();
        let sql = "DELETE FROM tSys_DocNoSeq WHERE DocTypeID = @p1 AND PeriodKey = @p2";
        let p: Vec<&dyn ToSql> = vec![&prefix, &pk_str];
        let r = conn.execute(sql, &p).await?;
        r.rows_affected().get(0).copied().unwrap_or(0)
    } else {
        // 重置所有期号
        let sql = "DELETE FROM tSys_DocNoSeq WHERE DocTypeID = @p1";
        let p: Vec<&dyn ToSql> = vec![&prefix];
        let r = conn.execute(sql, &p).await?;
        r.rows_affected().get(0).copied().unwrap_or(0)
    };

    Ok(Json(ApiResponse::msg(&format!(
        "已重置单据 [{}] 的序号（删除 {} 条记录）", prefix, deleted
    ))))
}

/// 单据号生成主函数
///
/// 统一使用 tSys_DocNoSeq 表生成单据号，格式：prefix + YYMM + 4位序号
/// 并发安全：UPDATE-OUTPUT 原子操作 + INSERT 失败重试
pub async fn generate_doc_no(
    State(_config): State<Config>,
    Json(params): Json<GenerateNoParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    // 前缀优先用 prefix 参数，否则用 doc_type
    let prefix = params.prefix.as_deref().unwrap_or(&params.doc_type);
    if prefix.is_empty() {
        return Ok(Json(ApiResponse::err("doc_type 或 prefix 不能为空")));
    }

    let doc_no = generate_via_docnoseq(&mut conn, prefix).await?;

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "doc_no": doc_no,
        "doc_type": params.doc_type,
        "prefix": prefix,
        "code_rule": "YYMM####",
    }))))
}

/// 统一的单据号生成（原子分配，绝不重复）
///
/// 格式：prefix + YYMM + 4位序号（如 PO26060001）
/// 算法：
///   1. 尝试 `UPDATE ... SET CurrentSeq = CurrentSeq + 1 OUTPUT INSERTED.CurrentSeq`
///      （原子自增，并发安全，多并发调用各拿不同序号）
///   2. 若记录不存在（首次），查实际单据表 MAX 序号 + 1 初始化后 INSERT
///      并发初始化冲突时，失败方重试 UPDATE
///
/// 特性：
///   - 绝不重复（原子递增）
///   - 首次初始化不跳号（基于实际表 MAX）
///   - 取消未保存的单据会跳号（已分配的序号不回收，这是并发安全的必要代价）
pub async fn generate_via_docnoseq(conn: &mut Conn, prefix: &str) -> Result<String> {
    let now = chrono::Local::now();
    let period = now.format("%y%m").to_string(); // YYMM
    let period_str = period.as_str();
    let full_prefix = format!("{}{}", prefix, period);

    // 重试上限：处理并发初始化时的主键冲突
    for _attempt in 0..5 {
        // 步骤 1：原子递增（记录存在时直接分配）
        let update_sql = "UPDATE tSys_DocNoSeq SET CurrentSeq = CurrentSeq + 1, LUTime = GETDATE() \
                          OUTPUT INSERTED.CurrentSeq \
                          WHERE DocTypeID = @p1 AND PeriodKey = @p2";
        let p: Vec<&dyn ToSql> = vec![&prefix, &period_str];
        if let Ok(stream) = conn.query(update_sql, &p).await {
            if let Ok(Some(row)) = stream.into_row().await {
                if let Some(seq) = row.get::<i64, _>(0) {
                    tracing::debug!(
                        "[generate_via_docnoseq] prefix={} period={} allocated_seq={}",
                        prefix, period_str, seq
                    );
                    return Ok(format!("{}{:04}", full_prefix, seq));
                }
            }
        }

        // 步骤 2：记录不存在，首次初始化
        // 查询实际单据表最大序号，确保不与已有单据冲突
        let max_seq = query_max_docno_seq(conn, &full_prefix).await;
        let init_seq = max_seq + 1; // 首次初始化 = MAX + 1，不跳号
        tracing::debug!(
            "[generate_via_docnoseq] prefix={} period={} init_seq={} (max_seq={})",
            prefix, period_str, init_seq, max_seq
        );

        // 尝试 INSERT，CurrentSeq 直接设为 init_seq（已分配的序号）
        let insert_sql = "INSERT INTO tSys_DocNoSeq (DocTypeID, PeriodKey, CurrentSeq, LUTime) \
                          VALUES (@p1, @p2, @p3, GETDATE())";
        let p: Vec<&dyn ToSql> = vec![&prefix, &period_str, &init_seq];
        match conn.execute(insert_sql, &p).await {
            Ok(r) => {
                let affected = r.rows_affected().iter().sum::<u64>();
                if affected > 0 {
                    return Ok(format!("{}{:04}", full_prefix, init_seq));
                }
                // affected = 0：异常，重试
            }
            Err(_e) => {
                // 主键冲突：并发已有人插入，回到循环顶部重试 UPDATE
                tracing::warn!(
                    "[generate_via_docnoseq] INSERT 冲突，重试 UPDATE (attempt={})",
                    _attempt + 1
                );
                continue;
            }
        }
    }

    Err(crate::error::AppError::Internal(format!(
        "单据号生成失败：连续 5 次重试均失败 prefix={} period={}",
        prefix, period_str
    )))
}

/// 查询所有单据表中以指定前缀开头的最大单号序号
/// 只查实际存在的表，跳过不存在的表和字段
async fn query_max_docno_seq(conn: &mut Conn, full_prefix: &str) -> i64 {
    // 单据表与单号字段映射（基于 doc_graph 实际配置）
    // 注意：tStk_IO 包含多种 Kind（PR/SD/SR/OTI/OTO 等），各 Kind 单号都存在 IONo 字段
    let tables: &[(&str, &str)] = &[
        ("tPur_Order", "PoNo"),
        ("tPur_Inv", "PiNo"),
        ("tPur_Return", "PrNo"),
        ("tSal_Order", "SoNo"),
        ("tSal_Inv", "SINo"),
        ("tStk_IO", "IONo"),
        ("tStk_Move", "MoveNO"),
        ("tStk_Tran", "TranNo"),
        ("tStk_ReplenishApply", "ReplenishApplyNo"),
        ("tSal_Quote", "SQNo"),
        ("tPur_Quote", "PqNo"),
        ("tPur_AdjPrice", "PAPNo"),
        ("tStk_StockCycle", "CycleNo"),
        ("tFin_Receipt", "RecNO"),
        ("tFin_Payment", "PayNO"),
        ("tFin_CashFlow", "CFNO"),
    ];

    let mut max_seq: i64 = 0;
    let like_pattern = format!("{}%", full_prefix);
    let like_pattern_str = like_pattern.as_str();
    for (table, no_field) in tables {
        // 先检查表是否存在且包含该字段，避免 208/207 错误消耗时间
        let check_sql = "SELECT 1 FROM sys.tables t \
                         INNER JOIN sys.columns c ON t.object_id = c.object_id \
                         WHERE t.name = @p1 AND c.name = @p2";
        let check_p: Vec<&dyn ToSql> = vec![table, no_field];
        let check_stream = match conn.query(check_sql, &check_p).await {
            Ok(s) => s,
            Err(_) => continue,
        };
        let exists = check_stream.into_row().await.ok().flatten().is_some();
        if !exists {
            continue;
        }

        // 查询该表中以 full_prefix 开头的最大单号
        // 用动态 SQL 避免字段名硬编码，同时过滤已删除记录
        let sql = format!(
            "SELECT MAX([{}]) AS M FROM [{}] WHERE [{}] LIKE @p1 AND ISNULL(State,'') <> 'D'",
            no_field, table, no_field
        );
        let p: Vec<&dyn ToSql> = vec![&like_pattern_str];
        if let Ok(stream) = conn.query(&sql, &p).await {
            if let Ok(Some(row)) = stream.into_row().await {
                if let Some(m) = row.get::<&str, _>("M") {
                    // 解析序号部分（full_prefix 之后的部分）
                    if m.len() > full_prefix.len() {
                        let seq_part = &m[full_prefix.len()..];
                        if let Ok(seq) = seq_part.parse::<i64>() {
                            if seq > max_seq {
                                max_seq = seq;
                            }
                        }
                    }
                }
            }
        }
    }
    max_seq
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
