use axum::{
    Router,
    extract::DefaultBodyLimit,
    middleware as axum_mw,
    routing::{get, post},
};
use erp_server::config::Config;
use erp_server::db::pool::init_pool;
use erp_server::handlers::*;
use erp_server::middleware::auth::auth_middleware;
use erp_server::middleware::rate_limit::RateLimitState;
use std::net::SocketAddr;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{EnvFilter, prelude::*};

/// 初始化日志系统：支持 stdout 与文件轮转两种模式。
///
/// - 若设置 `LOG_DIR` 环境变量，则按天滚动写入该目录下的 `erp-server.log`（非阻塞写入）。
/// - 否则输出到 stdout（开发环境默认）。
/// - 日志级别由 `RUST_LOG` 控制，默认 `info`。
///
/// 返回的 `WorkerGuard` 必须在 main 中持有到程序结束，确保所有日志被刷新。
fn init_logging() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let log_dir = std::env::var("LOG_DIR").ok().filter(|s| !s.is_empty());

    match log_dir {
        Some(dir) => {
            // 确保日志目录存在
            if let Err(e) = std::fs::create_dir_all(&dir) {
                eprintln!("WARN: 无法创建日志目录 {}，回退到 stdout: {}", dir, e);
                let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
                return None;
            }
            // 按天滚动：文件名形如 erp-server.log.2026-07-01
            let file_appender = tracing_appender::rolling::daily(&dir, "erp-server.log");
            let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
            let file_layer = tracing_subscriber::fmt::layer()
                .with_writer(non_blocking)
                .with_ansi(false); // 文件中不要 ANSI 颜色码

            // 同时保留 stdout 输出，便于容器化部署时通过 docker logs 查看
            let stdout_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stdout);

            let result = tracing_subscriber::registry()
                .with(filter)
                .with(file_layer)
                .with(stdout_layer)
                .try_init();
            if let Err(e) = result {
                eprintln!("WARN: 日志初始化失败: {}", e);
            } else {
                tracing::info!("日志已启用文件轮转，目录: {}（按天滚动）", dir);
            }
            Some(guard)
        }
        None => {
            let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
            tracing::info!("日志输出到 stdout（如需文件轮转请设置 LOG_DIR 环境变量）");
            None
        }
    }
}

