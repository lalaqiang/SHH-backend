//! 权限校验中间件
//!
//! 基于 HTTP 请求路径自动推断所需权限码，校验当前用户是否拥有该权限。
//! 配合 `handlers/permission::get_my_permissions` 返回的按钮级权限码使用。
//!
//! ## 权限码命名规范
//!   `${module}.${resource}.${action}`
//!   - module: base / inventory / purchase / sales / finance / system
//!   - resource: goods / customer / supplier / warehouse / io / move / order ...
//!   - action: read / create / update / delete / audit / print / export / assignPerm / assignRole
//!
//! ## 路径推断规则
//!   1. 高危接口（/api/permission/*, /api/system/*, /api/admin/*）使用专门映射表
//!   2. 业务接口（/api/base/*, /api/inventory/*, ...）按 `{module}.{resource}.{action}` 推断
//!   3. 通用接口（/api/generic/*）暂不校验权限码（由前端按钮控制）
//!   4. 白名单接口（/api/auth/me 等）直接放行
//!   5. 写操作动词兜底：未匹配路径若以写动词结尾（approve/save/delete/...），
//!      要求用户拥有对应动作类的任意权限（见 infer_common_action_permission）
//!
//! ## admin 超级权限
//!   工号 admin 的用户直接放行所有请求，不查 DB。
//!
//! ## 权限缓存
//!   用户权限缓存 5 分钟（TTL），避免每次请求查 DB。
//!   权限变更（角色分配/权限分配）后，前端需重新登录或调用 `/permission/my-permissions` 刷新。

use axum::{
    Json,
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::warn;

use crate::db::get_pool;
use crate::middleware::auth::Claims;

/// 权限缓存条目
struct CacheEntry {
    /// 用户拥有的权限码列表（如 ["system.user.create", "base.goods.read", ...]）
    /// 特殊值 ["*"] 表示 admin 超级权限
    permissions: Vec<String>,
    /// 缓存写入时间
    cached_at: Instant,
}

/// 全局权限缓存：emp_id → CacheEntry
type PermCache = Arc<Mutex<HashMap<String, CacheEntry>>>;

static PERM_CACHE: Lazy<PermCache> = Lazy::new(|| Arc::new(Mutex::new(HashMap::new())));

/// 缓存 TTL：5 分钟
const CACHE_TTL: Duration = Duration::from_secs(300);

/// P2-16 修复：缓存最大条目数
///   原无上限，大量用户登录后内存无限增长
///   限制为 10000 个用户（中等规模企业绰绰有余），超出时按 FIFO 淘汰最旧条目
const PERM_CACHE_MAX_ENTRIES: usize = 10000;

/// 构造 403 响应
fn forbidden_response(message: &str) -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(serde_json::json!({
            "success": false,
            "message": message,
            "code": "PERMISSION_DENIED"
        })),
    )
        .into_response()
}

/// 构造 503 响应（权限服务不可用）
/// P1-8 修复：原 DB 查询失败时 fail-open 放行，存在安全隐患（DB 故障期间所有用户都能访问所有接口）
///   改为 fail-closed 返回 503，让客户端重试
fn service_unavailable_response(message: &str) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(serde_json::json!({
            "success": false,
            "message": message,
            "code": "PERM_SERVICE_UNAVAILABLE"
        })),
    )
        .into_response()
}

