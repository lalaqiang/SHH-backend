//! PC 端「手机数据」管理模块
//!
//! 手机端提交的特价/奖励/赠品实际写入 tSys_Parameters 表（PKind 区分类型，PValue 存 JSON），
//! 补货走 tStk_ReplenishApply，盘点走 tStk_Move。
//! 本模块为 PC 端 DataPage 提供统一查询接口，自动解析 PValue JSON 并平铺到顶层字段。

use crate::config::Config;
use crate::db::get_pool;
use crate::error::Result;
use crate::handlers::base_data::row_to_json;
use crate::handlers::mobile::get_str;
use crate::middleware::auth::Claims;
use crate::services::inventory_ledger;
use crate::utils::{ApiResponse, build_pagination_sql_with_sort};
use axum::{Extension, Json, extract::State};
use bb8::PooledConnection;
use bb8_tiberius::ConnectionManager;
use serde::Deserialize;
use tiberius::{Row, ToSql};

type Conn = PooledConnection<'static, ConnectionManager>;

#[derive(Deserialize, Debug)]
pub struct MobileDataListParams {
    /// 业务类型：special_price / reward_product / gift_giving / replenishment / stock_check
    #[serde(rename = "kind")]
    pub kind: String,
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
    /// 提交人筛选（工号/姓名）
    pub submitter: Option<String>,
    /// 日期范围 [start, end]
    pub date_range: Option<(String, String)>,
    pub sort_prop: Option<String>,
    pub sort_order: Option<String>,
    /// 仓库筛选（补货/盘点）
    pub stk_name: Option<String>,
    /// 客户筛选（特价/奖励/赠品）
    pub cust_name: Option<String>,
}

/// 解析 PValue JSON 字符串并平铺到顶层
/// - 对 tSys_Parameters 类记录：PValue 含 OrigPrice/NewPrice/Qty/Reason 等业务字段
/// - 解析失败时保留原 PValue 字段，不影响其他字段
fn flatten_pvalue(mut item: serde_json::Value) -> serde_json::Value {
    if let Some(obj) = item.as_object_mut() {
        if let Some(pvalue) = obj
            .get("PValue")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
        {
            if let Ok(parsed) =
                serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&pvalue)
            {
                for (k, v) in parsed {
                    // 不覆盖已有字段（如 ParametersID/EUser/EDate/PKind/PHelp）
                    obj.entry(k).or_insert(v);
                }
            }
        }
    }
    item
}

/// 2005 兼容：批量解析 tSys_Parameters 记录中的 GDSID/CustID 到 GDSNO/GDSDesc/CustName
///
/// 替代 SQL 层的 `LEFT JOIN tBas_Goods g ON TRY_CAST(JSON_VALUE(p.PValue, '$.GDSID') ...)`（2012+）
/// 在 Rust 层先解析 PValue JSON 收集 ID，再批量查询关联表，最后注入名称字段。
async fn enrich_with_names(
    conn: &mut Conn,
    items: Vec<serde_json::Value>,
) -> Vec<serde_json::Value> {
    if items.is_empty() {
        return items;
    }

    // 1. 收集所有 GDSID / CustID（去重）
    use std::collections::HashSet;
    let mut gds_ids: HashSet<String> = HashSet::new();
    let mut cust_ids: HashSet<String> = HashSet::new();
    for item in &items {
        if let Some(pvalue) = item.get("PValue").and_then(|v| v.as_str()) {
            if let Ok(map) =
                serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(pvalue)
            {
                if let Some(g) = map.get("GDSID").and_then(|v| v.as_str()) {
                    gds_ids.insert(g.to_lowercase());
                }
                if let Some(c) = map.get("CustID").and_then(|v| v.as_str()) {
                    cust_ids.insert(c.to_lowercase());
                }
            }
        }
    }

    // 2. 批量查询 tBas_Goods → (GDSID, GDSNO, GDSDesc)
    let mut gds_map: std::collections::HashMap<String, (String, String)> =
        std::collections::HashMap::new();
    if !gds_ids.is_empty() {
        // 修复 SQL 注入：参数化 IN 列表，避免字符串拼接
        let placeholders: Vec<String> = (1..=gds_ids.len()).map(|i| format!("@p{}", i)).collect();
        let sql = format!(
            "SELECT [GDSID], [GDSNO], [GDSDesc] FROM [tBas_Goods] WHERE [GDSID] IN ({})",
            placeholders.join(",")
        );
        let gds_params: Vec<Option<String>> = gds_ids.iter().map(|s| Some(s.clone())).collect();
        let gds_param_refs: Vec<&dyn ToSql> = gds_params.iter().map(|v| v as &dyn ToSql).collect();
        if let Ok(stream) = conn.query(&sql, &gds_param_refs).await {
            if let Ok(rows) = stream.into_first_result().await {
                for row in &rows {
                    let id = row
                        .try_get::<uuid::Uuid, _>("GDSID")
                        .ok()
                        .flatten()
                        .map(|u| u.to_string())
                        .unwrap_or_default();
                    let no = row
                        .try_get::<&str, _>("GDSNO")
                        .ok()
                        .flatten()
                        .unwrap_or("")
                        .to_string();
                    let desc = row
                        .try_get::<&str, _>("GDSDesc")
                        .ok()
                        .flatten()
                        .unwrap_or("")
                        .to_string();
                    if !id.is_empty() {
                        gds_map.insert(id.to_lowercase(), (no, desc));
                    }
                }
            }
        }
    }

    // 3. 批量查询 tBas_Cust → (CustID, CustName)
    let mut cust_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    if !cust_ids.is_empty() {
        // 修复 SQL 注入：参数化 IN 列表，避免字符串拼接
        let placeholders: Vec<String> = (1..=cust_ids.len()).map(|i| format!("@p{}", i)).collect();
        let sql = format!(
            "SELECT [CustID], [CustName] FROM [tBas_Cust] WHERE [CustID] IN ({})",
            placeholders.join(",")
        );
        let cust_params: Vec<Option<String>> = cust_ids.iter().map(|s| Some(s.clone())).collect();
        let cust_param_refs: Vec<&dyn ToSql> =
            cust_params.iter().map(|v| v as &dyn ToSql).collect();
        if let Ok(stream) = conn.query(&sql, &cust_param_refs).await {
            if let Ok(rows) = stream.into_first_result().await {
                for row in &rows {
                    let id = row
                        .try_get::<uuid::Uuid, _>("CustID")
                        .ok()
                        .flatten()
                        .map(|u| u.to_string())
                        .unwrap_or_default();
                    let name = row
                        .try_get::<&str, _>("CustName")
                        .ok()
                        .flatten()
                        .unwrap_or("")
                        .to_string();
                    if !id.is_empty() {
                        cust_map.insert(id.to_lowercase(), name);
                    }
                }
            }
        }
    }

    // 4. 注入 GDSNO/GDSDesc/CustName 到每条记录（在 flatten_pvalue 之前注入，避免被覆盖）
    items
        .into_iter()
        .map(|mut item| {
            if let Some(obj) = item.as_object_mut() {
                if let Some(pvalue) = obj
                    .get("PValue")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                {
                    if let Ok(map) =
                        serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&pvalue)
                    {
                        if let Some(g) = map.get("GDSID").and_then(|v| v.as_str()) {
                            if let Some((no, desc)) = gds_map.get(&g.to_lowercase()) {
                                obj.insert(
                                    "GDSNO".to_string(),
                                    serde_json::Value::String(no.clone()),
                                );
                                obj.insert(
                                    "GDSDesc".to_string(),
                                    serde_json::Value::String(desc.clone()),
                                );
                            }
                        }
                        if let Some(c) = map.get("CustID").and_then(|v| v.as_str()) {
                            if let Some(name) = cust_map.get(&c.to_lowercase()) {
                                obj.insert(
                                    "CustName".to_string(),
                                    serde_json::Value::String(name.clone()),
                                );
                            }
                        }
                    }
                }
            }
            item
        })
        .collect()
}