#[tokio::main]
async fn main() {
    // dotenvy 先加载，使 init_logging 能读到 LOG_DIR / RUST_LOG
    dotenvy::dotenv().ok();
    let _log_guard = init_logging();

    let config = Config::from_env();
    let config_clone = config.clone();
    let rate_limit_state = RateLimitState {
        trust_proxy: config.trust_proxy,
    };
    init_pool(&config).await;
    erp_server::db::migrate::run_migrations().await;

    // CORS：若配置了 CORS_ORIGINS 环境变量则按白名单放行，否则开发环境放行所有
    // P3-29 修复：原 methods/headers 放行 Any，可被利用发送非标准方法或自定义头
    //   收紧为常用方法和常用头（GET/POST/PUT/DELETE/OPTIONS + 标准 HTTP 头）
    let cors = if config.cors_origins.is_empty() {
        tracing::warn!("CORS_ORIGINS 未配置，CORS 放行所有来源（仅建议开发环境使用）");
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods([
                axum::http::Method::GET,
                axum::http::Method::POST,
                axum::http::Method::PUT,
                axum::http::Method::DELETE,
                axum::http::Method::OPTIONS,
            ])
            .allow_headers([
                axum::http::header::AUTHORIZATION,
                axum::http::header::CONTENT_TYPE,
                axum::http::header::ACCEPT,
                axum::http::HeaderName::from_static("x-requested-with"),
                axum::http::HeaderName::from_static("x-csrf-token"),
            ])
    } else {
        let origins: Vec<axum::http::HeaderValue> = config
            .cors_origins
            .iter()
            .filter_map(|o| o.parse().ok())
            .collect();
        tracing::info!("CORS 白名单: {:?}", config.cors_origins);
        CorsLayer::new()
            .allow_origin(origins)
            .allow_methods([
                axum::http::Method::GET,
                axum::http::Method::POST,
                axum::http::Method::PUT,
                axum::http::Method::DELETE,
                axum::http::Method::OPTIONS,
            ])
            .allow_headers([
                axum::http::header::AUTHORIZATION,
                axum::http::header::CONTENT_TYPE,
                axum::http::header::ACCEPT,
                axum::http::HeaderName::from_static("x-requested-with"),
                axum::http::HeaderName::from_static("x-csrf-token"),
            ])
    };

    // Public routes (no auth required)
    // 登录类端点单独挂 login_rate_limit 限流（10 次/分钟/IP），防爆破；
    // health/metrics/stores/register 等公开端点不限流
    let login_routes = Router::new()
        .route("/api/auth/login", post(auth::login))
        .route("/api/login", post(auth::login)) // ★ 兑容前端 /api/login 路径
        .route("/api/mobile/login", post(mobile::mobile_login))
        .route_layer(axum_mw::from_fn_with_state(
            rate_limit_state.clone(),
            erp_server::middleware::rate_limit::login_rate_limit,
        ));

    let public_routes = Router::new()
        .route("/api/health", get(health::health_check))
        .route("/api/mobile/register", post(mobile::mobile_register))
        .route("/api/mobile/stores", get(mobile::list_stores)) // 公开门店列表（登录页用）
        .merge(login_routes)
        .with_state(config_clone.clone());

    // Protected routes (auth required)
    let protected_routes = Router::new()
        // ===== Monitoring (auth required) =====
        .route("/api/metrics", get(health::metrics)) // 监控指标（需鉴权）
        // ===== Auth =====
        .route("/api/auth/me", get(auth::user_info))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/auth/change-password", post(auth::change_password))
        // ===== User Preferences (跨设备同步：主题、布局等) =====
        .route(
            "/api/user/pref",
            get(auth::get_user_prefs).put(auth::set_user_pref),
        )
        // ===== Generic CRUD =====
        .route("/api/generic/query", post(generic::generic_query))
        .route("/api/generic/create", post(generic::generic_create))
        .route("/api/generic/update", post(generic::generic_update))
        .route("/api/generic/delete", post(generic::generic_delete))
        .route(
            "/api/generic/cleanup-orphan-stock",
            post(generic::generic_cleanup_orphan_stock),
        )
        .route("/api/generic/export", post(generic::generic_export))
        .route(
            "/api/generic/batch-update",
            post(generic::generic_batch_update),
        )
        .route("/api/generic/restore", post(generic::generic_restore))
        .route("/api/generic/tree", post(generic::generic_tree))
        .route("/api/generic/import", post(generic::generic_import))
        .route(
            "/api/generic/import-excel",
            post(generic::generic_import_excel),
        )
        .route(
            "/api/generic/export-excel",
            post(generic::generic_export_excel),
        )
        // /api/generic/oper-log 已移除（使用 /system/log/list 替代）
        // ★ 表结构元数据接口（基于 INFORMATION_SCHEMA.COLUMNS）
        .route("/api/generic/schema", post(generic::generic_table_schema))
        .route(
            "/api/doc_no/generate",
            post(erp_server::utils::doc_no::generate_doc_no),
        )
        .route(
            "/api/doc_no/list-types",
            get(erp_server::utils::doc_no::list_doc_types),
        )
        .route(
            "/api/doc_no/reset-seq",
            post(erp_server::utils::doc_no::reset_doc_seq),
        )
        .route(
            "/api/generic/docno/generate",
            post(erp_server::utils::doc_no::generate_doc_no),
        )
        // ===== Base Data =====
        .route("/api/base/goods", post(base_data::get_goods))
        .route("/api/base/goods/create", post(base_data::create_goods))
        .route("/api/base/goods/update", post(base_data::update_goods))
        .route("/api/base/goods/delete", post(base_data::delete_goods))
        .route("/api/base/customer", post(base_data::get_customers))
        .route(
            "/api/base/customer/create",
            post(base_data::create_customer),
        )
        .route(
            "/api/base/customer/update",
            post(base_data::update_customer),
        )
        .route(
            "/api/base/customer/delete",
            post(base_data::delete_customer),
        )
        .route("/api/base/supplier", post(base_data::get_suppliers))
        .route(
            "/api/base/supplier/create",
            post(base_data::create_supplier),
        )
        .route(
            "/api/base/supplier/update",
            post(base_data::update_supplier),
        )
        .route(
            "/api/base/supplier/delete",
            post(base_data::delete_supplier),
        )
        .route("/api/base/warehouse", post(base_data::get_warehouses))
        .route(
            "/api/base/warehouse/create",
            post(base_data::create_warehouse),
        )
        .route(
            "/api/base/warehouse/update",
            post(base_data::update_warehouse),
        )
        .route(
            "/api/base/warehouse/delete",
            post(base_data::delete_warehouse),
        )
        .route("/api/base/brand", post(base_data::get_brands))
        .route("/api/base/brand/create", post(base_data::create_brand))
        .route("/api/base/brand/update", post(base_data::update_brand))
        .route("/api/base/employee", post(base_data::get_employees))
        .route(
            "/api/base/employee/create",
            post(base_data::create_employee),
        )
        .route(
            "/api/base/employee/update",
            post(base_data::update_employee),
        )
        .route(
            "/api/base/stock-query",
            post(base_data::get_inventory_stock),
        )
        .route(
            "/api/base/dashboard-stats",
            post(base_data::get_dashboard_stats),
        )
        .route("/api/dashboard/stats", post(base_data::get_dashboard_stats))
        .route("/api/base/versions", post(base_data::get_base_versions))
        // ===== Categories =====
        .route("/api/categories", post(categories::get_categories))
        .route("/api/categories/create", post(categories::create_category))
        .route("/api/categories/update", post(categories::update_category))
        .route("/api/categories/delete", post(categories::delete_category))
        .route("/api/categories/tree", post(categories::get_category_tree))
        .route("/api/categories/flat", post(categories::get_category))
        // ===== Inventory =====
        .route("/api/inventory/io/list", post(inventory::get_io_list))
        .route("/api/inventory/io/create", post(inventory::create_io))
        .route("/api/inventory/io/update", post(inventory::update_io))
        .route("/api/inventory/io/delete", post(inventory::delete_io))
        .route("/api/inventory/io/detail", post(inventory::get_io_detail))
        .route("/api/inventory/move/list", post(inventory::get_move_list))
        .route("/api/inventory/move/create", post(inventory::create_move))
        .route("/api/inventory/move/update", post(inventory::update_move))
        .route("/api/inventory/move/delete", post(inventory::delete_move))
        .route(
            "/api/inventory/move/detail",
            post(inventory::get_move_detail),
        )
        .route("/api/inventory/check/list", post(inventory::get_check_list))
        .route("/api/inventory/check/create", post(inventory::create_check))
        .route("/api/inventory/check/update", post(inventory::update_check))
        .route("/api/inventory/check/delete", post(inventory::delete_check))
        .route(
            "/api/inventory/check/detail",
            post(inventory::get_check_detail),
        )
        .route(
            "/api/inventory/replenish/list",
            post(inventory::get_replenish_list),
        )
        .route(
            "/api/inventory/replenish/create",
            post(inventory::create_replenish),
        )
        .route("/api/inventory/month_settle", post(inventory::month_settle))
        .route(
            "/api/inventory/month_settle_rollback",
            post(inventory::month_settle_rollback),
        )
        .route(
            "/api/inventory/stock-query",
            post(base_data::get_inventory_stock),
        )
        .route("/api/inventory/flows", post(inventory::get_stock_flow))
        .route("/api/inventory/doc-flows", post(inventory::get_doc_flows))
        .route("/api/inventory/alerts", post(inventory::low_stock_alert))
        .route(
            "/api/inventory/alerts/replenish",
            post(inventory::replenish_from_alert),
        )
        .route(
            "/api/inventory/replenish-suggestions",
            post(inventory::get_replenish_suggestions),
        )
        .route("/api/inventory/adjust", post(inventory::inventory_adjust))
        // ===== Purchase =====
        .route(
            "/api/purchase/order/list",
            post(purchase::get_purchase_orders),
        )
        .route(
            "/api/purchase/order/create",
            post(purchase::create_purchase_order),
        )
        .route(
            "/api/purchase/order/update",
            post(purchase::update_purchase_order),
        )
        .route(
            "/api/purchase/inbound/list",
            post(purchase::get_purchase_inbound),
        )
        .route(
            "/api/purchase/inbound/create",
            post(purchase::create_purchase_inbound),
        )
        .route(
            "/api/purchase/return/list",
            post(purchase::get_purchase_return),
        )
        .route(
            "/api/purchase/return/create",
            post(purchase::create_purchase_return),
        )
        .route(
            "/api/purchase/quote/list",
            post(purchase::get_purchase_quotes),
        )
        .route(
            "/api/purchase/quote/create",
            post(purchase::create_purchase_quote),
        )
        .route(
            "/api/purchase/quote/update",
            post(purchase::update_purchase_quote),
        )
        .route(
            "/api/purchase/adjprice/list",
            post(purchase::get_purchase_adjprice),
        )
        .route(
            "/api/purchase/adjprice/create",
            post(purchase::create_purchase_adjprice),
        )
        .route("/api/purchase/query", post(purchase::get_purchase_query))
        // ===== Sales =====
        .route("/api/sales/order/list", post(sales::get_sales_orders))
        .route("/api/sales/order/create", post(sales::create_sales_order))
        .route("/api/sales/order/update", post(sales::update_sales_order))
        .route("/api/sales/outbound/list", post(sales::get_sales_outbound))
        .route(
            "/api/sales/outbound/create",
            post(sales::create_sales_outbound),
        )
        .route(
            "/api/sales/outbound/update",
            post(sales::update_sales_outbound),
        )
        .route("/api/sales/quote/list", post(sales::get_sales_quotes))
        .route("/api/sales/quote/create", post(sales::create_sales_quote))
        .route("/api/sales/quote/update", post(sales::update_sales_quote))
        .route("/api/sales/adjprice/list", post(sales::get_sales_adjprice))
        .route(
            "/api/sales/adjprice/create",
            post(sales::create_sales_adjprice),
        )
        // ===== Sales Return =====
        .route(
            "/api/sales/return/list",
            post(sales_return::list_sales_return),
        )
        .route(
            "/api/sales/return/create",
            post(sales_return::create_sales_return),
        )
        .route(
            "/api/sales/return/update",
            post(sales_return::update_sales_return),
        )
        // ===== Approval =====
        .route("/api/approval/print-log", post(approval::print_log))
        // ===== 统一单据服务（doc_service） =====
        .route("/api/doc/save", post(doc::doc_save))
        .route("/api/doc/approve", post(doc::doc_approve))
        .route("/api/doc/unapprove", post(doc::doc_unapprove))
        .route("/api/doc/void", post(doc::doc_void))
        .route(
            "/api/doc/generate-from-source",
            post(doc::doc_generate_from_source),
        )
        .route("/api/doc/graph", post(doc::doc_graph))
        // ===== Order Flow =====
        .route(
            "/api/order-flow/available-qty",
            post(order_flow::query_available),
        )
        .route(
            "/api/order-flow/source-detail",
            post(order_flow::query_source_detail),
        )
        // ===== Finance =====
        // AR/AP 派生查询（已实现，前端 FinanceReceivable/FinancePayable 使用）
        .route("/api/finance/ar/customer", post(finance::get_customer_ar))
        .route(
            "/api/finance/ar/customer/detail",
            post(finance::get_customer_ar_detail),
        )
        .route("/api/finance/ap/supplier", post(finance::get_supplier_ap))
        .route(
            "/api/finance/ap/supplier/detail",
            post(finance::get_supplier_ap_detail),
        )
        // 收款单专用接口（已实现，前端实际走通用 /doc/*，这里作为备用保留）
        .route("/api/finance/receipt/list", post(finance::get_receipt_list))
        .route("/api/finance/receipt/create", post(finance::create_receipt))
        .route("/api/finance/receipt/update", post(finance::update_receipt))
        .route("/api/finance/receipt/delete", post(finance::delete_receipt))
        .route("/api/finance/receipt/audit", post(finance::audit_receipt))
        // 超期账户查询（已实现，前端报表中心调用）
        .route("/api/finance/overdue", post(finance::get_overdue_accounts))
        // 核销明细查询（编辑模式回显用，已实现）
        .route(
            "/api/finance/writeoff/list",
            post(finance::get_writeoff_details),
        )
        // 对账单（P1 即将实现）
        .route(
            "/api/finance/statement/customer",
            post(finance::get_customer_statement),
        )
        .route(
            "/api/finance/statement/supplier",
            post(finance::get_supplier_statement),
        )
        // 说明：以下接口已废弃，原为 stub 返回"暂未实现"
        //   - payment/list/create/update/delete/audit（前端走通用 /doc/*）
        //   - cashflow/list/create/update/delete/audit（前端走通用 /doc/*）
        //   - receivable/list、payable/list（派生 AR/AP 已替代独立表方案）
        //   - payable/process/writeoff/adjust、receivable/process/writeoff/adjust（核销通过核销明细表实现）
        // ===== End Finance =====
        // ===== Commission & Pricing =====
        // 注：提成模板 CRUD 已统一改用 /generic/* 接口操作 tSys_Parameters 表
        .route(
            "/api/pricing/template/list",
            post(commission_pricing::get_pricing_templates),
        )
        .route(
            "/api/pricing/template/create",
            post(commission_pricing::create_pricing_template),
        )
        .route(
            "/api/pricing/template/update",
            post(commission_pricing::update_pricing_template),
        )
        .route(
            "/api/pricing/template/delete",
            post(commission_pricing::delete_pricing_template),
        )
        .route(
            "/api/pricing/rules",
            post(commission_pricing::get_pricing_rules),
        )
        .route(
            "/api/pricing/customer-prices",
            post(commission_pricing::get_customer_prices),
        )
        .route(
            "/api/pricing/customer-prices/save",
            post(commission_pricing::save_customer_price),
        )
        // ===== Tier 8: 提成计算引擎 =====
        .route(
            "/api/commission/calc/employee",
            post(commission_pricing_tier8::calculate_employee_commission),
        )
        .route(
            "/api/commission/calc/all",
            post(commission_pricing_tier8::calculate_all_commission),
        )
        .route(
            "/api/commission/details",
            post(commission_pricing_tier8::get_commission_details),
        )
        // ===== 提成重算（销售单保存后调用，写入明细 Commission 字段）=====
        .route(
            "/api/commission/recalc-invoice",
            post(commission::recalc_invoice),
        )
        // ===== 提成报表（汇总 + 明细，从 tSal_InvDetail.Commission 聚合）=====
        .route(
            "/api/report/commission-summary",
            post(commission::get_commission_summary),
        )
        .route(
            "/api/report/commission-detail",
            post(commission::get_commission_detail),
        )
        .route(
            "/api/report/commission-unified",
            post(commission::get_commission_unified),
        )
        // ===== 提成 Excel 导出（对齐 88 项目，rust_xlsxwriter 生成 xlsx）=====
        .route(
            "/api/report/commission-unified/export-summary",
            post(commission::export_commission_unified_summary),
        )
        .route(
            "/api/report/commission-unified/export-detail",
            post(commission::export_commission_unified_detail),
        )
        .route(
            "/api/report/commission/export-excel",
            post(commission::export_commission_report_excel),
        )
        .route(
            "/api/report/commission-detail/export-excel",
            post(commission::export_commission_detail_excel),
        )
        .route(
            "/api/commission-template/export-products",
            post(commission::export_product_rules),
        )
        .route(
            "/api/commission-template/export-brands",
            post(commission::export_brand_rules),
        )
        // ===== Tier 8: 价格模板应用引擎 =====
        .route(
            "/api/pricing/apply",
            post(commission_pricing_tier8::apply_pricing_for_customer),
        )
        .route(
            "/api/pricing/customer-list",
            post(commission_pricing_tier8::get_customer_price_list),
        )
        .route(
            "/api/pricing/bulk-apply",
            post(commission_pricing_tier8::bulk_apply_pricing_template),
        )
        // ===== 客户定价批量计算（销售单选商品时按客户定价填充价格）=====
        .route("/api/pricing/calc-batch", post(pricing_calc::calc_batch))
        // ===== Print =====
        .route("/api/print/template/list", post(print::get_print_templates))
        .route("/api/print/template/get", post(print::get_print_template))
        .route(
            "/api/print/template/create",
            post(print::create_print_template),
        )
        .route(
            "/api/print/template/update",
            post(print::update_print_template),
        )
        .route(
            "/api/print/template/delete",
            post(print::delete_print_template),
        )
        .route(
            "/api/print/template/restore",
            post(print::restore_print_template),
        )
        .route("/api/print/log/list", post(print::get_print_logs))
        .route("/api/print/log/create", post(print::create_print_log))
        .route("/api/print/config", post(print::get_print_config))
        .route("/api/print/config/save", post(print::save_print_config))
        // ===== 会计期间管理 (Tier 5) =====
        .route("/api/system/acc-period/list", post(system::list_acc_per))
        .route(
            "/api/system/acc-period/create",
            post(system::create_acc_per),
        )
        .route(
            "/api/system/acc-period/update",
            post(system::update_acc_per),
        )
        .route(
            "/api/system/acc-period/delete",
            post(system::delete_acc_per),
        )
        .route("/api/system/acc-period/close", post(system::close_period))
        .route("/api/system/acc-period/reopen", post(system::reopen_period))
        .route("/api/print/versions", post(print::get_print_versions))
        .route(
            "/api/print/versions/create",
            post(print::create_print_version),
        )
        .route(
            "/api/print/versions/rollback",
            post(print::rollback_print_version),
        )
        // ===== OA =====
        .route("/api/oa/workflow/list", post(oa::get_workflow_list))
        .route("/api/oa/workflow/approve", post(oa::approve_workflow))
        .route("/api/oa/notice/list", post(oa::get_notice_list))
        .route("/api/oa/email/list", post(oa::get_email_list))
        // ===== Report =====
        .route("/api/report/sales", post(report::get_sales_report))
        .route("/api/report/purchase", post(report::get_purchase_report))
        .route("/api/report/inventory", post(report::get_stock_report))
        .route("/api/report/business", post(report::get_business_report))
        .route("/api/report/profit", post(report::get_profit_analysis))
        .route(
            "/api/report/aging/receivable",
            post(report::get_receivable_aging),
        )
        .route("/api/report/aging/payable", post(report::get_payable_aging))
        .route(
            "/api/report/stock-turnover",
            post(report::get_stock_turnover),
        )
        .route("/api/report/alert-center", post(report::get_alert_center))
        .route(
            "/api/report/sales-task-summary",
            post(report::get_sales_task_summary),
        )
        // ===== Retail =====
        .route("/api/retail/sale", post(retail::retail_sale))
        .route("/api/retail/cashier", post(retail::get_cashier_info))
        // ===== Sales Input =====
        .route("/api/sales-input/list", post(sales_input::list_emp_sales))
        .route(
            "/api/sales-input/create",
            post(sales_input::create_emp_sales),
        )
        .route(
            "/api/sales-input/update",
            post(sales_input::update_emp_sales),
        )
        // ===== VIP =====
        .route("/api/vip/list", post(vip::list_vip))
        .route("/api/vip/create", post(vip::create_vip))
        .route("/api/vip/update", post(vip::update_vip))
        .route("/api/vip/delete", post(vip::delete_vip))
        // ===== Mobile =====
        .route(
            "/api/mobile/change-password",
            post(mobile::mobile_change_password),
        )
        .route("/api/mobile/sync-base-data", post(mobile::sync_base_data))
        .route(
            "/api/mobile/replenishment/submit",
            post(mobile::submit_replenishment),
        )
        .route(
            "/api/mobile/replenishment/list",
            post(mobile::get_replenishment_history),
        )
        .route(
            "/api/mobile/replenishment/detail",
            post(mobile::get_replenishment_detail),
        )
        .route(
            "/api/mobile/replenishment/for-transfer",
            post(mobile::get_replenishment_for_transfer),
        )
        .route(
            "/api/mobile/replenishment/for-sales",
            post(mobile::get_replenishment_for_sales),
        )
        .route(
            "/api/mobile/stock-check/submit",
            post(mobile::submit_stock_check),
        )
        .route(
            "/api/mobile/stock-check/list",
            post(mobile::get_stock_check_history),
        )
        .route(
            "/api/mobile/stock-check/detail",
            post(mobile::get_stock_check_detail),
        )
        .route(
            "/api/mobile/stock-query",
            post(mobile::get_mobile_stock_query),
        )
        .route(
            "/api/mobile/special-price/submit",
            post(mobile::submit_special_price),
        )
        .route(
            "/api/mobile/special-price/list",
            post(mobile::get_special_price_history),
        )
        .route(
            "/api/mobile/reward/submit",
            post(mobile::submit_reward_product),
        )
        .route(
            "/api/mobile/reward/list",
            post(mobile::get_reward_product_history),
        )
        .route("/api/mobile/gift/submit", post(mobile::submit_gift_giving))
        .route(
            "/api/mobile/gift/list",
            post(mobile::get_gift_giving_history),
        )
        .route("/api/mobile/submit-batch", post(mobile::submit_batch))
        .route("/api/mobile/shortages", post(mobile::get_mobile_shortages))
        .route("/api/mobile/shortage/submit", post(mobile::submit_shortage))
        .route(
            "/api/mobile/shortage/list",
            post(mobile::get_shortage_report_history),
        )
        .route(
            "/api/mobile/commission",
            post(mobile::get_mobile_commission),
        )
        .route(
            "/api/mobile/sales-task/current",
            post(mobile::get_current_sales_task),
        )
        .route(
            "/api/mobile/sales-task/list",
            post(mobile::get_sales_task_list),
        )
        .route(
            "/api/mobile/sales-task/create",
            post(mobile::create_sales_task),
        )
        .route(
            "/api/mobile/sales-task/update",
            post(mobile::update_sales_task),
        )
        .route(
            "/api/mobile/sales-task/delete",
            post(mobile::delete_sales_task),
        )
        .route(
            "/api/mobile/sales-task/record",
            post(mobile::submit_daily_sales_record),
        )
        .route(
            "/api/mobile/sales-task/records",
            post(mobile::get_sales_task_records),
        )
        .route(
            "/api/mobile/home/stats",
            post(mobile::get_mobile_home_stats),
        )
        // ===== PC 端「手机数据」管理 =====
        .route("/api/mobile-data/list", post(mobile_data::list_mobile_data))
        .route(
            "/api/mobile-data/create",
            post(mobile_data::create_mobile_data),
        )
        .route(
            "/api/mobile-data/update",
            post(mobile_data::update_mobile_data),
        )
        .route(
            "/api/mobile-data/delete",
            post(mobile_data::delete_mobile_data),
        )
        .route(
            "/api/mobile-data/detail",
            post(mobile_data::get_mobile_data_detail),
        )
        // ===== Online =====
        .route("/api/online/products", post(online::get_online_products))
        .route(
            "/api/online/products/create",
            post(online::create_online_product),
        )
        .route(
            "/api/online/products/update",
            post(online::update_online_product),
        )
        .route(
            "/api/online/products/delete",
            post(online::delete_online_product),
        )
        .route(
            "/api/online/products/browse",
            post(online::browse_online_products),
        )
        .route(
            "/api/online/products/browse/detail",
            post(online::browse_online_product),
        )
        .route("/api/online/order/create", post(online::place_online_order))
        .route("/api/online/order/list", post(online::get_online_orders))
        .route("/api/online/order/my", post(online::get_my_online_orders))
        .route("/api/online/order/detail", post(online::get_online_order))
        .route(
            "/api/online/order/confirm",
            post(online::confirm_online_order),
        )
        .route(
            "/api/online/order/cancel",
            post(online::cancel_online_order),
        )
        .route(
            "/api/online/order/ship",
            post(online::update_online_order_ship_info),
        )
        .route(
            "/api/online/order/batch-ship",
            post(online::batch_update_online_order_ship_info),
        )
        .route(
            "/api/online/order/batch-generate-so",
            post(online::batch_generate_sales_orders),
        )
        .route(
            "/api/online/payment/configs",
            post(online::get_payment_configs),
        )
        .route(
            "/api/online/payment/config/create",
            post(online::create_payment_config),
        )
        .route(
            "/api/online/payment/config/update",
            post(online::update_payment_config),
        )
        .route(
            "/api/online/payment/config/delete",
            post(online::delete_payment_config),
        )
        .route(
            "/api/online/payment/methods",
            post(online::get_available_payment_methods),
        )
        .route(
            "/api/online/payment/create",
            post(online::create_online_order_payment),
        )
        .route(
            "/api/online/payment/status",
            post(online::query_payment_status),
        )
        .route(
            "/api/online/payment/proof",
            post(online::upload_payment_proof),
        )
        .route("/api/online/payment/verify", post(online::verify_payment))
        .route(
            "/api/online/payment/claim",
            post(online::claim_personal_payment),
        )
        .route("/api/online/address/list", post(online::get_addresses))
        .route("/api/online/address/create", post(online::create_address))
        .route("/api/online/address/update", post(online::update_address))
        .route("/api/online/address/delete", post(online::delete_address))
        .route(
            "/api/online/address/default",
            post(online::set_default_address),
        )
        .route("/api/online/regions", post(online::get_regions))
        // ===== Permission =====
        .route("/api/permission/roles", post(permission::get_roles))
        .route("/api/permission/role/create", post(permission::create_role))
        .route("/api/permission/role/update", post(permission::update_role))
        .route("/api/permission/role/delete", post(permission::delete_role))
        .route(
            "/api/permission/role-permissions",
            post(permission::get_role_permissions),
        )
        .route(
            "/api/permission/assign",
            post(permission::assign_role_permissions),
        )
        .route(
            "/api/permission/assign-user-roles",
            post(permission::assign_user_roles),
        )
        .route(
            "/api/permission/user-permissions",
            post(permission::get_user_permissions),
        )
        .route(
            "/api/permission/my-permissions",
            get(permission::get_my_permissions),
        )
        .route("/api/permission/menus", get(permission::get_permissions))
        .route(
            "/api/permission/overview",
            post(permission::get_system_overview),
        )
        .route(
            "/api/permission/company-name",
            post(permission::get_public_company_name),
        )
        .route(
            "/api/permission/warehouses",
            post(permission::get_public_warehouses),
        )
        .route(
            "/api/permission/table-column-config/get",
            post(permission::get_table_column_config),
        )
        .route(
            "/api/permission/table-column-config/save",
            post(permission::save_table_column_config),
        )
        .route(
            "/api/permission/table-column-config/delete",
            post(permission::delete_table_column_config),
        )
        .route(
            "/api/permission/column-preset/save",
            post(permission::save_column_preset),
        )
        .route(
            "/api/permission/column-preset/list",
            post(permission::list_column_presets),
        )
        .route(
            "/api/permission/column-preset/delete",
            post(permission::delete_column_preset),
        )
        .route(
            "/api/permission/column-preset/apply",
            post(permission::apply_column_preset),
        )
        .route(
            "/api/permission/column-preset/set-default",
            post(permission::set_default_preset),
        )
        // ===== Workspace =====
        .route("/api/workspace/todo", post(workspace::get_todo_list))
        .route("/api/workspace/doing", post(workspace::get_doing_list))
        .route("/api/workspace/menus", post(workspace::get_common_menus))
        // ===== System =====
        // 用户管理路由已移除：员工即用户，统一由 tBas_Emp 管理（/api/base-data/employee/*）
        .route("/api/system/role/list", post(system::get_role_list))
        .route("/api/system/menu/list", post(system::get_menu_list))
        .route("/api/system/dict/list", post(system::get_dictionary_list))
        .route("/api/system/log/list", post(system::get_oper_log_list))
        .route("/api/system/log/create", post(system::create_oper_log))
        .route("/api/system/log/delete", post(system::delete_oper_log))
        .route("/api/system/log/cleanup", post(system::cleanup_oper_log))
        .route("/api/system/params", post(system::get_sys_params))
        .route("/api/system/params/save", post(system::save_sys_params))
        .route("/api/system/params/list", post(system::list_system_params))
        .route(
            "/api/system/params/dict",
            post(system::get_system_params_dict),
        )
        .route("/api/system/params/update", post(system::save_system_param))
        .route(
            "/api/system/params/delete",
            post(system::delete_system_param),
        )
        // ===== Notification & Backup =====
        .route(
            "/api/notification/list",
            post(notification_backup::get_notifications),
        )
        .route(
            "/api/notification/create",
            post(notification_backup::create_notification),
        )
        .route(
            "/api/notification/read",
            post(notification_backup::mark_notification_read),
        )
        .route(
            "/api/notification/unread-count",
            post(notification_backup::get_unread_count),
        )
        .route("/api/backup/list", post(notification_backup::get_backups))
        .route(
            "/api/backup/create",
            post(notification_backup::create_backup),
        )
        .route(
            "/api/backup/delete",
            post(notification_backup::delete_backup),
        )
        .route(
            "/api/system-config",
            post(notification_backup::get_system_config),
        )
        .route(
            "/api/system-config/save",
            post(notification_backup::save_system_config),
        )
        // ===== Admin =====
        .route("/api/admin/check_triggers", post(admin::check_triggers))
        .route(
            "/api/admin/dashboard-stats",
            post(notification_backup::get_dashboard_stats),
        )
        .route(
            "/api/admin/reset-password",
            post(auth::admin_reset_password),
        )
        .layer(axum_mw::from_fn(
            erp_server::middleware::permission::permission_middleware,
        ))
        .layer(axum_mw::from_fn_with_state(
            config_clone.clone(),
            auth_middleware,
        ))
        .layer(axum_mw::from_fn_with_state(
            rate_limit_state,
            erp_server::middleware::rate_limit::smart_rate_limit,
        ))
        .with_state(config_clone.clone());

    // Merge public + protected
    let api = public_routes
        .merge(protected_routes)
        .layer(cors)
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024)) // 10MB 请求体限制
        .layer(axum_mw::from_fn(health::request_counter)) // 全局请求计数
        .layer(
            // 请求可观测性：记录每个 HTTP 请求的方法/路径/状态码/耗时
            TraceLayer::new_for_http()
                .make_span_with(|request: &axum::http::Request<_>| {
                    let span = tracing::info_span!(
                        "http_request",
                        method = %request.method(),
                        uri = %request.uri(),
                        status = tracing::field::Empty,
                        latency = tracing::field::Empty,
                    );
                    span
                })
                .on_response(
                    |response: &axum::http::Response<_>,
                     latency: std::time::Duration,
                     span: &tracing::Span| {
                        span.record("status", tracing::field::display(response.status()));
                        span.record("latency", latency.as_millis() as u64);
                        tracing::info!("HTTP {} {}ms", response.status(), latency.as_millis());
                    },
                )
                .on_failure(
                    |error, _latency: std::time::Duration, _span: &tracing::Span| {
                        tracing::error!("HTTP request failed: {}", error);
                    },
                ),
        );

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    tracing::info!("ERP server listening on {}", addr);
    // P0-3 修复：原 unwrap() 在端口被占用/权限不足时 panic，容器看不到错误原因
    // 改为输出 error 后 exit(1)，便于运维定位
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(
                "绑定端口 {} 失败: {}（可能端口被占用或权限不足）",
                config.port,
                e
            );
            std::process::exit(1);
        }
    };
    // P1-10: 启用 ConnectInfo，让 handler 可通过 ConnectInfo<SocketAddr> 获取客户端真实 IP
    // P1 修复：优雅停机——收到 SIGTERM（容器停止）/Ctrl-C 后停止接收新连接，
    // 等待进行中的请求（含库存过账事务）完成再退出，避免硬断连依赖 DB 端回滚。
    if let Err(e) = axum::serve(
        listener,
        api.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    {
        tracing::error!("HTTP 服务异常退出: {}", e);
        std::process::exit(1);
    }
}

/// 优雅停机信号：监听 Ctrl-C（全平台）与 SIGTERM（unix 容器）
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("收到 Ctrl-C，开始优雅停机（等待进行中请求完成）..."),
        _ = terminate => tracing::info!("收到 SIGTERM，开始优雅停机（等待进行中请求完成）..."),
    }
}