/// 权限校验中间件
///
/// 执行顺序：auth_middleware → permission_middleware → handler
/// 此中间件依赖 auth_middleware 已将 Claims 注入到 request extensions 中。
pub async fn permission_middleware(req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();

    // 1. 推断所需权限码（None 表示白名单/未匹配路径，直接放行）
    let required_perm = match infer_permission_from_path(&path) {
        Some(p) => p,
        None => return next.run(req).await,
    };

    // 2. 从 extensions 提取 Claims
    let claims = match req.extensions().get::<Claims>().cloned() {
        Some(c) => c,
        None => {
            // auth_middleware 应已注入 Claims，此处兜底
            return forbidden_response("无法获取用户信息，请重新登录");
        }
    };

    // 3. admin 超级权限：直接放行
    if claims.user_code.eq_ignore_ascii_case("admin") {
        return next.run(req).await;
    }

    // 4. 获取用户权限列表（带缓存）
    let perms = match get_user_permissions_cached(&claims.emp_id, &claims.user_code).await {
        Ok(p) => p,
        Err(e) => {
            // P1-8 修复：原 fail-open 策略存在安全隐患（DB 故障期间所有用户都能访问所有接口）
            //   改为 fail-closed 返回 503，让客户端重试，避免越权访问
            //   权限缓存 5 分钟内有效，DB 短时故障期间已有缓存的用户仍可正常使用
            warn!(error = %e, emp_id = %claims.emp_id, "权限查询失败，拒绝访问（fail-closed）");
            return service_unavailable_response("权限服务暂时不可用，请稍后重试或联系管理员");
        }
    };

    // 5. 权限校验
    //    - perms 包含 "*" → 全权放行（理论上 admin 已在上一步放行，这里是兜底）
    //    - perms 为空 → 无任何角色，拒绝所有非白名单接口（避免新用户默认拥有全部权限）
    //    - generic.{action} → 特殊处理：用户拥有匹配 action 后缀的任意权限即放行
    //    - perms 包含 required_perm → 放行
    //    - 否则 → 403
    if perms.is_empty() {
        // 安全策略：未分配任何角色的用户拒绝所有非白名单接口
        // 避免新用户默认拥有全部权限的安全风险
        return forbidden_response("您未被分配任何角色，请联系管理员开通权限");
    }
    if perms.iter().any(|p| p == "*") || perms.iter().any(|p| p == &required_perm) {
        return next.run(req).await;
    }
    // P0-S5：generic.*/common.* 虚拟权限码特殊处理
    //   有 base.goods.update 等任意 *.update 权限的用户可调用 /api/generic/update；
    //   有任意 *.audit 权限的用户可调用 /api/doc/approve 等动词兜底路径
    if is_action_suffixed_permission(&required_perm)
        && has_matching_action_permission(&perms, &required_perm)
    {
        return next.run(req).await;
    }

    forbidden_response(&format!("无权限访问此功能（需要权限：{}）", required_perm))
}

/// 从 HTTP 路径推断所需权限码
///
/// 返回 None 表示该路径不需要权限校验（白名单）。
/// 返回 Some(code) 表示需要用户拥有该权限码。
fn infer_permission_from_path(path: &str) -> Option<String> {
    // ===== 白名单：不需要权限校验的路径 =====
    if is_whitelisted(path) {
        return None;
    }

    // ===== admin 专用接口：只有 admin 可访问 =====
    // /api/admin/* 已在中间件中由 admin 超级权限放行，非 admin 直接 403
    // 这里返回一个不存在的权限码，确保非 admin 用户被拒绝
    if path.starts_with("/api/admin/") {
        return Some("admin.only".to_string());
    }

    // ===== 高危接口：专门映射 =====
    if let Some(perm) = infer_high_risk_permission(path) {
        return Some(perm);
    }

    // P0-S5：通用 CRUD 接口权限码推断
    //   返回 generic.{action}，由 permission_middleware 中特殊处理：
    //   用户拥有 ANY `${module}.${resource}.${action}`（action 后缀匹配）即放行
    //   这样有 base.goods.update 权限的用户即可使用 /api/generic/update
    if let Some(perm) = infer_generic_permission(path) {
        return Some(perm);
    }

    // ===== 业务接口：按 {module}.{resource}.{action} 推断 =====
    if let Some(perm) = infer_business_permission(path) {
        return Some(perm);
    }

    // P1-9 修复：原逻辑对所有未匹配路径默认放行（None），存在安全风险
    //   （新增 API 路径未配置权限码时默认放行，攻击者可探测未保护的内部接口）
    //   改为：对高危模块（permission/system/admin/backup）下的未匹配路径默认拒绝
    //   其他路径（如 /api/generic/* 的新接口、未知路径）保持默认放行，避免锁死系统
    if path.starts_with("/api/permission/")
        || path.starts_with("/api/system/")
        || path.starts_with("/api/backup/")
    {
        // 返回一个通用权限码，确保只有显式分配了该权限的用户才能访问
        // 由于没有用户会被分配这个权限码（除 admin），实际上是拒绝所有非 admin 用户
        tracing::warn!(
            path = %path,
            "高危模块下出现未映射的路径，默认拒绝（非 admin 用户被拦截）"
        );
        return Some("system.unknown.deny".to_string());
    }

    // P0 修复：未匹配路径的写操作动词兜底（详见 infer_common_action_permission）
    //   只有末段命中写动词表的路径才会被收紧，读路径维持原放行策略
    if let Some(perm) = infer_common_action_permission(path) {
        return Some(perm);
    }

    // 其他未匹配路径：默认放行（避免锁死未知接口）
    None
}