/// 主查询入口
pub async fn list_mobile_data(
    State(_config): State<Config>,
    Json(params): Json<MobileDataListParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1).max(1);
    let page_size = params.page_size.unwrap_or(20).clamp(1, 1000);

    // 根据 kind 选择数据源
    let (base_query, is_params_table, date_field, _submitter_field): (String, bool, &str, &str) = match params.kind.as_str() {
        "special_price" => (
            // tSys_Parameters 按 PCode 批次分组（一单多商品明细）
            // MIN(PValue) 取一条代表记录用于 Rust 层提取 CustID → CustName
            r#"SELECT t.[PCode], t.[PName], t.[PHelp], t.[EDate], t.[DetailCount], t.[PValue],
               e.[EmpName]
               FROM (
                 SELECT [PCode], MAX([PName]) AS [PName], MAX([PHelp]) AS [PHelp],
                   MAX([EUser]) AS [EUser], MAX([EDate]) AS [EDate],
                   COUNT(*) AS [DetailCount], MIN([PValue]) AS [PValue]
                 FROM [tSys_Parameters]
                 WHERE [PKind] = 'special_price'
                 GROUP BY [PCode]
               ) t
               LEFT JOIN [tBas_Emp] e ON t.[EUser] = e.[EmpID]"#.to_string(),
            true,
            "EDate",
            "EUser",
        ),
        "reward_product" => (
            r#"SELECT t.[PCode], t.[PName], t.[PHelp], t.[EDate], t.[DetailCount], t.[PValue],
               e.[EmpName]
               FROM (
                 SELECT [PCode], MAX([PName]) AS [PName], MAX([PHelp]) AS [PHelp],
                   MAX([EUser]) AS [EUser], MAX([EDate]) AS [EDate],
                   COUNT(*) AS [DetailCount], MIN([PValue]) AS [PValue]
                 FROM [tSys_Parameters]
                 WHERE [PKind] = 'reward_product'
                 GROUP BY [PCode]
               ) t
               LEFT JOIN [tBas_Emp] e ON t.[EUser] = e.[EmpID]"#.to_string(),
            true,
            "EDate",
            "EUser",
        ),
        "gift_giving" => (
            r#"SELECT t.[PCode], t.[PName], t.[PHelp], t.[EDate], t.[DetailCount], t.[PValue],
               e.[EmpName]
               FROM (
                 SELECT [PCode], MAX([PName]) AS [PName], MAX([PHelp]) AS [PHelp],
                   MAX([EUser]) AS [EUser], MAX([EDate]) AS [EDate],
                   COUNT(*) AS [DetailCount], MIN([PValue]) AS [PValue]
                 FROM [tSys_Parameters]
                 WHERE [PKind] = 'gift_giving'
                 GROUP BY [PCode]
               ) t
               LEFT JOIN [tBas_Emp] e ON t.[EUser] = e.[EmpID]"#.to_string(),
            true,
            "EDate",
            "EUser",
        ),
        "replenishment" => (
            // tArd_AR 扁平明细行表（278万+行），无独立主表。分组键 = StkID + EDate(到天)。
            // 同一仓库同一天的多条明细 = 一张补货单（不管哪个员工提交）。
            // ApplyNo 显示用 仓库编码 + '-' + EDate到天 YYYYMMDD（2005 兼容：禁用 CONCAT）。
            // ★ 性能优化：日期条件下推到内层子查询（{REPLENISHMENT_INNER_WHERE} 占位符），
            //   让 GROUP BY 只处理过滤后的数据，避免对 278 万行全表聚合。
            //   同时合并内外层 tBas_Stock JOIN，内层用 MAX(sk.[StkName]) 取仓库名。
            r#"SELECT t.[ApplyNo], t.[EDate], t.[SaleDate], t.[StkID], t.[Used], t.[EmpID], t.[DetailCount], t.[SumQty], t.[SumAmt],
               t.[TargetStkName], e.[EmpName]
               FROM (
                 SELECT ISNULL(sk.[StkCode],'') + '-' + ISNULL(CONVERT(varchar(8),a.[EDate],112),'') AS [ApplyNo],
                   MIN(a.[EDate]) AS [EDate], MIN(a.[SaleDate]) AS [SaleDate],
                   a.[StkID], MAX(a.[Used]) AS [Used], MAX(a.[EmpID]) AS [EmpID],
                   COUNT(*) AS [DetailCount], SUM(a.[Qty]) AS [SumQty], SUM(a.[Amt]) AS [SumAmt],
                   MAX(sk.[StkName]) AS [TargetStkName]
                 FROM [tArd_AR] a
                 LEFT JOIN [tBas_Stock] sk ON a.[StkID] = sk.[StkID]
                 {REPLENISHMENT_INNER_WHERE}
                 GROUP BY a.[StkID], CONVERT(varchar(8),a.[EDate],112), sk.[StkCode]
               ) t
               LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID]"#.to_string(),
            false,
            "EDate",
            "EmpID",
        ),
        "stock_check" => (
            // 盘点走 tStk_Move 表（Kind='PD'，由移动端 submit_stock_check 写入）
            // 注意字段名是 MoveNO（不是 MoveNo）、FromStkID（不是 StkID）
            r#"SELECT m.[MoveID], m.[MoveNO] AS [MoveNo], m.[MoveDate], m.[FromStkID], m.[Kind], m.[State],
               m.[Note], m.[EUser], m.[EDate], m.[RSumAmt],
               sk.[StkName] AS [StkName], e.[EmpName]
               FROM [tStk_Move] m
               LEFT JOIN [tBas_Stock] sk ON m.[FromStkID] = sk.[StkID]
               LEFT JOIN [tBas_Emp] e ON m.[EUser] = e.[EmpID]
               WHERE m.[Kind] = 'PD' AND m.[State] <> 'D'"#.to_string(),
            false,
            "EDate",
            "EUser",
        ),
        _ => {
            return Ok(Json(ApiResponse::err(&format!("未知的 kind: {}", params.kind))));
        }
    };

    // 动态拼接 where 条件
    let mut where_clauses: Vec<String> = Vec::new();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    // ★ 日期范围优先处理（对 replenishment 下推到内层子查询，避免对 278 万行全表 GROUP BY）
    let mut replenishment_inner_where = String::new();
    // replenishment 无日期范围时默认最近 30 天，避免全表扫描
    let effective_date_range: Option<(String, String)> =
        if params.kind == "replenishment" && params.date_range.is_none() {
            let end = chrono::Local::now().format("%Y-%m-%d").to_string();
            let start = (chrono::Local::now() - chrono::Duration::days(30))
                .format("%Y-%m-%d")
                .to_string();
            Some((start, end))
        } else {
            params.date_range.clone()
        };
    if let Some((start, end)) = &effective_date_range {
        if !start.is_empty() && !end.is_empty() {
            if params.kind == "replenishment" {
                // 下推到内层子查询：WHERE a.[EDate] >= @p1 AND a.[EDate] <= @p2
                // 让 GROUP BY 只处理过滤后的数据
                replenishment_inner_where = format!(
                    " WHERE a.[{}] >= @p{} AND a.[{}] <= @p{}",
                    date_field,
                    pidx,
                    date_field,
                    pidx + 1
                );
            } else {
                let date_prefix = if is_params_table { "t" } else { "m" };
                where_clauses.push(format!(
                    "{}.[{}] >= @p{} AND {}.[{}] <= @p{}",
                    date_prefix,
                    date_field,
                    pidx,
                    date_prefix,
                    date_field,
                    pidx + 1
                ));
            }
            query_params.push(Some(format!("{} 00:00:00", start)));
            query_params.push(Some(format!("{} 23:59:59", end)));
            pidx += 2;
        }
    }

    // 关键词搜索（对不同表搜索不同字段）
    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            let kw_pattern = format!("%{}%", kw);
            if is_params_table {
                // 搜 PCode 单号和 PHelp 备注（分组后用 t. 前缀）
                where_clauses.push(format!(
                    "(t.[PCode] LIKE @p{} OR t.[PHelp] LIKE @p{})",
                    pidx,
                    pidx + 1
                ));
                query_params.push(Some(kw_pattern.clone()));
                query_params.push(Some(kw_pattern));
                pidx += 2;
            } else if params.kind == "replenishment" {
                where_clauses.push(format!("(t.[ApplyNo] LIKE @p{})", pidx));
                query_params.push(Some(kw_pattern.clone()));
                pidx += 1;
            } else {
                // stock_check: 搜 MoveNO 单号、Note 备注、StkName 仓库名
                where_clauses.push(format!(
                    "(m.[MoveNO] LIKE @p{} OR m.[Note] LIKE @p{} OR sk.[StkName] LIKE @p{})",
                    pidx,
                    pidx + 1,
                    pidx + 2
                ));
                query_params.push(Some(kw_pattern.clone()));
                query_params.push(Some(kw_pattern.clone()));
                query_params.push(Some(kw_pattern));
                pidx += 3;
            }
        }
    }

    // 提交人筛选（按员工名搜索，EUser 是 GUID 无法直接搜，改用 EmpName）
    if let Some(sub) = &params.submitter {
        if !sub.is_empty() {
            where_clauses.push(format!("e.[EmpName] LIKE @p{}", pidx));
            query_params.push(Some(format!("%{}%", sub)));
            pidx += 1;
        }
    }

    // 仓库筛选（补货/盘点）
    if let Some(stk) = &params.stk_name {
        if !stk.is_empty() {
            // replenishment 外层已无 sk 别名，改用 t.[TargetStkName]
            if params.kind == "replenishment" {
                where_clauses.push(format!("t.[TargetStkName] LIKE @p{}", pidx));
            } else {
                where_clauses.push(format!("sk.[StkName] LIKE @p{}", pidx));
            }
            query_params.push(Some(format!("%{}%", stk)));
        }
    }

    // 客户筛选（特价/奖励/赠品）
    // 2005 兼容：c 别名已移除，改为 Rust 层过滤（见 enrich_with_names 后的 retain）
    let cust_name_filter = params.cust_name.clone().unwrap_or_default();

    // 组装最终查询
    let final_query = if where_clauses.is_empty() {
        base_query
    } else {
        // where_clauses 内部已带表前缀，直接拼到 base_query 后
        // base_query 已含 WHERE 子句时用 AND 连接；否则用 WHERE 开头
        let has_where = base_query.to_uppercase().contains("WHERE");
        let connector = if has_where { " AND " } else { " WHERE " };
        format!("{}{}{}", base_query, connector, where_clauses.join(" AND "))
    };

    // 对 replenishment，把日期条件下推注入到内层子查询占位符
    let final_query =
        final_query.replace("{REPLENISHMENT_INNER_WHERE}", &replenishment_inner_where);

    // 计算总数
    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", final_query);
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params
        .iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();

    let mut total: i32 = 0;
    let count_stream = conn.query(&count_sql, &param_refs).await?;
    if let Some(row) = count_stream.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }

    // 排序：默认按日期字段降序
    let sort_prop = params.sort_prop.unwrap_or_else(|| date_field.to_string());
    let sort_order = params.sort_order.unwrap_or_else(|| "desc".to_string());

    let paginated_sql = build_pagination_sql_with_sort(
        &final_query,
        page,
        page_size,
        Some(&sort_prop),
        Some(&sort_order),
    );
    let data_stream = conn.query(&paginated_sql, &param_refs).await?;
    let rows: Vec<Row> = data_stream.into_first_result().await?;
    let mut data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    // 2005 兼容：对 tSys_Parameters 类记录，批量解析 GDSID/CustID 到 GDSNO/GDSDesc/CustName
    // 替代 SQL 层 TRY_CAST(JSON_VALUE(...)) JOIN（2012+/2016+）
    if is_params_table {
        data = enrich_with_names(&mut conn, data).await;

        // Rust 层过滤：cust_name（SQL 层已无法过滤，best-effort 仅过滤当前页）
        // 注：keyword 在 is_params_table 时仅匹配 PHelp（SQL 层），不再匹配 GDSDesc/CustName/GDSNO
        //   因为 SQL 已分页，Rust 层无法补充匹配未取到的行；如需 OR 语义需改为无分页模式
        if !cust_name_filter.is_empty() {
            let pat = cust_name_filter.to_lowercase();
            data.retain(|item| {
                item.get("CustName")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_lowercase().contains(&pat))
                    .unwrap_or(false)
            });
        }

        // 解析 PValue JSON 并平铺到顶层
        data = data.into_iter().map(flatten_pvalue).collect();
    }

    Ok(Json(ApiResponse::ok_paginated(
        data,
        total as u64,
        page,
        page_size,
    )))
}

