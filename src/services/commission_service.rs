//! 提成计算服务
//!
//! 完全对齐 88 文件项目 commission service 实现：
//! - 解析 tSys_Parameters.PValue JSON 获取提成模板配置
//! - 优先级：商品规则 > 品牌规则 > 默认提成率（阶梯/按销售额/按毛利/按数量）
//! - 商品/品牌规则均按比例计算：Commission = Amt × CommissionRate
//! - CommissionRate 存储为小数（0.12 = 12%），前端显示百分比（12）
//! - 结果写入 tSal_InvDetail 的 CommissionRate/CommissionType/Commission 字段
//!
//! 提成方式 (type)：
//! - 1=按销售额：提成 = Amt × rate/100
//! - 2=按销售毛利：提成 = (Amt - Cost) × rate/100
//! - 3=按销售量：提成 = Qty × rate/100
//! - 4=阶梯提成：按 Amt 匹配 Tiers 区间，提成 = Amt × tier.rate/100
//!
//! 调用时机：门店销售单保存/审核后由前端调用 /api/commission/recalc-invoice

use crate::utils::{row_get_f64, row_get_uuid_str};
use bb8::PooledConnection;
use bb8_tiberius::ConnectionManager;
use serde::Deserialize;
use tiberius::ToSql;

pub type Conn = PooledConnection<'static, ConnectionManager>;

/// PValue JSON 结构（与前端 commission-template/Index.vue 的 defaultForm 对齐）
///
/// ★ 对齐 88 项目：商品/品牌规则的 commission 字段存储提成比例（小数）
///   - 前端显示百分比（如 12），保存时 ÷100 转小数（0.12）存储
///   - 后端计算：提成 = 销售额 × 比例
#[derive(Debug, Deserialize, Default)]
pub struct CommissionTemplate {
    /// 提成方式: 1=按销售额, 2=按销售毛利, 3=按销售量, 4=阶梯提成
    #[serde(default)]
    pub r#type: i32,
    /// 默认提成率(%)，如 5 表示 5%
    #[serde(default)]
    pub rate: f64,
    /// 基础金额门槛（0=不限制）
    #[serde(default)]
    pub base_amount: f64,
    /// 封顶金额（0=不限封顶）
    #[serde(default)]
    pub max_amount: f64,
    /// 1=启用, 0=停用
    #[serde(default = "default_status")]
    pub status: i32,
    /// 模板生效起始日期（YYYY-MM-DD，空=不限制）
    #[serde(default)]
    pub eff_s_date: String,
    /// 模板生效结束日期（YYYY-MM-DD，空=不限制）
    #[serde(default)]
    pub eff_e_date: String,
    /// 商品提成规则（按比例，优先级最高）
    /// commission 字段为提成比例（小数，0.12=12%）
    #[serde(default)]
    pub product_rules: Vec<ProductRule>,
    /// 品牌提成规则（按比例）
    /// commission 字段为提成比例（小数，0.12=12%）
    #[serde(default)]
    pub brand_rules: Vec<BrandRule>,
    /// 阶梯提成区间（type=4 时使用）
    #[serde(default)]
    pub tiers: Vec<Tier>,
    /// 指定员工列表（PTerm='CUSTOM' 时仅这些员工生效，存 EmpID 数组）
    #[serde(default)]
    pub employee_ids: Vec<String>,
}

fn default_status() -> i32 {
    1
}

#[derive(Debug, Deserialize, Default)]
pub struct ProductRule {
    #[serde(default)]
    pub gds_id: String,
    /// 提成比例（小数，0.12=12%），对齐 88 项目
    #[serde(default)]
    pub commission: f64,
}

#[derive(Debug, Deserialize, Default)]
pub struct BrandRule {
    #[serde(default)]
    pub brand_id: String,
    /// 提成比例（小数，0.12=12%），对齐 88 项目
    #[serde(default)]
    pub commission: f64,
}

#[derive(Debug, Deserialize, Default)]
pub struct Tier {
    /// 区间下限（含）
    #[serde(default)]
    pub min: f64,
    /// 区间上限（不含，0=无上限）
    #[serde(default)]
    pub max: f64,
    /// 该区间提成率(%)
    #[serde(default)]
    pub rate: f64,
}