/// 判断路径是否为"写操作动词结尾"的路径（供限流等模块复用）。
/// 与 infer_common_action_permission 的动词表一致：末段命中写动词表
/// 或为显式登记的写端点（如 /api/retail/sale）即视为写操作。
pub fn is_write_action_path(path: &str) -> bool {
    infer_common_action_permission(path).is_some()
}

/// 判断路径是否在白名单中
fn is_whitelisted(path: &str) -> bool {
    const WHITELIST: &[&str] = &[
        "/api/auth/me",
        "/api/auth/change-password",
        "/api/permission/my-permissions",
        "/api/permission/company-name",
        "/api/permission/warehouses",
        "/api/permission/overview",
        "/api/permission/menus",
        "/api/permission/table-column-config/get",
        "/api/permission/table-column-config/save",
        "/api/permission/table-column-config/delete",
        "/api/permission/column-preset/save",
        "/api/permission/column-preset/list",
        "/api/permission/column-preset/delete",
        "/api/permission/column-preset/apply",
        // P0-S5 修复：通用 CRUD 接口不再无脑放行
        //   原放行策略让任何登录用户（包括未分配角色的）都能通过 generic 接口读写所有业务表，
        //   等同于把权限控制完全交给前端按钮可见性——前端绕过即可越权。
        //   改为：由 infer_generic_permission 推断 generic.{action} 权限码，
        //   用户必须拥有匹配 action 的任意权限（或显式 generic.{action}）才能调用。
        //   单据号生成接口仍保留（前端列表/选择器大量依赖）
        "/api/doc_no/generate",
        "/api/doc_no/list-types",
        "/api/doc_no/reset-seq",
        "/api/generic/docno/generate",
        // 工作台、通知、备份：所有登录用户可用
        "/api/workspace/todo",
        "/api/workspace/doing",
        "/api/workspace/menus",
        "/api/notification/list",
        "/api/notification/create",
        "/api/notification/read",
        "/api/notification/unread-count",
        "/api/system-config",
        "/api/system-config/save",
        "/api/dashboard/stats",
        "/api/base/dashboard-stats",
        "/api/base/versions",
        "/api/base/stock-query",
        "/api/inventory/stock-query",
        "/api/inventory/flows",
        "/api/health",
        "/api/metrics",
        // 登出：仅操作本机会话黑名单，所有登录用户可用
        "/api/auth/logout",
        // 打印审计日志：打印是各岗位日常操作，记录日志本身不应受写权限限制
        "/api/print/log/create",
    ];
    WHITELIST.contains(&path)
}

