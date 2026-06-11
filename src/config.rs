use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub db_server: String,
    pub db_port: u16,
    pub db_database: String,
    pub db_user: String,
    pub db_password: String,
    pub jwt_secret: String,
    pub host: String,
    pub port: u16,
}

impl Config {
    pub fn from_env() -> Self {
        dotenvy::dotenv().ok();
        Self {
            db_server: std::env::var("DB_SERVER").unwrap_or_else(|_| "127.0.0.1".into()),
            db_port: std::env::var("DB_PORT")
                .unwrap_or_else(|_| "1433".into())
                .parse()
                .unwrap_or(1433),
            db_database: std::env::var("DB_DATABASE").unwrap_or_else(|_| "TestERP".into()),
            db_user: std::env::var("DB_USER").unwrap_or_else(|_| "sa".into()),
            db_password: std::env::var("DB_PASSWORD").unwrap_or_else(|_| "sa123456".into()),
            jwt_secret: std::env::var("JWT_SECRET").unwrap_or_else(|_| "erp_secret_key".into()),
            host: std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".into()),
            port: std::env::var("PORT")
                .unwrap_or_else(|_| "8080".into())
                .parse()
                .unwrap_or(8080),
        }
    }

    pub fn tiberius_config(&self) -> tiberius::Config {
        let mut config = tiberius::Config::new();
        config.host(&self.db_server);
        config.port(self.db_port);
        config.database(&self.db_database);
        config.authentication(tiberius::AuthMethod::sql_server(
            &self.db_user,
            &self.db_password,
        ));
        config.trust_cert();
        // 本地 SQL Server Express 通常不支持加密,使用 NotSupported 避免 TLS 握手超时
        config.encryption(tiberius::EncryptionLevel::NotSupported);
        // 工作站 ID 便于 SQL Server 端日志排查
        config.application_name("ERP-Server");
        config
    }
}
