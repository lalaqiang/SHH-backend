use axum::{routing::{get, post}, Router, middleware as axum_middleware};
use std::net::SocketAddr;
use tower_http::cors::{Any, CorsLayer};
use erp_server::config::Config;
use erp_server::db::init_pool;
use erp_server::middleware::auth::auth_middleware;
use erp_server::utils::doc_no;
use erp_server::handlers;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_env();

    // 初始化数据库连接池
    init_pool(&config).await;

    // 启动时自检：缺失的关键表自动建（避免"对象名 xxx 无效"）
    auto_create_missing_tables().await;

    // 通用 CORS
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // 公开路由（不需要 JWT 验证）
    let public_routes = Router::new()
        .route("/api/login", post(handlers::auth::login))
        // 健康检查 + 监控指标（用于 K8s/Prometheus）
        .route("/api/health", get(handlers::health::health_check))
        .route("/api/metrics", get(handlers::health::metrics))
        // OpenAPI 文档（无需鉴权）
        .route("/api-docs", get(handlers::docs::swagger_ui))
        .route("/api-docs/", get(handlers::docs::swagger_ui))
        .route("/api-docs/openapi.yaml", get(handlers::docs::openapi_yaml))
        .route("/api-docs/openapi.json", get(handlers::docs::openapi_json))
        .route("/api-docs/swagger-ui", get(handlers::docs::swagger_ui_html))
        .route("/api-docs/redoc", get(handlers::docs::redoc_html));

    // 受保护路由（全部需要 JWT 验证）
    let protected_routes = Router::new()
        .route("/api/auth/me", get(handlers::auth::user_info))
        .route("/api/auth/logout", post(handlers::auth::logout))
        .route("/api/auth/change_password", post(handlers::auth::change_password))

        // ===== 通用 CRUD =====
        .route("/api/generic/query", post(handlers::generic::generic_query))
        .route("/api/generic/create", post(handlers::generic::generic_create))
        .route("/api/generic/update", post(handlers::generic::generic_update))
        .route("/api/generic/delete", post(handlers::generic::generic_delete))
        .route("/api/generic/restore", post(handlers::generic::generic_restore))
        .route("/api/generic/import", post(handlers::generic::generic_import))
        .route("/api/generic/batch_update", post(handlers::generic::generic_batch_update))
        .route("/api/generic/tree", post(handlers::generic::generic_tree))
        .route("/api/generic/export", post(handlers::generic::generic_export))
        .route("/api/generic/import_template", post(handlers::generic::generic_import_template))
        .route("/api/generic/import_excel", post(handlers::generic::generic_import_excel))
        .route("/api/generic/export_excel", post(handlers::generic::generic_export_excel))
        .route("/api/generic/oper_log", post(handlers::generic::generic_oper_log))

        // ===== 文档管理（单据编号）=====
        .route("/api/doc/approve", post(handlers::approval::approve_doc))
        .route("/api/doc/unapprove", post(handlers::approval::unapprove_doc))
        .route("/api/doc/generate-no", post(doc_no::generate_doc_no))
        .route("/api/doc/list-types", get(doc_no::list_doc_types))
        .route("/api/doc/reset-seq", post(doc_no::reset_doc_seq))
        .route("/api/doc/print-log", post(handlers::system::save_print_log))

        // ===== 基础资料 =====
        .route("/api/goods/list", post(handlers::base_data::get_goods))
        .route("/api/goods/create", post(handlers::base_data::create_goods))
        .route("/api/goods/update", post(handlers::base_data::update_goods))
        .route("/api/goods/delete", post(handlers::base_data::delete_goods))
        .route("/api/cust/list", post(handlers::base_data::get_customers))
        .route("/api/cust/create", post(handlers::base_data::create_customer))
        .route("/api/cust/update", post(handlers::base_data::update_customer))
        .route("/api/cust/delete", post(handlers::base_data::delete_customer))
        .route("/api/supp/list", post(handlers::base_data::get_suppliers))
        .route("/api/supp/delete", post(handlers::base_data::delete_supplier))
        .route("/api/stock/list", post(handlers::base_data::get_warehouses))
        .route("/api/stock/delete", post(handlers::base_data::delete_warehouse))
        // 别名：与前端 getWarehouseList / deleteWarehouse 调用路径一致
        .route("/api/warehouses", post(handlers::base_data::get_warehouses))
        .route("/api/warehouse/delete", post(handlers::base_data::delete_warehouse))
        .route("/api/brand/list", post(handlers::base_data::get_brands))
        .route("/api/emp/list", post(handlers::base_data::get_employees))
        .route("/api/tables", get(handlers::base_data::get_tables))
        .route("/api/table_data", post(handlers::base_data::get_table_data))
        .route("/api/base_versions", post(handlers::base_data::get_base_versions))
        .route("/api/retail/goods_search", post(handlers::base_data::retail_goods_search))
        .route("/api/retail/sales_settle", post(handlers::base_data::retail_sales_settle))
        .route("/api/dashboard/stats", post(handlers::base_data::get_dashboard_stats))
        .route("/api/report/sales_analysis", post(handlers::base_data::get_sales_analysis))
        .route("/api/report/purchase_analysis", post(handlers::base_data::get_purchase_analysis))
        .route("/api/report/profit_analysis", post(handlers::base_data::get_profit_analysis))

        // ===== 库存查询/调整 =====
        .route("/api/inventory/stock", post(handlers::base_data::get_inventory_stock))
        .route("/api/inventory/stock/summary", post(handlers::base_data::get_stock_summary))
        .route("/api/inventory/flow", post(handlers::inventory::get_stock_flow))
        .route("/api/inventory/adjust", post(handlers::inventory::inventory_adjust))
        .route("/api/inventory/month_settle", post(handlers::inventory::month_settle))
        .route("/api/inventory/low_stock_alert", post(handlers::inventory::low_stock_alert))
        .route("/api/inventory/replenish_from_alert", post(handlers::inventory::replenish_from_alert))

        // ===== 运维管理 =====
        .route("/api/admin/check_triggers", post(handlers::admin::check_triggers))

        // ===== 系统参数（tSys_Parameters / tSys_Params）=====
        .route("/api/system/params/list", post(handlers::system::list_system_params))
        .route("/api/system/params/dict", post(handlers::system::get_system_params_dict))
        .route("/api/system/params/save", post(handlers::system::save_system_param))
        .route("/api/system/params/delete", post(handlers::system::delete_system_param))
        .route("/api/system/sys_params/get", post(handlers::system::get_sys_params))
        .route("/api/system/sys_params/save", post(handlers::system::save_sys_params))

        // ===== 入出库单 =====
        .route("/api/inventory/io/list", post(handlers::inventory::get_io_list))
        .route("/api/inventory/io/create", post(handlers::inventory::create_io))
        .route("/api/inventory/io/update", post(handlers::inventory::update_io))
        .route("/api/inventory/io/detail", post(handlers::inventory::get_io_detail))
        .route("/api/inventory/io/delete", post(handlers::inventory::delete_io))
        // ===== 调拨单 =====
        .route("/api/inventory/move/list", post(handlers::inventory::get_move_list))
        .route("/api/inventory/move/create", post(handlers::inventory::create_move))
        .route("/api/inventory/move/update", post(handlers::inventory::update_move))
        .route("/api/inventory/move/detail", post(handlers::inventory::get_move_detail))
        .route("/api/inventory/move/delete", post(handlers::inventory::delete_move))
        // ===== 盘点单 =====
        .route("/api/inventory/check/list", post(handlers::inventory::get_check_list))
        .route("/api/inventory/check/create", post(handlers::inventory::create_check))
        .route("/api/inventory/check/update", post(handlers::inventory::update_check))
        .route("/api/inventory/check/detail", post(handlers::inventory::get_check_detail))
        .route("/api/inventory/check/delete", post(handlers::inventory::delete_check))
        // ===== 补货申请 =====
        .route("/api/inventory/replenish/list", post(handlers::inventory::get_replenish_list))
        .route("/api/inventory/replenish/create", post(handlers::inventory::create_replenish))

        // ===== 采购单据 =====
        .route("/api/purchase/orders/list", post(handlers::purchase::get_purchase_orders))
        .route("/api/purchase/order/create", post(handlers::purchase::create_purchase_order))
        .route("/api/purchase/order/update", post(handlers::purchase::update_purchase_order))
        .route("/api/purchase/inbound/list", post(handlers::purchase::get_purchase_inbound))
        .route("/api/purchase/inbound/create", post(handlers::purchase::create_purchase_inbound))
        .route("/api/purchase/return/list", post(handlers::purchase::get_purchase_return))
        .route("/api/purchase/return/create", post(handlers::purchase::create_purchase_return))
        .route("/api/purchase/quote/list", post(handlers::purchase::get_purchase_quotes))
        .route("/api/purchase/quote/create", post(handlers::purchase::create_purchase_quote))
        .route("/api/purchase/quote/update", post(handlers::purchase::update_purchase_quote))
        .route("/api/purchase/adjprice/list", post(handlers::purchase::get_purchase_adjprice))
        .route("/api/purchase/adjprice/create", post(handlers::purchase::create_purchase_adjprice))
        .route("/api/purchase/query", post(handlers::purchase::get_purchase_query))

        // ===== 销售单据 =====
        .route("/api/sales/orders/list", post(handlers::sales::get_sales_orders))
        .route("/api/sales/order/create", post(handlers::sales::create_sales_order))
        .route("/api/sales/order/update", post(handlers::sales::update_sales_order))
        .route("/api/sales/outbound/list", post(handlers::sales::get_sales_outbound))
        .route("/api/sales/outbound/create", post(handlers::sales::create_sales_outbound))
        .route("/api/sales/outbound/update", post(handlers::sales::update_sales_outbound))
        .route("/api/sales/return/list", post(handlers::sales_return::list_sales_return))
        .route("/api/sales/return/create", post(handlers::sales_return::create_sales_return))
        .route("/api/sales/return/update", post(handlers::sales_return::update_sales_return))
        .route("/api/sales/quote/list", post(handlers::sales::get_sales_quotes))
        .route("/api/sales/quote/create", post(handlers::sales::create_sales_quote))
        .route("/api/sales/quote/update", post(handlers::sales::update_sales_quote))
        .route("/api/sales/adjprice/list", post(handlers::sales::get_sales_adjprice))
        .route("/api/sales/adjprice/create", post(handlers::sales::create_sales_adjprice))

        // ===== 销售员业绩 =====
        .route("/api/sales_input/emp_sales/list", post(handlers::sales_input::list_emp_sales))
        .route("/api/sales_input/emp_sales/create", post(handlers::sales_input::create_emp_sales))
        .route("/api/sales_input/emp_sales/update", post(handlers::sales_input::update_emp_sales))

        // ===== 会员管理 =====
        .route("/api/vip/list", post(handlers::vip::list_vip))
        .route("/api/vip/create", post(handlers::vip::create_vip))
        .route("/api/vip/update", post(handlers::vip::update_vip))
        .route("/api/vip/delete", post(handlers::vip::delete_vip))

        // ===== 补全 P0-1：系统管理 =====
        .route("/api/system/user/list",         post(handlers::system::get_user_list))
        .route("/api/system/user/create",       post(handlers::system::create_user))
        .route("/api/system/user/update",       post(handlers::system::update_user))
        .route("/api/system/user/delete",       post(handlers::system::delete_user))
        .route("/api/system/role/list",         post(handlers::system::get_role_list))
        .route("/api/system/menu/list",         post(handlers::system::get_menu_list))
        .route("/api/system/dictionary/list",   post(handlers::system::get_dictionary_list))
        .route("/api/system/oper-log/list",     post(handlers::system::get_oper_log_list))

        // ===== 补全 P0-2：OA 中心 =====
        .route("/api/oa/workflow/list",         post(handlers::oa::get_workflow_list))
        .route("/api/oa/workflow/approve",      post(handlers::oa::approve_workflow))
        .route("/api/oa/notice/list",           post(handlers::oa::get_notice_list))
        .route("/api/oa/email/list",            post(handlers::oa::get_email_list))

        // ===== 补全 P0-3：财务系统 =====
        .route("/api/finance/receivable/list",  post(handlers::finance::get_receivable_list))
        .route("/api/finance/payable/list",     post(handlers::finance::get_payable_list))
        .route("/api/finance/receipt/list",     post(handlers::finance::get_receipt_list))
        .route("/api/finance/receipt/create",   post(handlers::finance::create_receipt))
        .route("/api/finance/receipt/update",   post(handlers::finance::update_receipt))
        .route("/api/finance/receipt/delete",   post(handlers::finance::delete_receipt))
        .route("/api/finance/receipt/audit",    post(handlers::finance::audit_receipt))
        .route("/api/finance/payment/list",     post(handlers::finance::get_payment_list))
        .route("/api/finance/payment/create",   post(handlers::finance::create_payment))
        .route("/api/finance/payment/update",   post(handlers::finance::update_payment))
        .route("/api/finance/payment/delete",   post(handlers::finance::delete_payment))
        .route("/api/finance/payment/audit",    post(handlers::finance::audit_payment))
        .route("/api/finance/cash-flow/list",   post(handlers::finance::get_cash_flow_list))
        .route("/api/finance/payable/process-payment", post(handlers::finance::process_payable_payment))
        .route("/api/finance/payable/writeoff", post(handlers::finance::writeoff_payable))
        .route("/api/finance/payable/adjust",   post(handlers::finance::adjust_payable))
        .route("/api/finance/receivable/process-refund", post(handlers::finance::process_receivable_refund))
        .route("/api/finance/receivable/writeoff", post(handlers::finance::writeoff_receivable))
        .route("/api/finance/receivable/adjust", post(handlers::finance::adjust_receivable))
        .route("/api/finance/overdue-accounts", get(handlers::finance::get_overdue_accounts))
        // ===== 派生 AR/AP（方案 B：从 tStk_IO 实时汇总，无需 tFin_Receivable/Payable）=====
        .route("/api/finance/ar/customer",       get(handlers::finance::get_customer_ar))
        .route("/api/finance/ar/customer/detail",get(handlers::finance::get_customer_ar_detail))
        .route("/api/finance/ap/supplier",       get(handlers::finance::get_supplier_ap))
        .route("/api/finance/ap/supplier/detail",get(handlers::finance::get_supplier_ap_detail))

        // ===== 补全 P0-4：报表中心 =====
        .route("/api/report/purchase",          post(handlers::report::get_purchase_report))
        .route("/api/report/sales",             post(handlers::report::get_sales_report))
        .route("/api/report/business",          post(handlers::report::get_business_report))
        .route("/api/report/stock",             post(handlers::report::get_stock_report))

        // ===== 补全 P0-5：零售收银 =====
        .route("/api/retail/sale",              post(handlers::retail::retail_sale))
        .route("/api/retail/cashier/info",      post(handlers::retail::get_cashier_info))

        // ===== 补全 P0-6：订单流程（P1 增强）=====
        .route("/api/inventory/available",      post(handlers::order_flow::query_available))
        .route("/api/order/source-detail",      post(handlers::order_flow::query_source_detail))

        // ===== 补全 P0-7：基础数据 create/update 缺失接口 =====
        // 客户（已有 create/update 单独接口但未注册）
        .route("/api/base/customer/create",     post(handlers::base_data::create_customer))
        .route("/api/base/customer/update",     post(handlers::base_data::update_customer))
        .route("/api/base/customer/delete",     post(handlers::base_data::delete_customer))
        // P2-3 新增：供应商/仓库/品牌/员工
        .route("/api/base/supplier/create",     post(handlers::base_data::create_supplier))
        .route("/api/base/supplier/update",     post(handlers::base_data::update_supplier))
        .route("/api/base/warehouse/create",    post(handlers::base_data::create_warehouse))
        .route("/api/base/warehouse/update",    post(handlers::base_data::update_warehouse))
        .route("/api/base/brand/create",        post(handlers::base_data::create_brand))
        .route("/api/base/brand/update",        post(handlers::base_data::update_brand))
        .route("/api/base/employee/create",     post(handlers::base_data::create_employee))
        .route("/api/base/employee/update",     post(handlers::base_data::update_employee))
        // 前端常用的简写别名（/api/customers、/api/suppliers、/api/employees、/api/brands）
        // 暂保留 /api/cust/list 等长路径，前端路径不一致待统一

        // ===== 补全 P0-8：基础数据别名（兼容前端简写路径）=====
        .route("/api/customers",                post(handlers::base_data::get_customers))
        .route("/api/suppliers",                post(handlers::base_data::get_suppliers))
        .route("/api/employees",                post(handlers::base_data::get_employees))
        .route("/api/brands",                   post(handlers::base_data::get_brands))
        .route("/api/goods",                    post(handlers::base_data::get_goods))

        // ===== 补全 P0-9：销售简化路径（前端老 API）=====
        .route("/api/sales/orders",             post(handlers::sales::get_sales_orders))
        .route("/api/sales/orders/create",      post(handlers::sales::create_sales_order))
        .route("/api/sales/orders/update",      post(handlers::sales::update_sales_order))
        .route("/api/sales/outbound",           post(handlers::sales::get_sales_outbound))
        .route("/api/sales/quotes",             post(handlers::sales::get_sales_quotes))
        .route("/api/sales/quotes/create",      post(handlers::sales::create_sales_quote))
        .route("/api/sales/quotes/update",      post(handlers::sales::update_sales_quote))

        // ===== 补全 P0-10：采购简化路径 =====
        .route("/api/purchase/orders",          post(handlers::purchase::get_purchase_orders))
        .route("/api/purchase/inbound",         post(handlers::purchase::get_purchase_inbound))
        .route("/api/purchase/return",          post(handlers::purchase::get_purchase_return))
        .route("/api/purchase/quote",           post(handlers::purchase::get_purchase_quotes))
        .route("/api/purchase/adjprice",        post(handlers::purchase::get_purchase_adjprice))

        // ===== 补全 P1-1：打印系统 =====
        .route("/api/print/templates",          post(handlers::print::get_print_templates))
        .route("/api/print/template/get",       post(handlers::print::get_print_template))
        .route("/api/print/template/create",    post(handlers::print::create_print_template))
        .route("/api/print/template/update",    post(handlers::print::update_print_template))
        .route("/api/print/template/delete",    post(handlers::print::delete_print_template))
        .route("/api/print/config",             post(handlers::print::get_print_config))
        .route("/api/print/config/save",        post(handlers::print::save_print_config))
        .route("/api/print/logs",               post(handlers::print::get_print_logs))
        .route("/api/print/log/create",         post(handlers::print::create_print_log))
        .route("/api/print/versions",           post(handlers::print::get_print_versions))
        .route("/api/print/version/create",     post(handlers::print::create_print_version))
        .route("/api/print/version/rollback",   post(handlers::print::rollback_print_version))

        // ===== 补全 P1-2：权限管理 =====
        .route("/api/permission/menus",                  post(handlers::permission::get_permissions))
        .route("/api/permission/role-permissions",       post(handlers::permission::get_role_permissions))
        .route("/api/permission/assign-role-permissions", post(handlers::permission::assign_role_permissions))
        .route("/api/permission/user-permissions",       post(handlers::permission::get_user_permissions))
        .route("/api/permission/roles",                  post(handlers::permission::get_roles))
        .route("/api/permission/role/create",            post(handlers::permission::create_role))
        .route("/api/permission/role/update",            post(handlers::permission::update_role))
        .route("/api/permission/role/delete",            post(handlers::permission::delete_role))
        .route("/api/permission/assign-user-roles",      post(handlers::permission::assign_user_roles))
        .route("/api/permission/table-column-config/save",   post(handlers::permission::save_table_column_config))
        .route("/api/permission/table-column-config/get",   post(handlers::permission::get_table_column_config))
        .route("/api/permission/table-column-config/delete", post(handlers::permission::delete_table_column_config))
        .route("/api/permission/column-preset/save",     post(handlers::permission::save_column_preset))
        .route("/api/permission/column-preset/list",     post(handlers::permission::list_column_presets))
        .route("/api/permission/column-preset/delete",   post(handlers::permission::delete_column_preset))
        .route("/api/permission/column-preset/apply",    post(handlers::permission::apply_column_preset))
        .route("/api/permission/uploaded-files",         post(handlers::permission::get_uploaded_files))
        .route("/api/permission/system-overview",        get(handlers::permission::get_system_overview))
        .route("/api/permission/public/company-name",    get(handlers::permission::get_public_company_name))
        .route("/api/permission/public/warehouses",      get(handlers::permission::get_public_warehouses))

        // ===== 补全 P1-3：工作台 =====
        .route("/api/workspace/todo",           post(handlers::workspace::get_todo_list))
        .route("/api/workspace/doing",          post(handlers::workspace::get_doing_list))
        .route("/api/workspace/common-menus",   post(handlers::workspace::get_common_menus))

        // ===== 补全 P1-4：通知系统 =====
        .route("/api/notification/list",        post(handlers::notification_backup::get_notifications))
        .route("/api/notification/create",      post(handlers::notification_backup::create_notification))
        .route("/api/notification/read",        post(handlers::notification_backup::mark_notification_read))
        .route("/api/notification/unread-count", post(handlers::notification_backup::get_unread_count))

        // ===== 补全 P1-5：备份系统 =====
        .route("/api/backup/list",              post(handlers::notification_backup::get_backups))
        .route("/api/backup/create",            post(handlers::notification_backup::create_backup))
        .route("/api/backup/delete",            post(handlers::notification_backup::delete_backup))

        // ===== 补全 P1-6：系统配置 =====
        .route("/api/system/config",            post(handlers::notification_backup::get_system_config))
        .route("/api/system/config/save",       post(handlers::notification_backup::save_system_config))

        // ===== 补全 P1-7：提成模板 =====
        .route("/api/commission/templates",     post(handlers::commission_pricing::get_commission_templates))
        .route("/api/commission/template/create", post(handlers::commission_pricing::create_commission_template))
        .route("/api/commission/template/update", post(handlers::commission_pricing::update_commission_template))
        .route("/api/commission/template/delete", post(handlers::commission_pricing::delete_commission_template))
        .route("/api/commission/rules",         post(handlers::commission_pricing::get_commission_rules))

        // ===== 补全 P1-8：定价模板 =====
        .route("/api/pricing/templates",        post(handlers::commission_pricing::get_pricing_templates))
        .route("/api/pricing/template/create",  post(handlers::commission_pricing::create_pricing_template))
        .route("/api/pricing/template/update",  post(handlers::commission_pricing::update_pricing_template))
        .route("/api/pricing/template/delete",  post(handlers::commission_pricing::delete_pricing_template))
        .route("/api/pricing/rules",            post(handlers::commission_pricing::get_pricing_rules))
        .route("/api/pricing/customer-prices",  post(handlers::commission_pricing::get_customer_prices))
        .route("/api/pricing/customer-price/save", post(handlers::commission_pricing::save_customer_price))

        // ===== 补全 P1-9：手机端（26 个）=====
        .route("/api/mobile/login",                 post(handlers::mobile::mobile_login))
        .route("/api/mobile/register",              post(handlers::mobile::mobile_register))
        .route("/api/mobile/change-password",       post(handlers::mobile::mobile_change_password))
        .route("/api/mobile/sync-base-data",        post(handlers::mobile::sync_base_data))
        .route("/api/mobile/replenishment/submit",  post(handlers::mobile::submit_replenishment))
        .route("/api/mobile/replenishment/history", post(handlers::mobile::get_replenishment_history))
        .route("/api/mobile/replenishment/transfer", post(handlers::mobile::get_replenishment_for_transfer))
        .route("/api/mobile/replenishment/sales",   post(handlers::mobile::get_replenishment_for_sales))
        .route("/api/mobile/stock-check/submit",    post(handlers::mobile::submit_stock_check))
        .route("/api/mobile/stock-check/history",   post(handlers::mobile::get_stock_check_history))
        .route("/api/mobile/stock-query",           post(handlers::mobile::get_mobile_stock_query))
        .route("/api/mobile/special-price/submit",  post(handlers::mobile::submit_special_price))
        .route("/api/mobile/special-price/history", post(handlers::mobile::get_special_price_history))
        .route("/api/mobile/reward-product/submit", post(handlers::mobile::submit_reward_product))
        .route("/api/mobile/reward-product/history", post(handlers::mobile::get_reward_product_history))
        .route("/api/mobile/gift-giving/submit",    post(handlers::mobile::submit_gift_giving))
        .route("/api/mobile/gift-giving/history",   post(handlers::mobile::get_gift_giving_history))
        .route("/api/mobile/shortages",             post(handlers::mobile::get_mobile_shortages))
        .route("/api/mobile/commission",            post(handlers::mobile::get_mobile_commission))
        .route("/api/mobile/sales-task/current",    post(handlers::mobile::get_current_sales_task))
        .route("/api/mobile/sales-task/list",       post(handlers::mobile::get_sales_task_list))
        .route("/api/mobile/sales-task/create",     post(handlers::mobile::create_sales_task))
        .route("/api/mobile/sales-task/update",     post(handlers::mobile::update_sales_task))
        .route("/api/mobile/sales-task/delete",     post(handlers::mobile::delete_sales_task))
        .route("/api/mobile/sales-record/submit",   post(handlers::mobile::submit_daily_sales_record))
        .route("/api/mobile/sales-record/list",     post(handlers::mobile::get_sales_task_records))

        // ===== 补全 P1-10：线上商城（33 个）=====
        .route("/api/online/products",                 post(handlers::online::get_online_products))
        .route("/api/online/product/get",              post(handlers::online::get_online_product))
        .route("/api/online/product/create",           post(handlers::online::create_online_product))
        .route("/api/online/product/update",           post(handlers::online::update_online_product))
        .route("/api/online/product/delete",           post(handlers::online::delete_online_product))
        .route("/api/online/browse",                   post(handlers::online::browse_online_products))
        .route("/api/online/browse/get",               post(handlers::online::browse_online_product))
        .route("/api/online/order/place",              post(handlers::online::place_online_order))
        .route("/api/online/orders",                   post(handlers::online::get_online_orders))
        .route("/api/online/my-orders",                post(handlers::online::get_my_online_orders))
        .route("/api/online/order/get",                post(handlers::online::get_online_order))
        .route("/api/online/order/confirm",            post(handlers::online::confirm_online_order))
        .route("/api/online/order/cancel",             post(handlers::online::cancel_online_order))
        .route("/api/online/order/ship-info",          post(handlers::online::update_online_order_ship_info))
        .route("/api/online/order/batch-ship-info",    post(handlers::online::batch_update_online_order_ship_info))
        .route("/api/online/order/batch-generate-sales", post(handlers::online::batch_generate_sales_orders))
        .route("/api/online/payment/configs",          post(handlers::online::get_payment_configs))
        .route("/api/online/payment/config/get",       post(handlers::online::get_payment_config))
        .route("/api/online/payment/config/create",    post(handlers::online::create_payment_config))
        .route("/api/online/payment/config/update",    post(handlers::online::update_payment_config))
        .route("/api/online/payment/config/delete",    post(handlers::online::delete_payment_config))
        .route("/api/online/payment/methods",          post(handlers::online::get_available_payment_methods))
        .route("/api/online/payment/create",           post(handlers::online::create_online_order_payment))
        .route("/api/online/payment/status",           post(handlers::online::query_payment_status))
        .route("/api/online/payment/upload-proof",     post(handlers::online::upload_payment_proof))
        .route("/api/online/payment/verify",           post(handlers::online::verify_payment))
        .route("/api/online/payment/claim-personal",    post(handlers::online::claim_personal_payment))
        .route("/api/online/addresses",                post(handlers::online::get_addresses))
        .route("/api/online/address/create",           post(handlers::online::create_address))
        .route("/api/online/address/update",           post(handlers::online::update_address))
        .route("/api/online/address/delete",           post(handlers::online::delete_address))
        .route("/api/online/address/set-default",      post(handlers::online::set_default_address))
        .route("/api/online/regions",                  post(handlers::online::get_regions));

    // 把 auth_middleware 只挂在受保护路由上（不影响 login）
    let protected_routes = protected_routes.layer(axum_middleware::from_fn_with_state(config.clone(), auth_middleware));

    let app = public_routes
        .merge(protected_routes)
        .layer(cors)
        .with_state(config);

    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    println!("ERP 后端服务启动: http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

/// 启动时自检：缺失的关键系统表自动建好（IF NOT EXISTS 模式，安全幂等）
/// 避免前端调接口时炸"对象名 'xxx' 无效"
async fn auto_create_missing_tables() {
    println!("=================================================================");
    println!("[auto_create] 启动自检：缺失的系统表自动建");
    println!("=================================================================");
    use erp_server::db::get_pool;
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[auto_create] 获取连接失败, 跳过: {}", e);
            return;
        }
    };
    // 打印当前 DB 名称，确认我们在对的数据库
    if let Ok(stream) = conn.query("SELECT DB_NAME() AS db", &[]).await {
        if let Ok(Some(r)) = stream.into_row().await {
            let db: &str = r.get::<&str, _>("db").unwrap_or("?");
            println!("[auto_create] 当前数据库: {}", db);
        }
    }
    println!("[auto_create] 已拿到连接, 准备执行 DDL");

    // 用 batch 一次性跑（多句之间用 GO 隔开），避免 tiberius 单语句模式 split 报错
    let batch_sql = r#"
