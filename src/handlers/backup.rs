//! 数据库备份管理（文件即真相）
//!
//! 设计：
//!   - 备份目录由 BACKUP_DIR 配置（默认 ./backups），备份文件即 `*.bak` 落盘文件，
//!     列表直接扫描目录生成（文件名/大小/mtime），不依赖任何 DB 表——
//!     避免"表里有记录、磁盘上没有文件"的假备份。
//!   - 文件名由服务端生成：`{库名}_{yyyymmdd_HHMMSS}[_{标签}].bak`，
//!     下载/删除只接受目录内合法文件名，杜绝路径穿越。
//!   - 定时自动备份：BACKUP_AUTO_ENABLED=true 时每天 BACKUP_AUTO_HOUR 点执行，
//!     标签为 auto；保留期清理只针对自动备份（手动备份不自动删）。
//!   - 恢复不提供一键还原（RESTORE 需独占库，失败会把库留在 RESTORING 态锁死系统），
//!     由前端"恢复指南"给出标准 SQL 与下载入口。
//!
//! 权限：全部端点映射 system.backup.manage（见 middleware/permission.rs）。

use axum::{
    Json,
    body::Body,
    extract::{Extension, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use tokio_util::io::ReaderStream;

use crate::config::Config;
use crate::db::get_pool;
use crate::error::Result;
use crate::middleware::auth::Claims;
use crate::utils::ApiResponse;

// ============================================================
// 文件名与路径安全
// ============================================================

/// 备份文件名合法字符：字母/数字/下划线/连字符/点，且必须 .bak 结尾、
/// 不含路径分隔符、不以点开头（防 `.bak` 之类的怪名与 `..` 穿越）
pub fn is_valid_backup_filename(name: &str) -> bool {
    if name.len() < 5 || !name.to_ascii_lowercase().ends_with(".bak") {
        return false;
    }
    if name.starts_with('.') || name.contains("..") {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
}

/// 清洗用户提供的备份标签：仅保留字母/数字/下划线/连字符，最长 30 字符；
/// 清洗后为空则返回 None（无标签）
pub fn sanitize_label(label: &str) -> Option<String> {
    let cleaned: String = label
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .take(30)
        .collect();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

/// 将文件名解析到备份目录内的绝对路径（要求文件已存在，用于下载/删除/验证）。
/// 通过 canonicalize + 前缀比对双重防路径穿越。
fn resolve_existing_in_dir(dir: &Path, name: &str) -> std::result::Result<PathBuf, String> {
    if !is_valid_backup_filename(name) {
        return Err(format!("非法备份文件名: {}", name));
    }
    let dir_canon = dir
        .canonicalize()
        .map_err(|_| "备份目录不存在或不可访问".to_string())?;
    let full = dir_canon.join(name);
    let canon = full
        .canonicalize()
        .map_err(|_| format!("备份文件不存在: {}", name))?;
    if !canon.starts_with(&dir_canon) || !canon.is_file() {
        return Err(format!("路径越界或不是文件: {}", name));
    }
    Ok(canon)
}

/// 确保备份目录存在
fn ensure_backup_dir(dir: &Path) -> std::result::Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("创建备份目录失败: {}", e))
}

// ============================================================
// 核心执行
// ============================================================

/// 执行一次完整备份，返回 (文件名, 字节数)
pub async fn run_backup(
    conn: &mut crate::services::inventory_ledger::Conn,
    config: &Config,
    label: Option<&str>,
) -> std::result::Result<(String, u64), String> {
    let dir = Path::new(&config.backup_dir);
    ensure_backup_dir(dir)?;

    let ts = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let label_part = label.map(|l| format!("_{}", l)).unwrap_or_default();
    let filename = format!("{}_{}{}.bak", config.db_database, ts, label_part);
    let path = dir.join(&filename);
    let path_str = path
        .to_str()
        .ok_or_else(|| "备份路径含非法字符".to_string())?
        .to_string();

    // 标识符/字符串字面量转义（db 名与路径均来自服务端配置与生成值）
    let db_ident = config.db_database.replace(']', "]]");
    let path_lit = path_str.replace('\'', "''");
    let sql = format!(
        "BACKUP DATABASE [{}] TO DISK = N'{}' WITH INIT, CHECKSUM, SKIP, NOUNLOAD, STATS = 25",
        db_ident, path_lit
    );

    let started = std::time::Instant::now();
    conn.execute(&sql, &[])
        .await
        .map_err(|e| format!("BACKUP DATABASE 执行失败: {}", e))?;

    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    tracing::info!(
        filename = %filename,
        size_mb = %(size / 1024 / 1024),
        elapsed_secs = %started.elapsed().as_secs(),
        "数据库备份完成"
    );
    Ok((filename, size))
}

/// 扫描目录生成备份文件列表（按修改时间倒序）
fn scan_backup_files(dir: &Path) -> Vec<serde_json::Value> {
    let mut files: Vec<(std::time::SystemTime, u64, String)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.to_ascii_lowercase().ends_with(".bak") {
                continue;
            }
            if let Ok(meta) = entry.metadata() {
                if meta.is_file() {
                    let mtime = meta.modified().ok().unwrap_or(std::time::UNIX_EPOCH);
                    files.push((mtime, meta.len(), name));
                }
            }
        }
    }
    files.sort_by(|a, b| b.0.cmp(&a.0));
    files
        .into_iter()
        .map(|(mtime, size, name)| {
            let dt: chrono::DateTime<chrono::Local> = mtime.into();
            serde_json::json!({
                "name": name,
                "size": size,
                "modified": dt.format("%Y-%m-%d %H:%M:%S").to_string(),
                "is_auto": name.contains("_auto.bak"),
            })
        })
        .collect()
}