/// 通用 CRUD 接口权限码推断
///
/// `/api/generic/{action}` → `generic.{action_code}`
/// action 映射：
///   - query / tree / schema / oper-log → generic.read（读操作）
///   - create / import / import-excel → generic.create
///   - update / batch-update / restore → generic.update
///   - delete → generic.delete
///   - export / export-excel → generic.export
///
/// 在 `permission_middleware` 中，generic.{action} 权限码有特殊处理：
///   用户拥有 ANY `${module}.${resource}.${action_code}`（action 后缀匹配）即放行，
///   这样有 base.goods.update 的用户即可调用 /api/generic/update。
fn infer_generic_permission(path: &str) -> Option<String> {
    let rest = path.strip_prefix("/api/generic/")?;
    let action = match rest {
        // 读操作
        "query" | "tree" | "schema" | "oper-log" => "read",
        // 写操作
        "create" | "import" | "import-excel" => "create",
        "update" | "batch-update" | "restore" => "update",
        "delete" => "delete",
        "export" | "export-excel" => "export",
        // 单据号生成、其他未匹配的 generic 子路径：放行（白名单已处理 /docno/*）
        _ => return None,
    };
    Some(format!("generic.{}", action))
}

/// 判断权限码是否为虚拟动作类权限（generic.* / common.*，需 action 后缀匹配）
fn is_action_suffixed_permission(perm: &str) -> bool {
    perm.starts_with("generic.") || perm.starts_with("common.")
}

/// 检查用户权限列表中是否包含匹配 generic.{action} / common.{action} 的权限
/// 规则：用户拥有 ANY `${module}.${resource}.${action}`（action 后缀匹配）即放行
/// 例如 generic.update 可由 base.goods.update / purchase.order.update 等任一权限满足；
///     common.write 是组合类，满足 .create 或 .update 任一后缀即可。
fn has_matching_action_permission(perms: &[String], suffixed_perm: &str) -> bool {
    let Some(action) = suffixed_perm
        .strip_prefix("generic.")
        .or_else(|| suffixed_perm.strip_prefix("common."))
    else {
        // 不带虚拟前缀的权限码不适用后缀匹配
        return false;
    };
    // 用户显式拥有该虚拟权限码（如 seed 数据中显式授予 generic.update）
    if perms.iter().any(|p| p == suffixed_perm) {
        return true;
    }
    // write 为组合类：create / update 任一后缀即可满足
    let actions: Vec<&str> = match action {
        "write" => vec!["create", "update"],
        a => vec![a],
    };
    // 隐式匹配：用户拥有任意 `${...}.${action}` 权限
    //   action 后缀必须严格匹配（避免 common.delete 被任意 .read 权限满足）
    perms.iter().any(|p| {
        let parts: Vec<&str> = p.split('.').collect();
        parts.len() >= 3 && parts.last().is_some_and(|last| actions.contains(last))
    })
}