IF NOT EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tSys_TableColumnConfig' AND xtype = 'U')
BEGIN
    CREATE TABLE tSys_TableColumnConfig (
        ColumnConfigID uniqueidentifier PRIMARY KEY DEFAULT NEWID(),
        EmpID          uniqueidentifier NULL,
        TableName      nvarchar(100)    NULL,
        ConfigData     nvarchar(max)    NULL,
        LUTime         datetime         DEFAULT GETDATE()
    )
END
IF NOT EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tSys_UploadFile' AND xtype = 'U')
BEGIN
    CREATE TABLE tSys_UploadFile (
        FileID   uniqueidentifier PRIMARY KEY DEFAULT NEWID(),
        FileName nvarchar(200)    NULL,
        FilePath nvarchar(500)    NULL,
        FileSize int              DEFAULT 0,
        FileType nvarchar(50)     NULL,
        BizType  nvarchar(50)     NULL,
        BizID    uniqueidentifier NULL,
        State    char(1)          DEFAULT 'A',
        EUser    nvarchar(50)     NULL,
        EDate    datetime         DEFAULT GETDATE(),
        LUTime   datetime         DEFAULT GETDATE()
    )
END
"#;
    match conn.execute(batch_sql, &[]).await {
        Ok(_) => println!("[auto_create] DDL 批量执行成功 (tSys_TableColumnConfig / tSys_UploadFile)"),
        Err(e) => eprintln!("[auto_create] DDL 批量执行失败: {}", e),
    }

    // 单独加索引
    let idx = r#"