/// 单行提成计算结果
struct CommissionResult {
    rate: f64,   // 提成比例（小数，如 0.12）
    ctype: i32,  // 0=无, 1=商品规则, 2=品牌规则
    amount: f64, // 提成金额
}

/// 根据模板和商品信息计算单行提成
///
/// ★ 完全对齐 88 文件项目 services/commission.go 的 CalculateCommission：
///   1. 模板停用 → 无提成
///   2. 商品规则匹配（精确匹配 GDSID）→ 提成 = Amt × commission
///   3. 品牌规则匹配（匹配 BrandID）→ 提成 = Amt × commission
///   4. 默认提成率 → 按 type 区分计算方式（销售额/毛利/数量/阶梯）
///
/// amt = Qty × Price（行金额）
/// cost = Qty × CostPrice（行成本，type=2 时用）
/// product_brand_id = 商品的 BrandID
fn calc_row_commission(
    tpl: &CommissionTemplate,
    gds_id: &str,
    product_brand_id: &str,
    amt: f64,
    qty: f64,
    cost: f64,
) -> CommissionResult {
    // 1. 模板停用 → 无提成
    if tpl.status != 1 {
        return CommissionResult {
            rate: 0.0,
            ctype: 0,
            amount: 0.0,
        };
    }

    // 2. 优先匹配商品规则（按比例：提成 = 销售额 × 比例）
    if !gds_id.is_empty() {
        for rule in &tpl.product_rules {
            if rule.gds_id.eq_ignore_ascii_case(gds_id) && rule.commission > 0.0 {
                // ★ 对齐 88 项目：commission 是小数比例（0.12=12%），提成 = Amt × commission
                let comm = amt * rule.commission;
                return CommissionResult {
                    rate: rule.commission,
                    ctype: 1,
                    amount: comm,
                };
            }
        }
    }

    // 3. 品牌规则（按比例：提成 = 销售额 × 比例）
    if !product_brand_id.is_empty() {
        for rule in &tpl.brand_rules {
            if rule.brand_id.eq_ignore_ascii_case(product_brand_id) && rule.commission > 0.0 {
                // ★ 对齐 88 项目：commission 是小数比例（0.12=12%），提成 = Amt × commission
                let comm = amt * rule.commission;
                return CommissionResult {
                    rate: rule.commission,
                    ctype: 2,
                    amount: comm,
                };
            }
        }
    }

    // 4. 默认提成率：按 type 区分计算方式
    if tpl.rate > 0.0 || tpl.r#type == 4 {
        // 基础金额门槛（按销售额判断）
        if tpl.base_amount > 0.0 && amt < tpl.base_amount {
            return CommissionResult {
                rate: 0.0,
                ctype: 0,
                amount: 0.0,
            };
        }

        let (rate, raw_comm) = match tpl.r#type {
            // 按销售毛利：提成 = (Amt - Cost) × rate/100
            2 => {
                let profit = amt - cost;
                if profit <= 0.0 || tpl.rate <= 0.0 {
                    return CommissionResult {
                        rate: 0.0,
                        ctype: 0,
                        amount: 0.0,
                    };
                }
                let r = tpl.rate / 100.0;
                (r, profit * r)
            }
            // 按销售量：提成 = Qty × rate/100
            3 => {
                if tpl.rate <= 0.0 {
                    return CommissionResult {
                        rate: 0.0,
                        ctype: 0,
                        amount: 0.0,
                    };
                }
                let r = tpl.rate / 100.0;
                (r, qty * r)
            }
            // 阶梯提成：按 Amt 匹配 Tiers 区间，提成 = Amt × tier.rate/100
            4 => {
                if tpl.tiers.is_empty() {
                    return CommissionResult {
                        rate: 0.0,
                        ctype: 0,
                        amount: 0.0,
                    };
                }
                // 匹配区间：min <= amt < max（max=0 表示无上限）
                let tier = tpl
                    .tiers
                    .iter()
                    .find(|t| t.rate > 0.0 && amt >= t.min && (t.max <= 0.0 || amt < t.max));
                match tier {
                    Some(t) => {
                        let r = t.rate / 100.0;
                        (r, amt * r)
                    }
                    None => {
                        return CommissionResult {
                            rate: 0.0,
                            ctype: 0,
                            amount: 0.0,
                        };
                    }
                }
            }
            // 默认（type=1 或未设置）：按销售额，提成 = Amt × rate/100
            _ => {
                if tpl.rate <= 0.0 {
                    return CommissionResult {
                        rate: 0.0,
                        ctype: 0,
                        amount: 0.0,
                    };
                }
                let r = tpl.rate / 100.0;
                (r, amt * r)
            }
        };

        // 封顶
        let final_comm = if tpl.max_amount > 0.0 && raw_comm > tpl.max_amount {
            tpl.max_amount
        } else {
            raw_comm
        };
        return CommissionResult {
            rate,
            ctype: 0,
            amount: final_comm,
        };
    }

    CommissionResult {
        rate: 0.0,
        ctype: 0,
        amount: 0.0,
    }
}

