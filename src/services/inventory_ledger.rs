//! 库存三件套过账服务（post_ledger）
//!
//! 把 `handlers/approval.rs` 中的库存过账函数集中到 services 层，
//! 作为 doc_service 和 approval 调用的统一入口。
//!
//! 涉及表：
//!   - tStk_Stock（当前余额：Qty/QQty）
//!   - tStk_StockTranHis（最近一次交易流水）
//!   - tStk_StockYM（按月结存：InitQty/inQty/OutQty/EndQty）
//!   - tStk_Qty（物化快照）
//!
//! 业务方向：+1 入库 / -1 出库 / 0 调拨（双边）
//! 安全网：`docs/库存安全网触发器.md` 中的 4 触发器 + 3 CHECK 约束

use crate::utils::row_get_f64;
use bb8::PooledConnection;
use bb8_tiberius::ConnectionManager;
use tiberius::ToSql;

pub type Conn = PooledConnection<'static, ConnectionManager>;

/// 读 tStk_Stock 当前 Qty
pub async fn query_stock_qty(conn: &mut Conn, gdsid: &str, stkid: &str) -> f64 {
    if gdsid.is_empty() || stkid.is_empty() {
        return 0.0;
    }
    let sql = "SELECT ISNULL(CAST(Qty AS NVARCHAR(50)),'0') AS Q FROM tStk_Stock WHERE GDSID = @p1 AND StkID = @p2";
    let params: Vec<&dyn ToSql> = vec![&gdsid, &stkid];
    match conn.query(sql, &params).await {
        Ok(stream) => {
            if let Ok(Some(row)) = stream.into_row().await {
                let q_str = row.get::<&str, _>("Q").unwrap_or("0");
                q_str.parse().unwrap_or(0.0)
            } else {
                0.0
            }
        }
        Err(_) => 0.0,
    }
}

/// 读 tStk_Stock 当前 QQty
pub async fn query_qqty(conn: &mut Conn, gdsid: &str, stkid: &str) -> f64 {
    if gdsid.is_empty() || stkid.is_empty() {
        return 0.0;
    }
    let sql = "SELECT ISNULL(CAST(QQty AS NVARCHAR(50)),'0') AS Q FROM tStk_Stock WHERE GDSID = @p1 AND StkID = @p2";
    let params: Vec<&dyn ToSql> = vec![&gdsid, &stkid];
    match conn.query(sql, &params).await {
        Ok(stream) => {
            if let Ok(Some(row)) = stream.into_row().await {
                let q_str = row.get::<&str, _>("Q").unwrap_or("0");
                q_str.parse().unwrap_or(0.0)
            } else {
                0.0
            }
        }
        Err(_) => 0.0,
    }
}

/// UPSERT tStk_Stock（只动 Qty，不动 QQty），返回更新后的 Qty
/// QQty（预占量）只由 apply_qqty_delta 显式调用时才动
///
/// P0 修复（防超卖竞态）：原实现"先 SELECT 校验、再 UPDATE"，两个并发审核事务
/// 在 READ COMMITTED 下都能读到同一个旧库存值并双双通过校验，导致超卖/负库存。
/// 现将充足性校验合并进 UPDATE 谓词（仅当行存在且更新后 Qty >= -0.0001 才生效）：
/// 数据库对同一行的 UPDATE 串行执行，后到的事务以提交后的最新值重估谓词。
/// 注意兼容 SQL Server 2005，不使用 MERGE（2008 才引入）。
/// 返回值约定：成功返回新 Qty；库存不足或 SQL 失败返回 -1.0（调用方按 < -0.5 判失败）。
pub async fn apply_stock_delta(conn: &mut Conn, gdsid: &str, stkid: &str, delta: f64) -> f64 {
    if gdsid.is_empty() || stkid.is_empty() {
        tracing::debug!("[apply_stock_delta] 跳过: gdsid 或 stkid 为空");
        return 0.0;
    }
    tracing::debug!(
        "[apply_stock_delta] 执行: gdsid={} stkid={} delta={}",
        gdsid,
        stkid,
        delta
    );
    let guard_sql = r#"UPDATE tStk_Stock SET Qty = ISNULL(Qty,0) + @p3
                 WHERE GDSID = @p1 AND StkID = @p2 AND ISNULL(Qty,0) + @p3 >= -0.0001"#;
    let params: Vec<&dyn ToSql> = vec![&gdsid, &stkid, &delta];
    match conn.execute(guard_sql, &params).await {
        Ok(r) => {
            let ra = r.rows_affected().get(0).copied().unwrap_or(0);
            if ra == 1 {
                let q = query_stock_qty(conn, gdsid, stkid).await;
                tracing::debug!(
                    "[apply_stock_delta] 条件更新成功, rows_affected={}, new_qty={}",
                    ra,
                    q
                );
                return q;
            }
            tracing::debug!(
                "[apply_stock_delta] 条件更新命中 0 行（行不存在或库存不足），进入插入分支"
            );
        }
        Err(e) => {
            tracing::error!("[apply_stock_delta] SQL 执行失败: err={}", e);
            return -1.0;
        }
    }
    // 0 行更新：行不存在（首笔入库）或库存不足。
    // 仅当行确实不存在且 delta 非负时插入新行；负数首笔等同于库存不足。
    let exists_sql = "SELECT 1 FROM tStk_Stock WHERE GDSID = @p1 AND StkID = @p2";
    match conn.query(exists_sql, &params[..2]).await {
        Ok(stream) => {
            if let Ok(Some(_)) = stream.into_row().await {
                tracing::warn!(
                    "[apply_stock_delta] 库存不足，拒绝扣减: gdsid={} stkid={} delta={}",
                    gdsid,
                    stkid,
                    delta
                );
                return -1.0;
            }
        }
        Err(e) => {
            tracing::error!("[apply_stock_delta] 探测库存行失败: err={}", e);
            return -1.0;
        }
    }
    if delta < -0.0001 {
        tracing::warn!(
            "[apply_stock_delta] 首笔即负数，拒绝: gdsid={} stkid={} delta={}",
            gdsid,
            stkid,
            delta
        );
        return -1.0;
    }
    let insert_sql = r#"INSERT INTO tStk_Stock (GDSStockID, GDSID, StkID, Qty, QQty)
                 VALUES (NEWID(), @p1, @p2, @p3, 0)"#;
    match conn.execute(insert_sql, &params).await {
        Ok(_) => {
            tracing::debug!(
                "[apply_stock_delta] 插入新库存行: gdsid={} stkid={} qty={}",
                gdsid,
                stkid,
                delta
            );
            delta
        }
        Err(e) => {
            // 并发插入撞主键（其他事务已抢先建行）：回退为条件更新重试一次。
            // delta >= 0 时谓词必然满足，重试等价于把本次增量补上；仍失败则冒泡由业务事务回滚。
            tracing::warn!("[apply_stock_delta] 插入冲突，回退条件更新: err={}", e);
            match conn.execute(guard_sql, &params).await {
                Ok(r) if r.rows_affected().get(0).copied().unwrap_or(0) == 1 => {
                    query_stock_qty(conn, gdsid, stkid).await
                }
                _ => {
                    tracing::error!(
                        "[apply_stock_delta] 回退更新仍失败: gdsid={} stkid={} delta={}",
                        gdsid,
                        stkid,
                        delta
                    );
                    -1.0
                }
            }
        }
    }
}