/// 计算下一次自动备份的时刻（本地时区，今天未到则今天，否则明天）
fn next_auto_run(hour: u32) -> chrono::DateTime<chrono::Local> {
    let now = chrono::Local::now();
    let today_target = now.date_naive().and_hms_opt(hour, 0, 0).expect("合法时刻");
    let today_dt = chrono::Local
        .from_local_datetime(&today_target)
        .single()
        .unwrap_or(now);
    if today_dt > now {
        today_dt
    } else {
        chrono::Local
            .from_local_datetime(&(today_target + chrono::Duration::days(1)))
            .single()
            .unwrap_or(now)
    }
}
use chrono::TimeZone;

// ============================================================
// HTTP handlers
// ============================================================

/// 列出备份文件 + 当前备份配置概览
pub async fn list_backups(
    State(config): State<Config>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let dir = Path::new(&config.backup_dir);
    if let Err(e) = ensure_backup_dir(dir) {
        return Ok(Json(ApiResponse::err(&e)));
    }
    let files = scan_backup_files(dir);
    let next = if config.backup_auto_enabled {
        serde_json::Value::String(
            next_auto_run(config.backup_auto_hour)
                .format("%Y-%m-%d %H:%M")
                .to_string(),
        )
    } else {
        serde_json::Value::Null
    };
    Ok(Json(ApiResponse::ok(serde_json::json!({
        "files": files,
        "config": {
            "backup_dir": config.backup_dir,
            "db": config.db_database,
            "auto_enabled": config.backup_auto_enabled,
            "auto_hour": config.backup_auto_hour,
            "keep_days": config.backup_keep_days,
            "next_auto_run": next,
        }
    }))))
}

#[derive(Deserialize)]
pub struct CreateBackupParams {
    /// 可选标签（清洗后拼入文件名）
    pub label: Option<String>,
}

/// 手动创建完整备份
pub async fn create_backup(
    Extension(claims): Extension<Claims>,
    State(config): State<Config>,
    Json(body): Json<CreateBackupParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let label = body.label.as_deref().and_then(sanitize_label);
    let mut conn = get_pool().get().await?;
    tracing::info!(user = %claims.user_code, label = ?label, "发起手动数据库备份");
    match run_backup(&mut conn, &config, label.as_deref()).await {
        Ok((filename, size)) => Ok(Json(ApiResponse::ok(serde_json::json!({
            "filename": filename,
            "size": size,
        })))),
        Err(e) => {
            tracing::error!(user = %claims.user_code, error = %e, "手动备份失败");
            Ok(Json(ApiResponse::err("数据库备份失败，详见服务端日志")))
        }
    }
}

#[derive(Deserialize)]
pub struct FileParams {
    pub file: String,
}