/// 检查模板是否对指定员工生效
/// - PTerm='ALL' 或空：对所有员工生效
/// - PTerm='CUSTOM'：仅对 employee_ids 中的员工生效
fn is_template_for_employee(tpl: &CommissionTemplate, emp_id: &str, pterm: &str) -> bool {
    if pterm.eq_ignore_ascii_case("CUSTOM") {
        // 指定员工模式：检查 emp_id 是否在 employee_ids 列表中
        if emp_id.is_empty() {
            return false;
        }
        return tpl
            .employee_ids
            .iter()
            .any(|id| id.eq_ignore_ascii_case(emp_id));
    }
    // ALL 或其他：对所有员工生效
    true
}

/// 检查模板是否在有效期内
fn is_template_in_effect(tpl: &CommissionTemplate, today: &str) -> bool {
    // 空日期不限制
    if !tpl.eff_s_date.is_empty() && today < tpl.eff_s_date.as_str() {
        return false;
    }
    if !tpl.eff_e_date.is_empty() && today > tpl.eff_e_date.as_str() {
        return false;
    }
    true
}

/// 重算指定销售单的所有明细行提成
/// 调用时机：前端保存/审核门店销售单后
pub async fn recalc_invoice_commission(conn: &mut Conn, siid: &str) -> Result<usize, String> {
    // 1. 查销售单主表，获取 StkID + EmpID + 单据日期
    let master_sql = r#"SELECT CONVERT(varchar(40), StkID) AS StkID,
                               CONVERT(varchar(40), EmpID) AS EmpID,
                               CONVERT(varchar(10), EDate, 120) AS EDate
                        FROM tSal_Inv WHERE SIID = @p1 AND State <> 'D'"#;
    let stream = conn
        .query(master_sql, &[&siid.to_string()])
        .await
        .map_err(|e| format!("查询销售单主表失败: {}", e))?;
    let row = stream
        .into_row()
        .await
        .map_err(|e| format!("读取销售单主表失败: {}", e))?
        .ok_or_else(|| "销售单不存在或已删除".to_string())?;
    let stk_id: String = row.get::<&str, _>("StkID").unwrap_or("").to_string();
    let emp_id: String = row.get::<&str, _>("EmpID").unwrap_or("").to_string();
    let edate: String = row.get::<&str, _>("EDate").unwrap_or("").to_string();
    if stk_id.is_empty() {
        return Err("销售单 StkID 为空".to_string());
    }

    // 2. 查仓库的提成模板ID + PTerm
    let wh_sql = r#"SELECT CONVERT(varchar(40), CommissionTemplateID) AS CommissionTemplateID,
                           ISNULL(PTerm, 'ALL') AS PTerm
                    FROM tBas_Stock s
                    LEFT JOIN tSys_Parameters p ON s.CommissionTemplateID = p.ParametersID
                    WHERE s.StkID = @p1"#;
    let stream = conn
        .query(wh_sql, &[&stk_id])
        .await
        .map_err(|e| format!("查询仓库提成模板失败: {}", e))?;
    let row = stream
        .into_row()
        .await
        .map_err(|e| format!("读取仓库提成模板失败: {}", e))?
        .ok_or_else(|| format!("仓库不存在: {}", stk_id))?;
    let tpl_id: String = row
        .get::<&str, _>("CommissionTemplateID")
        .unwrap_or("")
        .to_string();
    let pterm: String = row.get::<&str, _>("PTerm").unwrap_or("ALL").to_string();

    // 3. 加载提成模板
    let tpl = if tpl_id.is_empty() {
        // 仓库未挂模板 → 尝试全局默认模板（PTerm='ALL'）
        load_default_template(conn).await?
    } else {
        load_template_by_id(conn, &tpl_id).await?
    };

    // 4. 校验模板是否对当前员工生效（PTerm='CUSTOM' 时仅指定员工生效）
    if !is_template_for_employee(&tpl, &emp_id, &pterm) {
        tracing::info!(
            "recalc_invoice_commission: 模板 {} 不对员工 {} 生效（PTerm={}），跳过提成计算",
            tpl_id,
            emp_id,
            pterm
        );
        // 清零所有明细行的提成
        clear_invoice_commission(conn, siid).await?;
        return Ok(0);
    }

    // 5. 校验模板有效期（用单据日期判断）
    let today = if edate.len() >= 10 {
        edate[..10].to_string()
    } else {
        String::new()
    };
    if !is_template_in_effect(&tpl, &today) {
        tracing::info!(
            "recalc_invoice_commission: 模板 {} 不在有效期（{}/{}），跳过提成计算",
            tpl_id,
            tpl.eff_s_date,
            tpl.eff_e_date
        );
        clear_invoice_commission(conn, siid).await?;
        return Ok(0);
    }

    // 6. 查销售单明细行 + 商品BrandID + 成本价
    let detail_sql = r#"SELECT CONVERT(varchar(40), d.SIDetailID) AS SIDetailID,
                               CONVERT(varchar(40), d.GDSID) AS GDSID,
                               ISNULL(d.Qty, 0) AS Qty, ISNULL(d.Price, 0) AS Price,
                               ISNULL(d.Amt, 0) AS Amt,
                               CONVERT(varchar(40), g.BrandID) AS BrandID,
                               ISNULL(g.AInPrice, 0) AS AInPrice
                        FROM tSal_InvDetail d
                        LEFT JOIN tBas_Goods g ON d.GDSID = g.GDSID
                        WHERE d.SIID = @p1"#;
    let stream = conn
        .query(detail_sql, &[&siid.to_string()])
        .await
        .map_err(|e| format!("查询销售单明细失败: {}", e))?;
    let rows: Vec<tiberius::Row> = stream
        .into_first_result()
        .await
        .map_err(|e| format!("读取销售单明细失败: {}", e))?;

    if rows.is_empty() {
        return Ok(0);
    }

    // 7. 批量计算所有明细行的提成（内存计算，避免 N 次数据库往返）
    let mut updated = 0usize;
    let mut total_commission = 0.0f64;
    // 收集 (detail_id, rate, ctype, amount) 用于批量 UPDATE
    let mut batch: Vec<(String, f64, i32, f64)> = Vec::with_capacity(rows.len());

    for row in &rows {
        let detail_id = row_get_uuid_str(row, "SIDetailID");
        let gds_id = row_get_uuid_str(row, "GDSID");
        let brand_id = row_get_uuid_str(row, "BrandID");
        let qty = row_get_f64(row, "Qty");
        let amt = row_get_f64(row, "Amt");
        let cost_price = row_get_f64(row, "AInPrice");
        let cost = qty * cost_price; // 行成本 = 数量 × 进价

        let result = calc_row_commission(&tpl, &gds_id, &brand_id, amt, qty, cost);
        total_commission += result.amount;
        updated += 1;
        batch.push((detail_id, result.rate, result.ctype, result.amount));
    }

    // 8. 批量更新：用 CASE WHEN 单条 SQL 更新所有明细行（避免 N 次 UPDATE 往返）
    if !batch.is_empty() {
        let mut case_rate = String::from("CASE SIDetailID ");
        let mut case_type = String::from("CASE SIDetailID ");
        let mut case_amt = String::from("CASE SIDetailID ");
        let mut params: Vec<Option<String>> = Vec::with_capacity(batch.len() * 4);

        for (i, (did, rate, ctype, amt)) in batch.iter().enumerate() {
            let pidx = i * 4 + 1;
            case_rate.push_str(&format!(
                " WHEN CAST(@p{} AS uniqueidentifier) THEN CAST(@p{} AS float) ",
                pidx,
                pidx + 1
            ));
            case_type.push_str(&format!(
                " WHEN CAST(@p{} AS uniqueidentifier) THEN CAST(@p{} AS int) ",
                pidx,
                pidx + 2
            ));
            case_amt.push_str(&format!(
                " WHEN CAST(@p{} AS uniqueidentifier) THEN CAST(@p{} AS float) ",
                pidx,
                pidx + 3
            ));
            params.push(Some(did.clone()));
            params.push(Some(format!("{}", rate)));
            params.push(Some(format!("{}", ctype)));
            params.push(Some(format!("{}", amt)));
        }
        case_rate.push_str("ELSE CommissionRate END");
        case_type.push_str("ELSE CommissionType END");
        case_amt.push_str("ELSE Commission END");

        let batch_sql = format!(
            "UPDATE tSal_InvDetail SET CommissionRate = {}, CommissionType = {}, Commission = {} WHERE SIDetailID IN ({})",
            case_rate,
            case_type,
            case_amt,
            batch
                .iter()
                .enumerate()
                .map(|(i, _)| format!("CAST(@p{} AS uniqueidentifier)", i * 4 + 1))
                .collect::<Vec<_>>()
                .join(",")
        );

        let param_refs: Vec<&dyn ToSql> = params.iter().map(|v| v as &dyn ToSql).collect();
        let batch_err = match conn.query(&batch_sql, &param_refs).await {
            Ok(_) => None,
            Err(e) => Some(e),
        };
        if let Some(e) = batch_err {
            tracing::warn!("批量更新提成失败，回退到清零: {}", e);
            let _ = clear_invoice_commission(conn, siid).await;
        }
    }

    // 同步更新主表 TotalCommission 字段（便于列表显示，无需逐行聚合）
    // 使用 IF EXISTS 兼容旧表（未应用迁移 022 时跳过）
    let sync_sql = r#"IF EXISTS (SELECT 1 FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_NAME = 'tSal_Inv' AND COLUMN_NAME = 'TotalCommission')
        UPDATE tSal_Inv SET TotalCommission = @p1 WHERE SIID = @p2"#;
    let _ = conn
        .query(sync_sql, &[&total_commission, &siid.to_string()])
        .await
        .map_err(|e| {
            tracing::warn!(
                "同步 tSal_Inv.TotalCommission 失败（不影响明细计算）: {}",
                e
            );
        });

    tracing::info!(
        "recalc_invoice_commission: siid={} updated={}/{} total_commission={}",
        siid,
        updated,
        rows.len(),
        total_commission
    );
    Ok(updated)
}