/// 仅调整 QQty（销售订单预占/释放用，Qty 不变）
///
/// P0 修复（防预占竞态）：负向预占（扣减预占量）改为条件更新，
/// 校验合入 UPDATE 谓词，避免并发下重复预占/预占为负。
pub async fn apply_qqty_delta(conn: &mut Conn, gdsid: &str, stkid: &str, delta: f64) -> bool {
    if gdsid.is_empty() || stkid.is_empty() {
        return true;
    }
    if delta < 0.0 {
        let sql = r#"UPDATE tStk_Stock SET QQty = ISNULL(QQty,0) + @p3
                 WHERE GDSID = @p1 AND StkID = @p2 AND ISNULL(QQty,0) + @p3 >= -0.0001"#;
        let params: Vec<&dyn ToSql> = vec![&gdsid, &stkid, &delta];
        return match conn.execute(sql, &params).await {
            Ok(r) => {
                let ra = r.rows_affected().get(0).copied().unwrap_or(0);
                if ra == 1 {
                    true
                } else {
                    // 0 行 = 行不存在（当前预占 0）或预占不足，均为失败
                    tracing::warn!(
                        "[apply_qqty_delta] 预占不足，拒绝: gdsid={} stkid={} delta={}",
                        gdsid,
                        stkid,
                        delta
                    );
                    false
                }
            }
            Err(e) => {
                tracing::error!(
                    "[apply_qqty_delta] SQL 执行失败: gdsid={}, stkid={}, delta={}, err={}",
                    gdsid,
                    stkid,
                    delta,
                    e
                );
                false
            }
        };
    }
    let sql = r#"IF EXISTS (SELECT 1 FROM tStk_Stock WHERE GDSID = @p1 AND StkID = @p2)
                 UPDATE tStk_Stock SET QQty = ISNULL(QQty,0) + @p3 WHERE GDSID = @p1 AND StkID = @p2
                 ELSE
                 INSERT INTO tStk_Stock (GDSStockID, GDSID, StkID, Qty, QQty) VALUES (NEWID(), @p1, @p2, 0, @p3)"#;
    let params: Vec<&dyn ToSql> = vec![&gdsid, &stkid, &delta];
    match conn.execute(sql, &params).await {
        Ok(_) => true,
        Err(e) => {
            tracing::error!(
                "[apply_qqty_delta] SQL 执行失败: gdsid={}, stkid={}, delta={}, err={}",
                gdsid,
                stkid,
                delta,
                e
            );
            false
        }
    }
}

