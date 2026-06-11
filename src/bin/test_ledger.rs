use tiberius::{AuthMethod, Config, EncryptionLevel};
use bb8::Pool;
use bb8_tiberius::ConnectionManager;
use uuid::Uuid;

type Conn<'a> = bb8::PooledConnection<'a, ConnectionManager>;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = Config::new();
    config.host("127.0.0.1");
    config.port(1433);
    config.database("TestERP");
    config.authentication(AuthMethod::sql_server("sa", "sa123456"));
    config.trust_cert();
    config.encryption(EncryptionLevel::NotSupported);

    let manager = ConnectionManager::new(config);
    let pool = Pool::builder().max_size(2).connection_timeout(std::time::Duration::from_secs(15)).build(manager).await?;
    let mut conn = pool.get().await?;

    let gdsid = "799C223D-F630-456B-AEC6-0014E0E52705";
    let stkid = "5BCE8811-DDD4-45B0-8821-E11C83F8BF54";
    let tran_id = Uuid::new_v4().to_string();
    let tran_detail_id = Uuid::new_v4().to_string();

    println!("=== 初始库存 ===");
    query_stock(&mut conn, gdsid, stkid).await;
    query_stock_tran_his(&mut conn, gdsid, stkid).await;
    query_stock_ym(&mut conn, gdsid, stkid).await;

    println!("\n=== 测试1: 入库 +5 (RI 入库) ===");
    post_ledger(&mut conn, gdsid, stkid, 5.0, 1.0, &tran_id, &tran_detail_id).await;
    query_stock(&mut conn, gdsid, stkid).await;
    query_stock_tran_his(&mut conn, gdsid, stkid).await;
    query_stock_ym(&mut conn, gdsid, stkid).await;

    println!("\n=== 测试2: 出库 -3 (SD 出库) ===");
    post_ledger(&mut conn, gdsid, stkid, 3.0, -1.0, &tran_id, &tran_detail_id).await;
    query_stock(&mut conn, gdsid, stkid).await;
    query_stock_tran_his(&mut conn, gdsid, stkid).await;
    query_stock_ym(&mut conn, gdsid, stkid).await;

    println!("\n=== 测试3: 调拨 -2 调出, +2 调入 (另一仓库) ===");
    let stkid2 = "417DF111-8F8C-4660-A0CE-D6B3E597ED5E";
    let init_sql = "IF NOT EXISTS (SELECT 1 FROM tStk_Stock WHERE GDSID=@p1 AND StkID=@p2) INSERT INTO tStk_Stock (GDSStockID, GDSID, StkID, Qty, QQty) VALUES (NEWID(), @p1, @p2, 0, 0)";
    let p: Vec<&dyn tiberius::ToSql> = vec![&gdsid, &stkid2];
    let _ = conn.execute(init_sql, &p).await;
    post_ledger(&mut conn, gdsid, stkid, 2.0, -1.0, &tran_id, &tran_detail_id).await;
    post_ledger(&mut conn, gdsid, stkid2, 2.0, 1.0, &tran_id, &tran_detail_id).await;
    query_stock(&mut conn, gdsid, stkid).await;
    query_stock(&mut conn, gdsid, stkid2).await;

    println!("\n=== 测试4: 回滚入库 -5 ===");
    post_ledger(&mut conn, gdsid, stkid, 5.0, -1.0, &tran_id, &tran_detail_id).await;
    query_stock(&mut conn, gdsid, stkid).await;

    println!("\n=== 测试5: 回滚出库 +3 ===");
    post_ledger(&mut conn, gdsid, stkid, 3.0, 1.0, &tran_id, &tran_detail_id).await;
    query_stock(&mut conn, gdsid, stkid).await;

    println!("\n=== 测试6: 回滚调拨 ===");
    post_ledger(&mut conn, gdsid, stkid, 2.0, 1.0, &tran_id, &tran_detail_id).await;
    post_ledger(&mut conn, gdsid, stkid2, 2.0, -1.0, &tran_id, &tran_detail_id).await;
    query_stock(&mut conn, gdsid, stkid).await;
    query_stock(&mut conn, gdsid, stkid2).await;

    println!("\n=== 测试7: 库存不足保护 ===");
    let result = post_ledger(&mut conn, gdsid, stkid, 999.0, -1.0, &tran_id, &tran_detail_id).await;
    println!("预期: false (库存不足)，实际: {}", result);

    Ok(())
}