/// P0 修复：未映射路径的写操作动词兜底
///
/// 背景：原实现对推断不出的路径一律放行，导致 `/api/doc/approve`、`/api/doc/void`、
/// `/api/vip/delete`、`/api/sales-input/update`、`/api/generic/cleanup-orphan-stock`
/// 等两段式/未注册模块的写接口仅需登录即可调用——任何用户（含零角色用户）都能
/// 审核、作废单据或执行破坏性维护操作。
///
/// 规则：取路径最后一段，命中写操作动词表则要求用户拥有对应动作类的任意权限码，
/// 复用 generic.* 的 action 后缀匹配机制：
///   - audit 类  (approve/void/月结/清理等)  → 需任意 *.audit
///   - delete 类 (delete/remove)             → 需任意 *.delete
///   - create 类 (create/add)                → 需任意 *.create
///   - update 类 (update/restore/edit)       → 需任意 *.update
///   - write 类  (save/submit/import/...)    → 需任意 *.create 或 *.update
///   - export 类 (export*/...)               → 需任意 *.export
///
/// 读操作与非动词结尾的路径（如 /api/base/goods、/api/finance/ap/supplier、
/// /api/online/order/my）不受影响，维持原有放行策略；动词表是唯一调参点，
/// 漏配某个动词只会回到"放行"而不会锁死业务。
fn infer_common_action_permission(path: &str) -> Option<String> {
    // 非动词结尾但实为写操作的端点，显式登记
    if path == "/api/retail/sale" {
        // 收银开单：零售销售落库 + 库存过账
        return Some("common.write".to_string());
    }
    let rest = path.strip_prefix("/api/")?;
    let last = rest.rsplit('/').next()?;
    let class = match last {
        // 审核/作废/维护类：改单据状态或执行破坏性维护，要求 .audit
        "approve"
        | "unapprove"
        | "void"
        | "audit"
        | "close"
        | "reopen"
        | "rollback"
        | "month_settle"
        | "month_settle_rollback"
        | "cleanup"
        | "cleanup-orphan-stock"
        | "clear"
        | "recalc"
        | "recalc-invoice"
        | "reset" => "audit",
        // 删除类
        "delete" | "remove" => "delete",
        // 新建类
        "create" | "add" => "create",
        // 修改类
        "update" | "batch-update" | "restore" | "edit" => "update",
        // 通用写入：日常保存/提交/导入等，create 或 update 任一即可
        "save"
        | "submit"
        | "submit-batch"
        | "import"
        | "import-excel"
        | "apply"
        | "bulk-apply"
        | "adjust"
        | "settle"
        | "confirm"
        | "cancel"
        | "ship"
        | "batch-ship"
        | "batch-generate-so"
        | "generate-from-source"
        | "record" => "write",
        // 导出类
        "export" | "export-excel" | "export-brands" | "export-products" | "export-detail"
        | "export-summary" => "export",
        _ => return None,
    };
    Some(format!("common.{}", class))
}

/// 高危接口权限码映射
///
/// /api/permission/* 和 /api/system/* 中的写操作需要专门权限。
fn infer_high_risk_permission(path: &str) -> Option<String> {
    // /api/permission/role/create|update|delete
    if path == "/api/permission/role/create" {
        return Some("system.role.create".to_string());
    }
    if path == "/api/permission/role/update" {
        return Some("system.role.update".to_string());
    }
    if path == "/api/permission/role/delete" {
        return Some("system.role.delete".to_string());
    }
    // 角色权限分配
    if path == "/api/permission/assign" {
        return Some("system.role.assignPerm".to_string());
    }
    // 用户角色分配
    if path == "/api/permission/assign-user-roles" {
        return Some("system.user.assignRole".to_string());
    }
    // 用户管理路由已废弃：员工即用户，统一由 tBas_Emp 管理（/api/base-data/employee/*）
    // 系统参数（高危）
    if path == "/api/system/params/save"
        || path == "/api/system/params/update"
        || path == "/api/system/params/delete"
    {
        return Some("system.params.update".to_string());
    }
    // 备份（高危）：全部端点统一要求 system.backup.manage
    if path == "/api/backup/create"
        || path == "/api/backup/delete"
        || path == "/api/backup/list"
        || path == "/api/backup/verify"
        || path == "/api/backup/download"
    {
        return Some("system.backup.manage".to_string());
    }
    None
}

