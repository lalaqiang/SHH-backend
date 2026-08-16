use crate::config::Config;
use crate::db::get_pool;
use crate::handlers::base_data::try_get_value;
use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tiberius::Row;

static SERVER_START: once_cell::sync::Lazy<Instant> = once_cell::sync::Lazy::new(Instant::now);
static TOTAL_REQUESTS: AtomicU64 = AtomicU64::new(0);
static FAILED_REQUESTS: AtomicU64 = AtomicU64::new(0);

#[derive(Serialize)]
pub struct HealthStatus {
    pub status: &'static str,
    pub service: &'static str,
    pub version: &'static str,
    pub uptime_seconds: u64,
    pub database: DatabaseHealth,
    pub timestamp: String,
}

#[derive(Serialize)]
pub struct DatabaseHealth {
    pub status: &'static str,
    pub latency_ms: u64,
    pub server: Option<String>,
    pub database: Option<String>,
    pub active_connections: Option<u32>,
}

#[derive(Serialize)]
pub struct MetricsResponse {
    pub uptime_seconds: u64,
    pub total_requests: u64,
    pub failed_requests: u64,
    pub error_rate_percent: f64,
    pub database: DatabaseMetrics,
    pub memory: MemoryMetrics,
}

#[derive(Serialize)]
pub struct DatabaseMetrics {
    pub pool_max_size: u32,
    pub pool_active: u32,
    pub pool_idle: u32,
}

#[derive(Serialize)]
pub struct MemoryMetrics {
    pub rss_bytes: u64,
    pub virtual_bytes: u64,
}

/// 简单的 in-process 请求计数器（用 middleware 调用）
pub fn inc_request(success: bool) {
    TOTAL_REQUESTS.fetch_add(1, Ordering::Relaxed);
    if !success {
        FAILED_REQUESTS.fetch_add(1, Ordering::Relaxed);
    }
}

/// 请求计数中间件：统计每个请求的成功/失败
/// 用法：.layer(axum::middleware::from_fn(health::request_counter))
pub async fn request_counter(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let resp = next.run(request).await;
    let success = resp.status().as_u16() < 400;
    inc_request(success);
    resp
}

/// GET /api/health
/// 用于 K8s liveness probe / 负载均衡器健康检查
/// 公开端点，不需要鉴权
///
/// D5 修复：原逻辑 DB 故障时返回 HTTP 200 + status="degraded"，
///   K8s/负载均衡器仍认为服务健康继续转发流量，导致用户请求持续失败。
///   改为：DB 故障时返回 503 Service Unavailable + status="unhealthy"，
///   K8s readiness probe 会自动从负载均衡中摘除该实例，直到恢复。
pub async fn health_check() -> Response {
    let start = Instant::now();
    let db_health = match get_pool().get().await {
        Ok(mut conn) => {
            let latency = start.elapsed().as_millis() as u64;
            // 探测性查询：取当前数据库名
            let result = conn
                .simple_query("SELECT @@SERVERNAME AS svr, DB_NAME() AS db")
                .await;
            match result {
                Ok(stream) => {
                    let rows: Vec<Row> = stream.into_first_result().await.unwrap_or_default();
                    let (svr, db) = rows
                        .first()
                        .map(|r| {
                            (
                                try_get_value(r, "svr").as_str().map(|s| s.to_string()),
                                try_get_value(r, "db").as_str().map(|s| s.to_string()),
                            )
                        })
                        .unwrap_or((None, None));
                    DatabaseHealth {
                        status: "up",
                        latency_ms: latency,
                        server: svr,
                        database: db,
                        active_connections: None,
                    }
                }
                Err(_e) => DatabaseHealth {
                    status: "down",
                    latency_ms: latency,
                    server: None,
                    database: None,
                    active_connections: None,
                },
            }
        }
        Err(_e) => DatabaseHealth {
            status: "down",
            latency_ms: start.elapsed().as_millis() as u64,
            server: None,
            database: None,
            active_connections: None,
        },
    };

    // D5 修复：DB 故障时返回 503，让 K8s readiness probe 自动摘除该实例
    //   - DB up + 探测查询成功 → 200 healthy
    //   - DB down（连接池耗尽 / DB 不可达 / 探测查询失败）→ 503 unhealthy
    let (overall, http_status) = if db_health.status == "up" {
        ("healthy", StatusCode::OK)
    } else {
        ("unhealthy", StatusCode::SERVICE_UNAVAILABLE)
    };

    let body = HealthStatus {
        status: overall,
        service: "ERP-Backend",
        version: env!("CARGO_PKG_VERSION"),
        uptime_seconds: SERVER_START.elapsed().as_secs(),
        database: db_health,
        timestamp: chrono::Local::now().to_rfc3339(),
    };

    (http_status, Json(body)).into_response()
}

