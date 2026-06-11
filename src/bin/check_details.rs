use tiberius::{Config, AuthMethod, EncryptionLevel};
use bb8::Pool;
use bb8_tiberius::ConnectionManager;
use tokio;

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
    let pool = Pool::builder().max_size(2).build(manager).await?;
    let mut conn = pool.get().await?;

    // 查主表+明细表的对应关系
    let pairs = vec![
        ("tSal_Order", "tSal_OrderDetail", "SOID"),
        ("tSal_Quote", "tSal_QuoteDetail", "SQID"),
        ("tSal_AdjPrice", "tSal_AdjPriceDetail", "SAPID"),
        ("tPur_Order", "tPur_OrderDetail", "POID"),
        ("tPur_Quote", "tPur_QuoteDetail", "PQID"),
        ("tPur_AdjPrice", "tPur_AdjPriceDetail", "PAPID"),
        ("tStk_IO(SD)", "tStk_IODetail", "IOID"),
    ];

    for (master, detail, fk) in &pairs {
        // 主表总数
        let sql = if *master == "tStk_IO(SD)" {
            format!("SELECT COUNT(*) AS cnt FROM tStk_IO WHERE Kind = 'SD' AND State <> 'D'")
        } else {
            format!("SELECT COUNT(*) AS cnt FROM {}", master)
        };
        let stream = conn.query(&sql, &[]).await?;
        let rows = stream.into_first_result().await?;
        let mut master_cnt: i32 = 0;
        for r in &rows {
            master_cnt = r.get::<i32, _>("cnt").unwrap_or(0);
        }

        // 主表有明细关联的条数
        let sql2 = if *master == "tStk_IO(SD)" {
            format!(
                "SELECT COUNT(DISTINCT m.IOID) AS cnt FROM tStk_IO m INNER JOIN tStk_IODetail d ON m.IOID = d.IOID WHERE m.Kind = 'SD' AND m.State <> 'D'"
            )
        } else {
            format!(
                "SELECT COUNT(DISTINCT m.{}) AS cnt FROM {} m INNER JOIN {} d ON m.{} = d.{}",
                fk, master, detail, fk, fk
            )
        };
        let stream2 = conn.query(&sql2, &[]).await?;
        let rows2 = stream2.into_first_result().await?;
        let mut with_detail: i32 = 0;
        for r in &rows2 {
            with_detail = r.get::<i32, _>("cnt").unwrap_or(0);
        }

        println!(
            "{:<20} 主表={}  含明细={}  (fk={})",
            master, master_cnt, with_detail, fk
        );
    }

    Ok(())
}
