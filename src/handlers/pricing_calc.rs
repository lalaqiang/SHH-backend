use crate::config::Config;
use crate::db::get_pool;
use crate::error::Result;
use crate::utils::{ApiResponse, row_get_f64};
use axum::{Json, extract::State};
use serde::Deserialize;
use serde_json::Value;
use tiberius::Row;

// =====================================================================
// 客户定价批量计算引擎
// 参考 88 文件 services/pricing.go 的 CalculatePrice + getMultiplierWithPriority
// 模板匹配优先级：客户表 PricingTemplateID > PTerm=CUSTOM 匹配 > 无模板（返回零售价）
// 价格规则优先级（用户需求）：自定义价格 > 商品规则 > 品牌规则 > 默认基数
//   ★ 所有策略类型都先查商品规则，再查品牌规则，最后用默认基数
// 策略类型：1=成本价×基数, 2=零售价×基数, 3=品牌成本价×基数, 4=品牌零售价×基数, 6=自定义价格
// =====================================================================

#[derive(Deserialize)]
pub struct PricingCalcBatchParams {
    pub cust_id: Option<String>,
    pub gds_ids: Option<Vec<String>>,
    // 无模板时返回的默认价字段：'SPrice'(零售价) | 'BPrice'(批发价) | 'AInPrice'(成本价)
    // 默认 'SPrice'
    pub price_field: Option<String>,
}

#[derive(Debug, Clone)]
struct PricingTemplate {
    id: String,
    name: String,
    strategy_type: i64,
    multiplier: f64,
    status: i64,
    product_rules: Vec<ProductRule>,
    brand_rules: Vec<BrandRule>,
    custom_prices: Vec<CustomPrice>, // 策略 6 自定义价格列表
    customer_rules: Vec<String>,     // cust_id 列表
    pterm: String,
}

#[derive(Debug, Clone)]
struct ProductRule {
    gds_id: String,
    multiplier: f64,
}

#[derive(Debug, Clone)]
struct BrandRule {
    brand_id: String,
    multiplier: f64,
}

#[derive(Debug, Clone)]
struct CustomPrice {
    gds_id: String,
    price: f64,
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

// 解析 PValue JSON 为 PricingTemplate
fn parse_template(id: &str, name: &str, pterm: &str, pvalue: &str) -> Option<PricingTemplate> {
    let cfg: Value = serde_json::from_str(pvalue).ok()?;
    let strategy_type = cfg
        .get("strategyType")
        .and_then(|v| v.as_i64())
        .unwrap_or(1);
    let multiplier = cfg
        .get("multiplier")
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0);
    let status = cfg.get("status").and_then(|v| v.as_i64()).unwrap_or(1);