async fn query_stock<'a>(conn: &mut Conn<'a>, gdsid: &str, stkid: &str) {
    let sql = "SELECT ISNULL(CAST(Qty AS NVARCHAR(50)),'0') AS Q, ISNULL(CAST(QQty AS NVARCHAR(50)),'0') AS Qq FROM tStk_Stock WHERE GDSID = @p1 AND StkID = @p2";
    let p: Vec<&dyn tiberius::ToSql> = vec![&gdsid, &stkid];
    if let Ok(stream) = conn.query(sql, &p).await {
        if let Ok(Some(row)) = stream.into_row().await {
            let q: &str = row.get::<&str, _>("Q").unwrap_or("0");
            let qq: &str = row.get::<&str, _>("Qq").unwrap_or("0");
            println!("  tStk_Stock[GDS={} Stk={}] Qty={} QQty={}", &gdsid[..8], &stkid[..8], q, qq);
            return;
        }
    }
    println!("  tStk_Stock[GDS={} Stk={}] 不存在", &gdsid[..8], &stkid[..8]);
}

async fn query_stock_tran_his<'a>(conn: &mut Conn<'a>, gdsid: &str, stkid: &str) {
    let sql = "SELECT ISNULL(CAST(InQty AS NVARCHAR(50)),'0') AS InQ, ISNULL(CAST(OutQty AS NVARCHAR(50)),'0') AS OutQ, ISNULL(CAST(EndQty AS NVARCHAR(50)),'0') AS EndQ, CAST(TranID AS NVARCHAR(40)) AS TID, CAST(TranDetailID AS NVARCHAR(40)) AS TDID FROM tStk_StockTranHis WHERE GDSID = @p1 AND StkID = @p2";
    let p: Vec<&dyn tiberius::ToSql> = vec![&gdsid, &stkid];
    if let Ok(stream) = conn.query(sql, &p).await {
        if let Ok(Some(row)) = stream.into_row().await {
            let inq: &str = row.get::<&str, _>("InQ").unwrap_or("0");
            let outq: &str = row.get::<&str, _>("OutQ").unwrap_or("0");
            let endq: &str = row.get::<&str, _>("EndQ").unwrap_or("0");
            let tid: &str = row.get::<&str, _>("TID").unwrap_or("");
            let tdid: &str = row.get::<&str, _>("TDID").unwrap_or("");
            println!("  tStk_StockTranHis[GDS={} Stk={}] InQty={} OutQty={} EndQty={} TranID={} TranDetailID={}", &gdsid[..8], &stkid[..8], inq, outq, endq, tid, tdid);
            return;
        }
    }
    println!("  tStk_StockTranHis[GDS={} Stk={}] 不存在", &gdsid[..8], &stkid[..8]);
}

async fn query_stock_ym<'a>(conn: &mut Conn<'a>, gdsid: &str, stkid: &str) {
    let sql = "SELECT AccYM, ISNULL(CAST(InitQty AS NVARCHAR(50)),'0') AS IQ, ISNULL(CAST(InQty AS NVARCHAR(50)),'0') AS InQ, ISNULL(CAST(OutQty AS NVARCHAR(50)),'0') AS OutQ, ISNULL(CAST(EndQty AS NVARCHAR(50)),'0') AS EndQ FROM tStk_StockYM WHERE StkID = @p1 AND GDSID = @p2";
    let p: Vec<&dyn tiberius::ToSql> = vec![&stkid, &gdsid];
    if let Ok(stream) = conn.query(sql, &p).await {
        if let Ok(rows) = stream.into_first_result().await {
            for row in &rows {
                let ym: i32 = row.get::<i32, _>("AccYM").unwrap_or(0);
                let iq: &str = row.get::<&str, _>("IQ").unwrap_or("0");
                let inq: &str = row.get::<&str, _>("InQ").unwrap_or("0");
                let outq: &str = row.get::<&str, _>("OutQ").unwrap_or("0");
                let endq: &str = row.get::<&str, _>("EndQ").unwrap_or("0");
                println!("  tStk_StockYM[YM={} GDS={} Stk={}] Init={} In={} Out={} End={}", ym, &gdsid[..8], &stkid[..8], iq, inq, outq, endq);
            }
            return;
        }
    }
    println!("  tStk_StockYM[GDS={} Stk={}] 不存在", &gdsid[..8], &stkid[..8]);
}