IF NOT EXISTS (SELECT 1 FROM sysindexes WHERE name = 'IX_tSys_TableColumnConfig_EmpTable' AND id = OBJECT_ID('tSys_TableColumnConfig'))
BEGIN
    CREATE UNIQUE NONCLUSTERED INDEX IX_tSys_TableColumnConfig_EmpTable
    ON tSys_TableColumnConfig (EmpID, TableName)
    WHERE EmpID IS NOT NULL
END
"#;
    match conn.execute(idx, &[]).await {
        Ok(_) => println!("[auto_create] 索引 IX_tSys_TableColumnConfig_EmpTable OK"),
        Err(e) => eprintln!("[auto_create] 索引创建失败(可忽略): {}", e),
    }

    // tSys_ColumnPreset 列预设表
    let preset_sql = r#"
IF NOT EXISTS (SELECT * FROM sysobjects WHERE name='tSys_ColumnPreset' AND xtype='U')
CREATE TABLE tSys_ColumnPreset (
    PresetID   uniqueidentifier PRIMARY KEY DEFAULT NEWID(),
    EmpID      uniqueidentifier NULL,
    TableName  nvarchar(100)    NULL,
    PresetName nvarchar(100)    NULL,
    ConfigData nvarchar(max)    NULL,
    IsDefault  bit DEFAULT 0,
    LUTime     datetime DEFAULT GETDATE()
)
"#;
    match conn.execute(preset_sql, &[]).await {
        Ok(_) => println!("[auto_create] tSys_ColumnPreset 表创建(或已存在)"),
        Err(e) => eprintln!("[auto_create] tSys_ColumnPreset DDL 失败: {}", e),
    }

    let preset_idx = r#"