/// 清零销售单所有明细行的提成字段
async fn clear_invoice_commission(conn: &mut Conn, siid: &str) -> Result<usize, String> {
    let sql = "UPDATE tSal_InvDetail SET CommissionRate = 0, CommissionType = 0, Commission = 0 WHERE SIID = @p1";
    let _ = conn
        .query(sql, &[&siid.to_string()])
        .await
        .map_err(|e| format!("清零提成失败: {}", e))?;
    Ok(0)
}

/// 按模板ID加载提成模板
async fn load_template_by_id(conn: &mut Conn, tpl_id: &str) -> Result<CommissionTemplate, String> {
    let sql =
        "SELECT PValue FROM tSys_Parameters WHERE ParametersID = @p1 AND PKind = 'commission'";
    let stream = conn
        .query(sql, &[&tpl_id.to_string()])
        .await
        .map_err(|e| format!("查询提成模板失败: {}", e))?;
    let row = stream
        .into_row()
        .await
        .map_err(|e| format!("读取提成模板失败: {}", e))?
        .ok_or_else(|| "提成模板不存在".to_string())?;
    let pvalue: String = row.get::<&str, _>("PValue").unwrap_or("").to_string();
    if pvalue.is_empty() {
        return Ok(CommissionTemplate::default());
    }
    parse_template_json(&pvalue)
}

