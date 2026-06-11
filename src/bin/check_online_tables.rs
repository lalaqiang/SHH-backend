use tiberius::{AuthMethod, Config, EncryptionLevel};
use bb8::Pool;
use bb8_tiberius::ConnectionManager;

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

    let find_sql = r#"
        SELECT TABLE_NAME
        FROM INFORMATION_SCHEMA.TABLES
        WHERE TABLE_TYPE = 'BASE TABLE'
          AND (
            TABLE_NAME LIKE '%Online%'
            OR TABLE_NAME LIKE '%online%'
            OR TABLE_NAME LIKE '%Shop%'
            OR TABLE_NAME LIKE '%shop%'
            OR TABLE_NAME LIKE '%Mobile%'
            OR TABLE_NAME LIKE '%mobile%'
            OR TABLE_NAME LIKE '%Address%'
            OR TABLE_NAME LIKE '%address%'
            OR TABLE_NAME LIKE '%Payment%'
            OR TABLE_NAME LIKE '%payment%'
            OR TABLE_NAME LIKE '%Region%'
            OR TABLE_NAME LIKE '%region%'
            OR TABLE_NAME LIKE '%Cart%'
            OR TABLE_NAME LIKE '%cart%'
            OR TABLE_NAME LIKE '%Order%'
            OR TABLE_NAME LIKE '%Wx%'
            OR TABLE_NAME LIKE '%wx%'
            OR TABLE_NAME LIKE '%WeChat%'
            OR TABLE_NAME LIKE '%wechat%'
            OR TABLE_NAME LIKE '%Mall%'
            OR TABLE_NAME LIKE '%mall%'
            OR TABLE_NAME LIKE '%App%'
            OR TABLE_NAME LIKE '%Delivery%'
            OR TABLE_NAME LIKE '%delivery%'
            OR TABLE_NAME LIKE '%Shipping%'
            OR TABLE_NAME LIKE '%shipping%'
            OR TABLE_NAME LIKE '%Freight%'
            OR TABLE_NAME LIKE '%freight%'
          )
        ORDER BY TABLE_NAME
    "#;

    let stream = conn.query(find_sql, &[]).await?;
    let rows = stream.into_first_result().await?;
    let mut table_names: Vec<String> = Vec::new();
    for r in &rows {
        let name: &str = r.get::<&str, _>("TABLE_NAME").unwrap_or("");
        table_names.push(name.to_string());
    }

    if table_names.is_empty() {
        println!("No matching tables found.");
        return Ok(());
    }

    println!("Found {} matching table(s):\n", table_names.len());
    for t in &table_names {
        println!("  {}", t);
    }
    println!();

    for table in &table_names {
        let col_sql = format!(
            "SELECT COLUMN_NAME, DATA_TYPE, CHARACTER_MAXIMUM_LENGTH, IS_NULLABLE FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_NAME = '{}' ORDER BY ORDINAL_POSITION",
            table
        );
        println!("===== {} =====", table);
        let col_stream = conn.query(&col_sql, &[]).await?;
        let col_rows = col_stream.into_first_result().await?;
        for cr in &col_rows {
            let col_name: &str = cr.get::<&str, _>("COLUMN_NAME").unwrap_or("");
            let data_type: &str = cr.get::<&str, _>("DATA_TYPE").unwrap_or("");
            let max_len: Option<i32> = cr.try_get::<i32, _>("CHARACTER_MAXIMUM_LENGTH").ok().flatten();
            let nullable: &str = cr.get::<&str, _>("IS_NULLABLE").unwrap_or("");
            let len_str = max_len.map(|l| format!("({})", l)).unwrap_or_default();
            println!("  {} {}{} {}", col_name, data_type, len_str, nullable);
        }
        println!();
    }

    Ok(())
}