// ==================== 增删改接口 ====================

#[derive(Deserialize, Debug)]
pub struct MobileDataCreateParams {
    pub kind: String,
    pub data: serde_json::Value,
    #[serde(default)]
    pub details: Vec<serde_json::Value>,
}

#[derive(Deserialize, Debug)]
pub struct MobileDataUpdateParams {
    pub kind: String,
    pub id: String,
    pub data: serde_json::Value,
    #[serde(default)]
    pub details: Vec<serde_json::Value>,
}

#[derive(Deserialize, Debug)]
pub struct MobileDataDeleteParams {
    pub kind: String,
    pub ids: Vec<String>,
}

/// 读取 JSON Value 中的字符串字段
fn jstr(v: &serde_json::Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string()
}

/// 读取 JSON Value 中的 f64 字段
fn jf64(v: &serde_json::Value, key: &str) -> f64 {
    v.get(key).and_then(|x| x.as_f64()).unwrap_or(0.0)
}

const ZERO_UUID: &str = "00000000-0000-0000-0000-000000000000";

fn empty_or_zero(s: &str) -> &str {
    if s.is_empty() { ZERO_UUID } else { s }
}

/// kind → (PKind, PName, PCode前缀) 映射
fn kind_to_pkind(kind: &str) -> Option<(&str, &str, &str)> {
    match kind {
        "special_price" => Some(("special_price", "特价申请", "SP")),
        "reward_product" => Some(("reward_product", "奖励产品申请", "RP")),
        "gift_giving" => Some(("gift_giving", "赠品赠送申请", "GG")),
        _ => None,
    }
}

