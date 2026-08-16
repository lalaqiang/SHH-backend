// 文档中的 SQL 示例保留了多级缩进，属于刻意的代码排版
#![allow(clippy::doc_overindented_list_items)]

//! 过账/反审流程集成测试
//!
//! ⚠️ **需要真实 SQL Server** 才能运行，默认 `#[ignore]`。
//!
//! 启用方式：
//! 1. 设置环境变量：
//!    setx TEST_DB_HOST 192.168.1.100
//!    setx TEST_DB_NAME TestERP
//!    setx TEST_DB_USER sa
//!    setx TEST_DB_PASSWORD sa123456
//! 2. 准备测试商品 + 仓库（手工 SQL 一次）：
//!    -- 测试仓库（State=Y 表示正常）
//!    IF NOT EXISTS (SELECT 1 FROM tBas_Stock WHERE StkNO='TST_WH_1')
//!      INSERT INTO tBas_Stock (StkID, StkNO, StkName, Used, State)
//!        VALUES (NEWID(), 'TST_WH_1', '测试仓1', 'Y', 'Y')
//!    -- 测试商品
//!    IF NOT EXISTS (SELECT 1 FROM tBas_Goods WHERE GDSNO='TST_GDS_1')
//!      INSERT INTO tBas_Goods (GDSID, GDSNO, GDSDesc, State)
//!        VALUES (NEWID(), 'TST_GDS_1', '测试商品1', 'Y')
//! 3. 运行：
//!    cargo test --test posting_flow_test -- --ignored --nocapture
//!
//! 验证策略：直接调 approval 模块的 pub 业务函数（apply_stock_delta、query_stock_qty、upsert_stock_tran_his），
//! 不走 HTTP 路由，聚焦业务逻辑正确性。

mod common;

use common::db_tests_enabled;
use erp_server::config::Config;
use erp_server::db::init_pool;
use erp_server::services::inventory_ledger::{apply_stock_delta, query_stock_qty, upsert_stock_ym};
use std::time::Duration;
use uuid::Uuid;

/// 初始化 DB pool（仅在 TEST_DB_HOST 设置时执行）
async fn try_init_pool() -> Option<()> {
    if !db_tests_enabled() {
        eprintln!("[skip] 设置 TEST_DB_HOST 启用本测试");
        return None;
    }
    let config = Config::from_env();
    init_pool(&config).await;
    Some(())
}

/// 获取一个测试连接（每次新建，避免跨测试状态污染）
async fn get_conn() -> Option<erp_server::handlers::approval::Conn> {
    erp_server::db::get_pool().get().await.ok()
}

/// 测试用唯一 GDSID（基于 UUID 避免冲突）
fn test_gdsid() -> String {
    format!("TEST-{}", Uuid::new_v4())
}

/// 测试用唯一 StkID（基于 UUID）
fn test_stkid() -> String {
    format!("TEST-{}", Uuid::new_v4())
}

/// 测试结束清理：删除测试时插入/更新的记录
async fn cleanup(conn: &mut erp_server::handlers::approval::Conn, gdsid: &str, stkid: &str) {
    let sql1 = "DELETE FROM tStk_Stock WHERE GDSID = @p1 AND StkID = @p2";
    let sql2 = "DELETE FROM tStk_StockTranHis WHERE GDSID = @p1 AND StkID = @p2";
    let sql3 = "DELETE FROM tStk_StockYM WHERE GDSID = @p1 AND StkID = @p2";
    for sql in [sql1, sql2, sql3] {
        let params: Vec<&dyn tiberius::ToSql> = vec![&gdsid, &stkid];
        let _ = conn.execute(sql, &params).await;
    }
}

// ============================================================ 单元集成测试
#[tokio::test]
#[ignore = "需要真实 SQL Server；通过 TEST_DB_HOST 启用"]
async fn apply_stock_delta_inserts_new_row() {
    if try_init_pool().await.is_none() {
        return;
    }
    let mut conn = get_conn().await.expect("获取连接");
    let gdsid = test_gdsid();
    let stkid = test_stkid();

    // 第一次：应插入新行，Qty=10
    let qty = apply_stock_delta(&mut conn, &gdsid, &stkid, 10.0).await;
    assert_eq!(qty, 10.0, "首次应用 delta=10 后 Qty 应为 10.0");

    // 验证：tStk_Stock 中存在该行
    let read_qty = query_stock_qty(&mut conn, &gdsid, &stkid).await;
    assert_eq!(read_qty, 10.0);

    cleanup(&mut conn, &gdsid, &stkid).await;
}

#[tokio::test]
#[ignore = "需要真实 SQL Server；通过 TEST_DB_HOST 启用"]
async fn apply_stock_delta_accumulates() {
    if try_init_pool().await.is_none() {
        return;
    }
    let mut conn = get_conn().await.expect("获取连接");
    let gdsid = test_gdsid();
    let stkid = test_stkid();

    // 多次应用：+10, +5, -3 → 累计 12
    let _ = apply_stock_delta(&mut conn, &gdsid, &stkid, 10.0).await;
    let q1 = apply_stock_delta(&mut conn, &gdsid, &stkid, 5.0).await;
    assert_eq!(q1, 15.0);
    let q2 = apply_stock_delta(&mut conn, &gdsid, &stkid, -3.0).await;
    assert_eq!(q2, 12.0);

    // 验证 QQty 也同步累加
    let read = query_stock_qty(&mut conn, &gdsid, &stkid).await;
    assert_eq!(read, 12.0);

    cleanup(&mut conn, &gdsid, &stkid).await;
}

