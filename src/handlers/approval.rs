use axum::{extract::State, Extension, Json};
use serde::Deserialize;
use bb8::PooledConnection;
use bb8_tiberius::ConnectionManager;
use crate::config::Config;
use crate::db::get_pool;
use crate::error::Result;
use crate::utils::ApiResponse;
use crate::middleware::auth::Claims;
use crate::services::inventory_ledger as ledger;

pub type Conn = PooledConnection<'static, ConnectionManager>;

#[derive(Deserialize)]
pub struct PrintLogParams {
    pub table: String,
    pub primary_key: String,
    pub id: String,
    pub copies: Option<i32>,
}

/// 月结：把指定月份所有 (StkID, GDSID) 的 EndQty 作为下月 InitQty
/// DB 规则：月初把上月 EndQty 复制为 InitQty
pub async fn month_end_settle(
    conn: &mut Conn,
    from_ym: i32,  // 来源月份 YYYYMM
    to_ym: i32,    // 目标月份 YYYYMM
) -> i32 {
    // 1) 检查目标月份是否已结存（如果 InitQty 已经从历史继承则跳过，避免重复累加）
    let sql = r#"
        INSERT INTO tStk_StockYM (AccYM, StkID, GDSID, InitQty, inQty, OutQty, EndQty)
        SELECT @p1, m.StkID, m.GDSID, m.EndQty, 0, 0, m.EndQty
        FROM tStk_StockYM m
        WHERE m.AccYM = @p2
          AND NOT EXISTS (
              SELECT 1 FROM tStk_StockYM t
              WHERE t.AccYM = @p1 AND t.StkID = m.StkID AND t.GDSID = m.GDSID
          );
    "#;
    let params: Vec<&dyn tiberius::ToSql> = vec![&to_ym, &from_ym];
    let result = conn.execute(sql, &params).await;
    match result {
        Ok(r) => r.rows_affected().iter().sum::<u64>() as i32,
        // P2-6 修复：Err 静默吞掉改为 tracing::error，便于排查月结失败原因
        // 返回 -1 表示失败，调用方据此判断（inventory.rs::month_settle 已检查 rows < 0）
        Err(e) => {
            tracing::error!("[month_end_settle] 月结失败: from_ym={} to_ym={} err={}", from_ym, to_ym, e);
            -1
        }
    }
}

/// 月结回滚：删除指定月份的 StockYM 记录，使其恢复"未结存"状态
/// 安全策略：如果该月 inQty/OutQty 非0（已有业务活动），默认拒绝回滚
///
/// 返回值约定（与原 sp_MonthSettleRollback 存储过程保持兼容）：
///   -1: 参数错误
///   -2: 已有业务活动，拒绝回滚（force=0 时）
///   -3: 跳过（无 StockYM 记录）
///   >=0: 删除的行数
///
/// P5 修复：原调用不存在的存储过程 sp_MonthSettleRollback 导致回滚始终失败。
/// 改为 Rust 内联 SQL 实现，避免依赖数据库端未提供的 SP。
pub async fn month_rollback(conn: &mut Conn, to_ym: i32, force: i32) -> i32 {
    // 1) 参数校验
    if to_ym < 200001 || to_ym > 209912 {
        return -1;
    }
    if force != 0 && force != 1 {
        return -1;
    }

    // 2) 统计目标月份记录数与业务活动量
    // ★ SUM(decimal) 返回 NUMERIC，tiberius 的 row.get::<f64,_> 不支持，
    //   需用 CAST AS FLOAT 强制返回 FLOAT 类型才能正确读取为 f64
    //   参考：doc_service.rs:346-348 同样问题的注释
    let cnt_sql = "SELECT COUNT(*) AS cnt, \
                   CAST(ISNULL(SUM(ABS(ISNULL(InQty,0)) + ABS(ISNULL(OutQty,0))),0) AS FLOAT) AS biz \
                   FROM tStk_StockYM WHERE AccYM = @p1";
    let params: Vec<&dyn tiberius::ToSql> = vec![&to_ym];
    let row_opt = match conn.query(cnt_sql, &params).await {
        Ok(s) => s.into_row().await.ok().flatten(),
        Err(_) => None,
    };
    let (cnt, biz): (i32, f64) = match row_opt {
        Some(r) => (
            // SQL Server COUNT(*) 返回 int (I32)，tiberius 不支持跨整数类型转换，
            // 必须用 i32 读取，否则 panic
            r.get::<i32, _>("cnt").unwrap_or(0),
            // biz 是 SUM(decimal)，tiberius 对 NUMERIC 类型 row.get::<f64,_> 返回 None，
            // 但因为这里用了 CAST AS FLOAT，可以正确读取为 f64
            r.get::<f64, _>("biz").unwrap_or(0.0),
        ),
        None => return -1,
    };

    // 3) 无记录：跳过
    if cnt == 0 {
        return -3;
    }

    // 4) 安全模式：已有业务活动则拒绝
    if force == 0 && biz > 0.0001 {
        return -2;
    }

    // 5) 执行删除
    let del_sql = "DELETE FROM tStk_StockYM WHERE AccYM = @p1";
    let del_params: Vec<&dyn tiberius::ToSql> = vec![&to_ym];
    match conn.execute(del_sql, &del_params).await {
        Ok(r) => r.rows_affected().iter().sum::<u64>() as i32,
        Err(e) => {
            tracing::error!("[month_rollback] 回滚失败: to_ym={} force={} err={}", to_ym, force, e);
            -1
        }
    }
}