/// 新增
pub async fn create_mobile_data(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<MobileDataCreateParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let emp_id = if claims.emp_id.is_empty() {
        ZERO_UUID.to_string()
    } else {
        claims.emp_id.clone()
    };

    if let Some((p_kind, p_name, p_code_prefix)) = kind_to_pkind(&params.kind) {
        // ===== tSys_Parameters 批次模式（特价/奖励/赠品）=====
        // 生成共享 PCode，循环插入每条明细
        if params.details.is_empty() {
            return Ok(Json(ApiResponse::err("明细不能为空")));
        }
        let p_code = format!(
            "{}{}",
            p_code_prefix,
            chrono::Local::now().format("%Y%m%d%H%M%S")
        );
        let cust_id = jstr(&params.data, "CustID");
        let remark = jstr(&params.data, "Remark");
        let reason = jstr(&params.data, "Reason");
        // ★ tSys_Parameters 有唯一索引 idx_Parameters_CodeTerm(PCode, PTerm)
        //   同一批次多明细必须设置不同 PTerm，否则违反唯一约束
        //   用行号（1-based）作为 PTerm，保证唯一
        let sql = r#"INSERT INTO [tSys_Parameters] ([ParametersID], [PCode], [PTerm], [PName], [PKind], [PHelp], [PValue], [EUser], [EDate])
            VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8)"#;
        for (idx, d) in params.details.iter().enumerate() {
            let gds_id = jstr(d, "GDSID");
            let p_term = format!("{}", idx + 1);
            let p_value = match params.kind.as_str() {
                "special_price" => {
                    serde_json::json!({
                        "CustID": cust_id, "GDSID": gds_id,
                        "OrigPrice": jf64(d, "OrigPrice"), "NewPrice": jf64(d, "NewPrice"),
                        "StartDate": jstr(d, "StartDate"), "EndDate": jstr(d, "EndDate"),
                    })
                }
                _ => {
                    serde_json::json!({
                        "CustID": cust_id, "GDSID": gds_id,
                        "Qty": jf64(d, "Qty"), "Reason": reason,
                    })
                }
            };
            let p_value_str = p_value.to_string();
            let params_vec: Vec<&dyn tiberius::ToSql> = vec![
                &p_code,
                &p_term,
                &p_name,
                &p_kind,
                &remark,
                &p_value_str,
                &emp_id,
                &now,
            ];
            conn.execute(sql, &params_vec).await?;
        }
        return Ok(Json(ApiResponse::ok(
            serde_json::json!({ "PCode": p_code, "count": params.details.len() }),
        )));
    }

    match params.kind.as_str() {
        "stock_check" => {
            // ===== 盘点 tStk_Move + tStk_MoveDetail =====
            let move_id = format!("{}", uuid::Uuid::new_v4());
            let stk_id = empty_or_zero(&jstr(&params.data, "StkID")).to_string();
            // 统一单据号生成：使用 tSys_DocNoSeq 原子分配，格式 PD{YYMM}{NNNN}
            let move_no = crate::utils::doc_no::generate_via_docnoseq(&mut conn, "PD").await?;
            let move_date = jstr(&params.data, "MoveDate");
            let move_date = if move_date.is_empty() {
                chrono::Local::now().format("%Y-%m-%d").to_string()
            } else {
                move_date
            };
            let note = jstr(&params.data, "Remark");

            // 事务包裹：INSERT 主表 + INSERT 明细 原子化，任一明细失败回滚
            let tx_result: std::result::Result<(), String> = async {
                inventory_ledger::begin_tran(&mut conn).await.map_err(|e| e.to_string())?;

                let header_sql = r#"INSERT INTO [tStk_Move] ([MoveID], [MoveNO], [MoveDate], [FromStkID], [Kind], [State], [Note], [EUser], [EDate])
                    VALUES (@p1, @p2, @p3, @p4, 'PD', 'N', @p5, @p6, @p7)"#;
                let header_params: Vec<&dyn tiberius::ToSql> = vec![
                    &move_id, &move_no, &move_date, &stk_id, &note, &emp_id, &now,
                ];
                conn.execute(header_sql, &header_params).await.map_err(|e| format!("保存盘点主表失败: {}", e))?;

                // 插入明细
                // 字段映射：RealQty→Qty(实存), DiffQty→AQty(差异), SysQty→StkQty(系统库存)
                // P5 修复：tStk_MoveDetail.CNVQty/StdQty 是 NOT NULL decimal，原 INSERT 缺这两列导致
                //   "Cannot insert the value NULL into column 'CNVQty'" 报错。
                //   与 mobile.rs:631 的处理一致：CNVQty=StdQty=Qty（盘点无单位换算）
                for d in &params.details {
                    let detail_id = format!("{}", uuid::Uuid::new_v4());
                    let gds_id = empty_or_zero(&jstr(d, "GDSID")).to_string();
                    let sys_qty = jf64(d, "SysQty");
                    let real_qty = jf64(d, "RealQty");
                    let diff_qty = jf64(d, "DiffQty");
                    let detail_sql = r#"INSERT INTO [tStk_MoveDetail] ([MoveDetailID], [MoveID], [GDSID], [Qty], [CNVQty], [StdQty], [AQty], [StkQty])
                        VALUES (@p1, @p2, @p3, @p4, @p4, @p4, @p5, @p6)"#;
                    let dp: Vec<&dyn tiberius::ToSql> = vec![&detail_id, &move_id, &gds_id, &real_qty, &diff_qty, &sys_qty];
                    conn.execute(detail_sql, &dp).await.map_err(|e| format!("保存盘点明细失败: {}", e))?;
                }

                inventory_ledger::commit_tran(&mut conn).await.map_err(|e| e.to_string())?;
                Ok(())
            }.await;
            if let Err(e) = tx_result {
                inventory_ledger::rollback_tran(&mut conn).await;
                return Ok(Json(ApiResponse::err(&format!("盘点单保存失败: {}", e))));
            }
            return Ok(Json(ApiResponse::ok(
                serde_json::json!({ "id": move_id, "MoveNo": move_no }),
            )));
        }
        "replenishment" => {
            // ===== 补货 tArd_AR 扁平明细表（分组键 = StkID + 日期(到天)）=====
            // 同一天同一仓库的补货 = 同一单：有则追加明细，无则新建
            let stk_id = empty_or_zero(&jstr(&params.data, "StkID")).to_string();
            let apply_date = jstr(&params.data, "SaleDate");
            // 日期默认今天；前端传的日期格式为 YYYY-MM-DD
            let date_str = if apply_date.is_empty() {
                chrono::Local::now().format("%Y-%m-%d").to_string()
            } else {
                apply_date
            };
            if params.details.is_empty() {
                return Ok(Json(ApiResponse::err("补货明细不能为空")));
            }

            // 检查当天该仓库是否已有补货记录
            let check_sql = "SELECT TOP 1 [EDate] FROM [tArd_AR] WHERE [StkID] = @p1 AND CONVERT(varchar(8),[EDate],112) = @p2";
            let check_date_key = chrono::NaiveDate::parse_from_str(&date_str, "%Y-%m-%d")
                .map(|d| d.format("%Y%m%d").to_string())
                .unwrap_or_else(|_| date_str.replace("-", ""));
            let check_row = conn
                .query(check_sql, &[&stk_id, &check_date_key])
                .await?
                .into_row()
                .await?;
            // 有旧记录则沿用其 EDate 时间戳（保持同一天同一分组），否则用当前时间戳
            let batch_time = if let Some(row) = check_row {
                get_str(&row, "EDate")
            } else {
                format!("{} 00:00:00", date_str)
            };

            let used = "Y";
            let zero_price: f64 = 0.0;
            for d in &params.details {
                let gds_id = empty_or_zero(&jstr(d, "GDSID")).to_string();
                let qty = jf64(d, "Qty");
                let detail_sql = r#"INSERT INTO [tArd_AR] ([RowID], [StkID], [EmpID], [EDate], [SaleDate], [GDSID], [Qty], [Price], [Amt], [Used])
                    VALUES (NEWID(), @p1, @p2, @p3, @p3, @p4, @p5, @p6, @p7, @p8)"#;
                let dp: Vec<&dyn tiberius::ToSql> = vec![
                    &stk_id,
                    &emp_id,
                    &batch_time,
                    &gds_id,
                    &qty,
                    &zero_price,
                    &zero_price,
                    &used,
                ];
                conn.execute(detail_sql, &dp).await?;
            }
            // 生成显示用单号：仓库编码-YYYYMMDD
            let code_sql = "SELECT TOP 1 [StkCode] FROM [tBas_Stock] WHERE [StkID] = @p1";
            let code_row = conn.query(code_sql, &[&stk_id]).await?.into_row().await?;
            let stk_code = code_row
                .as_ref()
                .map(|r| get_str(r, "StkCode"))
                .unwrap_or_default();
            let apply_no = format!("{}-{}", stk_code, check_date_key);
            return Ok(Json(ApiResponse::ok(
                serde_json::json!({ "count": params.details.len(), "ApplyNo": apply_no }),
            )));
        }
        _ => Ok(Json(ApiResponse::err(&format!(
            "不支持的新增类型: {}",
            params.kind
        )))),
    }
}