async fn post_ledger<'a>(
    conn: &mut Conn<'a>,
    gdsid: &str,
    stkid: &str,
    qty: f64,
    direction: f64,
    tran_id: &str,
    tran_detail_id: &str,
) -> bool {
    let delta = direction * qty;
    if gdsid.is_empty() || stkid.is_empty() || qty == 0.0 { return true; }
    if delta < 0.0 {
        let sql = "SELECT ISNULL(CAST(Qty AS NVARCHAR(50)),'0') AS Q FROM tStk_Stock WHERE GDSID = @p1 AND StkID = @p2";
        let p: Vec<&dyn tiberius::ToSql> = vec![&gdsid, &stkid];
        if let Ok(stream) = conn.query(sql, &p).await {
            if let Ok(Some(row)) = stream.into_row().await {
                let q_str: &str = row.get::<&str, _>("Q").unwrap_or("0");
                let cur: f64 = q_str.parse().unwrap_or(0.0);
                if cur + delta < -0.0001 {
                    println!("  [post_ledger] ✗ 库存不足: 现有{} 申请{}", cur, delta);
                    return false;
                }
            }
        }
    }
    let sql = r#"IF EXISTS (SELECT 1 FROM tStk_Stock WHERE GDSID = @p1 AND StkID = @p2)
                 UPDATE tStk_Stock SET Qty = ISNULL(Qty,0) + @p3, QQty = ISNULL(QQty,0) + @p3 WHERE GDSID = @p1 AND StkID = @p2
                 ELSE
                 INSERT INTO tStk_Stock (GDSStockID, GDSID, StkID, Qty, QQty) VALUES (NEWID(), @p1, @p2, @p3, @p3)"#;
    let p: Vec<&dyn tiberius::ToSql> = vec![&gdsid, &stkid, &delta];
    if let Err(e) = conn.execute(sql, &p).await {
        println!("  [post_ledger] ✗ tStk_Stock 写入失败: {:?}", e);
        return false;
    }
    let new_qty: f64 = {
        let sql = "SELECT ISNULL(CAST(Qty AS NVARCHAR(50)),'0') AS Q FROM tStk_Stock WHERE GDSID = @p1 AND StkID = @p2";
        let p: Vec<&dyn tiberius::ToSql> = vec![&gdsid, &stkid];
        let mut v = 0.0;
        if let Ok(stream) = conn.query(sql, &p).await {
            if let Ok(Some(row)) = stream.into_row().await {
                let q: &str = row.get::<&str, _>("Q").unwrap_or("0");
                v = q.parse().unwrap_or(0.0);
            }
        }
        v
    };
    let in_qty = if delta > 0.0 { delta } else { 0.0 };
    let out_qty = if delta < 0.0 { -delta } else { 0.0 };
    let prev_qty = new_qty - delta;
    let sql = r#"IF EXISTS (SELECT 1 FROM tStk_StockTranHis WHERE GDSID = @p1 AND StkID = @p2)
                 UPDATE tStk_StockTranHis
                 SET LastTranDate = GETDATE(), Qty = @p7, TranID = @p5, TranDetailID = @p6, InQty = @p3, OutQty = @p4, EndQty = @p7
                 WHERE GDSID = @p1 AND StkID = @p2
                 ELSE
                 INSERT INTO tStk_StockTranHis (GDSID, StkID, LastTranDate, Qty, TranID, TranDetailID, InQty, OutQty, EndQty)
                 VALUES (@p1, @p2, GETDATE(), @p7, @p5, @p6, @p3, @p4, @p7)"#;
    let p: Vec<&dyn tiberius::ToSql> = vec![
        &gdsid, &stkid,
        &in_qty, &out_qty,
        &tran_id, &tran_detail_id,
        &new_qty,
    ];
    let _ = conn.execute(sql, &p).await;
    let ym: i32 = chrono::Local::now().format("%Y%m").to_string().parse().unwrap_or(202501);
    let sql = r#"IF EXISTS (SELECT 1 FROM tStk_StockYM WHERE AccYM = @p1 AND StkID = @p2 AND GDSID = @p3)
                 UPDATE tStk_StockYM SET InQty = ISNULL(InQty,0) + @p4, OutQty = ISNULL(OutQty,0) + @p5, EndQty = ISNULL(EndQty,0) + @p6
                 WHERE AccYM = @p1 AND StkID = @p2 AND GDSID = @p3
                 ELSE
                 INSERT INTO tStk_StockYM (AccYM, StkID, GDSID, InitQty, InQty, OutQty, EndQty)
                 VALUES (@p1, @p2, @p3, 0, @p4, @p5, @p6)"#;
    let p: Vec<&dyn tiberius::ToSql> = vec![
        &ym, &stkid, &gdsid,
        &in_qty, &out_qty, &delta,
    ];
    let _ = conn.execute(sql, &p).await;
    println!("  [post_ledger] ✓ delta={}  {} → {}", delta, prev_qty, new_qty);
    true
}