/// 业务接口权限码推断
///
/// 路径模式：/api/{module}/{resource}/{action}
/// 推断为：{module}.{resource}.{action_mapped}
fn infer_business_permission(path: &str) -> Option<String> {
    // 去掉 /api/ 前缀
    let rest = path.strip_prefix("/api/")?;
    let parts: Vec<&str> = rest.split('/').collect();
    if parts.len() < 3 {
        return None;
    }

    let module = parts[0];
    let resource = parts[1];
    let action = parts[2];

    // 模块映射：路径段 → 权限码模块名
    let module_code = match module {
        "base" => "base",
        "inventory" => "inventory",
        "purchase" => "purchase",
        "sales" => "sales",
        "finance" => "finance",
        "wholesale" => "wholesale",
        "online" => "online",
        "mobile" => "mobile",
        _ => return None,
    };

    // 动作映射：路径段 → 权限码动作名
    let action_code = match action {
        "create" => "create",
        "update" => "update",
        "delete" => "delete",
        "approve" => "audit",
        "unapprove" => "audit",
        "void" => "audit",
        "export" => "export",
        "export-excel" => "export",
        "print" => "print",
        "print-log" => "print",
        // list / get / detail 等读操作不校验（前端菜单权限已控制可见性）
        "list"
        | "get"
        | "detail"
        | "tree"
        | "flat"
        | "categories"
        | "regions"
        | "methods"
        | "configs"
        | "status"
        | "proof"
        | "verify"
        | "claim"
        | "default"
        | "batch-ship"
        | "batch-generate-so"
        | "month_settle"
        | "month_settle_rollback"
        | "replenish"
        | "register"
        | "stores" => return None,
        _ => return None,
    };

    Some(format!("{}.{}.{}", module_code, resource, action_code))
}

/// 获取用户权限列表（带缓存）
///
/// 优先从内存缓存读取，缓存过期或不存在时查 DB。
async fn get_user_permissions_cached(
    emp_id: &str,
    user_code: &str,
) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    // admin 直接返回 ["*"]
    if user_code.eq_ignore_ascii_case("admin") {
        return Ok(vec!["*".to_string()]);
    }

    // 检查缓存
    {
        let cache = PERM_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = cache.get(emp_id) {
            if entry.cached_at.elapsed() < CACHE_TTL {
                return Ok(entry.permissions.clone());
            }
        }
    }

    // 查 DB
    let perms = fetch_user_permissions_from_db(emp_id).await?;

    // 写入缓存
    {
        let mut cache = PERM_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        // P2-16 修复：缓存淘汰策略
        //   原无大小限制，大量用户登录后 HashMap 无限增长导致内存泄漏
        //   当缓存条目超过 PERM_CACHE_MAX_ENTRIES 时，淘汰 cached_at 最旧的条目（近似 LRU）
        //   同时清理已过期条目（双重保险）
        if cache.len() >= PERM_CACHE_MAX_ENTRIES {
            // 1. 先清理所有已过期条目（可能已腾出足够空间）
            let now = Instant::now();
            cache.retain(|_, entry| now.duration_since(entry.cached_at) < CACHE_TTL);
            // 2. 仍然超限，则按 cached_at 升序淘汰最旧的若干条目
            if cache.len() >= PERM_CACHE_MAX_ENTRIES {
                let to_remove: Vec<String> = cache
                    .iter()
                    .min_by_key(|(_, e)| e.cached_at)
                    .map(|(k, _)| vec![k.clone()])
                    .unwrap_or_default();
                for k in &to_remove {
                    cache.remove(k);
                }
            }
        }
        cache.insert(
            emp_id.to_string(),
            CacheEntry {
                permissions: perms.clone(),
                cached_at: Instant::now(),
            },
        );
    }

    Ok(perms)
}