/// 加载全局默认提成模板（PTerm='ALL' 且 status=1，最新一条）
async fn load_default_template(conn: &mut Conn) -> Result<CommissionTemplate, String> {
    // ★ 过滤 status=1（启用），避免加载已停用的模板
    //   PValue 中 status 字段可能未设置（旧数据），用 ISNULL + LIKE 兜底
    let sql = r#"SELECT TOP 1 PValue FROM tSys_Parameters
                 WHERE PKind = 'commission' AND PTerm = 'ALL'
                 AND (PValue LIKE '%"status":1%' OR PValue NOT LIKE '%"status":%')
                 ORDER BY EDate DESC"#;
    let stream = conn
        .query(sql, &[])
        .await
        .map_err(|e| format!("查询默认提成模板失败: {}", e))?;
    let row = match stream.into_row().await {
        Ok(Some(r)) => r,
        _ => return Ok(CommissionTemplate::default()),
    };
    let pvalue: String = row.get::<&str, _>("PValue").unwrap_or("").to_string();
    if pvalue.is_empty() {
        return Ok(CommissionTemplate::default());
    }
    parse_template_json(&pvalue)
}

/// 解析 PValue JSON（兼容前端 camelCase 和 snake_case）
///
/// ★ 对齐 88 项目：商品/品牌规则的 commission 字段为提成比例（小数）
///   前端保存时：用户输入 12（百分比） → ÷100 → 存储 0.12（小数）
///   后端读取时：0.12 × 销售额 = 提成金额
fn parse_template_json(json_str: &str) -> Result<CommissionTemplate, String> {
    let val: serde_json::Value =
        serde_json::from_str(json_str).map_err(|e| format!("解析提成模板JSON失败: {}", e))?;

    let tpl = CommissionTemplate {
        r#type: val
            .get("type")
            .or_else(|| val.get("commissionType"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32,
        rate: val.get("rate").and_then(|v| v.as_f64()).unwrap_or(0.0),
        base_amount: val
            .get("baseAmount")
            .or_else(|| val.get("base_amount"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        max_amount: val
            .get("maxAmount")
            .or_else(|| val.get("max_amount"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        status: val.get("status").and_then(|v| v.as_i64()).unwrap_or(1) as i32,
        eff_s_date: val
            .get("effSDate")
            .or_else(|| val.get("eff_s_date"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        eff_e_date: val
            .get("effEDate")
            .or_else(|| val.get("eff_e_date"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        product_rules: val
            .get("productRules")
            .or_else(|| val.get("product_rules"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|v| ProductRule {
                        gds_id: v
                            .get("gdsId")
                            .or_else(|| v.get("gds_id"))
                            .or_else(|| v.get("GDSID"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        commission: v.get("commission").and_then(|v| v.as_f64()).unwrap_or(0.0),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        brand_rules: val
            .get("brandRules")
            .or_else(|| val.get("brand_rules"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|v| BrandRule {
                        brand_id: v
                            .get("brandId")
                            .or_else(|| v.get("brand_id"))
                            .or_else(|| v.get("BrandID"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        commission: v.get("commission").and_then(|v| v.as_f64()).unwrap_or(0.0),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        tiers: val
            .get("tiers")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|v| Tier {
                        min: v
                            .get("min")
                            .or_else(|| v.get("Min"))
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0),
                        max: v
                            .get("max")
                            .or_else(|| v.get("Max"))
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0),
                        rate: v
                            .get("rate")
                            .or_else(|| v.get("Rate"))
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        employee_ids: val
            .get("employeeIds")
            .or_else(|| val.get("employee_ids"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default(),
    };

    Ok(tpl)
}