/// 写 tStk_StockTranHis（UPSERT，主键为 GDSID+StkID，记录最近一次交易快照）
///
/// 该表主键是 (GDSID, StkID)，只保留每个商品+仓库组合的最近一次交易，
/// 不是流水表。每次审核覆盖更新。
pub async fn insert_stock_tran_his(
    conn: &mut Conn,
    gdsid: &str,
    stkid: &str,
    tran_id: &str,
    tran_detail_id: &str,
    in_qty: f64,
    out_qty: f64,
    end_qty: f64,
) -> bool {
    if gdsid.is_empty() || stkid.is_empty() {
        return true;
    }
    let sql = r#"IF EXISTS (SELECT 1 FROM tStk_StockTranHis WHERE GDSID = @p1 AND StkID = @p2)
                 UPDATE tStk_StockTranHis
                 SET LastTranDate = GETDATE(),
                     Qty = @p7 - @p3 + @p4,
                     TranID = @p5,
                     TranDetailID = @p6,
                     InQty = @p3,
                     OutQty = @p4,
                     EndQty = @p7
                 WHERE GDSID = @p1 AND StkID = @p2
                 ELSE
                 INSERT INTO tStk_StockTranHis
                 (GDSID, StkID, LastTranDate, Qty, TranID, TranDetailID, InQty, OutQty, EndQty)
                 VALUES (@p1, @p2, GETDATE(), @p7 - @p3 + @p4, @p5, @p6, @p3, @p4, @p7)"#;
    let params: Vec<&dyn ToSql> = vec![
        &gdsid,
        &stkid,
        &in_qty,
        &out_qty,
        &tran_id,
        &tran_detail_id,
        &end_qty,
    ];
    match conn.execute(sql, &params).await {
        Ok(_) => true,
        Err(e) => {
            tracing::warn!(
                "[insert_stock_tran_his] 写入流水失败: gdsid={}, stkid={}, err={}",
                gdsid,
                stkid,
                e
            );
            false
        }
    }
}

/// 兼容旧调用：追加式 INSERT（已废弃，新代码请用 insert_stock_tran_his）
pub async fn upsert_stock_tran_his(
    conn: &mut Conn,
    gdsid: &str,
    stkid: &str,
    tran_id: &str,
    tran_detail_id: &str,
    in_qty: f64,
    out_qty: f64,
    end_qty: f64,
) {
    insert_stock_tran_his(
        conn,
        gdsid,
        stkid,
        tran_id,
        tran_detail_id,
        in_qty,
        out_qty,
        end_qty,
    )
    .await;
}

/// 累加 tStk_StockYM（按当前月份 YYYYMM）
pub async fn upsert_stock_ym(
    conn: &mut Conn,
    gdsid: &str,
    stkid: &str,
    in_qty: f64,
    out_qty: f64,
) -> bool {
    // P3-27 修复：原 fallback 写死 202501（已过期值）
    //   改为：解析失败时用 UTC 当前年月（容器时区错误时仍能写入合理值）
    //   推荐：部署时设置 TZ=Asia/Shanghai（已在 docker-compose.yml 配置）
    let ym: i32 = chrono::Local::now()
        .format("%Y%m")
        .to_string()
        .parse()
        .unwrap_or_else(|_| {
            let utc_ym = chrono::Utc::now().format("%Y%m").to_string();
            tracing::warn!("本地时间月份解析失败，回退到 UTC 年月: {}", utc_ym);
            utc_ym.parse().unwrap_or(202601)
        });
    upsert_stock_ym_with_period(conn, gdsid, stkid, in_qty, out_qty, ym).await
}

/// 累加 tStk_StockYM（按指定月份 YYYYMM）
pub async fn upsert_stock_ym_with_period(
    conn: &mut Conn,
    gdsid: &str,
    stkid: &str,
    in_qty: f64,
    out_qty: f64,
    ym: i32,
) -> bool {
    if gdsid.is_empty() || stkid.is_empty() || (in_qty == 0.0 && out_qty == 0.0) {
        return true;
    }
    let delta = in_qty - out_qty;
    let sql = r#"IF EXISTS (SELECT 1 FROM tStk_StockYM WHERE AccYM = @p1 AND StkID = @p2 AND GDSID = @p3)
                 UPDATE tStk_StockYM
                 SET inQty = ISNULL(inQty,0) + @p4,
                     OutQty = ISNULL(OutQty,0) + @p5,
                     EndQty = ISNULL(EndQty,0) + @p6
                 WHERE AccYM = @p1 AND StkID = @p2 AND GDSID = @p3
                 ELSE
                 INSERT INTO tStk_StockYM (AccYM, StkID, GDSID, InitQty, inQty, OutQty, EndQty)
                 VALUES (@p1, @p2, @p3, 0, @p4, @p5, @p6)"#;
    let params: Vec<&dyn ToSql> = vec![&ym, &stkid, &gdsid, &in_qty, &out_qty, &delta];
    match conn.execute(sql, &params).await {
        Ok(_) => true,
        Err(e) => {
            tracing::warn!(
                "[upsert_stock_ym_with_period] 写入月结存失败: ym={}, gdsid={}, stkid={}, err={}",
                ym,
                gdsid,
                stkid,
                e
            );
            false
        }
    }
}

/// 维护 tStk_Qty 物化快照表（与 tStk_Stock 同步）
pub async fn upsert_stock_qty_snapshot(conn: &mut Conn, gdsid: &str, stkid: &str) -> bool {
    if gdsid.is_empty() || stkid.is_empty() {
        return true;
    }
    let qty: f64 = query_stock_qty(conn, gdsid, stkid).await;
    let sql = r#"IF EXISTS (SELECT 1 FROM tStk_Qty WHERE GDSID = @p1 AND StkID = @p2)
                 UPDATE tStk_Qty SET Qty = @p3, LUTime = GETDATE()
                 WHERE GDSID = @p1 AND StkID = @p2
                 ELSE
                 INSERT INTO tStk_Qty (GDSID, StkID, Qty, LUTime)
                 VALUES (@p1, @p2, @p3, GETDATE())"#;
    let params: Vec<&dyn ToSql> = vec![&gdsid, &stkid, &qty];
    match conn.execute(sql, &params).await {
        Ok(_) => true,
        Err(e) => {
            tracing::warn!(
                "[upsert_stock_qty_snapshot] 写入快照失败: gdsid={}, stkid={}, err={}",
                gdsid,
                stkid,
                e
            );
            false
        }
    }
}

