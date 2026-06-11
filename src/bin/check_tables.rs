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

    let tables_to_check = vec![
        "tBas_CustPriceTac", "tSys_Msg", "tSys_AutoMsg", "tSys_AutoMsgRule",
        "tSys_Parameters", "tSys_Params", "tSys_RptPrintHis", "tSys_RptPrintNum",
        "tSys_Rpt", "tSys_Warning", "tSys_Company",
    ];

    for table in &tables_to_check {
        let col_sql = format!(
            "SELECT COLUMN_NAME, DATA_TYPE, CHARACTER_MAXIMUM_LENGTH, IS_NULLABLE FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_NAME = '{}' ORDER BY ORDINAL_POSITION",
            table
        );
        println!("\n===== {} =====", table);
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
    }

    Ok(())
}
