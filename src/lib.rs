//! 库入口：把各子模块声明为 crate 公开模块
//!
//! 设计：把 main.rs 中的 mod 声明集中到 lib.rs，
//! 这样 integration tests 也能 import crate 模块
//! （binary crate 不可被集成测试引用，必须有 lib 入口）
//!
//! 包名：`erp_server`（见 Cargo.toml [package].name）

pub mod config;
pub mod db;
pub mod error;
pub mod handlers;
pub mod middleware;
pub mod utils;