/// 反审时删除 tStk_StockTranHis 中关联此单据的流水记录
pub async fn delete_stock_tran_his(conn: &mut Conn, tran_id: &str) -> bool {
    if tran_id.is_empty() {
        return true;
    }
    let sql = "DELETE FROM tStk_StockTranHis WHERE TranID = @p1";
    let params: Vec<&dyn ToSql> = vec![&tran_id];
    match conn.execute(sql, &params).await {
        Ok(_) => true,
        Err(e) => {
            tracing::warn!(
                "[delete_stock_tran_his] 删除流水失败: tran_id={}, err={}",
                tran_id,
                e
            );
            false
        }
    }
}

/// 统一的库存"过账"：更新三件套（tStk_Stock + tStk_StockTranHis + tStk_StockYM）
/// 并同步 tStk_Qty 物化快照
/// 返回 (new_qty, success)
pub async fn post_ledger(
    conn: &mut Conn,
    gdsid: &str,
    stkid: &str,
    qty: f64,
    direction: f64,
    tran_id: &str,
    tran_detail_id: &str,
) -> (f64, bool) {
    // 默认用当前月份
    // P3-27 修复：同 upsert_stock_ym，fallback 改为 UTC 年月（不再写死 202501）
    let ym: i32 = chrono::Local::now()
        .format("%Y%m")
        .to_string()
        .parse()
        .unwrap_or_else(|_| {
            let utc_ym = chrono::Utc::now().format("%Y%m").to_string();
            tracing::warn!("本地时间月份解析失败，回退到 UTC 年月: {}", utc_ym);
            utc_ym.parse().unwrap_or(202601)
        });
    post_ledger_with_period(
        conn,
        gdsid,
        stkid,
        qty,
        direction,
        tran_id,
        tran_detail_id,
        ym,
    )
    .await
}

/// 按指定会计月份过账（支持按单据日期所在月结存）
pub async fn post_ledger_with_period(
    conn: &mut Conn,
    gdsid: &str,
    stkid: &str,
    qty: f64,
    direction: f64,
    tran_id: &str,
    tran_detail_id: &str,
    ym: i32,
) -> (f64, bool) {
    tracing::debug!(
        "[post_ledger] 入参: gdsid={} stkid={} qty={} direction={} tran_id={} did={} ym={}",
        gdsid,
        stkid,
        qty,
        direction,
        tran_id,
        tran_detail_id,
        ym
    );
    if gdsid.is_empty() || stkid.is_empty() || qty == 0.0 {
        tracing::debug!("[post_ledger] 跳过: 空值或 qty=0");
        return (0.0, true);
    }
    let delta = direction * qty;
    tracing::debug!("[post_ledger] delta={}", delta);
    if delta < 0.0 {
        // 预检查仅用于快速失败与友好提示；并发安全的权威校验
        // 在 apply_stock_delta 的条件 UPDATE 谓词中完成（P0 防超卖修复）
        let cur = query_stock_qty(conn, gdsid, stkid).await;
        tracing::debug!(
            "[post_ledger] 出库校验: 当前库存={} 需求={} (cur+delta={})",
            cur,
            qty,
            cur + delta
        );
        if cur + delta < -0.0001 {
            tracing::warn!(
                "[post_ledger] 库存不足，返回 false: gdsid={} stkid={} cur={} need={}",
                gdsid,
                stkid,
                cur,
                qty
            );
            return (cur, false);
        }
    }
    let new_qty = apply_stock_delta(conn, gdsid, stkid, delta).await;
    tracing::debug!("[post_ledger] apply_stock_delta 返回 new_qty={}", new_qty);
    if new_qty < -0.5 {
        tracing::error!(
            "[post_ledger] new_qty < -0.5，返回 false: gdsid={} stkid={} delta={}",
            gdsid,
            stkid,
            delta
        );
        return (0.0, false);
    }
    let in_qty = if delta > 0.0 { delta } else { 0.0 };
    let out_qty = if delta < 0.0 { -delta } else { 0.0 };
    if !insert_stock_tran_his(
        conn,
        gdsid,
        stkid,
        tran_id,
        tran_detail_id,
        in_qty,
        out_qty,
        new_qty,
    )
    .await
    {
        tracing::error!(
            "[post_ledger] insert_stock_tran_his 失败: gdsid={} stkid={} tran_id={}",
            gdsid,
            stkid,
            tran_id
        );
        return (new_qty, false);
    }
    if !upsert_stock_ym_with_period(conn, gdsid, stkid, in_qty, out_qty, ym).await {
        tracing::error!(
            "[post_ledger] upsert_stock_ym_with_period 失败: gdsid={} stkid={} ym={}",
            gdsid,
            stkid,
            ym
        );
        return (new_qty, false);
    }
    if !upsert_stock_qty_snapshot(conn, gdsid, stkid).await {
        tracing::error!(
            "[post_ledger] upsert_stock_qty_snapshot 失败: gdsid={} stkid={}",
            gdsid,
            stkid
        );
        return (new_qty, false);
    }
    tracing::debug!("[post_ledger] 成功: new_qty={}", new_qty);
    (new_qty, true)
}

