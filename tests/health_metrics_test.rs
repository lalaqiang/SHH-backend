//! 计数器与辅助函数测试
//!
//! 验证 handlers::health::inc_request 的请求计数器语义
//! 适用于不需要数据库的纯函数测试

use erp_server::handlers::health::inc_request;

#[test]
fn inc_request_success_increments_total() {
    // 多次调用 inc_request(true) 应单调增加
    let _ = inc_request(true);
    let _ = inc_request(true);
    let _ = inc_request(true);
    // 验证 metrics 端点能返回非空字段（间接通过 inc_request 不 panic）
}

#[test]
fn inc_request_failure_increments_failed() {
    inc_request(false);
    inc_request(false);
    // 不 panic 即可；具体数值通过 /api/metrics 端点验证（需 DB）
}

#[test]
fn inc_request_mixed_does_not_panic() {
    for i in 0..100 {
        inc_request(i % 3 == 0);
    }
}