/// 从数据库查询用户权限码列表
///
/// 查询逻辑与 `handlers::permission::get_my_permissions` 一致：
/// tSys_UserRule → tSys_RuleMenu → tSys_Menus，生成 `${base_code}.${action}` 权限码。
/// base_code 优先级：PermCode > MDCallName > SYM_NO > SYM_ID
async fn fetch_user_permissions_from_db(
    emp_id: &str,
) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    let mut conn = get_pool().get().await?;
    let sql = r#"SELECT m.SYM_NO, m.MDCallName, m.SYM_ID, m.PermCode,
                 rm.CanRead, rm.CanCreate, rm.CanUpdate, rm.CanDelete, rm.CanAudit, rm.CanPrint, rm.CanExport
                 FROM tSys_UserRule ur
                 INNER JOIN tSys_RuleMenu rm ON ur.RuleID = rm.RuleID
                 LEFT JOIN tSys_Menus m ON rm.MenuID = m.SYM_ID
                 WHERE ur.EmpID = @p1 AND ISNULL(m.Used, 'Y') = 'Y'"#;
    let stream = conn.query(sql, &[&emp_id.to_string()]).await?;
    let rows = stream.into_first_result().await?;

    let mut perms: Vec<String> = Vec::new();
    for r in &rows {
        let sym_no: String = r
            .try_get::<&str, _>("SYM_NO")
            .ok()
            .flatten()
            .unwrap_or("")
            .to_string();
        let md_call: String = r
            .try_get::<&str, _>("MDCallName")
            .ok()
            .flatten()
            .unwrap_or("")
            .to_string();
        let sym_id: String = r
            .try_get::<&str, _>("SYM_ID")
            .ok()
            .flatten()
            .unwrap_or("")
            .to_string();
        let perm_code: String = r
            .try_get::<&str, _>("PermCode")
            .ok()
            .flatten()
            .unwrap_or("")
            .to_string();

        // 权限码优先级：PermCode（语义化） > MDCallName > SYM_NO > SYM_ID
        // 与 handlers/permission.rs 的 get_my_permissions 保持一致
        let code = if !perm_code.is_empty() {
            perm_code
        } else if !md_call.is_empty() {
            md_call
        } else if !sym_no.is_empty() {
            sym_no
        } else {
            sym_id
        };
        if code.is_empty() {
            continue;
        }

        let can_read = read_flag(r, "CanRead");
        let can_create = read_flag(r, "CanCreate");
        let can_update = read_flag(r, "CanUpdate");
        let can_delete = read_flag(r, "CanDelete");
        let can_audit = read_flag(r, "CanAudit");
        let can_print = read_flag(r, "CanPrint");
        let can_export = read_flag(r, "CanExport");

        if can_read {
            perms.push(format!("{}.read", code));
        }
        if can_create {
            perms.push(format!("{}.create", code));
        }
        if can_update {
            perms.push(format!("{}.update", code));
        }
        if can_delete {
            perms.push(format!("{}.delete", code));
        }
        if can_audit {
            perms.push(format!("{}.audit", code));
        }
        if can_print {
            perms.push(format!("{}.print", code));
        }
        if can_export {
            perms.push(format!("{}.export", code));
        }
    }

    Ok(perms)
}

/// 读取权限标志位（int 或 "Y"/"N" 字符串）
fn read_flag(row: &tiberius::Row, col: &str) -> bool {
    if let Ok(Some(v)) = row.try_get::<i32, _>(col) {
        return v != 0;
    }
    if let Ok(Some(s)) = row.try_get::<&str, _>(col) {
        return s.eq_ignore_ascii_case("Y") || s == "1";
    }
    false
}

/// 清除指定用户的权限缓存
///
/// 在用户角色变更、角色权限变更后调用，确保下次请求重新查 DB。
pub fn invalidate_user_permission_cache(emp_id: &str) {
    let mut cache = PERM_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.remove(emp_id);
}