/// 更新
pub async fn update_mobile_data(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<MobileDataUpdateParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let emp_id = if claims.emp_id.is_empty() {
        ZERO_UUID.to_string()
    } else {
        claims.emp_id.clone()
    };

    if let Some((p_kind, p_name, _p_code_prefix)) = kind_to_pkind(&params.kind) {
        // ===== tSys_Parameters 批次模式：先删旧批次(PCode)，再插新明细 =====
        let p_code = &params.id; // id 就是 PCode
        let cust_id = jstr(&params.data, "CustID");
        let remark = jstr(&params.data, "Remark");
        let reason = jstr(&params.data, "Reason");
        // 删旧记录
        conn.execute(
            "DELETE FROM [tSys_Parameters] WHERE [PCode] = @p1",
            &[p_code],
        )
        .await?;
        // 插新明细（PTerm 用行号保证唯一索引 idx_Parameters_CodeTerm 不冲突）
        let sql = r#"INSERT INTO [tSys_Parameters] ([ParametersID], [PCode], [PTerm], [PName], [PKind], [PHelp], [PValue], [EUser], [EDate])
            VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8)"#;
        for (idx, d) in params.details.iter().enumerate() {
            let gds_id = jstr(d, "GDSID");
            let p_term = format!("{}", idx + 1);
            let p_value = match params.kind.as_str() {
                "special_price" => {
                    serde_json::json!({
                        "CustID": cust_id, "GDSID": gds_id,
                        "OrigPrice": jf64(d, "OrigPrice"), "NewPrice": jf64(d, "NewPrice"),
                        "StartDate": jstr(d, "StartDate"), "EndDate": jstr(d, "EndDate"),
                    })
                }
                _ => {
                    serde_json::json!({
                        "CustID": cust_id, "GDSID": gds_id,
                        "Qty": jf64(d, "Qty"), "Reason": reason,
                    })
                }
            };
            let p_value_str = p_value.to_string();
            let params_vec: Vec<&dyn tiberius::ToSql> = vec![
                p_code,
                &p_term,
                &p_name,
                &p_kind,
                &remark,
                &p_value_str,
                &emp_id,
                &now,
            ];
            conn.execute(sql, &params_vec).await?;
        }
        return Ok(Json(ApiResponse::msg("更新成功")));
    }

    match params.kind.as_str() {
        "stock_check" => {
            // ===== 盘点：更新主表 + 先删旧明细再插新明细 =====
            let move_id = &params.id;
            let note = jstr(&params.data, "Remark");
            let sql = "UPDATE [tStk_Move] SET [Note] = @p1, [EDate] = @p2, [EUser] = @p3 WHERE [MoveID] = @p4";
            let p: Vec<&dyn tiberius::ToSql> = vec![&note, &now, &emp_id, move_id];
            // 事务包裹：UPDATE 主表 + DELETE 旧明细 + INSERT 新明细 原子化
            let tx_result: std::result::Result<(), String> = async {
                inventory_ledger::begin_tran(&mut conn).await.map_err(|e| e.to_string())?;
                conn.execute(sql, &p).await.map_err(|e| e.to_string())?;
                // 删旧明细
                conn.execute("DELETE FROM [tStk_MoveDetail] WHERE [MoveID] = @p1", &[move_id]).await.map_err(|e| e.to_string())?;
                // 插新明细（字段映射：RealQty→Qty, DiffQty→AQty, SysQty→StkQty）
                for d in &params.details {
                    let detail_id = format!("{}", uuid::Uuid::new_v4());
                    let gds_id = empty_or_zero(&jstr(d, "GDSID")).to_string();
                    let sys_qty = jf64(d, "SysQty");
                    let real_qty = jf64(d, "RealQty");
                    let diff_qty = jf64(d, "DiffQty");
                    let detail_sql = r#"INSERT INTO [tStk_MoveDetail] ([MoveDetailID], [MoveID], [GDSID], [Qty], [AQty], [StkQty])
                        VALUES (@p1, @p2, @p3, @p4, @p5, @p6)"#;
                    let dp: Vec<&dyn tiberius::ToSql> = vec![&detail_id, move_id, &gds_id, &real_qty, &diff_qty, &sys_qty];
                    conn.execute(detail_sql, &dp).await.map_err(|e| e.to_string())?;
                }
                inventory_ledger::commit_tran(&mut conn).await.map_err(|e| e.to_string())?;
                Ok(())
            }.await;
            if let Err(e) = tx_result {
                inventory_ledger::rollback_tran(&mut conn).await;
                return Ok(Json(ApiResponse::err(&format!("盘点更新失败: {}", e))));
            }
            return Ok(Json(ApiResponse::msg("更新成功")));
        }
        "replenishment" => {
            // ===== 补货：按 ApplyNo（仓库编码-YYYYMMDD）解析，先删旧明细再插新 =====
            let apply_no = &params.id;
            // 解析 ApplyNo: 仓库编码-YYYYMMDD
            let (stk_code, date_key) = match apply_no.rsplit_once('-') {
                Some((code, date)) if date.len() == 8 => (code.to_string(), date.to_string()),
                _ => return Ok(Json(ApiResponse::err("无效的补货单号"))),
            };
            // 通过仓库编码查 StkID
            let find_sql = "SELECT TOP 1 [StkID] FROM [tBas_Stock] WHERE [StkCode] = @p1";
            let find_row = conn.query(find_sql, &[&stk_code]).await?.into_row().await?;
            let stk_id = if let Some(row) = find_row {
                get_str(&row, "StkID")
            } else {
                return Ok(Json(ApiResponse::err("未找到仓库")));
            };
            // 删除旧明细（该仓库该天所有记录）+ 插入新明细（事务包裹）
            let del_sql = "DELETE FROM [tArd_AR] WHERE [StkID] = @p1 AND CONVERT(varchar(8),[EDate],112) = @p2";
            // 插入新明细
            let date_str = format!(
                "{}-{}-{}",
                &date_key[0..4],
                &date_key[4..6],
                &date_key[6..8]
            );
            let batch_time = format!("{} 00:00:00", date_str);
            let used = "Y";
            let zero_price: f64 = 0.0;
            let tx_result: std::result::Result<(), String> = async {
                inventory_ledger::begin_tran(&mut conn).await.map_err(|e| e.to_string())?;
                conn.execute(del_sql, &[&stk_id, &date_key]).await.map_err(|e| e.to_string())?;
                for d in &params.details {
                    let gds_id = empty_or_zero(&jstr(d, "GDSID")).to_string();
                    let qty = jf64(d, "Qty");
                    let detail_sql = r#"INSERT INTO [tArd_AR] ([RowID], [StkID], [EmpID], [EDate], [SaleDate], [GDSID], [Qty], [Price], [Amt], [Used])
                        VALUES (NEWID(), @p1, @p2, @p3, @p3, @p4, @p5, @p6, @p7, @p8)"#;
                    let dp: Vec<&dyn tiberius::ToSql> = vec![
                        &stk_id, &emp_id, &batch_time, &gds_id, &qty, &zero_price, &zero_price, &used,
                    ];
                    conn.execute(detail_sql, &dp).await.map_err(|e| e.to_string())?;
                }
                inventory_ledger::commit_tran(&mut conn).await.map_err(|e| e.to_string())?;
                Ok(())
            }.await;
            if let Err(e) = tx_result {
                inventory_ledger::rollback_tran(&mut conn).await;
                return Ok(Json(ApiResponse::err(&format!("补货更新失败: {}", e))));
            }
            return Ok(Json(ApiResponse::msg("更新成功")));
        }
        _ => Ok(Json(ApiResponse::err(&format!(
            "不支持的更新类型: {}",
            params.kind
        )))),
    }
}