/// 反审时"反过账"：方向相反
pub async fn post_ledger_reverse(
    conn: &mut Conn,
    gdsid: &str,
    stkid: &str,
    qty: f64,
    direction: f64,
    tran_id: &str,
    tran_detail_id: &str,
) -> bool {
    post_ledger(conn, gdsid, stkid, qty, -direction, tran_id, tran_detail_id)
        .await
        .1
}

/// 反审专用：只反向调整 Qty + StockYM + 快照，不写 TranHis
///
/// 反审时先调用本函数反向调整库存，再调用 delete_stock_tran_his 删除原流水。
/// 避免追加式流水表先 INSERT 再 DELETE 的浪费。
pub async fn reverse_stock_delta_only(
    conn: &mut Conn,
    gdsid: &str,
    stkid: &str,
    qty: f64,
    direction: f64,
    ym: i32,
) -> bool {
    if gdsid.is_empty() || stkid.is_empty() || qty == 0.0 {
        return true;
    }
    // 反向 delta = -direction * qty
    let delta = -direction * qty;
    let new_qty = apply_stock_delta(conn, gdsid, stkid, delta).await;
    if new_qty < -0.5 {
        return false;
    }
    // 反向 StockYM：原入库变出库，原出库变入库
    let in_qty = if delta > 0.0 { delta } else { 0.0 };
    let out_qty = if delta < 0.0 { -delta } else { 0.0 };
    upsert_stock_ym_with_period(conn, gdsid, stkid, in_qty, out_qty, ym).await;
    upsert_stock_qty_snapshot(conn, gdsid, stkid).await;
    true
}

/// 月结：把指定月份所有 (StkID, GDSID) 的 EndQty 作为下月 InitQty
///
/// 返回值：>=0 表示成功写入的行数；-1 表示执行失败（详见 tracing 日志）
pub async fn month_end_settle(conn: &mut Conn, from_ym: i32, to_ym: i32) -> i32 {
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
    let params: Vec<&dyn ToSql> = vec![&to_ym, &from_ym];
    match conn.execute(sql, &params).await {
        Ok(r) => r.rows_affected().iter().sum::<u64>() as i32,
        Err(e) => {
            tracing::error!(
                "[month_end_settle] 月结失败: from_ym={}, to_ym={}, err={}",
                from_ym,
                to_ym,
                e
            );
            -1
        }
    }
}

/// 回填详情表的库存快照 StkQty/AQty
pub async fn fill_detail_stock_snapshot(
    conn: &mut Conn,
    detail_table: &str,
    detail_pk: &str,
    detail_id: &str,
) {
    if detail_id.is_empty() {
        return;
    }
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
    }
    if gdsid.is_empty() || stkid.is_empty() {
        return;
    }
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
    let upd = format!(
        "UPDATE [{}] SET StkQty = @p1, AQty = @p2 WHERE {} = @p3",
        detail_table, detail_pk
    );
    let params: Vec<&dyn ToSql> = vec![&qty, &qqty, &detail_id];
    let _ = conn.execute(&upd, &params).await;
}

/// 批量回填 tStk_IODetail 的 StkQty/AQty
pub async fn fill_io_detail_stock_snapshot(conn: &mut Conn, ioid: &str) {
    if ioid.is_empty() {
        return;
    }
    let sql = "UPDATE d SET d.StkQty = ISNULL(s.Qty, 0), d.AQty = ISNULL(s.QQty, 0) \
               FROM tStk_IODetail d \
               LEFT JOIN tStk_Stock s ON s.GDSID = d.GDSID AND s.StkID = d.StkID \
               WHERE d.IOID = @p1";
    let params: Vec<&dyn ToSql> = vec![&ioid];
    let _ = conn.execute(sql, &params).await;
}

/// 批量回填 tStk_MoveDetail 的 StkQty/AQty
pub async fn fill_move_detail_stock_snapshot(conn: &mut Conn, move_id: &str) {
    if move_id.is_empty() {
        return;
    }
    let sql = "UPDATE d SET d.StkQty = ISNULL(s.Qty, 0), d.AQty = ISNULL(s.QQty, 0) \
               FROM tStk_MoveDetail d \
               LEFT JOIN tStk_Stock s ON s.GDSID = d.GDSID AND s.StkID = d.StkID \
               WHERE d.MoveID = @p1";
    let params: Vec<&dyn ToSql> = vec![&move_id];
    let _ = conn.execute(sql, &params).await;
}

/// 批量回填 tStk_TranDetail 的 StkQty/AQty
pub async fn fill_tran_detail_stock_snapshot(conn: &mut Conn, tran_id: &str) {
    if tran_id.is_empty() {
        return;
    }
    // tStk_TranDetail 实际列为 AccQty(账存) / RealQty(实存) / DiffQty(差异)
    // 账存 = 当前库存数量 (tStk_Stock.Qty)
    let sql = "UPDATE d SET d.AccQty = ISNULL(s.Qty, 0) \
               FROM tStk_TranDetail d \
               LEFT JOIN tStk_Stock s ON s.GDSID = d.GDSID AND s.StkID = d.StkID \
               WHERE d.TranID = @p1";
    let params: Vec<&dyn ToSql> = vec![&tran_id];
    let _ = conn.execute(sql, &params).await;
}

/// 读单据当前 State
pub async fn query_doc_state(conn: &mut Conn, table: &str, primary_key: &str, id: &str) -> String {
    if id.is_empty() {
        return String::new();
    }
    let sql = format!(
        "SELECT ISNULL(State,'') AS S FROM [{}] WHERE [{}] = @p1",
        table, primary_key
    );
    let params: Vec<&dyn ToSql> = vec![&id];
    if let Ok(stream) = conn.query(&sql, &params).await {
        if let Ok(Some(row)) = stream.into_row().await {
            return row.get::<&str, _>("S").unwrap_or("").to_string();
        }
    }
    String::new()
}