/// 清除所有用户的权限缓存
pub fn invalidate_all_permission_cache() {
    let mut cache = PERM_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P0 修复回归：审核/作废/维护类动词必须收紧到 common.audit
    #[test]
    fn audit_verbs_require_audit_suffix() {
        for path in [
            "/api/doc/approve",
            "/api/doc/unapprove",
            "/api/doc/void",
            "/api/oa/workflow/approve",
            "/api/finance/receipt/audit",
            "/api/inventory/month_settle",
            "/api/inventory/month_settle_rollback",
            "/api/generic/cleanup-orphan-stock",
            "/api/print/versions/rollback",
            "/api/commission/recalc-invoice",
        ] {
            assert_eq!(
                infer_permission_from_path(path),
                Some("common.audit".to_string()),
                "{} 应推断为 common.audit",
                path
            );
        }
    }

    /// P0 修复回归：删除/修改/保存/导出类动词按对应动作类收紧
    #[test]
    fn crud_and_write_verbs_infer_common_classes() {
        assert_eq!(
            infer_permission_from_path("/api/vip/delete"),
            Some("common.delete".to_string())
        );
        assert_eq!(
            infer_permission_from_path("/api/sales-input/update"),
            Some("common.update".to_string())
        );
        assert_eq!(
            infer_permission_from_path("/api/doc/save"),
            Some("common.write".to_string())
        );
        assert_eq!(
            infer_permission_from_path("/api/doc/generate-from-source"),
            Some("common.write".to_string())
        );
        assert_eq!(
            infer_permission_from_path("/api/retail/sale"),
            Some("common.write".to_string())
        );
        assert_eq!(
            infer_permission_from_path("/api/categories/create"),
            Some("common.create".to_string())
        );
        assert_eq!(
            infer_permission_from_path("/api/report/sales/export-excel"),
            Some("common.export".to_string())
        );
        assert_eq!(
            infer_permission_from_path("/api/pricing/apply"),
            Some("common.write".to_string())
        );
    }

    /// 读路径与非动词结尾路径必须维持放行（不误伤只读接口）
    #[test]
    fn read_and_non_verb_paths_stay_allowed() {
        for path in [
            "/api/base/goods",
            "/api/base/brand",
            "/api/finance/ap/supplier",
            "/api/finance/ar/customer/detail",
            "/api/online/order/my",
            "/api/online/payment/claim",
            "/api/doc/graph",
            "/api/retail/cashier",
            "/api/inventory/alerts/replenish",
            "/api/auth/logout",
            "/api/print/log/create",
            "/api/mobile/change-password",
            "/api/mobile/sync-base-data",
        ] {
            assert_eq!(
                infer_permission_from_path(path),
                None,
                "{} 应保持放行",
                path
            );
        }
    }

    /// 已有映射链路保持不变：业务三段式仍要求精确权限码，generic 走 generic.*
    #[test]
    fn mapped_business_paths_keep_exact_codes() {
        assert_eq!(
            infer_permission_from_path("/api/base/goods/create"),
            Some("base.goods.create".to_string())
        );
        assert_eq!(
            infer_permission_from_path("/api/purchase/order/approve"),
            Some("purchase.order.audit".to_string())
        );
        assert_eq!(
            infer_permission_from_path("/api/generic/update"),
            Some("generic.update".to_string())
        );
        // 高危模块未映射路径：默认拒绝
        assert_eq!(
            infer_permission_from_path("/api/system/unknown-op"),
            Some("system.unknown.deny".to_string())
        );
    }

    /// write 组合类：.create 或 .update 任一后缀可满足；其他类严格匹配
    #[test]
    fn write_class_matches_create_or_update_suffix() {
        let creator = vec!["sales.order.create".to_string()];
        assert!(has_matching_action_permission(&creator, "common.write"));
        assert!(has_matching_action_permission(&creator, "common.create"));
        assert!(!has_matching_action_permission(&creator, "common.update"));
        assert!(!has_matching_action_permission(&creator, "common.audit"));
        assert!(!has_matching_action_permission(&creator, "common.delete"));
        assert!(!has_matching_action_permission(&creator, "common.export"));

        let auditor = vec!["purchase.order.audit".to_string()];
        assert!(has_matching_action_permission(&auditor, "common.audit"));
        assert!(!has_matching_action_permission(&auditor, "common.write"));

        // 显式授予的虚拟权限码同样生效
        let explicit = vec!["common.audit".to_string()];
        assert!(has_matching_action_permission(&explicit, "common.audit"));
    }
}