    let mut product_rules = Vec::new();
    if let Some(arr) = cfg.get("productRules").and_then(|v| v.as_array()) {
        for r in arr {
            let gds_id = r
                .get("gdsId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let mult = r.get("multiplier").and_then(|v| v.as_f64()).unwrap_or(0.0);
            if !gds_id.is_empty() && mult > 0.0 {
                product_rules.push(ProductRule {
                    gds_id,
                    multiplier: mult,
                });
            }
        }
    }

    let mut brand_rules = Vec::new();
    if let Some(arr) = cfg.get("brandRules").and_then(|v| v.as_array()) {
        for r in arr {
            let brand_id = r
                .get("brandId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let mult = r.get("multiplier").and_then(|v| v.as_f64()).unwrap_or(0.0);
            if !brand_id.is_empty() && mult > 0.0 {
                brand_rules.push(BrandRule {
                    brand_id,
                    multiplier: mult,
                });
            }
        }
    }

    let mut customer_rules = Vec::new();
    if let Some(arr) = cfg.get("customerRules").and_then(|v| v.as_array()) {
        for r in arr {
            if let Some(cid) = r.get("custId").and_then(|v| v.as_str()) {
                if !cid.is_empty() {
                    customer_rules.push(cid.to_string());
                }
            }
        }
    }

    // 解析自定义价格列表（策略 6）
    let mut custom_prices = Vec::new();
    if let Some(arr) = cfg.get("customPrices").and_then(|v| v.as_array()) {
        for r in arr {
            let gds_id = r
                .get("gdsId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let price = r.get("price").and_then(|v| v.as_f64()).unwrap_or(0.0);
            if !gds_id.is_empty() && price > 0.0 {
                custom_prices.push(CustomPrice { gds_id, price });
            }
        }
    }

    Some(PricingTemplate {
        id: id.to_string(),
        name: name.to_string(),
        strategy_type,
        multiplier,
        status,
        product_rules,
        brand_rules,
        custom_prices,
        customer_rules,
        pterm: pterm.to_string(),
    })
}

// 按优先级获取基数：商品规则 > 品牌规则 > 默认基数
// ★ 用户需求：所有策略类型都先查商品规则（优先级最高），再查品牌规则，最后用默认基数
fn get_multiplier_with_priority(
    tpl: &PricingTemplate,
    gds_id: &str,
    brand_id: &str,
) -> (f64, &'static str) {
    // ★ UUID 大小写不敏感比较（前端可能传小写，模板配置可能存大写）
    let gid_upper = gds_id.to_uppercase();
    let bid_upper = brand_id.to_uppercase();
    // 优先级 1：商品规则（最高优先级，用户明确要求）
    for r in &tpl.product_rules {
        if r.gds_id.to_uppercase() == gid_upper {
            return (r.multiplier, "商品规则");
        }
    }
    // 优先级 2：品牌规则
    if !bid_upper.is_empty() {
        for r in &tpl.brand_rules {
            if r.brand_id.to_uppercase() == bid_upper {
                return (r.multiplier, "品牌规则");
            }
        }
    }
    // 优先级 3：默认基数
    (tpl.multiplier, "默认基数")
}

// 根据策略计算价格，返回 (final_price, base_price, multiplier, matched_rule)
fn calculate_price(
    tpl: &PricingTemplate,
    cost_price: f64,
    retail_price: f64,
    wholesale_price: f64,
    gds_id: &str,
    brand_id: &str,
) -> (f64, f64, f64, String) {
    // 策略 6：自定义价格（优先级最高，直接返回预设价格）
    // 未命中的商品回退到默认零售价
    if tpl.strategy_type == 6 {
        let gid_upper = gds_id.to_uppercase();
        for cp in &tpl.custom_prices {
            if cp.gds_id.to_uppercase() == gid_upper {
                return (
                    round2(cp.price).max(0.0),
                    cp.price,
                    1.0,
                    "自定义价格".to_string(),
                );
            }
        }
        // 自定义价格未命中：回退到零售价
        return (
            round2(retail_price).max(0.0),
            retail_price,
            1.0,
            "默认零售价".to_string(),
        );
    }

    // ★ 所有策略类型都先查商品规则 > 品牌规则 > 默认基数（用户需求：商品定价规则优先级最高）
    let (multiplier, matched_rule) = get_multiplier_with_priority(tpl, gds_id, brand_id);

    // 策略 1/3 用成本价作为基准价，策略 2/4 用零售价作为基准价
    let base_price = match tpl.strategy_type {
        1 | 3 => cost_price,
        2 | 4 => retail_price,
        _ => retail_price,
    };
    let _ = wholesale_price; // 预留：未来策略可支持批发价基准
    let final_price = round2(base_price * multiplier).max(0.0);
    (
        final_price,
        base_price,
        multiplier,
        matched_rule.to_string(),
    )
}

pub async fn calc_batch(
    State(_config): State<Config>,
    Json(params): Json<PricingCalcBatchParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let cust_id = params.cust_id.as_deref().unwrap_or("");
    let gds_ids: Vec<String> = params.gds_ids.clone().unwrap_or_default();
    let price_field = params.price_field.as_deref().unwrap_or("SPrice");
    if cust_id.is_empty() {
        return Ok(Json(ApiResponse::err("cust_id 必填")));
    }
    if gds_ids.is_empty() {
        return Ok(Json(ApiResponse::ok(serde_json::json!({
            "custId": cust_id,
            "templateId": "",
            "templateName": "",
            "items": [],
        }))));
    }

    // 1. 查客户绑定的 PricingTemplateID
    // ★ uniqueidentifier 列必须 CONVERT 成 varchar，否则 tiberius row.get::<&str, _> 会 panic
    let cust_sql = "SELECT CONVERT(varchar(40), PricingTemplateID) AS PricingTemplateID FROM tBas_Cust WHERE CustID = @p1";
    let cust_stream = conn.query(cust_sql, &[&cust_id]).await?;
    let cust_row = match cust_stream.into_row().await? {
        Some(r) => r,
        None => return Ok(Json(ApiResponse::err("客户不存在"))),
    };
    let bound_tpl_id: Option<String> = cust_row
        .get::<&str, _>("PricingTemplateID")
        .map(|s| s.to_string());

    // 2. 加载所有定价模板（tSys_Parameters 没有 State 字段，启用状态在 PValue JSON.status）
    let tpl_sql = "SELECT CONVERT(varchar(40), ParametersID) AS ParametersID, PCode, PName, PTerm, PValue FROM tSys_Parameters WHERE PKind = 'pricing' ORDER BY EDate DESC";
    let tpl_stream = conn.query(tpl_sql, &[]).await?;
    let tpl_rows: Vec<Row> = tpl_stream.into_first_result().await?;
    let mut all_templates: Vec<PricingTemplate> = Vec::new();
    for r in tpl_rows.iter() {
        let id = r.get::<&str, _>("ParametersID").unwrap_or("").to_string();
        let name = r.get::<&str, _>("PName").unwrap_or("").to_string();
        let pterm = r.get::<&str, _>("PTerm").unwrap_or("ALL").to_string();
        let pvalue = r.get::<&str, _>("PValue").unwrap_or("{}").to_string();
        if id.is_empty() {
            continue;
        }
        if let Some(tpl) = parse_template(&id, &name, &pterm, &pvalue) {
            all_templates.push(tpl);
        }
    }

    // 3. 按优先级选出对当前客户生效的模板
    //    ★ 用户需求：客户没有客户定价时，直接用商品零售价（不做 PTerm=ALL 全局兜底）
    //    优先级：客户表 PricingTemplateID > PTerm=CUSTOM 匹配 > 无模板（返回零售价）
    let mut matched_template: Option<&PricingTemplate> = None;
    // 3.1 客户表直接绑定（最高优先级）
    if let Some(bid) = &bound_tpl_id {
        if !bid.is_empty() && bid != "00000000-0000-0000-0000-000000000000" {
            matched_template = all_templates.iter().find(|t| t.id == *bid && t.status == 1);
        }
    }
    // 3.2 PTerm=CUSTOM 且 customerRules 含 custId
    if matched_template.is_none() {
        matched_template = all_templates.iter().find(|t| {
            t.status == 1 && t.pterm == "CUSTOM" && t.customer_rules.iter().any(|c| c == cust_id)
        });
    }
    // ★ 不做 PTerm=ALL 全局兜底：客户没有专属定价模板时，直接返回商品零售价

    let (template_id, template_name) = match matched_template {
        Some(t) => (t.id.clone(), t.name.clone()),
        None => (String::new(), String::new()),
    };

    // 4. 批量查询商品信息（GDSID, GDSNO, GDSDesc, BrandID, AInPrice, SPrice, BPrice）
    //    用 IN 子查询，避免逐个查询
    // ★ SQL Server collation gb18030 默认大小写敏感，uniqueidentifier 比较要统一转大写
    //   否则前端传入的小写 UUID 可能匹配不上数据库存储的 GUID 字符串
    let gds_ids_str: Vec<String> = gds_ids
        .iter()
        .map(|s| format!("'{}'", s.to_uppercase().replace('\'', "''")))
        .collect();
    let in_clause = gds_ids_str.join(",");
    let gds_sql = format!(
        "SELECT CONVERT(varchar(40), GDSID) AS GDSID, GDSNO, GDSDesc, CONVERT(varchar(40), BrandID) AS BrandID, \
         ISNULL(AInPrice, 0) AS AInPrice, ISNULL(SPrice, 0) AS SPrice, ISNULL(BPrice, 0) AS BPrice \
         FROM tBas_Goods WHERE UPPER(CONVERT(varchar(40), GDSID)) IN ({}) AND State <> 'D'",
        in_clause
    );
    let gds_stream = conn.query(&gds_sql, &[]).await?;
    let gds_rows: Vec<Row> = gds_stream.into_first_result().await?;

    // 5. 对每个商品计算价格
    let mut items = Vec::new();
    for gid in &gds_ids {
        let gid_norm = gid.trim();
        if gid_norm.is_empty() {
            continue;
        }
        // ★ 大小写不敏感比较（UUID 在 SQL Server 返回大写，前端可能传小写）
        let gid_upper = gid_norm.to_uppercase();
        let gds_row = gds_rows.iter().find(|r| {
            let v = r.get::<&str, _>("GDSID").unwrap_or("");
            v.to_uppercase() == gid_upper
        });
        let (final_price, base_price, multiplier, matched_rule, gds_no, gds_desc, brand_id) =
            if let Some(r) = gds_row {
                let gds_no = r.get::<&str, _>("GDSNO").unwrap_or("").to_string();
                let gds_desc = r.get::<&str, _>("GDSDesc").unwrap_or("").to_string();
                let brand_id = r.get::<&str, _>("BrandID").unwrap_or("").to_string();
                let cost_price = row_get_f64(r, "AInPrice");
                let retail_price = row_get_f64(r, "SPrice");
                let wholesale_price = row_get_f64(r, "BPrice");
                if let Some(tpl) = matched_template {
                    let (fp, bp, m, rule) = calculate_price(
                        tpl,
                        cost_price,
                        retail_price,
                        wholesale_price,
                        gid_norm,
                        &brand_id,
                    );
                    (fp, bp, m, rule, gds_no, gds_desc, brand_id)
                } else {
                    // 无模板：返回默认价（price_field 指定）
                    let default_price = match price_field {
                        "BPrice" => wholesale_price,
                        "AInPrice" => cost_price,
                        _ => retail_price,
                    };
                    (
                        round2(default_price).max(0.0),
                        default_price,
                        1.0,
                        "默认价".to_string(),
                        gds_no,
                        gds_desc,
                        brand_id,
                    )
                }
            } else {
                // 商品不存在
                (
                    0.0,
                    0.0,
                    1.0,
                    "商品不存在".to_string(),
                    String::new(),
                    String::new(),
                    String::new(),
                )
            };
        items.push(serde_json::json!({
            "gdsId": gid_norm,
            "gdsNo": gds_no,
            "gdsDesc": gds_desc,
            "brandId": brand_id,
            "basePrice": round2(base_price),
            "multiplier": multiplier,
            "matchedRule": matched_rule,
            "finalPrice": final_price,
        }));
    }

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "custId": cust_id,
        "templateId": template_id,
        "templateName": template_name,
        "priceField": price_field,
        "count": items.len(),
        "items": items,
    }))))
}