/// 删除
pub async fn delete_mobile_data(
    Extension(_claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<MobileDataDeleteParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    if params.ids.is_empty() {
        return Ok(Json(ApiResponse::err("未选择记录")));
    }

    if kind_to_pkind(&params.kind).is_some() {
        // ===== tSys_Parameters 批次删除：按 PCode 删除整批 =====
        for p_code in &params.ids {
            conn.execute(
                "DELETE FROM [tSys_Parameters] WHERE [PCode] = @p1",
                &[p_code],
            )
            .await?;
        }
        return Ok(Json(ApiResponse::msg("删除成功")));
    }

    match params.kind.as_str() {
        "stock_check" => {
            // ===== 盘点：软删除 State='D' =====
            for id in &params.ids {
                conn.execute(
                    "UPDATE [tStk_Move] SET [State] = 'D' WHERE [MoveID] = @p1",
                    &[id],
                )
                .await?;
            }
            return Ok(Json(ApiResponse::msg("删除成功")));
        }
        "replenishment" => {
            // ===== 补货：按 ApplyNo（仓库编码-YYYYMMDD）解析，物理删除 =====
            for apply_no in &params.ids {
                let (stk_code, date_key) = match apply_no.rsplit_once('-') {
                    Some((code, date)) if date.len() == 8 => (code.to_string(), date.to_string()),
                    _ => continue,
                };
                // 通过仓库编码查 StkID
                let find_sql = "SELECT TOP 1 [StkID] FROM [tBas_Stock] WHERE [StkCode] = @p1";
                let stream = match conn.query(find_sql, &[&stk_code]).await {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                let row = match stream.into_row().await {
                    Ok(Some(r)) => r,
                    _ => continue,
                };
                let stk_id = get_str(&row, "StkID");
                conn.execute(
                    "DELETE FROM [tArd_AR] WHERE [StkID] = @p1 AND CONVERT(varchar(8),[EDate],112) = @p2",
                    &[&stk_id, &date_key],
                ).await?;
            }
            return Ok(Json(ApiResponse::msg("删除成功")));
        }
        _ => Ok(Json(ApiResponse::err(&format!(
            "不支持的删除类型: {}",
            params.kind
        )))),
    }
}

/// 查询补货明细（按 ApplyNo 分组键查询所有明细行）
#[derive(Deserialize, Debug)]
pub struct MobileDataDetailParams {
    pub kind: String,
    pub id: String,
}

pub async fn get_mobile_data_detail(
    State(_config): State<Config>,
    Json(params): Json<MobileDataDetailParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;

    match params.kind.as_str() {
        "replenishment" => {
            // 按 ApplyNo（仓库编码-YYYYMMDD）查询 tArd_AR 明细
            let (stk_code, date_key) = match params.id.rsplit_once('-') {
                Some((code, date)) if date.len() == 8 => (code.to_string(), date.to_string()),
                _ => return Ok(Json(ApiResponse::err("无效的补货单号"))),
            };
            // 通过仓库编码查 StkID
            let find_sql = "SELECT TOP 1 [StkID] FROM [tBas_Stock] WHERE [StkCode] = @p1";
            let find_row = conn.query(find_sql, &[&stk_code]).await?.into_row().await?;
            let stk_id = if let Some(row) = find_row {
                get_str(&row, "StkID")
            } else {
                return Ok(Json(ApiResponse::err("未找到仓库")));
            };
            let sql = r#"SELECT a.[RowID], a.[GDSID], a.[Qty], a.[EDate],
                g.[GDSNO], g.[GDSDesc], g.[GDSSpec], g.[UnitNO]
                FROM [tArd_AR] a
                LEFT JOIN [tBas_Goods] g ON a.[GDSID] = g.[GDSID]
                WHERE a.[StkID] = @p1 AND CONVERT(varchar(8), a.[EDate], 112) = @p2
                ORDER BY g.[GDSNO]"#;
            let stream = conn.query(sql, &[&stk_id, &date_key]).await?;
            let rows: Vec<Row> = stream.into_first_result().await?;
            let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
            return Ok(Json(ApiResponse::ok(data)));
        }
        "stock_check" => {
            // 按 MoveID 查询 tStk_MoveDetail 明细
            let sql = r#"SELECT d.[MoveDetailID], d.[MoveID], d.[GDSID], d.[Qty] AS [RealQty], d.[AQty] AS [DiffQty], d.[StkQty] AS [SysQty],
                g.[GDSNO], g.[GDSDesc], g.[GDSSpec], g.[UnitNO]
                FROM [tStk_MoveDetail] d
                LEFT JOIN [tBas_Goods] g ON d.[GDSID] = g.[GDSID]
                WHERE d.[MoveID] = @p1
                ORDER BY g.[GDSNO]"#;
            let stream = conn.query(sql, &[&params.id]).await?;
            let rows: Vec<Row> = stream.into_first_result().await?;
            let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
            return Ok(Json(ApiResponse::ok(data)));
        }
        "special_price" | "reward_product" | "gift_giving" => {
            // 按 PCode 查询该批次所有商品明细
            let sql = r#"SELECT [ParametersID], [PCode], [PValue], [EDate]
                FROM [tSys_Parameters]
                WHERE [PCode] = @p1
                ORDER BY [EDate]"#;
            let stream = conn.query(sql, &[&params.id]).await?;
            let rows: Vec<Row> = stream.into_first_result().await?;
            let mut items: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
            // 批量解析 GDSID → GDSNO/GDSDesc，CustID → CustName
            items = enrich_with_names(&mut conn, items).await;
            // 解析 PValue JSON 平铺到顶层（提取 OrigPrice/NewPrice/Qty/Reason/StartDate/EndDate 等）
            items = items.into_iter().map(flatten_pvalue).collect();
            return Ok(Json(ApiResponse::ok(items)));
        }
        _ => Ok(Json(ApiResponse::ok(vec![]))),
    }
}