/// 校验备份文件可读完整（RESTORE VERIFYONLY）
pub async fn verify_backup(
    State(config): State<Config>,
    Json(body): Json<FileParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let path = match resolve_existing_in_dir(Path::new(&config.backup_dir), &body.file) {
        Ok(p) => p,
        Err(e) => return Ok(Json(ApiResponse::err(&e))),
    };
    let path_lit = path.to_string_lossy().replace('\'', "''");
    let sql = format!(
        "RESTORE VERIFYONLY FROM DISK = N'{}' WITH NOUNLOAD",
        path_lit
    );
    let mut conn = get_pool().get().await?;
    match conn.execute(&sql, &[]).await {
        Ok(_) => Ok(Json(ApiResponse::msg("备份文件校验通过，可正常恢复"))),
        Err(e) => {
            tracing::error!(file = %body.file, error = %e, "备份文件校验失败");
            Ok(Json(ApiResponse::err("备份文件校验失败：文件可能已损坏")))
        }
    }
}

/// 流式下载备份文件（POST 以复用 Authorization 头；Content-Length 支持进度显示）
pub async fn download_backup(
    State(config): State<Config>,
    Json(body): Json<FileParams>,
) -> Result<Response> {
    let path = match resolve_existing_in_dir(Path::new(&config.backup_dir), &body.file) {
        Ok(p) => p,
        Err(e) => {
            return Ok((
                StatusCode::NOT_FOUND,
                Json(ApiResponse::<serde_json::Value>::err(&e)),
            )
                .into_response());
        }
    };
    let len = match std::fs::metadata(&path) {
        Ok(m) => m.len(),
        Err(_) => {
            return Ok((
                StatusCode::NOT_FOUND,
                Json(ApiResponse::<serde_json::Value>::err("读取备份文件失败")),
            )
                .into_response());
        }
    };
    let file = match tokio::fs::File::open(&path).await {
        Ok(f) => f,
        Err(e) => {
            return Ok((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::<serde_json::Value>::err(&format!(
                    "打开备份文件失败: {}",
                    e
                ))),
            )
                .into_response());
        }
    };
    let stream = ReaderStream::new(file);
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&len.to_string()).unwrap_or(HeaderValue::from_static("0")),
    );
    // filename 用 ASCII 安全名，同时给 UTF-8 编码原名
    let ascii_name = body
        .file
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    let utf8_name = url_escape(&body.file);
    if let Ok(v) = HeaderValue::from_str(&format!(
        "attachment; filename=\"{}\"; filename*=UTF-8''{}",
        ascii_name, utf8_name
    )) {
        headers.insert(header::CONTENT_DISPOSITION, v);
    }
    Ok((StatusCode::OK, headers, Body::from_stream(stream)).into_response())
}

/// 百分号编码（Header 值只允许可见 ASCII，中文文件名需编码）
fn url_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// 删除备份文件
pub async fn delete_backup(
    State(config): State<Config>,
    Json(body): Json<FileParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let path = match resolve_existing_in_dir(Path::new(&config.backup_dir), &body.file) {
        Ok(p) => p,
        Err(e) => return Ok(Json(ApiResponse::err(&e))),
    };
    match std::fs::remove_file(&path) {
        Ok(_) => {
            tracing::info!(file = %body.file, "删除备份文件");
            Ok(Json(ApiResponse::msg("备份文件已删除")))
        }
        Err(e) => Ok(Json(ApiResponse::err(&format!("删除失败: {}", e)))),
    }
}

// ============================================================
// 定时自动备份调度器
// ============================================================