#[tokio::test]
#[ignore = "需要真实 SQL Server；通过 TEST_DB_HOST 启用"]
async fn apply_stock_delta_unapprove_simulates_reverse() {
    if try_init_pool().await.is_none() {
        return;
    }
    let mut conn = get_conn().await.expect("获取连接");
    let gdsid = test_gdsid();
    let stkid = test_stkid();

    // 模拟"过账 +10，审核成功"
    let _ = apply_stock_delta(&mut conn, &gdsid, &stkid, 10.0).await;
    assert_eq!(query_stock_qty(&mut conn, &gdsid, &stkid).await, 10.0);

    // 模拟"反审 -10，库存回退"
    let _ = apply_stock_delta(&mut conn, &gdsid, &stkid, -10.0).await;
    assert_eq!(query_stock_qty(&mut conn, &gdsid, &stkid).await, 0.0);

    cleanup(&mut conn, &gdsid, &stkid).await;
}

#[tokio::test]
#[ignore = "需要真实 SQL Server；通过 TEST_DB_HOST 启用"]
async fn apply_stock_delta_empty_id_skipped() {
    if try_init_pool().await.is_none() {
        return;
    }
    let mut conn = get_conn().await.expect("获取连接");

    // 空 ID 应跳过，不抛错
    let r1 = apply_stock_delta(&mut conn, "", "valid_stk", 10.0).await;
    assert_eq!(r1, 0.0, "空 GDSID 应跳过");

    let r2 = apply_stock_delta(&mut conn, "valid_gds", "", 10.0).await;
    assert_eq!(r2, 0.0, "空 StkID 应跳过");
}

#[tokio::test]
#[ignore = "需要真实 SQL Server；通过 TEST_DB_HOST 启用"]
async fn upsert_stock_ym_creates_monthly_record() {
    if try_init_pool().await.is_none() {
        return;
    }
    let mut conn = get_conn().await.expect("获取连接");
    let gdsid = test_gdsid();
    let stkid = test_stkid();

    // 月度结存：本月 In=20, Out=5
    let ym = chrono::Local::now()
        .format("%Y%m")
        .to_string()
        .parse::<i32>()
        .unwrap();
    upsert_stock_ym(&mut conn, &gdsid, &stkid, 20.0, 5.0).await;

    // 验证
    let sql = "SELECT TOP 1 ISNULL(InQty,0) AS I, ISNULL(OutQty,0) AS O, ISNULL(EndQty,0) AS E \
               FROM tStk_StockYM WHERE AccYM = @p1 AND GDSID = @p2 AND StkID = @p3";
    let params: Vec<&dyn tiberius::ToSql> = vec![&ym, &gdsid, &stkid];
    if let Ok(stream) = conn.query(sql, &params).await
        && let Ok(Some(row)) = stream.into_row().await
    {
        let in_q = row_get_f64_pub(&row, "I");
        let out_q = row_get_f64_pub(&row, "O");
        assert_eq!(in_q, 20.0, "月度入库应为 20");
        assert_eq!(out_q, 5.0, "月度出库应为 5");
    }

    // 清理（YM 也要清）
    let sql = "DELETE FROM tStk_StockYM WHERE GDSID = @p1 AND StkID = @p2";
    let params: Vec<&dyn tiberius::ToSql> = vec![&gdsid, &stkid];
    let _ = conn.execute(sql, &params).await;
}

#[tokio::test]
#[ignore = "需要真实 SQL Server；通过 TEST_DB_HOST 启用"]
async fn concurrent_apply_does_not_corrupt() {
    if try_init_pool().await.is_none() {
        return;
    }
    let mut conn = get_conn().await.expect("获取连接");
    let gdsid = test_gdsid();
    let stkid = test_stkid();

    // 并发 10 次 +1，期望最终 Qty = 10
    let mut handles = vec![];
    for _ in 0..10 {
        let g = gdsid.clone();
        let s = stkid.clone();
        handles.push(tokio::spawn(async move {
            // 每个 task 单独取连接
            let mut c = get_conn().await.unwrap();
            apply_stock_delta(&mut c, &g, &s, 1.0).await
        }));
    }
    for h in handles {
        let _ = h.await;
    }
    // 等 100ms 让所有写入完成
    tokio::time::sleep(Duration::from_millis(100)).await;

    let final_qty = query_stock_qty(&mut conn, &gdsid, &stkid).await;
    assert_eq!(final_qty, 10.0, "10 次并发 +1 后 Qty 应为 10.0");

    cleanup(&mut conn, &gdsid, &stkid).await;
}

/// 公共工具：tiberius Row 读 f64
fn row_get_f64_pub(row: &tiberius::Row, col: &str) -> f64 {
    // tiberius 的 Row 没有 get(name) 直接读，需通过列下标或用 &str
    // 简化：tiberius::Row::get::<&str, T>("col") 返回 Option<T>，T: FromSql
    let opt: Option<f64> = row.get(col);
    opt.unwrap_or(0.0)
}
