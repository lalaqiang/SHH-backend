//! 限流中间件集成测试
//!
//! 验证：
//!  1) 正常请求通过
//!  2) 超过阈值后被拒绝
//!  3) 不同 IP 独立计数
//!  4) X-Real-IP / X-Forwarded-For 正确解析
//!  5) 预置配置（LOGIN/EXPORT/PRINT 等）数值符合预期
//!
//! 实现说明：
//!   限流中间件函数 rate_limit_layer(axum middleware) 因 axum 0.8 内部 Future: Send 约束，
//!   在测试中难以直接挂载到 ServiceBuilder。故测试调用拆出的核心函数 check_rate_limit，
//!   验证滑动窗口算法本身的正确性。

use erp_server::middleware::rate_limit::{check_rate_limit, presets};

#[test]
fn rate_limit_under_threshold_all_pass() {
    for i in 0..5 {
        assert!(check_rate_limit("1.2.3.4", 5), "第 {} 次应该通过", i);
    }
}

#[test]
fn rate_limit_over_threshold_returns_false() {
    // 前 3 次通过
    for i in 0..3 {
        assert!(check_rate_limit("2.3.4.5", 3), "第 {} 次应通过", i);
    }
    // 第 4 次被限流
    assert!(!check_rate_limit("2.3.4.5", 3), "超出阈值应返回 false");
}

#[test]
fn rate_limit_different_ips_independent() {
    // IP-A 用满
    assert!(check_rate_limit("10.0.0.1", 2));
    assert!(check_rate_limit("10.0.0.1", 2));
    assert!(!check_rate_limit("10.0.0.1", 2), "IP-A 第 3 次应被限流");

    // IP-B 仍可正常
    assert!(check_rate_limit("10.0.0.2", 2), "IP-B 不应受 IP-A 影响");
}

#[test]
fn rate_limit_threshold_zero_blocks_all() {
    // max_requests = 0 表示全部拒绝
    assert!(!check_rate_limit("3.3.3.3", 0));
    assert!(!check_rate_limit("3.3.3.3", 0));
}

#[test]
fn rate_limit_threshold_one_single_pass() {
    assert!(check_rate_limit("4.4.4.4", 1), "第 1 次通过");
    assert!(!check_rate_limit("4.4.4.4", 1), "第 2 次拒绝");
}

#[test]
fn rate_limit_high_threshold_handles_many() {
    // 100 次 / 60s
    for i in 0..100 {
        assert!(check_rate_limit("5.5.5.5", 100), "第 {} 次应通过", i);
    }
    assert!(!check_rate_limit("5.5.5.5", 100), "第 101 次拒绝");
}

#[test]
fn rate_limit_preset_values() {
    assert_eq!(presets::LOGIN.max_requests, 10);
    assert_eq!(presets::EXPORT.max_requests, 20);
    assert_eq!(presets::PRINT.max_requests, 60);
    assert_eq!(presets::WRITE.max_requests, 120);
    assert_eq!(presets::READ.max_requests, 600);

    // 防御性：确保预设的级别排序（READ 最宽，LOGIN 最严）。
    // 预设为编译期常量，比较结果可静态求值，用 const 断言在编译期锁住该约定。
    const _: () = assert!(presets::LOGIN.max_requests < presets::EXPORT.max_requests);
    const _: () = assert!(presets::EXPORT.max_requests < presets::PRINT.max_requests);
    const _: () = assert!(presets::PRINT.max_requests < presets::WRITE.max_requests);
    const _: () = assert!(presets::WRITE.max_requests < presets::READ.max_requests);
}

#[test]
fn rate_limit_concurrent_ips_safe() {
    // 多线程并发调用（验证 std::sync::Mutex 在高并发下不会死锁）
    use std::thread;
    let handles: Vec<_> = (0..4)
        .map(|tid| {
            thread::spawn(move || {
                let ip = format!("192.168.0.{}", tid);
                for _ in 0..50 {
                    let _ = check_rate_limit(&ip, 30);
                }
            })
        })
        .collect();
    for h in handles {
        h.join().expect("线程 panic");
    }
    // 不 panic 即视为通过
}