/// 启动自动备份调度（BACKUP_AUTO_ENABLED=true 时）。
/// 每天 BACKUP_AUTO_HOUR 点执行完整备份（标签 auto），随后清理超期的自动备份。
/// 配置在进程启动时快照，修改需重启服务。
pub fn spawn_auto_backup_scheduler(config: Config) {
    if !config.backup_auto_enabled {
        tracing::info!("定时自动备份未开启（设置 BACKUP_AUTO_ENABLED=true 启用）");
        return;
    }
    tracing::info!(
        hour = config.backup_auto_hour,
        keep_days = config.backup_keep_days,
        "定时自动备份已开启"
    );
    tokio::spawn(async move {
        loop {
            let next = next_auto_run(config.backup_auto_hour);
            let next_ts = next.timestamp();
            tracing::info!(next = %next.format("%Y-%m-%d %H:%M"), "下次自动备份时间");
            let dur = std::time::Duration::from_secs(
                (next_ts - chrono::Local::now().timestamp()).max(1) as u64,
            );
            tokio::time::sleep(dur).await;

            let result: std::result::Result<String, String> = async {
                let mut conn = get_pool()
                    .get()
                    .await
                    .map_err(|e| format!("获取数据库连接失败: {}", e))?;
                let (filename, _) = run_backup(&mut conn, &config, Some("auto")).await?;
                Ok(filename)
            }
            .await;
            match result {
                Ok(filename) => tracing::info!(filename = %filename, "自动备份完成"),
                Err(e) => tracing::error!(error = %e, "自动备份失败"),
            }

            if let Err(e) = cleanup_expired_auto_backups(&config) {
                tracing::warn!(error = %e, "清理超期自动备份失败");
            }
        }
    });
}

/// 清理超过保留期的自动备份（只删 `*_auto.bak`，手动备份不受影响）
fn cleanup_expired_auto_backups(config: &Config) -> std::result::Result<usize, String> {
    let dir = Path::new(&config.backup_dir);
    if !dir.exists() {
        return Ok(0);
    }
    let cutoff = std::time::SystemTime::now()
        - std::time::Duration::from_secs(config.backup_keep_days as u64 * 24 * 3600);
    let mut removed = 0;
    for entry in std::fs::read_dir(dir).map_err(|e| format!("读取备份目录失败: {}", e))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with("_auto.bak") {
            continue;
        }
        if let Ok(meta) = entry.metadata() {
            if meta.modified().map(|t| t < cutoff).unwrap_or(false) {
                if std::fs::remove_file(entry.path()).is_ok() {
                    removed += 1;
                    tracing::info!(file = %name, "清理超期自动备份");
                }
            }
        }
    }
    Ok(removed)
}

use axum::response::{IntoResponse, Response};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filename_validation() {
        assert!(is_valid_backup_filename("TestERP_20260817_020000.bak"));
        assert!(is_valid_backup_filename("TestERP_20260817_020000_auto.bak"));
        assert!(!is_valid_backup_filename("../evil.bak"));
        assert!(!is_valid_backup_filename("sub/dir/x.bak"));
        assert!(!is_valid_backup_filename("..\\evil.bak"));
        assert!(!is_valid_backup_filename("x.exe"));
        assert!(!is_valid_backup_filename(".bak"));
        assert!(!is_valid_backup_filename("a.bak.."));
        assert!(!is_valid_backup_filename("含中文.bak"));
    }

    #[test]
    fn label_sanitization() {
        assert_eq!(sanitize_label("月结前备份"), None); // 非法字符全部清洗后为空
        assert_eq!(sanitize_label("pre_month"), Some("pre_month".into()));
        assert_eq!(sanitize_label("a b/c"), Some("abc".into()));
        let long: String = std::iter::repeat('x').take(50).collect();
        assert_eq!(sanitize_label(&long).map(|s| s.len()), Some(30));
        assert_eq!(sanitize_label(""), None);
    }

    #[test]
    fn path_traversal_blocked() {
        let dir = std::env::temp_dir().join(format!("erp_backup_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ok.bak"), b"x").unwrap();

        assert!(resolve_existing_in_dir(&dir, "ok.bak").is_ok());
        // 穿越与不存在
        assert!(resolve_existing_in_dir(&dir, "../secret.bak").is_err());
        assert!(resolve_existing_in_dir(&dir, "nope.bak").is_err());
        // 在子目录放一个 .bak，确认目录外/子目录内的文件被拒
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/inner.bak"), b"x").unwrap();
        assert!(resolve_existing_in_dir(&dir, "sub/inner.bak").is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn url_escape_keeps_safe_chars() {
        assert_eq!(url_escape("TestERP_2026.bak"), "TestERP_2026.bak");
        assert_eq!(url_escape("a b"), "a%20b");
        assert_eq!(url_escape("中"), "%E4%B8%AD");
    }
}