/// 更新单据 State
///
/// `expect_states`：期望的当前状态集合（CAS 前置条件）。
/// - 传 Some([...]) 时，UPDATE 会带 `AND State IN (...)`，避免 TOCTOU 竞态
///   （例如审核流程已校验 State=N/E，但 UPDATE 之前被其他请求改为 D，无 CAS 会成功覆盖）。
/// - 传 None 时退化为无条件 UPDATE（保持原有行为）。
///
/// 返回值：true 表示实际更新了 1 行（前置条件满足）；false 表示 0 行（状态不匹配或失败）。
pub async fn update_doc_state(
    conn: &mut Conn,
    table: &str,
    primary_key: &str,
    id: &str,
    new_state: &str,
    user: &str,
) -> bool {
    update_doc_state_with_cas(conn, table, primary_key, id, new_state, user, None).await
}

/// 带 CAS 前置条件的 update_doc_state
pub async fn update_doc_state_with_cas(
    conn: &mut Conn,
    table: &str,
    primary_key: &str,
    id: &str,
    new_state: &str,
    user: &str,
    expect_states: Option<&[&str]>,
) -> bool {
    if id.is_empty() {
        return false;
    }
    let now = chrono::Local::now().naive_local();
    // 构造 SQL：带可选的状态前置条件（CAS）
    let sql = if let Some(states) = expect_states {
        if states.is_empty() {
            format!(
                "UPDATE [{}] SET State = @p1, AUser = @p2, ADate = @p3 WHERE [{}] = @p4",
                table, primary_key
            )
        } else {
            // 构造 IN 子句占位符：@p5, @p6, ...
            let placeholders: Vec<String> =
                (0..states.len()).map(|i| format!("@p{}", i + 5)).collect();
            format!(
                "UPDATE [{}] SET State = @p1, AUser = @p2, ADate = @p3 WHERE [{}] = @p4 AND State IN ({})",
                table,
                primary_key,
                placeholders.join(", ")
            )
        }
    } else {
        format!(
            "UPDATE [{}] SET State = @p1, AUser = @p2, ADate = @p3 WHERE [{}] = @p4",
            table, primary_key
        )
    };
    // 构造参数
    let mut params: Vec<&dyn ToSql> = vec![&new_state, &user, &now, &id];
    let mut extra: Vec<&str> = Vec::new();
    if let Some(states) = expect_states {
        for s in states {
            extra.push(*s);
        }
        for s in &extra {
            params.push(s);
        }
    }
    match conn.execute(&sql, &params).await {
        Ok(rs) => {
            // tiberius execute 返回的行数（SQL Server 的 @@ROWCOUNT）
            // 0 行表示前置条件不满足（状态已被其他请求改掉）
            rs.total() > 0
        }
        Err(_) => false,
    }
}

/// 写操作日志（同时写 tSys_OperLog 结构化表 + tSys_OperHis 旧表）
///
/// tSys_OperLog 是结构化审计日志表，/system/log/list 优先读此表；
/// tSys_OperHis 是旧表（OpenMsg 管道分隔格式），保留写入以兼容 vSys_OperHis 视图。
///
/// OperType 取值集合：CREATE / UPDATE / DELETE / APPROVE / UNAPPROVE / VOID / PRINT / POST / EXPORT / IMPORT / LOGIN / LOGOUT / PWD
///
/// before_data / after_data：修改前后的完整数据 JSON（用于详情弹窗显示变更明细）
pub async fn record_oper(
    conn: &mut Conn,
    oper_type: &str,
    table_name: &str,
    key_value: &str,
    user_code: &str,
    doc_no: Option<&str>,
    remark: Option<&str>,
) {
    record_oper_with_data(
        conn, oper_type, table_name, key_value, user_code, doc_no, remark, None, None,
    )
    .await
}

