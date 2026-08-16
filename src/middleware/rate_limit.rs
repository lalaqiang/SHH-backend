use axum::extract::{ConnectInfo, Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::warn;

/// 滑动窗口：记录每个 IP 的请求时间戳列表
type Bucket = Arc<Mutex<HashMap<String, Vec<Instant>>>>;

const WINDOW: Duration = Duration::from_secs(60);
const CLEANUP_INTERVAL: Duration = Duration::from_secs(300);

/// P2-16 修复：BUCKETS HashMap 最大 IP 数
///   原无上限，攻击者可伪造不同 X-Forwarded-For 头填充 HashMap 耗尽内存
///   限制为 100000 个 IP（cleanup 线程 5 分钟清理一次，期间最多累积 10 万条目）
///   超出时拒绝新 IP 的请求（已有 IP 不受影响），并发 warn 日志
const MAX_BUCKETS: usize = 100000;

/// 全局限流桶
static BUCKETS: Lazy<Bucket> = Lazy::new(|| {
    let b: Bucket = Arc::new(Mutex::new(HashMap::<String, Vec<Instant>>::new()));
    // 启动后台清理任务（用 std 线程，避免依赖 tokio runtime 上下文）
    // P1-10 修复：原 expect("failed to spawn rate-limit cleanup thread") 在资源不足时 panic
    //   （限流模块是 Lazy 初始化，第一次请求时才触发 panic，导致整个服务不可用）
    //   改为：失败时降级为无限流清理模式（限流仍可用，仅内存会缓慢增长，cleanup 由 check_rate_limit 内联完成）
    if let Err(e) = std::thread::Builder::new()
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
    {
        warn!(
            error = %e,
            "rate-limit cleanup 线程启动失败，降级为内联清理模式（限流仍可用，内存可能缓慢增长）"
        );
    }
    b
});

/// 限流配置
#[derive(Clone, Copy)]
pub struct RateLimitConfig {
    pub max_requests: usize,
    pub description: &'static str,
}

/// 限流中间件运行时状态（通过 axum State 注入）
#[derive(Clone)]
pub struct RateLimitState {
    /// 是否信任 X-Forwarded-For / X-Real-IP（仅当服务只经受信反向代理访问时开启）
    pub trust_proxy: bool,
}

impl RateLimitConfig {
    pub const fn new(max_requests: usize, description: &'static str) -> Self {
        Self {
            max_requests,
            description,
        }
    }
}

/// 限流核心逻辑：判断给定 IP 在当前窗口内是否还能放行
/// 测试可直接调用此函数验证算法
pub fn check_rate_limit(client_ip: &str, max_requests: usize) -> bool {
    let now = Instant::now();
    // 容忍 mutex poison（前任线程 panic 后仍能继续服务）
    let mut map = BUCKETS.lock().unwrap_or_else(|e| e.into_inner());
    // P2-16 修复：限制 HashMap 最大条目数，防止攻击者用伪造 IP 耗尽内存
    //   超出限制时拒绝新 IP（已有 IP 仍可正常请求），并周期性 warn
    let bucket_count = map.len();
    let need_insert = !map.contains_key(client_ip);
    if need_insert && bucket_count >= MAX_BUCKETS {
        // 先尝试清理过期条目腾出空间
        map.retain(|_, ts| {
            ts.retain(|t| now.duration_since(*t) < WINDOW);
            !ts.is_empty()
        });
        // 仍然超限 → 拒绝新 IP
        if map.len() >= MAX_BUCKETS {
            if bucket_count % 1000 == 0 {
                warn!(
                    "rate-limit BUCKETS 已达上限 {}，拒绝新 IP 请求（client_ip={}）",
                    MAX_BUCKETS, client_ip
                );
            }
            return false;
        }
    }
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

/// 构造 429 限流响应（带 JSON body，便于前端统一处理）
fn rate_limited_response(description: &str, max: usize) -> Response {
    let body = serde_json::json!({
        "success": false,
        "message": format!("请求过于频繁（{} 限制 {}/分钟），请稍后再试", description, max),
        "code": "RATE_LIMITED"
    });
    (StatusCode::TOO_MANY_REQUESTS, axum::Json(body)).into_response()
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

/// 登录接口专用限流中间件（from_fn 兼容签名）
/// 用法：.route_layer(axum::middleware::from_fn(login_rate_limit))
pub async fn login_rate_limit(
    State(state): State<RateLimitState>,
    request: Request,
    next: Next,
) -> Result<Response, Response> {
    rate_limit_layer(state.trust_proxy, presets::LOGIN, request, next).await
}

pub async fn export_rate_limit(
    State(state): State<RateLimitState>,
    request: Request,
    next: Next,
) -> Result<Response, Response> {
    rate_limit_layer(state.trust_proxy, presets::EXPORT, request, next).await
}

pub async fn print_rate_limit(
    State(state): State<RateLimitState>,
    request: Request,
    next: Next,
) -> Result<Response, Response> {
    rate_limit_layer(state.trust_proxy, presets::PRINT, request, next).await
}

/// 智能限流中间件：根据请求路径自动匹配限流预设
pub async fn smart_rate_limit(
    State(state): State<RateLimitState>,
    request: Request,
    next: Next,
) -> Result<Response, Response> {
    let path = request.uri().path();
    // P1 修复：原仅对 /export 与 /api/print/* 限流，其余接口完全不限。
    // 现按语义分层挂载全部预设：
    //   导出（20/min）< 打印（60/min）< 写操作（120/min）< 通用读取（600/min）
    // 写操作识别复用权限中间件的动词表（save/create/approve/... 结尾），
    // 避免把大量"POST 即查询"的读接口误伤进写入档。
    let c = if path.contains("/export") {
        presets::EXPORT
    } else if path.starts_with("/api/print/") || path == "/api/approval/print-log" {
        presets::PRINT
    } else if crate::middleware::permission::is_write_action_path(path) {
        presets::WRITE
    } else {
        presets::READ
    };
    let client_ip = extract_client_ip(&request, state.trust_proxy);
    if !check_rate_limit(&client_ip, c.max_requests) {
        warn!(
            "[RateLimit] 限流触发 ip={} desc={} path={}",
            client_ip, c.description, path
        );
        return Err(rate_limited_response(c.description, c.max_requests));
    }
    Ok(next.run(request).await)
}

/// 限流中间件（通用入口）
async fn rate_limit_layer(
    trust_proxy: bool,
    config: RateLimitConfig,
    request: Request,
    next: Next,
) -> Result<Response, Response> {
    let client_ip = extract_client_ip(&request, trust_proxy);

    if !check_rate_limit(&client_ip, config.max_requests) {
        warn!(
            "[RateLimit] 限流触发 ip={} desc={} max={}/min",
            client_ip, config.description, config.max_requests
        );
        return Err(rate_limited_response(
            config.description,
            config.max_requests,
        ));
    }

    Ok(next.run(request).await)
}

/// 提取客户端 IP：
/// - 仅在 trust_proxy=true 时信任 X-Forwarded-For / X-Real-IP（反向代理后场景）
/// - 否则直接取 TCP 连接地址，避免伪造转发头绕过限流
fn extract_client_ip(request: &Request, trust_proxy: bool) -> String {
    if trust_proxy {
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
    }
    // 3) 取直连地址（不信任任何转发头）
    if let Some(connect) = request.extensions().get::<ConnectInfo<SocketAddr>>() {
        return connect.0.ip().to_string();
    }
    // 4) 兜底
    "unknown".to_string()
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
