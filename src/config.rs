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
    pub cors_origins: Vec<String>,
    /// 连接池最大连接数（默认 15）
    pub db_pool_max_size: u32,
    /// 运行环境：development / production
    pub rust_env: String,
    /// 是否信任反向代理（nginx 等）设置的 X-Forwarded-For / X-Real-IP。
    /// 仅当服务只经受信代理暴露时设为 true；否则攻击者可伪造转发头绕过限流。
    pub trust_proxy: bool,
    /// SQL Server 连接加密级别：off / on / not_supported（默认）/ required。
    /// 默认 not_supported 保持对旧版 SQL Server Express 的兼容；
    /// 生产环境跨网段访问数据库时建议 required（见 .env.example）。
    pub db_encryption: String,
    /// 是否跳过 DB TLS 证书校验（默认 true，兼容自签证书——加密但不去验身份）。
    /// 生产建议在 DB 侧配置有效证书后改为 false。
    pub db_trust_cert: bool,
    /// 是否开放移动端自助注册（默认关闭）。
    /// /api/mobile/register 是公开端点，开启意味着任何人可匿名创建可登录账号，
    /// 仅在确有业务需要（如门店导购自助开通）且配合验证码/限流时才设为 true。
    pub allow_mobile_register: bool,
}

impl Config {
    pub fn from_env() -> Self {
        dotenvy::dotenv().ok();
        let rust_env = std::env::var("RUST_ENV").unwrap_or_else(|_| "development".into());
        let is_prod = rust_env.eq_ignore_ascii_case("production");

        // 生产环境强制必填关键密钥，避免弱默认值泄露
        let require_or_default = |name: &str, default: &str| -> String {
            match std::env::var(name) {
                Ok(v) if !v.is_empty() => v,
                _ => {
                    if is_prod {
                        eprintln!(
                            "FATAL: 环境变量 {} 未设置，生产环境禁止使用默认值。请在 .env 中配置。",
                            name
                        );
                        std::process::exit(1);
                    }
                    tracing::warn!(
                        "{} 未设置，使用弱默认值（仅限开发环境，生产环境必须设置）",
                        name
                    );
                    default.into()
                }
            }
        };

        let db_pool_max_size = std::env::var("DB_POOL_MAX_SIZE")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .filter(|&n| n > 0)
            .unwrap_or(15);

        Self {
            db_server: std::env::var("DB_SERVER").unwrap_or_else(|_| "127.0.0.1".into()),
            db_port: std::env::var("DB_PORT")
                .unwrap_or_else(|_| "1433".into())
                .parse()
                .unwrap_or(1433),
            db_database: std::env::var("DB_DATABASE").unwrap_or_else(|_| "TestERP".into()),
            db_user: std::env::var("DB_USER").unwrap_or_else(|_| "sa".into()),
            db_password: require_or_default("DB_PASSWORD", "sa123456"),
            jwt_secret: require_or_default("JWT_SECRET", "erp_secret_key"),
            host: std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".into()),
            port: std::env::var("PORT")
                .unwrap_or_else(|_| "8080".into())
                .parse()
                .unwrap_or(8080),
            cors_origins: std::env::var("CORS_ORIGINS")
                .unwrap_or_default()
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            db_pool_max_size,
            rust_env,
            trust_proxy: std::env::var("TRUST_PROXY")
                .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
                .unwrap_or(false),
            db_encryption: std::env::var("DB_ENCRYPTION")
                .unwrap_or_else(|_| "not_supported".into()),
            db_trust_cert: std::env::var("DB_TRUST_CERT")
                .map(|v| !(v.eq_ignore_ascii_case("false") || v == "0"))
                .unwrap_or(true),
            allow_mobile_register: std::env::var("ALLOW_MOBILE_REGISTER")
                .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
                .unwrap_or(false),
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
        // P1 修复：加密级别改为可配置（DB_ENCRYPTION），默认 NotSupported 保持
        // 对本地 SQL Server Express 的兼容；跨网段部署建议 required。
        // 证书校验同样可配置（DB_TRUST_CERT，默认 true 兼容自签证书）：
        // trust_cert=true 时只加密不验身份，防被动窃听但不防中间人。
        match self.db_encryption.to_ascii_lowercase().as_str() {
            "off" => config.encryption(tiberius::EncryptionLevel::Off),
            "on" => config.encryption(tiberius::EncryptionLevel::On),
            "required" => config.encryption(tiberius::EncryptionLevel::Required),
            _ => config.encryption(tiberius::EncryptionLevel::NotSupported),
        }
        if self.db_trust_cert {
            config.trust_cert();
        }
        // 工作站 ID 便于 SQL Server 端日志排查
        config.application_name("ERP-Server");
        config
    }
}