/// 带数据快照的 record_oper（before_data/after_data 为 JSON 字符串）
/// 只写 tSys_OperLog（结构化表）；写入失败时 fallback 到 tSys_OperHis（旧表）
pub async fn record_oper_with_data(
    conn: &mut Conn,
    oper_type: &str,
    table_name: &str,
    key_value: &str,
    user_code: &str,
    doc_no: Option<&str>,
    remark: Option<&str>,
    before_data: Option<&str>,
    after_data: Option<&str>,
) {
    let now = chrono::Local::now().naive_local();
    let doc_no_owned = doc_no.unwrap_or("").to_string();
    let remark_owned = remark.unwrap_or("").to_string();
    let zero_uuid = "00000000-0000-0000-0000-000000000000".to_string();

    // 查操作人 EmpID + EmpName
    // emp_id_opt: None 表示查不到（写入 NULL），Some(uuid) 表示查到（写入有效 UUID）
    // 这样可避免把零 UUID 字符串传给 uniqueidentifier 列导致的类型转换问题
    let (emp_id_opt, emp_name) = lookup_emp_by_code(conn, user_code).await;

    // --- 1) 写 tSys_OperLog（结构化表，/system/log/list 优先读此表）---
    // Remark 拼接：备注 + 单据号
    let mut log_remark_parts: Vec<String> = Vec::new();
    if !remark_owned.is_empty() {
        log_remark_parts.push(remark_owned.clone());
    }
    if !doc_no_owned.is_empty() {
        log_remark_parts.push(format!("单据号:{}", doc_no_owned));
    }
    let log_remark = log_remark_parts.join(" ");
    // EmpID 是 uniqueidentifier，必须用 Option<&str>（None=NULL，Some=有效 UUID 字符串）
    // 空字符串或非 UUID 字符串会导致类型转换失败
    let emp_id_param: Option<&str> = emp_id_opt.as_deref();
    let before_val: Option<&str> = before_data.filter(|s| !s.is_empty());
    let after_val: Option<&str> = after_data.filter(|s| !s.is_empty());
    let sql_new = "INSERT INTO tSys_OperLog \
                   (OperLogID, OperType, TableName, KeyValue, UserCode, EmpID, UserName, ClientIP, OperDate, Remark, OldData, NewData) \
                   VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, '', @p7, @p8, @p9, @p10)";
    let p_new: Vec<&dyn ToSql> = vec![
        &oper_type,
        &table_name,
        &key_value,
        &user_code,
        &emp_id_param,
        &emp_name,
        &now,
        &log_remark,
        &before_val,
        &after_val,
    ];
    if let Err(e) = conn.execute(sql_new, &p_new).await {
        tracing::error!(
            "[record_oper] 写入 tSys_OperLog 失败: table={}, key={}, user_code={}, emp_id={:?}, err={}",
            table_name,
            key_value,
            user_code,
            emp_id_param,
            e
        );
        // --- 2) 失败时 fallback 写 tSys_OperHis（旧表，OpenMsg 管道格式）---
        let mut parts: Vec<String> = vec![oper_type.to_string(), table_name.to_string()];
        if !doc_no_owned.is_empty() {
            parts.push(doc_no_owned.clone());
        }
        if !remark_owned.is_empty() {
            parts.push(remark_owned.clone());
        }
        if !user_code.is_empty() {
            parts.push(format!("操作人:{}", user_code));
        }
        let open_msg = parts.join(" | ");
        let doc_uuid = if is_valid_uuid(key_value) {
            key_value.to_string()
        } else {
            zero_uuid.clone()
        };
        let sql_old = "INSERT INTO tSys_OperHis (OperHisID, DocID, EmpID, MenusID, OperDate, OpenMsg) \
                       VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5)";
        let p_old: Vec<&dyn ToSql> = vec![&doc_uuid, &emp_id_param, &zero_uuid, &now, &open_msg];
        if let Err(e2) = conn.execute(sql_old, &p_old).await {
            tracing::error!(
                "[record_oper] fallback 写入 tSys_OperHis 也失败: table={}, key={}, err={}",
                table_name,
                key_value,
                e2
            );
        }
    }
}

/// 判断字符串是否为合法 UUID 格式（用于 uniqueidentifier 列写入前校验）
fn is_valid_uuid(s: &str) -> bool {
    s.len() == 36 && s.chars().filter(|c| *c == '-').count() == 4
}

/// 根据工号查 EmpID + EmpName，供 record_oper 写入操作人信息
/// 返回 (Option<EmpID>, EmpName)：
///   - None 表示查不到（写入时为 NULL，避免零 UUID 污染关联查询）
///   - Some(uuid) 表示查到有效 EmpID
async fn lookup_emp_by_code(conn: &mut Conn, user_code: &str) -> (Option<String>, String) {
    if user_code.is_empty() {
        return (None, String::new());
    }
    // 如果已经是 UUID 格式，直接返回（无 EmpName）
    if is_valid_uuid(user_code) {
        return (Some(user_code.to_string()), String::new());
    }
    // 按工号查 tBas_Emp
    let sql = "SELECT TOP 1 CAST(EmpID AS NVARCHAR(36)) AS EmpID, EmpName FROM tBas_Emp WHERE EmpNo = @p1";
    match conn.query(sql, &[&user_code]).await {
        Ok(stream) => {
            match stream.into_first_result().await {
                Ok(rows) => {
                    if let Some(row) = rows.into_iter().next() {
                        let emp_id = row
                            .try_get::<&str, _>("EmpID")
                            .ok()
                            .flatten()
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty() && is_valid_uuid(s));
                        let emp_name = row
                            .try_get::<&str, _>("EmpName")
                            .ok()
                            .flatten()
                            .map(|s| s.trim().to_string())
                            .unwrap_or_default();
                        return (emp_id, emp_name);
                    }
                    // 未找到员工记录
                    tracing::warn!(
                        "[lookup_emp_by_code] 未找到员工记录: user_code={}",
                        user_code
                    );
                }
                Err(e) => {
                    tracing::error!(
                        "[lookup_emp_by_code] into_first_result 失败: user_code={}, err={}",
                        user_code,
                        e
                    );
                }
            }
        }
        Err(e) => {
            tracing::error!(
                "[lookup_emp_by_code] 查询 tBas_Emp 失败: user_code={}, err={}",
                user_code,
                e
            );
        }
    }
    (None, String::new())
}

/// 批量回填 tStk_OrderDetail / tStk_Pur_OrderDetail 等的 StkQty/AQty
pub async fn fill_order_detail_stock_snapshot(conn: &mut Conn, table: &str, master_id: &str) {
    if master_id.is_empty() {
        return;
    }
    let sql = format!(
        "UPDATE d SET d.StkQty = ISNULL(s.Qty, 0), d.AQty = ISNULL(s.QQty, 0) \
         FROM [{}] d \
         LEFT JOIN tStk_Stock s ON s.GDSID = d.GDSID AND s.StkID = d.StkID \
         WHERE d.GDSID IS NOT NULL",
        table
    );
    let _ = conn.execute(&sql, &[&master_id]).await;
}

