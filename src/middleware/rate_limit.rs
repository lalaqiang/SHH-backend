use axum::extract::{Request, ConnectInfo};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use once_cell::sync::Lazy;
use tracing::warn;

/// 滑动窗口：记录每个 IP 的请求时间戳列表
type Bucket = Arc<Mutex<HashMap<String, Vec<Instant>>>>;

const WINDOW: Duration = Duration::from_secs(60);
const CLEANUP_INTERVAL: Duration = Duration::from_secs(300);

/// 全局限流桶
static BUCKETS: Lazy<Bucket> = Lazy::new(|| {
    let b: Bucket = Arc::new(Mutex::new(HashMap::<String, Vec<Instant>>::new()));
    // 启动后台清理任务（用 std 线程，避免依赖 tokio runtime 上下文）
    std::thread::Builder::new()
        .name("rate-limit-cleanup".into())
        .spawn({
            let b = b.clone();
            move || loop {
                std::thread::sleep(CLEANUP_INTERVAL);
                let now = Instant::now();
                let mut map = b.lock().unwrap_or_else(|e| e.into_inner());
                map.retain(|_, ts| {
                    ts.retain(|t| now.duration_since(*t) < WINDOW);
                    !ts.is_empty()
                });
            }
        })
        .expect("failed to spawn rate-limit cleanup thread");
    b
});

#[allow(dead_code)]
async fn cleanup_loop(buckets: Bucket) {
    let mut interval = tokio::time::interval(CLEANUP_INTERVAL);
    loop {
        interval.tick().await;
        let now = Instant::now();
        let mut map = buckets.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, ts| {
            ts.retain(|t| now.duration_since(*t) < WINDOW);
            !ts.is_empty()
        });
    }
}

/// 限流配置
#[derive(Clone, Copy)]
pub struct RateLimitConfig {
    pub max_requests: usize,
    pub description: &'static str,
}

impl RateLimitConfig {
    pub const fn new(max_requests: usize, description: &'static str) -> Self {
        Self { max_requests, description }
    }
}

/// 限流核心逻辑：判断给定 IP 在当前窗口内是否还能放行
/// 测试可直接调用此函数验证算法
pub fn check_rate_limit(client_ip: &str, max_requests: usize) -> bool {
    let now = Instant::now();
    // 容忍 mutex poison（前任线程 panic 后仍能继续服务）
    let mut map = BUCKETS.lock().unwrap_or_else(|e| e.into_inner());
    let entry = map.entry(client_ip.to_string()).or_default();
    // 清理窗口外的旧记录
    entry.retain(|t| now.duration_since(*t) < WINDOW);
    if entry.len() >= max_requests {
        false
    } else {
        entry.push(now);
        true
    }
}

/// 限流中间件
/// 用法：.route_layer(middleware::from_fn(rate_limit_layer(RateLimitConfig::new(5, "登录"))))
pub async fn rate_limit_layer(
    config: RateLimitConfig,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let client_ip = extract_client_ip(&request);

    if !check_rate_limit(&client_ip, config.max_requests) {
        warn!(
            "[RateLimit] 限流触发 ip={} desc={} max={}/min",
            client_ip, config.description, config.max_requests
        );
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }

    Ok(next.run(request).await)
}

fn extract_client_ip(request: &Request) -> String {
    // 1) 优先取 X-Forwarded-For 头（反向代理后场景）
    if let Some(xff) = request.headers().get("X-Forwarded-For") {
        if let Ok(s) = xff.to_str() {
            if let Some(first) = s.split(',').next() {
                return first.trim().to_string();
            }
        }
    }
    // 2) 取 X-Real-IP
    if let Some(xri) = request.headers().get("X-Real-IP") {
        if let Ok(s) = xri.to_str() {
            return s.trim().to_string();
        }
    }
    // 3) 取直连地址
    if let Some(connect) = request.extensions().get::<ConnectInfo<SocketAddr>>() {
        return connect.0.ip().to_string();
    }
    // 4) 兜底
    "unknown".to_string()
}

/// 预置限流配置
pub mod presets {
    use super::RateLimitConfig;
    pub const LOGIN: RateLimitConfig = RateLimitConfig::new(10, "登录");
    pub const EXPORT: RateLimitConfig = RateLimitConfig::new(20, "导出");
    pub const PRINT: RateLimitConfig = RateLimitConfig::new(60, "打印");
    pub const WRITE: RateLimitConfig = RateLimitConfig::new(120, "写入");
    pub const READ: RateLimitConfig = RateLimitConfig::new(600, "读取");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limit_config() {
        let cfg = RateLimitConfig::new(5, "test");
        assert_eq!(cfg.max_requests, 5);
    }
}