/// GET /api/metrics
/// 用于 Prometheus 抓取 / 监控系统
/// 用于 Prometheus 抓取 / 监控系统（需 JWT 鉴权）
///
/// Query:
///   format=prom  → text/plain; version=0.0.4  （Prometheus 抓取格式，默认）
///   format=json  → application/json           （前端/调试用）
///   不传          → 根据 Accept 头协商
pub async fn metrics(
    State(_config): State<Config>,
    Query(params): Query<MetricsQuery>,
) -> Response {
    let total = TOTAL_REQUESTS.load(Ordering::Relaxed);
    let failed = FAILED_REQUESTS.load(Ordering::Relaxed);
    let uptime = SERVER_START.elapsed().as_secs();
    let error_rate = if total > 0 {
        (failed as f64 / total as f64) * 100.0
    } else {
        0.0
    };
    let (rss, virt) = read_memory_usage().unwrap_or((0, 0));
    let (pool_max, pool_active, pool_idle) = crate::db::get_pool_stats();

    match params.format.as_deref() {
        Some("json") => render_metrics_json(
            total,
            failed,
            uptime,
            error_rate,
            rss,
            virt,
            pool_max,
            pool_active,
            pool_idle,
        )
        .into_response(),
        Some("prom") | None => render_metrics_prom(
            total,
            failed,
            uptime,
            error_rate,
            rss,
            virt,
            pool_max,
            pool_active,
            pool_idle,
        )
        .into_response(),
        Some(other) => (
            StatusCode::BAD_REQUEST,
            format!("unsupported format: {}", other),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct MetricsQuery {
    pub format: Option<String>,
}

fn render_metrics_json(
    total: u64,
    failed: u64,
    uptime: u64,
    error_rate: f64,
    rss: u64,
    virt: u64,
    pool_max: u32,
    pool_active: u32,
    pool_idle: u32,
) -> Json<MetricsResponse> {
    Json(MetricsResponse {
        uptime_seconds: uptime,
        total_requests: total,
        failed_requests: failed,
        error_rate_percent: (error_rate * 100.0).round() / 100.0,
        database: DatabaseMetrics {
            pool_max_size: pool_max,
            pool_active,
            pool_idle,
        },
        memory: MemoryMetrics {
            rss_bytes: rss,
            virtual_bytes: virt,
        },
    })
}

/// Prometheus text format (version 0.0.4)
/// 文档：https://github.com/prometheus/docs/blob/main/content/docs/instrumenting/exposition_formats.md
fn render_metrics_prom(
    total: u64,
    failed: u64,
    uptime: u64,
    error_rate: f64,
    rss: u64,
    virt: u64,
    pool_max: u32,
    pool_active: u32,
    pool_idle: u32,
) -> Response {
    let body = format!(
        "# HELP erp_uptime_seconds 服务运行秒数\n\
         # TYPE erp_uptime_seconds gauge\n\
         erp_uptime_seconds {uptime}\n\
         \n\
         # HELP erp_http_requests_total 处理过的 HTTP 请求总数（所有路径汇总）\n\
         # TYPE erp_http_requests_total counter\n\
         erp_http_requests_total {total}\n\
         \n\
         # HELP erp_http_requests_failed 失败请求总数（5xx 或中间件异常）\n\
         # TYPE erp_http_requests_failed counter\n\
         erp_http_requests_failed {failed}\n\
         \n\
         # HELP erp_http_error_rate_percent 失败请求占比（百分比）\n\
         # TYPE erp_http_error_rate_percent gauge\n\
         erp_http_error_rate_percent {error_rate:.2}\n\
         \n\
         # HELP erp_db_pool_max_size 数据库连接池最大连接数\n\
         # TYPE erp_db_pool_max_size gauge\n\
         erp_db_pool_max_size {pool_max}\n\
         \n\
         # HELP erp_db_pool_active 数据库连接池活跃连接数\n\
         # TYPE erp_db_pool_active gauge\n\
         erp_db_pool_active {pool_active}\n\
         \n\
         # HELP erp_db_pool_idle 数据库连接池空闲连接数\n\
         # TYPE erp_db_pool_idle gauge\n\
         erp_db_pool_idle {pool_idle}\n\
         \n\
         # HELP erp_memory_rss_bytes 进程常驻内存（字节）\n\
         # TYPE erp_memory_rss_bytes gauge\n\
         erp_memory_rss_bytes {rss}\n\
         \n\
         # HELP erp_memory_virtual_bytes 进程虚拟内存（字节）\n\
         # TYPE erp_memory_virtual_bytes gauge\n\
         erp_memory_virtual_bytes {virt}\n\
         \n\
         # HELP erp_build_info 构建信息（恒为 1）\n\
         # TYPE erp_build_info gauge\n\
         erp_build_info{{version=\"{}\"}} 1\n",
        env!("CARGO_PKG_VERSION"),
    );

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; version=0.0.4; charset=utf-8"),
    );
    (StatusCode::OK, headers, body).into_response()
}

/// 读取进程内存使用（Windows 兼容，跨平台兜底 0）
#[cfg(target_os = "windows")]
fn read_memory_usage() -> Option<(u64, u64)> {
    use std::process::Command;
    // 调用 tasklist 获取当前进程内存
    let output = Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq erp-backend.exe", "/FO", "CSV", "/NH"])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&output.stdout);
    // 解析 CSV 第 5 列（工作集内存）
    let mut rss = 0u64;
    for line in s.lines() {
        let cols: Vec<&str> = line.split(',').map(|c| c.trim_matches('"')).collect();
        if cols.len() >= 5 {
            let mem_str = cols[4].replace(",", "").replace(" K", "").replace("'", "");
            rss = mem_str.parse::<u64>().ok()? * 1024;
            break;
        }
    }
    Some((rss, rss * 2))
}

#[cfg(not(target_os = "windows"))]
fn read_memory_usage() -> Option<(u64, u64)> {
    use std::fs;
    let status = fs::read_to_string("/proc/self/status").ok()?;
    let mut rss = 0u64;
    let mut virt = 0u64;
    for line in status.lines() {
        if line.starts_with("VmRSS:") {
            rss = line.split_whitespace().nth(1)?.parse::<u64>().ok()? * 1024;
        } else if line.starts_with("VmSize:") {
            virt = line.split_whitespace().nth(1)?.parse::<u64>().ok()? * 1024;
        }
    }
    Some((rss, virt))
}