/// 回填详情表的库存快照 StkQty/AQty（便于后续单据看到当前库存）
///
/// P2-3 深度防御：加 identifier 校验，避免调用方传入未校验的 table/pk
/// P2-7 错误处理：静默 execute 改为 tracing::warn，便于排查库存快照回填失败
pub async fn fill_detail_stock_snapshot(
    conn: &mut Conn,
    detail_table: &str,
    detail_pk: &str,
    detail_id: &str,
) {
    if detail_id.is_empty() {
        return;
    }
    // P2-3: identifier 校验（深度防御，调用方通常传字面量，但仍校验）
    if !is_valid_identifier(detail_table) || !is_valid_identifier(detail_pk) {
        tracing::warn!("[fill_detail_stock_snapshot] 非法标识符: table={} pk={}", detail_table, detail_pk);
        return;
    }
    // 1. 取这个 detail 的 GDSID, StkID
    let sql = format!(
        "SELECT CAST(GDSID AS NVARCHAR(40)) AS GDSID, CAST(StkID AS NVARCHAR(40)) AS StkID \
         FROM [{}] WHERE {} = @p1",
        detail_table, detail_pk
    );
    let mut gdsid = String::new();
    let mut stkid = String::new();
    if let Ok(stream) = conn.query(&sql, &[&detail_id]).await {
        if let Ok(Some(row)) = stream.into_row().await {
            gdsid = row.get::<&str, _>("GDSID").unwrap_or("").to_string();
            stkid = row.get::<&str, _>("StkID").unwrap_or("").to_string();
        }
    } else {
        tracing::warn!("[fill_detail_stock_snapshot] 查询 GDSID/StkID 失败: table={} pk={} id={}", detail_table, detail_pk, detail_id);
    }
    if gdsid.is_empty() || stkid.is_empty() {
        return;
    }
    // 2. 读 Qty/QQty
    let stk_sql = "SELECT ISNULL(CAST(Qty AS NVARCHAR(50)),'0') AS Q, ISNULL(CAST(QQty AS NVARCHAR(50)),'0') AS Qq FROM tStk_Stock WHERE GDSID = @p1 AND StkID = @p2";
    let mut qty: f64 = 0.0;
    let mut qqty: f64 = 0.0;
    if let Ok(stream) = conn.query(stk_sql, &[&gdsid, &stkid]).await {
        if let Ok(Some(row)) = stream.into_row().await {
            let q_str = row.get::<&str, _>("Q").unwrap_or("0");
            let qq_str = row.get::<&str, _>("Qq").unwrap_or("0");
            qty = q_str.parse().unwrap_or(0.0);
            qqty = qq_str.parse().unwrap_or(0.0);
        }
    }
    // 3. 回填
    let upd = format!("UPDATE [{}] SET StkQty = @p1, AQty = @p2 WHERE {} = @p3", detail_table, detail_pk);
    let params: Vec<&dyn tiberius::ToSql> = vec![&qty, &qqty, &detail_id];
    // P2-7: 静默 execute 改为记录 warn 日志
    if let Err(e) = conn.execute(&upd, &params).await {
        tracing::warn!("[fill_detail_stock_snapshot] 回填 StkQty/AQty 失败: table={} pk={} id={} err={}", detail_table, detail_pk, detail_id, e);
    }
}

