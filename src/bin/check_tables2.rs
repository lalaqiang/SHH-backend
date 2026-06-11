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
    let pool = Pool::builder().max_size(2).connection_timeout(std::time::Duration::from_secs(30)).build(manager).await?;
    let mut conn = pool.get().await?;

    let exact_tables = vec![
        "tBas_GDSType",
        "tBas_GDSProperty",
        "tBas_GDSKind",
        "tSys_Menus",
        "tSys_Rule",
        "tAcc_PayOut",
        "tAcc_PayIn",
        "tArd_PD",
        "tArd_AR",
        // tFin_CashFlow 不存在
    ];

    println!("\n========== EXACT TABLE LOOKUPS ==========\n");
    for table in &exact_tables {
        let col_sql = format!(
            "SELECT COLUMN_NAME, DATA_TYPE, CHARACTER_MAXIMUM_LENGTH, IS_NULLABLE FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_NAME = '{}' ORDER BY ORDINAL_POSITION",
            table
        );
        let col_stream = conn.query(&col_sql, &[]).await?;
        let col_rows = col_stream.into_first_result().await?;
        if col_rows.is_empty() {
            println!("===== {} ===== NOT FOUND", table);
        } else {
            println!("\n===== {} ===== ({} columns)", table, col_rows.len());
            for cr in &col_rows {
                let col_name: &str = cr.get::<&str, _>("COLUMN_NAME").unwrap_or("");
                let data_type: &str = cr.get::<&str, _>("DATA_TYPE").unwrap_or("");
                let max_len: Option<i32> = cr.try_get::<i32, _>("CHARACTER_MAXIMUM_LENGTH").ok().flatten();
                let nullable: &str = cr.get::<&str, _>("IS_NULLABLE").unwrap_or("");
                let len_str = max_len.map(|l| format!("({})", l)).unwrap_or_default();
                println!("  {} {}{} {}", col_name, data_type, len_str, nullable);
            }
        }
    }

    println!("\n========== WILDCARD TABLE SEARCHES ==========\n");

    let wildcards = vec![
        ("tSys_Permission%", "Permission tables"),
        ("tSys_RolePermission%", "RolePermission tables"),
        ("tSys_Upload%", "Upload tables"),
        ("tSys_File%", "File tables"),
        ("tSys_TableColumnConfig%", "TableColumnConfig tables"),
    ];

    for (pattern, label) in &wildcards {
        let find_sql = format!(
            "SELECT TABLE_NAME FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_TYPE = 'BASE TABLE' AND TABLE_NAME LIKE '{}'",
            pattern
        );
        let stream = conn.query(&find_sql, &[]).await?;
        let rows = stream.into_first_result().await?;
        if rows.is_empty() {
            println!("===== {} (pattern: {}) ===== NO MATCHING TABLES", label, pattern);
        } else {
            for r in &rows {
                let tname: &str = r.get::<&str, _>("TABLE_NAME").unwrap_or("");
                let col_sql = format!(
                    "SELECT COLUMN_NAME, DATA_TYPE, CHARACTER_MAXIMUM_LENGTH, IS_NULLABLE FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_NAME = '{}' ORDER BY ORDINAL_POSITION",
                    tname
                );
                let col_stream = conn.query(&col_sql, &[]).await?;
                let col_rows = col_stream.into_first_result().await?;
                println!("\n===== {} ===== ({} columns)", tname, col_rows.len());
                for cr in &col_rows {
                    let col_name: &str = cr.get::<&str, _>("COLUMN_NAME").unwrap_or("");
                    let data_type: &str = cr.get::<&str, _>("DATA_TYPE").unwrap_or("");
                    let max_len: Option<i32> = cr.try_get::<i32, _>("CHARACTER_MAXIMUM_LENGTH").ok().flatten();
                    let nullable: &str = cr.get::<&str, _>("IS_NULLABLE").unwrap_or("");
                    let len_str = max_len.map(|l| format!("({})", l)).unwrap_or_default();
                    println!("  {} {}{} {}", col_name, data_type, len_str, nullable);
                }
            }
        }
    }

    Ok(())
}
