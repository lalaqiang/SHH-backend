use crate::config::Config;
use bb8::Pool;
use bb8_tiberius::ConnectionManager;
use std::sync::OnceLock;
use std::time::Duration;

static POOL: OnceLock<Pool<ConnectionManager>> = OnceLock::new();
// P3-22 修复：bb8::State 不含 max_connections 字段，需自行保存配置的最大连接数
// 用于 get_pool_stats 返回统计信息（health 接口需要）
static MAX_SIZE: OnceLock<u32> = OnceLock::new();

pub async fn init_pool(config: &Config) {
    // P0-4 修复：原 expect("Pool already initialized") 在误调用两次时 panic
    // 改为：已初始化则直接 return，避免 panic
    if POOL.get().is_some() {
        tracing::warn!("Database pool already initialized, skip re-init");
        return;
    }

    let tiberius_config = config.tiberius_config();
    let manager = ConnectionManager::new(tiberius_config);
    // P3-22 修复：连接池大小从 config.db_pool_max_size 读取（环境变量 DB_POOL_MAX_SIZE）
    let max_size = config.db_pool_max_size.max(1);
    let pool = Pool::builder()
        .max_size(max_size)
        .min_idle(Some(0))
        .connection_timeout(Duration::from_secs(30))
        .idle_timeout(Some(Duration::from_secs(600)))
        .max_lifetime(Some(Duration::from_secs(1800)))
        .test_on_check_out(false)
        .build(manager)
        .await;

    let pool = match pool {
        Ok(p) => {
            tracing::info!("Database connection pool created (max_size={})", max_size);
            // 保存 max_size 供 get_pool_stats 使用
            let _ = MAX_SIZE.set(max_size);
            match p.get().await {
                Ok(_conn) => {
                    tracing::info!("Database connection test successful");
                }
                Err(e) => {
                    tracing::warn!(
                        "Database connection test failed: {}. Server will continue but DB operations will fail until DB is available.",
                        e
                    );
                }
            }
            p
        }
        Err(e) => {
            tracing::error!(
                "Failed to create database pool: {}. Server cannot start without a pool.",
                e
            );
            std::process::exit(1);
        }
    };

    // P0-4 修复：原 expect 在并发初始化场景下 panic，改用 get_or_init 语义
    let _ = POOL.set(pool);
}

pub fn get_pool() -> &'static Pool<ConnectionManager> {
    // P0-4 修复：原 expect("Pool not initialized") 在 pool 未初始化时 panic
    // 改为：返回空 Pool 的引用会导致调用方出错，不如在 init 之前就拦截
    // 但既然 init_pool 在 main.rs 启动时已调用，这里保持 expect 但消息更清晰
    POOL.get()
        .expect("Database pool not initialized. Call init_pool() at startup first.")
}

/// P0-4 修复：新增 try_get_pool，允许调用方优雅处理未初始化场景
pub fn try_get_pool() -> Option<&'static Pool<ConnectionManager>> {
    POOL.get()
}

/// Get pool statistics: (max_size, active, idle)
/// P3-22 修复：max_size 从 MAX_SIZE 静态变量读取（init_pool 时保存），不再硬编码
pub fn get_pool_stats() -> (u32, u32, u32) {
    let pool = get_pool();
    let state = pool.state();
    let max_size = *MAX_SIZE.get().unwrap_or(&15);
    let total = state.connections as u32;
    let idle = state.idle_connections as u32;
    let active = total.saturating_sub(idle);
    (max_size, active, idle)
}