/// 反审前置检查：会计期间是否已结账
/// DB 规则：月初 month_end_settle 后该月视为"已结账"，禁止反审
pub async fn check_period_closed(
    conn: &mut Conn,
    action_date: chrono::NaiveDate,
) -> Option<String> {
    let ym: i32 = action_date
        .format("%Y%m")
        .to_string()
        .parse()
        .unwrap_or(202501);
    let next_ym: i32 = if ym % 100 == 12 {
        (ym / 100 + 1) * 100 + 1
    } else {
        ym + 1
    };
    let sql =
        "SELECT TOP 1 ISNULL(InitQty, 0) AS IQ FROM tStk_StockYM WHERE AccYM = @p1 AND InitQty > 0";
    let row = match conn.query(sql, &[&next_ym]).await {
        Ok(s) => match s.into_row().await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(
                    "[check_period_closed] into_row 失败: next_ym={}, err={}",
                    next_ym,
                    e
                );
                return None;
            }
        },
        Err(e) => {
            tracing::error!(
                "[check_period_closed] 查询 tStk_StockYM 失败: next_ym={}, err={}",
                next_ym,
                e
            );
            return None;
        }
    };
    if let Some(r) = row {
        let init_qty = row_get_f64(&r, "IQ");
        if init_qty > 0.0 {
            return Some(format!("会计期间 {} 已月结账，无法反审核", ym));
        }
    }
    None
}

/// 事务辅助：开启事务
///
/// SQL Server 会在批处理/存储过程退出时检查 @@TRANCOUNT 是否与进入时一致，
/// 如果不一致就抛出错误 266 "EXECUTE 后的事务计数指示 BEGIN 和 COMMIT 语句的数目不匹配"。
///
/// 无论是 `execute`（包装在 sp_executesql 中）还是 `simple_query`（SQLBatch），
/// 只要 `BEGIN TRAN` 让 @@TRANCOUNT 从 0 变 1，退出当前作用域时就会触发 266。
///
/// **关键**：错误 266 是"警告性质"的错误 — 事务确实已经启动，只是作用域退出时计数不匹配。
/// SQL Server 不会因为 266 回滚事务。所以我们需要忽略 266，事务是有效的。
pub async fn begin_tran(conn: &mut Conn) -> Result<(), String> {
    match conn.simple_query("BEGIN TRAN").await {
        Ok(stream) => {
            // ★ 显式检查 into_first_result() 的错误，不能再用 `let _ =`
            // 之前 bug：let _ = 丢弃了 266 错误，但错误 token 仍残留在连接状态中，
            // 后续 SQL 操作时才被传播出来（表现为"操作失败"）
            match stream.into_first_result().await {
                Ok(_) => Ok(()),
                Err(e) => {
                    if is_tran_count_error(&e) {
                        Ok(())
                    } else {
                        Err(format!("开启事务失败: {}", e))
                    }
                }
            }
        }
        Err(e) => {
            // 检查是否为错误 266（事务计数不匹配）— 这是 false alarm，事务已启动
            if is_tran_count_error(&e) {
                Ok(())
            } else {
                Err(format!("开启事务失败: {}", e))
            }
        }
    }
}

/// 事务辅助：提交事务
pub async fn commit_tran(conn: &mut Conn) -> Result<(), String> {
    match conn.simple_query("COMMIT TRAN").await {
        Ok(stream) => {
            // ★ 显式检查 into_first_result() 的错误
            match stream.into_first_result().await {
                Ok(_) => Ok(()),
                Err(e) => {
                    if is_tran_count_error(&e) {
                        Ok(())
                    } else {
                        Err(format!("提交事务失败: {}", e))
                    }
                }
            }
        }
        Err(e) => {
            if is_tran_count_error(&e) {
                Ok(())
            } else {
                Err(format!("提交事务失败: {}", e))
            }
        }
    }
}

/// 事务辅助：回滚事务
pub async fn rollback_tran(conn: &mut Conn) {
    if let Ok(stream) = conn.simple_query("ROLLBACK TRAN").await {
        // ★ 回滚时也显式消费 stream 的结果，避免错误 token 残留
        let _ = stream.into_first_result().await;
    }
    // 回滚时忽略所有错误（包括 266），因为无论怎样事务都会被清理
}

/// 判断 tiberius 错误是否为 SQL Server 错误 266（事务计数不匹配）。
/// 错误 266 是 BEGIN/COMMIT TRAN 在 sp_executesql 或 SQLBatch 作用域退出时的 false alarm，
/// 事务实际上已经生效。
fn is_tran_count_error(e: &tiberius::error::Error) -> bool {
    use tiberius::error::Error;
    // 先检查 Server error 的 code（最准确）
    if let Error::Server(te) = e {
        if te.code() == 266 {
            return true;
        }
    }
    // ★ fallback: 字符串匹配（覆盖所有错误类型，包括 Token error 包装的 266）
    // 之前 bug：match 分支匹配 Error::Server 后不再进入 _ 分支，导致 code 不匹配时返回 false
    let msg = format!("{}", e);
    msg.contains("266") || msg.contains("事务计数") || msg.contains("transaction count")
}
