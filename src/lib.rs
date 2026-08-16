//! 库入口：把各子模块声明为 crate 公开模块
//!
//! 设计：把 main.rs 中的 mod 声明集中到 lib.rs，
//! 这样 integration tests 也能 import crate 模块
//! （binary crate 不可被集成测试引用，必须有 lib 入口）
//!
//! 包名：`erp_server`（见 Cargo.toml [package].name）

// 结构体字段名与 SQL Server 列名保持一致（StkID/EmpID/PassWordStr 等），
// 由 serde 直接映射，故统一放行 non_snake_case。
#![allow(non_snake_case)]
// ============================================================================
// 存量 clippy 风格债务集中放行（TODO: 逐步清理后逐项移除）
//
// 背景：本仓库历史上未在本地跑过 clippy，累计约 470 处纯风格类违规
// （详见 git 提交记录），批量 rustfix 在 json! 宏内会产生编译错误，
// 故先 crate 级放行这些"零语义影响"的机械 lint，保证 CI 门禁可落地；
// 其余 lint 仍保持 -D warnings 严格拦截新代码。
// 清理方式：删除下方任一 allow 后 `cargo clippy --fix` 该 lint 的命中项。
// ============================================================================
#![allow(clippy::collapsible_if)]
#![allow(clippy::collapsible_else_if)]
#![allow(clippy::collapsible_str_replace)]
#![allow(clippy::unnecessary_cast)]
#![allow(clippy::needless_borrow)]
#![allow(clippy::single_match)]
#![allow(clippy::question_mark)]
#![allow(clippy::get_first)]
#![allow(clippy::needless_return)]
#![allow(clippy::redundant_closure)]
#![allow(clippy::useless_format)]
#![allow(clippy::const_is_empty)]
#![allow(clippy::assertions_on_constants)]
#![allow(clippy::unnecessary_map_or)]
#![allow(clippy::unwrap_or_default)]
#![allow(clippy::manual_unwrap_or_default)]
#![allow(clippy::manual_strip)]
#![allow(clippy::manual_is_multiple_of)]
#![allow(clippy::manual_range_contains)]
#![allow(clippy::manual_ok_err)]
#![allow(clippy::manual_clamp)]
#![allow(clippy::let_and_return)]
#![allow(clippy::option_map_unit_fn)]
#![allow(clippy::option_as_ref_deref)]
#![allow(clippy::match_like_matches_macro)]
#![allow(clippy::iter_next_slice)]
#![allow(clippy::if_same_then_else)]
#![allow(clippy::doc_lazy_continuation)]
#![allow(clippy::bool_comparison)]
// axum handler 天然多参数（Path/State/Json/Extension 组合），字段类型跟随
// tiberius Row 映射也偏复杂，这两类设计型 lint 对本代码弊大于利
#![allow(clippy::too_many_arguments)]
#![allow(clippy::type_complexity)]
pub mod config;
pub mod db;
pub mod error;
pub mod handlers;
pub mod metadata;
pub mod middleware;
pub mod services;
pub mod utils;