/// 复用 generic.rs 的 identifier 校验逻辑（避免循环依赖，本地复制一份）
fn is_valid_identifier(s: &str) -> bool {
    if s.is_empty() || s.len() > 128 {
        return false;
    }
    s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// 批量回填 tStk_IODetail 的 StkQty/AQty（按 IOID）
/// 用于 create_io / update_io 时让草稿也能看到当前库存
///
/// P2-7 错误处理：静默 execute 改为 tracing::warn，便于排查库存快照回填失败
pub async fn fill_io_detail_stock_snapshot(conn: &mut Conn, ioid: &str) {
    if ioid.is_empty() {
        return;
    }
    // 用单条 SQL 批量 UPDATE（避免 N+1 查询）
    let sql = "UPDATE d SET d.StkQty = ISNULL(s.Qty, 0), d.AQty = ISNULL(s.QQty, 0) \
               FROM tStk_IODetail d \
               LEFT JOIN tStk_Stock s ON s.GDSID = d.GDSID AND s.StkID = d.StkID \
               WHERE d.IOID = @p1";
    let params: Vec<&dyn tiberius::ToSql> = vec![&ioid];
    if let Err(e) = conn.execute(sql, &params).await {
        tracing::warn!("[fill_io_detail_stock_snapshot] 回填失败: IOID={} err={}", ioid, e);
    }
}

/// 批量回填 tStk_MoveDetail 的 StkQty/AQty（按 MoveID）
pub async fn fill_move_detail_stock_snapshot(conn: &mut Conn, move_id: &str) {
    if move_id.is_empty() {
        return;
    }
    let sql = "UPDATE d SET d.StkQty = ISNULL(s.Qty, 0), d.AQty = ISNULL(s.QQty, 0) \
               FROM tStk_MoveDetail d \
               LEFT JOIN tStk_Stock s ON s.GDSID = d.GDSID AND s.StkID = d.StkID \
               WHERE d.MoveID = @p1";
    let params: Vec<&dyn tiberius::ToSql> = vec![&move_id];
    if let Err(e) = conn.execute(sql, &params).await {
        tracing::warn!("[fill_move_detail_stock_snapshot] 回填失败: MoveID={} err={}", move_id, e);
    }
}

/// 批量回填 tStk_TranDetail 的 StkQty/AQty（按 TranID）
pub async fn fill_tran_detail_stock_snapshot(conn: &mut Conn, tran_id: &str) {
    if tran_id.is_empty() {
        return;
    }
    let sql = "UPDATE d SET d.StkQty = ISNULL(s.Qty, 0), d.AQty = ISNULL(s.QQty, 0) \
               FROM tStk_TranDetail d \
               LEFT JOIN tStk_Stock s ON s.GDSID = d.GDSID AND s.StkID = d.StkID \
               WHERE d.TranID = @p1";
    let params: Vec<&dyn tiberius::ToSql> = vec![&tran_id];
    if let Err(e) = conn.execute(sql, &params).await {
        tracing::warn!("[fill_tran_detail_stock_snapshot] 回填失败: TranID={} err={}", tran_id, e);
    }
}

/// 统一的库存"过账"：委托给 inventory_ledger::post_ledger_with_period
/// 修复：只动 Qty 不动 QQty（apply_stock_delta）、追加式 StockTranHis、按单据日期月记账
/// ym=0 时用当前月份
pub async fn post_ledger(
    conn: &mut Conn,
    gdsid: &str,
    stkid: &str,
    qty: f64,
    direction: f64,
    tran_id: &str,
    tran_detail_id: &str,
    ym: i32,
) -> (f64, bool) {
    ledger::post_ledger_with_period(conn, gdsid, stkid, qty, direction, tran_id, tran_detail_id, ym).await
}

pub async fn print_log(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<PrintLogParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let copies = params.copies.unwrap_or(1);
    let remark = format!("打印{}份", copies);
    ledger::record_oper(&mut conn, "PRINT", &params.table, &params.id, &claims.user_code, None, Some(&remark)).await;

    Ok(Json(ApiResponse::msg("打印记录已保存")))
}