IF NOT EXISTS (SELECT * FROM sys.indexes WHERE name='IX_tSys_ColumnPreset_EmpTable' AND object_id=OBJECT_ID('tSys_ColumnPreset'))
CREATE UNIQUE INDEX IX_tSys_ColumnPreset_EmpTable ON tSys_ColumnPreset(EmpID, TableName, PresetName)
"#;
    match conn.execute(preset_idx, &[]).await {
        Ok(_) => println!("[auto_create] 索引 IX_tSys_ColumnPreset_EmpTable OK"),
        Err(e) => eprintln!("[auto_create] 索引 IX_tSys_ColumnPreset_EmpTable 失败(可忽略): {}", e),
    }

    // 验一下表真的在
    match conn.query(
        "SELECT name FROM sysobjects WHERE name IN ('tSys_TableColumnConfig', 'tSys_UploadFile') AND xtype = 'U'",
        &[]
    ).await {
        Ok(stream) => {
            let rows: Vec<tiberius::Row> = stream.into_first_result().await.unwrap_or_default();
            let names: Vec<String> = rows.iter()
                .filter_map(|r| r.get::<&str, _>("name").map(String::from))
                .collect();
            println!("[auto_create] 自检结果, 已存在的系统表: {:?}", names);
        }
        Err(e) => eprintln!("[auto_create] 验证查询失败: {}", e),
    }
    println!("[auto_create] 自检完成");
    println!("=================================================================");
}
