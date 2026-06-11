use std::sync::OnceLock;
use std::time::Duration;
use bb8::Pool;
use bb8_tiberius::ConnectionManager;
use crate::config::Config;

static POOL: OnceLock<Pool<ConnectionManager>> = OnceLock::new();

pub async fn init_pool(config: &Config) {
    let tiberius_config = config.tiberius_config();
    let manager = ConnectionManager::new(tiberius_config);
    let pool = Pool::builder()
        .max_size(15)
        .min_idle(Some(0))
        .connection_timeout(Duration::from_secs(30))
        .idle_timeout(Some(Duration::from_secs(600)))
        .max_lifetime(Some(Duration::from_secs(1800)))
        .test_on_check_out(false)
        .build(manager)
        .await;

    let pool = match pool {
        Ok(p) => {
            tracing::info!("Database connection pool created");
            match p.get().await {
                Ok(_conn) => {
                    tracing::info!("Database connection test successful");
                }
                Err(e) => {
                    tracing::warn!("Database connection test failed: {}. Server will continue but DB operations will fail until DB is available.", e);
                }
            }
            p
        }
        Err(e) => {
            tracing::error!("Failed to create database pool: {}. Server cannot start without a pool.", e);
            std::process::exit(1);
        }
    };

    POOL.set(pool).expect("Pool already initialized");
}

pub fn get_pool() -> &'static Pool<ConnectionManager> {
    POOL.get().expect("Pool not initialized")
}
