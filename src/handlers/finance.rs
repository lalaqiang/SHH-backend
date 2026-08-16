use crate::config::Config;
use crate::db::get_pool;
use crate::error::Result;
use crate::handlers::base_data::row_to_json;
use crate::middleware::auth::Claims;
use crate::utils::{ApiResponse, build_pagination_sql_with_sort};
use axum::extract::{Extension, Json, State};
use serde::Deserialize;
use tiberius::Row;

#[derive(Deserialize)]
pub struct PaginationParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
    pub sort_prop: Option<String>,
    pub sort_order: Option<String>,
    /// 是否包含已删除（State='D'）的单据：true=含已删除，false=仅未删除
    /// 默认 false（与原行为一致），前端查询已删除单时传 true
    pub include_deleted: Option<bool>,
}

/// 从 Claims 提取当前登录用户 EmpID；若 token 无 emp_id（旧 token），回退到 ZERO_UUID
/// 避免破坏兼容性，同时记录 warn 日志便于运维排查
/// P5 修复：原回退到 user_code（如 "admin"）或 "system"，但 tFin_Receipt.EUser/SUser 是
///   uniqueidentifier 列，传非 UUID 字符串会报 "Conversion failed when converting from a
///   character string to uniqueidentifier"。改为统一回退 ZERO_UUID。
fn current_user_emp_id(claims: &Claims) -> String {
    const ZERO_UUID: &str = "00000000-0000-0000-0000-000000000000";
    if !claims.emp_id.is_empty() {
        claims.emp_id.clone()
    } else {
        tracing::warn!(
            user_code = %claims.user_code,
            "JWT 缺少 emp_id 字段，EUser 回退为 ZERO_UUID；建议重新登录获取新 token"
        );
        ZERO_UUID.to_string()
    }
}

pub async fn get_receipt_list(
    Extension(_claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 1000);
    let include_deleted = params.include_deleted.unwrap_or(false);

    // tFin_Receipt 主表 + LEFT JOIN 名称字段（与 doc_graph.rs 定义一致）
    // 修复 P1：根据 include_deleted 参数动态构造 State 过滤条件，支持查询已删除单
    let state_filter = if include_deleted {
        "r.State IS NOT NULL".to_string()
    } else {
        "r.State <> 'D'".to_string()
    };
    let mut base_query = format!(
        "SELECT r.RecID, r.RecNO, r.RecDate, r.CustID, r.DeptID, r.EmpID, r.StkID, \
         r.RecAmt, r.RecType, r.BankName, r.BankAccount, r.DocID, r.DocNo, \
         r.Remark, r.State, r.LUTime, r.EUser, r.EDate, r.SUser, r.SDate, \
         ISNULL(c.CustName,'') AS CustName, \
         ISNULL(e.EmpName,'') AS EmpName, \
         ISNULL(d.DeptName,'') AS DeptName, \
         ISNULL(s.StkName,'') AS StkName \
         FROM tFin_Receipt r \
         LEFT JOIN tBas_Cust c ON c.CustID = r.CustID \
         LEFT JOIN tBas_Emp e ON e.EmpID = r.EmpID \
         LEFT JOIN tBas_Dept d ON d.DeptID = r.DeptID \
         LEFT JOIN tBas_Stock s ON s.StkID = r.StkID \
         WHERE {}",
        state_filter
    );
    let mut query_params: Vec<Option<String>> = Vec::new();
    let pidx = 1;

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(&format!(
                " AND (r.RecNO LIKE @p{} OR r.Remark LIKE @p{})",
                pidx,
                pidx + 1
            ));
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
        }
    }

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(
        &base_query,
        page,
        page_size,
        params.sort_prop.as_deref(),
        params.sort_order.as_deref(),
    );
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params
        .iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();

    let mut total: i32 = 0;
    let count_stream = conn.query(&count_sql, &param_refs).await?;
    if let Some(row) = count_stream.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }

    let data_stream = conn.query(&paginated_sql, &param_refs).await?;
    let rows: Vec<Row> = data_stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok_paginated(
        data,
        total as u64,
        page,
        page_size,
    )))
}

#[derive(Deserialize)]
pub struct CreateReceiptParams {
    // tFin_Receipt 字段（与 doc_graph.rs + DDL 一致）
    pub RecNO: Option<String>,
    pub RecDate: Option<String>,
    pub CustID: Option<String>,
    pub DeptID: Option<String>,
    pub EmpID: Option<String>,
    pub StkID: Option<String>,
    pub RecAmt: Option<f64>,
    pub RecType: Option<String>,
    pub BankName: Option<String>,
    pub BankAccount: Option<String>,
    pub DocID: Option<String>,
    pub DocNo: Option<String>,
    pub Remark: Option<String>,
}

pub async fn create_receipt(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<CreateReceiptParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now_naive = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let rec_no = body.RecNO.as_deref().unwrap_or("");
    if rec_no.is_empty() {
        return Ok(Json(ApiResponse::err("RecNO 单号不能为空")));
    }
    // 修复 P2：插入前校验 RecNO 重复，避免唯一约束冲突或数据混乱
    // 仅检查未删除单（State <> 'D'），已删除的单号允许复用
    let dup_check = conn
        .query(
            "SELECT TOP 1 1 FROM tFin_Receipt WHERE RecNO = @p1 AND State <> 'D'",
            &[&rec_no],
        )
        .await?;
    if dup_check.into_row().await?.is_some() {
        return Ok(Json(ApiResponse::err(&format!(
            "收款单号 {} 已存在，请重新生成",
            rec_no
        ))));
    }
    let rec_date = body.RecDate.as_deref().unwrap_or(&now_naive);
    // P5 修复：tFin_Receipt 的 CustID/DeptID/EmpID/StkID/DocID 均为 uniqueidentifier（可空），
    //   但 tiberius 把空字符串传给 uniqueidentifier 列会报 "将字符串转换为 uniqueidentifier 时失败"。
    //   解决：空值统一转为 ZERO_UUID（与 inventory.rs::empty_or_zero 风格一致），让数据库存 NULL
    //   需要进一步改造为 Option<&str> 才能让数据库真正存 NULL，目前用 ZERO_UUID 占位
    const ZERO_UUID: &str = "00000000-0000-0000-0000-000000000000";
    let cust_id = body
        .CustID
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(ZERO_UUID);
    let dept_id = body
        .DeptID
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(ZERO_UUID);
    let emp_id = body
        .EmpID
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(ZERO_UUID);
    let stk_id = body
        .StkID
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(ZERO_UUID);
    let rec_amt = body.RecAmt.unwrap_or(0.0);
    let rec_type = body.RecType.as_deref().unwrap_or("cash");
    let bank_name = body.BankName.as_deref().unwrap_or("");
    let bank_account = body.BankAccount.as_deref().unwrap_or("");
    let doc_id = body
        .DocID
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(ZERO_UUID);
    let doc_no = body.DocNo.as_deref().unwrap_or("");
    let remark = body.Remark.as_deref().unwrap_or("");
    // 修复 P0：EUser 从认证 token 提取当前登录用户 EmpID（原硬编码 "system"）
    let e_user = current_user_emp_id(&claims);

    let sql = r#"INSERT INTO [tFin_Receipt] (RecID, RecNO, RecDate, CustID, DeptID, EmpID, StkID,
                 RecAmt, RecType, BankName, BankAccount, DocID, DocNo, Remark, State, EDate, EUser)
                 VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10, @p11, @p12, @p13, @p14, @p15, @p16)"#;

    conn.execute(
        sql,
        &[
            &rec_no,
            &rec_date,
            &cust_id,
            &dept_id,
            &emp_id,
            &stk_id,
            &rec_amt,
            &rec_type,
            &bank_name,
            &bank_account,
            &doc_id,
            &doc_no,
            &remark,
            &crate::handlers::doc_state::STATE_NEW,
            &now_naive,
            &e_user,
        ],
    )
    .await?;

    Ok(Json(ApiResponse::msg("收款单创建成功")))
}

// ============================================================================
// 已废弃的 stub 函数已删除（前端走通用 /doc/* 路由）：
//   - get_payment_list / create_payment / update_payment / delete_payment / audit_payment
//   - get_cash_flow_list / create_cash_flow / update_cash_flow / delete_cash_flow / audit_cash_flow
//   - get_receivable_list / get_payable_list（派生 AR/AP 已替代独立表方案）
//   - process_payable_payment / writeoff_payable / adjust_payable
//   - process_receivable_refund / writeoff_receivable / adjust_receivable
// 这些函数原返回 "此功能暂未实现"，且未在 main.rs 注册路由，属纯死代码。
// 财务单据（收款单/付款单/现金流量单）的 CRUD/审核统一走 /api/doc/* 路由，
// 由 doc_service.rs 处理（支持事务、操作日志、库存/财务副作用）。
// ============================================================================

#[derive(Deserialize)]
pub struct UpdateReceiptParams {
    pub RecID: String,
    pub RecDate: Option<String>,
    pub CustID: Option<String>,
    pub DeptID: Option<String>,
    pub EmpID: Option<String>,
    pub StkID: Option<String>,
    pub RecAmt: Option<f64>,
    pub RecType: Option<String>,
    pub BankName: Option<String>,
    pub BankAccount: Option<String>,
    pub DocID: Option<String>,
    pub DocNo: Option<String>,
    pub Remark: Option<String>,
}

pub async fn update_receipt(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<UpdateReceiptParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    // 编辑锁：只允许 N/E 状态编辑
    {
        let state_check = conn
            .query(
                "SELECT State FROM tFin_Receipt WHERE RecID=@p1",
                &[&body.RecID],
            )
            .await?;
        if let Some(row) = state_check.into_row().await? {
            let state: String = row.get::<&str, _>(0).unwrap_or("").to_string();
            if !crate::handlers::doc_state::is_editable(&state) {
                let msg = format!(
                    "单据已{}，不可编辑，请先反审",
                    crate::handlers::doc_state::label(&state)
                );
                return Ok(Json(ApiResponse::err(&msg)));
            }
        }
    }
    let now_naive = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let rec_id = body.RecID.as_str();
    let rec_date = body.RecDate.as_deref().unwrap_or(&now_naive);
    let cust_id = body.CustID.as_deref().unwrap_or("");
    let dept_id = body.DeptID.as_deref().unwrap_or("");
    let emp_id = body.EmpID.as_deref().unwrap_or("");
    let stk_id = body.StkID.as_deref().unwrap_or("");
    let rec_amt = body.RecAmt.unwrap_or(0.0);
    let rec_type = body.RecType.as_deref().unwrap_or("cash");
    let bank_name = body.BankName.as_deref().unwrap_or("");
    let bank_account = body.BankAccount.as_deref().unwrap_or("");
    let doc_id = body.DocID.as_deref().unwrap_or("");
    let doc_no = body.DocNo.as_deref().unwrap_or("");
    let remark = body.Remark.as_deref().unwrap_or("");
    // 修复 P0：EUser 从认证 token 提取当前登录用户 EmpID（原硬编码 "system"）
    let e_user = current_user_emp_id(&claims);

    let sql = r#"UPDATE [tFin_Receipt] SET RecDate=@p1, CustID=@p2, DeptID=@p3, EmpID=@p4, StkID=@p5,
                 RecAmt=@p6, RecType=@p7, BankName=@p8, BankAccount=@p9, DocID=@p10, DocNo=@p11, Remark=@p12,
                 EDate=@p13, EUser=@p14 WHERE RecID=@p15"#;

    conn.execute(
        sql,
        &[
            &rec_date,
            &cust_id,
            &dept_id,
            &emp_id,
            &stk_id,
            &rec_amt,
            &rec_type,
            &bank_name,
            &bank_account,
            &doc_id,
            &doc_no,
            &remark,
            &now_naive,
            &e_user,
            &rec_id,
        ],
    )
    .await?;

    Ok(Json(ApiResponse::msg("收款单更新成功")))
}

#[derive(Deserialize)]
pub struct DeleteReceiptParams {
    pub RecID: String,
}

pub async fn delete_receipt(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<DeleteReceiptParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let rec_id = body.RecID.as_str();

    // 软删除（State='D'）
    // 修复 P1-8：加 State 前置条件 + 检查 rows_affected，避免单据不存在或已审核时仍返回"删除成功"
    let del_sql = "UPDATE [tFin_Receipt] SET State = 'D' WHERE RecID = @p1 AND State IN ('N','E')";
    let result = conn.execute(del_sql, &[&rec_id]).await?;
    let rows = result.rows_affected().first().copied().unwrap_or(0);
    if rows == 0 {
        return Ok(Json(ApiResponse::err(
            "收款单不存在或状态不允许删除（仅新建/编辑中可删除）",
        )));
    }

    // P1-9 审计日志：记录删除操作者和单据 ID
    let audit_remark = format!("删除收款单：RecID={}", rec_id);
    crate::services::inventory_ledger::record_oper(
        &mut conn,
        "DELETE",
        "tFin_Receipt",
        rec_id,
        &claims.user_code,
        None,
        Some(&audit_remark),
    )
    .await;

    Ok(Json(ApiResponse::msg("收款单删除成功")))
}

#[derive(Deserialize)]
pub struct AuditReceiptParams {
    pub RecID: String,
}

pub async fn audit_receipt(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<AuditReceiptParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now_naive = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let rec_id = body.RecID.as_str();
    // 修复 P0：SUser 从认证 token 提取当前登录用户 EmpID（原硬编码 "system"）
    let s_user = current_user_emp_id(&claims);

    // 审核：N/E → S
    let sql = r#"UPDATE [tFin_Receipt] SET State = 'S', SDate = @p1, SUser = @p2 WHERE RecID = @p3 AND State IN ('N','E')"#;
    // 修复 P1-8：检查 rows_affected，避免单据不存在或已审核时仍返回"审核成功"
    let result = conn.execute(sql, &[&now_naive, &s_user, &rec_id]).await?;
    let rows = result.rows_affected().first().copied().unwrap_or(0);
    if rows == 0 {
        return Ok(Json(ApiResponse::err(
            "收款单不存在或状态不允许审核（仅新建/编辑中可审核）",
        )));
    }

    Ok(Json(ApiResponse::msg("收款单审核成功")))
}

#[derive(Deserialize)]
pub struct OverdueAccountsQuery {
    pub kind: Option<String>,
}

pub async fn get_overdue_accounts(
    State(_config): State<Config>,
    Json(params): Json<OverdueAccountsQuery>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    // ⚠️ tArd_AR/PD 是订阅表（TelCode/ProvidersName/SubscriberId），不是财务 AR/AP
    // 改用派生 AR/AP：从 tStk_IO 按 Kind 汇总（无需新建表）
    let mut conn = get_pool().get().await?;
    let kind = params.kind.as_deref().unwrap_or("");

    let mut data: Vec<serde_json::Value> = Vec::new();

    if kind.is_empty() || kind == "receivable" {
        // 派生 AR：销售出库(SD/SI/POS) - 销售退货(SR) - 已审核收款单核销金额
        // 与 get_customer_ar_detail 保持口径一致：使用 tFin_ReceiptDtl.Amt 而非整单 RecAmt
        // 修复 P1-6：原 SQL 使用整单 RecAmt，与明细 Amt 口径不一致，导致 OpenAR 虚高
        let sql = r#"
            SELECT TOP 500
                io.CustID,
                c.CustName,
                ISNULL(SUM(CASE WHEN io.Kind IN ('SD','SI','POS') THEN io.SumAmt ELSE 0 END), 0) AS TotalAmt,
                ISNULL(SUM(CASE WHEN io.Kind = 'SR' THEN io.SumAmt ELSE 0 END), 0) AS ReturnedAmt,
                ISNULL(SUM(CASE WHEN io.Kind IN ('SD','SI','POS') THEN io.SumAmt ELSE 0 END), 0) -
                ISNULL(SUM(CASE WHEN io.Kind = 'SR' THEN io.SumAmt ELSE 0 END), 0) -
                ISNULL(rc.ReceivedAmt, 0) AS OpenAR,
                MAX(io.IoDate) AS LastDate
            FROM tStk_IO io
            LEFT JOIN tBas_Cust c ON c.CustID = io.CustID
            LEFT JOIN (
                SELECT r.CustID, SUM(d.Amt) AS ReceivedAmt
                FROM tFin_ReceiptDtl d
                INNER JOIN tFin_Receipt r ON r.RecID = d.RecID
                WHERE r.State IN ('S','Y')
                GROUP BY r.CustID
            ) rc ON rc.CustID = io.CustID
            WHERE io.State IN ('S','Y')
              AND io.CustID IS NOT NULL
              AND io.Kind IN ('SD','SI','POS','SR')
              AND io.IoDate < DATEADD(DAY, -30, CAST(GETDATE() AS DATE))
            GROUP BY io.CustID, c.CustName, rc.ReceivedAmt
            HAVING ABS(ISNULL(SUM(CASE WHEN io.Kind IN ('SD','SI','POS') THEN io.SumAmt ELSE 0 END), 0) -
                    ISNULL(SUM(CASE WHEN io.Kind = 'SR' THEN io.SumAmt ELSE 0 END), 0) -
                    ISNULL(rc.ReceivedAmt, 0)) > 0.01
            ORDER BY OpenAR DESC
        "#;
        let stream = conn.query(sql, &[]).await?;
        let rows: Vec<Row> = stream.into_first_result().await?;
        for row in &rows {
            data.push(row_to_json(row));
        }
    }

    if kind.is_empty() || kind == "payable" {
        // 派生 AP：采购入库(PD) - 采购退货(PR) - 已审核付款单核销金额
        // 与 get_supplier_ap_detail 保持口径一致：使用 tFin_PaymentDtl.Amt 而非整单 PayAmt
        // 修复 P1-6：原 SQL 使用整单 PayAmt，与明细 Amt 口径不一致，导致 OpenAP 虚高
        let sql = r#"
            SELECT TOP 500
                io.SuppID,
                s.SuppName,
                ISNULL(SUM(CASE WHEN io.Kind = 'PD' THEN io.SumAmt ELSE 0 END), 0) AS TotalAmt,
                ISNULL(SUM(CASE WHEN io.Kind = 'PR' THEN io.SumAmt ELSE 0 END), 0) AS ReturnedAmt,
                ISNULL(SUM(CASE WHEN io.Kind = 'PD' THEN io.SumAmt ELSE 0 END), 0) -
                ISNULL(SUM(CASE WHEN io.Kind = 'PR' THEN io.SumAmt ELSE 0 END), 0) -
                ISNULL(pm.PaidAmt, 0) AS OpenAP,
                MAX(io.IoDate) AS LastDate
            FROM tStk_IO io
            LEFT JOIN tBas_Supp s ON s.SuppID = io.SuppID
            LEFT JOIN (
                SELECT p.SuppID, SUM(d.Amt) AS PaidAmt
                FROM tFin_PaymentDtl d
                INNER JOIN tFin_Payment p ON p.PayID = d.PayID
                WHERE p.State IN ('S','Y')
                GROUP BY p.SuppID
            ) pm ON pm.SuppID = io.SuppID
            WHERE io.State IN ('S','Y')
              AND io.SuppID IS NOT NULL
              AND io.Kind IN ('PD','PR')
              AND io.IoDate < DATEADD(DAY, -30, CAST(GETDATE() AS DATE))
            GROUP BY io.SuppID, s.SuppName, pm.PaidAmt
            HAVING ABS(ISNULL(SUM(CASE WHEN io.Kind = 'PD' THEN io.SumAmt ELSE 0 END), 0) -
                    ISNULL(SUM(CASE WHEN io.Kind = 'PR' THEN io.SumAmt ELSE 0 END), 0) -
                    ISNULL(pm.PaidAmt, 0)) > 0.01
            ORDER BY OpenAP DESC
        "#;
        let stream = conn.query(sql, &[]).await?;
        let rows: Vec<Row> = stream.into_first_result().await?;
        for row in &rows {
            data.push(row_to_json(row));
        }
    }

    Ok(Json(ApiResponse::ok(data)))
}

// ============================================================================
// 派生 AR/AP 实时查询（方案 B）
//   - 不维护 tFin_Receivable/Payable 表
//   - 直接从 tStk_IO（已审核单据）按 Kind 维度计算
//   - 实时准确但有性能成本（大表需走索引）
// ============================================================================

/// 单客户应收汇总
pub async fn get_customer_ar(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 1000);

    let mut base_query = r#"
        SELECT
            io.CustID,
            c.CustName,
            ISNULL(SUM(CASE WHEN io.Kind IN ('SD','SI','POS') THEN io.SumAmt ELSE 0 END), 0) AS SalesAmt,
            ISNULL(SUM(CASE WHEN io.Kind = 'SR' THEN io.SumAmt ELSE 0 END), 0) AS ReturnAmt,
            ISNULL(SUM(CASE WHEN io.Kind IN ('SD','SI','POS') THEN io.SumAmt ELSE 0 END), 0) -
            ISNULL(SUM(CASE WHEN io.Kind = 'SR' THEN io.SumAmt ELSE 0 END), 0) AS GrossAR,
            ISNULL(rc.ReceivedAmt, 0) AS ReceivedAmt,
            (ISNULL(SUM(CASE WHEN io.Kind IN ('SD','SI','POS') THEN io.SumAmt ELSE 0 END), 0) -
             ISNULL(SUM(CASE WHEN io.Kind = 'SR' THEN io.SumAmt ELSE 0 END), 0) -
             ISNULL(rc.ReceivedAmt, 0)) AS OpenAR,
            COUNT(DISTINCT CASE WHEN io.Kind IN ('SD','SI','POS') THEN io.IOID END) AS DocCount,
            MAX(io.IoDate) AS LastSaleDate
        FROM tStk_IO io
        LEFT JOIN tBas_Cust c ON c.CustID = io.CustID
        LEFT JOIN (
            SELECT CustID, ISNULL(SUM(RecAmt), 0) AS ReceivedAmt
            FROM tFin_Receipt
            WHERE State IN ('S','Y') AND CustID IS NOT NULL
            GROUP BY CustID
        ) rc ON rc.CustID = io.CustID
        WHERE io.State IN ('S','Y')
          AND io.CustID IS NOT NULL
          AND io.Kind IN ('SD','SI','POS','SR')
    "#
    .to_string();

    let mut query_params: Vec<Option<String>> = Vec::new();
    let pidx = 1;

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            // 修复 P1-10：用 TRY_CAST 替代 CAST，非 UUID 字符串时返回 NULL 而非报错
            // 否则用户输入中文姓名会让整个查询失败
            base_query.push_str(&format!(
                " AND (c.CustName LIKE @p{} OR io.CustID = TRY_CAST(@p{} AS uniqueidentifier))",
                pidx,
                pidx + 1
            ));
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(kw.clone()));
        }
    }

    // 修复 P1-15：HAVING 用 ABS(...) > 0.01 避免浮点精度导致已结清客户仍被列出
    base_query.push_str(
        " GROUP BY io.CustID, c.CustName, rc.ReceivedAmt \
          HAVING ABS(ISNULL(SUM(CASE WHEN io.Kind IN ('SD','SI','POS') THEN io.SumAmt ELSE 0 END), 0) - \
                  ISNULL(SUM(CASE WHEN io.Kind = 'SR' THEN io.SumAmt ELSE 0 END), 0) - \
                  ISNULL(rc.ReceivedAmt, 0)) > 0.01"
    );

    let count_sql = format!("SELECT COUNT(*) AS cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(
        &base_query,
        page,
        page_size,
        params.sort_prop.as_deref(),
        params.sort_order.as_deref(),
    );
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params
        .iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();

    let mut total: i32 = 0;
    let count_stream = conn.query(&count_sql, &param_refs).await?;
    if let Some(row) = count_stream.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }

    let data_stream = conn.query(&paginated_sql, &param_refs).await?;
    let rows: Vec<Row> = data_stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok_paginated(
        data,
        total as u64,
        page,
        page_size,
    )))
}

/// 单客户应收明细（单据级）
#[derive(Deserialize)]
pub struct CustomerARDetailQuery {
    pub cust_id: String,
}

/// 单供应商应付明细（单据级）
/// P3-24 修复：原 get_supplier_ap_detail 复用 CustomerARDetailQuery，参数键 cust_id 与语义不符
///   改为独立结构体 + supp_id 字段，前端同步改为传 supp_id
#[derive(Deserialize)]
pub struct SupplierAPDetailQuery {
    pub supp_id: String,
}

pub async fn get_customer_ar_detail(
    State(_config): State<Config>,
    Json(params): Json<CustomerARDetailQuery>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let cust_id = &params.cust_id;

    // 派生：列出该客户所有 SD/SI/POS + SR 单据（已审）+ 已核销金额 + 未核销金额
    // LEFT JOIN tFin_ReceiptDtl 聚合已审核收款单中针对此源单的核销金额合计
    let sql = r#"
        SELECT
            io.IOID, io.IONo, io.IoDate, io.Kind, io.SumAmt, io.SumQty,
            io.Note, io.State,
            c.CustName,
            ISNULL(rd.AlreadyWriteoff, 0) AS AlreadyWriteoff,
            (io.SumAmt - ISNULL(rd.AlreadyWriteoff, 0)) AS OpenAmt
        FROM tStk_IO io
        LEFT JOIN tBas_Cust c ON c.CustID = io.CustID
        LEFT JOIN (
            SELECT d.SourceDocID, SUM(d.Amt) AS AlreadyWriteoff
            FROM tFin_ReceiptDtl d
            INNER JOIN tFin_Receipt r ON r.RecID = d.RecID
            WHERE r.State IN ('S','Y')
              AND d.SourceDocID IS NOT NULL
            GROUP BY d.SourceDocID
        ) rd ON rd.SourceDocID = io.IOID
        WHERE io.CustID = @p1
          AND io.Kind IN ('SD','SI','POS','SR')
          AND io.State IN ('S','Y')
        ORDER BY io.IoDate DESC
    "#;
    let stream = conn.query(sql, &[&cust_id.as_str()]).await?;
    let rows: Vec<Row> = stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
    Ok(Json(ApiResponse::ok(data)))
}

/// 单供应商应付汇总
pub async fn get_supplier_ap(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 1000);

    let mut base_query = r#"
        SELECT
            io.SuppID,
            s.SuppName,
            ISNULL(SUM(CASE WHEN io.Kind = 'PD' THEN io.SumAmt ELSE 0 END), 0) AS PurchaseAmt,
            ISNULL(SUM(CASE WHEN io.Kind = 'PR' THEN io.SumAmt ELSE 0 END), 0) AS ReturnAmt,
            ISNULL(SUM(CASE WHEN io.Kind = 'PD' THEN io.SumAmt ELSE 0 END), 0) -
            ISNULL(SUM(CASE WHEN io.Kind = 'PR' THEN io.SumAmt ELSE 0 END), 0) AS GrossAP,
            ISNULL(pm.PaidAmt, 0) AS PaidAmt,
            (ISNULL(SUM(CASE WHEN io.Kind = 'PD' THEN io.SumAmt ELSE 0 END), 0) -
             ISNULL(SUM(CASE WHEN io.Kind = 'PR' THEN io.SumAmt ELSE 0 END), 0) -
             ISNULL(pm.PaidAmt, 0)) AS OpenAP,
            COUNT(DISTINCT CASE WHEN io.Kind = 'PD' THEN io.IOID END) AS DocCount,
            MAX(io.IoDate) AS LastPurchaseDate
        FROM tStk_IO io
        LEFT JOIN tBas_Supp s ON s.SuppID = io.SuppID
        LEFT JOIN (
            SELECT SuppID, ISNULL(SUM(PayAmt), 0) AS PaidAmt
            FROM tFin_Payment
            WHERE State IN ('S','Y') AND SuppID IS NOT NULL
            GROUP BY SuppID
        ) pm ON pm.SuppID = io.SuppID
        WHERE io.State IN ('S','Y')
          AND io.SuppID IS NOT NULL
          AND io.Kind IN ('PD','PR')
    "#
    .to_string();

    let mut query_params: Vec<Option<String>> = Vec::new();
    let pidx = 1;

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            // 修复 P1-10：用 TRY_CAST 替代 CAST，非 UUID 字符串时返回 NULL 而非报错
            base_query.push_str(&format!(
                " AND (s.SuppName LIKE @p{} OR io.SuppID = TRY_CAST(@p{} AS uniqueidentifier))",
                pidx,
                pidx + 1
            ));
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(kw.clone()));
        }
    }

    // 修复 P1-15：HAVING 用 ABS(...) > 0.01 避免浮点精度问题
    base_query.push_str(
        " GROUP BY io.SuppID, s.SuppName, pm.PaidAmt \
          HAVING ABS(ISNULL(SUM(CASE WHEN io.Kind = 'PD' THEN io.SumAmt ELSE 0 END), 0) - \
                  ISNULL(SUM(CASE WHEN io.Kind = 'PR' THEN io.SumAmt ELSE 0 END), 0) - \
                  ISNULL(pm.PaidAmt, 0)) > 0.01",
    );

    let count_sql = format!("SELECT COUNT(*) AS cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(
        &base_query,
        page,
        page_size,
        params.sort_prop.as_deref(),
        params.sort_order.as_deref(),
    );
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params
        .iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();

    let mut total: i32 = 0;
    let count_stream = conn.query(&count_sql, &param_refs).await?;
    if let Some(row) = count_stream.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }

    let data_stream = conn.query(&paginated_sql, &param_refs).await?;
    let rows: Vec<Row> = data_stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok_paginated(
        data,
        total as u64,
        page,
        page_size,
    )))
}

/// 单供应商应付明细
pub async fn get_supplier_ap_detail(
    State(_config): State<Config>,
    Json(params): Json<SupplierAPDetailQuery>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let supp_id = &params.supp_id; // P3-24 修复：使用 SupplierAPDetailQuery.supp_id

    let sql = r#"
        SELECT
            io.IOID, io.IONo, io.IoDate, io.Kind, io.SumAmt, io.SumQty,
            io.Note, io.State,
            s.SuppName,
            ISNULL(pd.AlreadyWriteoff, 0) AS AlreadyWriteoff,
            (io.SumAmt - ISNULL(pd.AlreadyWriteoff, 0)) AS OpenAmt
        FROM tStk_IO io
        LEFT JOIN tBas_Supp s ON s.SuppID = io.SuppID
        LEFT JOIN (
            SELECT d.SourceDocID, SUM(d.Amt) AS AlreadyWriteoff
            FROM tFin_PaymentDtl d
            INNER JOIN tFin_Payment p ON p.PayID = d.PayID
            WHERE p.State IN ('S','Y')
              AND d.SourceDocID IS NOT NULL
            GROUP BY d.SourceDocID
        ) pd ON pd.SourceDocID = io.IOID
        WHERE io.SuppID = @p1
          AND io.Kind IN ('PD','PR')
          AND io.State IN ('S','Y')
        ORDER BY io.IoDate DESC
    "#;
    let stream = conn.query(sql, &[&supp_id.as_str()]).await?;
    let rows: Vec<Row> = stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
    Ok(Json(ApiResponse::ok(data)))
}

// ============================================================================
// 客户/供应商对账单（按客户/供应商 + 期间，期初/期间新增/期间收款付款/期末）
// 数据源（派生）：tStk_IO + tFin_Receipt / tFin_Payment
//   - 客户对账：销售出库(SD/SI/POS) 借方 / 销售退货(SR) 贷方 / 收款单(tFin_Receipt) 贷方
//   - 供应商对账：采购入库(PD) 贷方 / 采购退货(PR) 借方 / 付款单(tFin_Payment) 借方
// 余额方向：
//   - 客户应收余额 = 借方(销售) - 贷方(退货+收款)，正数代表客户欠款
//   - 供应商应付余额 = 贷方(采购) - 借方(退货+付款)，正数代表我方欠供应商
// ============================================================================

#[derive(Deserialize)]
pub struct StatementParams {
    pub cust_id: Option<String>,    // 客户（或供应商）ID
    pub start_date: Option<String>, // 期间开始（YYYY-MM-DD）
    pub end_date: Option<String>,   // 期间结束（YYYY-MM-DD）
    pub include_void: Option<bool>, // 是否包含作废单据（默认 false，仅查询 S/Y）
}

pub async fn get_customer_statement(
    State(_config): State<Config>,
    Json(params): Json<StatementParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let cust_id = match &params.cust_id {
        Some(v) if !v.is_empty() => v.clone(),
        _ => return Ok(Json(ApiResponse::err("请选择客户"))),
    };
    let start_date = params.start_date.as_deref().unwrap_or("");
    let end_date = params.end_date.as_deref().unwrap_or("");
    // 单据状态过滤：默认仅 S/Y（已审核/已确认），含作废时加入 C
    let state_filter = if params.include_void.unwrap_or(false) {
        "('S','Y','C')"
    } else {
        "('S','Y')"
    };

    let mut conn = get_pool().get().await?;

    // 1) 期初余额：start_date 之前的销售-退货-收款累计
    //    注意：SQL Server 的 SUM(DECIMAL) 返回 NUMERIC，tiberius 无法直接转 f64，需 CAST AS FLOAT
    let mut opening: f64 = 0.0;
    if !start_date.is_empty() {
        let opening_sql = format!(
            r#"
            SELECT CAST(
                ISNULL(SUM(CASE WHEN io.Kind IN ('SD','SI','POS') THEN io.SumAmt ELSE 0 END), 0) -
                ISNULL(SUM(CASE WHEN io.Kind = 'SR' THEN io.SumAmt ELSE 0 END), 0) -
                ISNULL((SELECT SUM(r.RecAmt) FROM tFin_Receipt r
                        WHERE r.State IN {} AND r.CustID = @p1 AND r.RecDate < @p2), 0)
                AS FLOAT) AS OpeningBalance
            FROM tStk_IO io
            WHERE io.State IN {}
              AND io.CustID = @p1
              AND io.Kind IN ('SD','SI','POS','SR')
              AND io.IoDate < @p2
        "#,
            state_filter, state_filter
        );
        let row = conn
            .query(&opening_sql, &[&cust_id.as_str(), &start_date])
            .await?
            .into_row()
            .await?;
        if let Some(r) = row {
            opening = r.get::<f64, _>("OpeningBalance").unwrap_or(0.0);
        }
    }

    // 2) 期间交易明细（销售/退货/收款 UNION ALL）
    // 修复 P1-3：end_date 边界用 < DATEADD(DAY, 1, @p3)，避免当天 00:00 之后的交易被漏算
    // 原因：SQL Server 将 'YYYY-MM-DD' 隐式转为 YYYY-MM-DD 00:00:00.000，<= 会漏掉当天非 0 点交易
    let tx_sql = format!(
        r#"
        SELECT * FROM (
            SELECT
                io.IoDate AS TxDate,
                io.IONo AS DocNo,
                CASE io.Kind
                    WHEN 'SD' THEN '销售出库'
                    WHEN 'SI' THEN '门店销售'
                    WHEN 'POS' THEN 'POS收银'
                    WHEN 'SR' THEN '销售退货'
                END AS TxType,
                io.Kind AS Kind,
                CAST(CASE WHEN io.Kind IN ('SD','SI','POS') THEN io.SumAmt ELSE 0 END AS FLOAT) AS Debit,
                CAST(CASE WHEN io.Kind = 'SR' THEN io.SumAmt ELSE 0 END AS FLOAT) AS Credit,
                io.Note AS Note,
                io.State AS State
            FROM tStk_IO io
            WHERE io.State IN {}
              AND io.CustID = @p1
              AND io.Kind IN ('SD','SI','POS','SR')
              AND (@p2 = '' OR io.IoDate >= @p2)
              AND (@p3 = '' OR io.IoDate < DATEADD(DAY, 1, @p3))
            UNION ALL
            SELECT
                r.RecDate AS TxDate,
                r.RecNO AS DocNo,
                CASE WHEN r.RecType = 'cash' THEN '现金收款'
                     WHEN r.RecType = 'bank' THEN '银行收款'
                     ELSE '收款' END AS TxType,
                'RECEIPT' AS Kind,
                CAST(0 AS FLOAT) AS Debit,
                CAST(r.RecAmt AS FLOAT) AS Credit,
                ISNULL(r.BankName,'') + ' ' + ISNULL(r.Remark,'') AS Note,
                r.State AS State
            FROM tFin_Receipt r
            WHERE r.State IN {}
              AND r.CustID = @p1
              AND (@p2 = '' OR r.RecDate >= @p2)
              AND (@p3 = '' OR r.RecDate < DATEADD(DAY, 1, @p3))
        ) t
        ORDER BY TxDate ASC, DocNo ASC
    "#,
        state_filter, state_filter
    );
    let stream = conn
        .query(&tx_sql, &[&cust_id.as_str(), &start_date, &end_date])
        .await?;
    let rows: Vec<Row> = stream.into_first_result().await?;
    let mut transactions: Vec<serde_json::Value> = Vec::with_capacity(rows.len());
    let mut running = opening;
    for r in &rows {
        let debit: f64 = r.get::<f64, _>("Debit").unwrap_or(0.0);
        let credit: f64 = r.get::<f64, _>("Credit").unwrap_or(0.0);
        running = running + debit - credit;
        let mut obj = row_to_json(r);
        if let Some(obj_map) = obj.as_object_mut() {
            obj_map.insert("OpeningBalance".to_string(), serde_json::json!(opening));
            obj_map.insert("RunningBalance".to_string(), serde_json::json!(running));
        }
        transactions.push(obj);
    }

    // 3) 汇总：期间借方/贷方/期末余额
    let period_debit: f64 = transactions
        .iter()
        .map(|v| v.get("Debit").and_then(|x| x.as_f64()).unwrap_or(0.0))
        .sum();
    let period_credit: f64 = transactions
        .iter()
        .map(|v| v.get("Credit").and_then(|x| x.as_f64()).unwrap_or(0.0))
        .sum();
    let ending_balance = opening + period_debit - period_credit;

    // 4) 客户信息
    let mut cust_name = String::new();
    {
        let nrow = conn
            .query(
                "SELECT CustName FROM tBas_Cust WHERE CustID=@p1",
                &[&cust_id.as_str()],
            )
            .await?
            .into_row()
            .await?;
        if let Some(r) = nrow {
            cust_name = r.get::<&str, _>("CustName").unwrap_or("").to_string();
        }
    }

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "party_id": cust_id,
        "party_name": cust_name,
        "party_type": "customer",
        "start_date": start_date,
        "end_date": end_date,
        "opening_balance": opening,
        "period_debit": period_debit,
        "period_credit": period_credit,
        "ending_balance": ending_balance,
        "transactions": transactions,
    }))))
}

pub async fn get_supplier_statement(
    State(_config): State<Config>,
    Json(params): Json<StatementParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let supp_id = match &params.cust_id {
        Some(v) if !v.is_empty() => v.clone(),
        _ => return Ok(Json(ApiResponse::err("请选择供应商"))),
    };
    let start_date = params.start_date.as_deref().unwrap_or("");
    let end_date = params.end_date.as_deref().unwrap_or("");
    let state_filter = if params.include_void.unwrap_or(false) {
        "('S','Y','C')"
    } else {
        "('S','Y')"
    };

    let mut conn = get_pool().get().await?;

    // 1) 期初余额：start_date 之前的采购-退货-付款累计
    //    正数代表我方欠供应商
    //    注意：SQL Server 的 SUM(DECIMAL) 返回 NUMERIC，tiberius 无法直接转 f64，需 CAST AS FLOAT
    let mut opening: f64 = 0.0;
    if !start_date.is_empty() {
        let opening_sql = format!(
            r#"
            SELECT CAST(
                ISNULL(SUM(CASE WHEN io.Kind = 'PD' THEN io.SumAmt ELSE 0 END), 0) -
                ISNULL(SUM(CASE WHEN io.Kind = 'PR' THEN io.SumAmt ELSE 0 END), 0) -
                ISNULL((SELECT SUM(p.PayAmt) FROM tFin_Payment p
                        WHERE p.State IN {} AND p.SuppID = @p1 AND p.PayDate < @p2), 0)
                AS FLOAT) AS OpeningBalance
            FROM tStk_IO io
            WHERE io.State IN {}
              AND io.SuppID = @p1
              AND io.Kind IN ('PD','PR')
              AND io.IoDate < @p2
        "#,
            state_filter, state_filter
        );
        let row = conn
            .query(&opening_sql, &[&supp_id.as_str(), &start_date])
            .await?
            .into_row()
            .await?;
        if let Some(r) = row {
            opening = r.get::<f64, _>("OpeningBalance").unwrap_or(0.0);
        }
    }

    // 2) 期间交易明细（采购入库/采购退货/付款 UNION ALL）
    //    贷方 = 增加应付（采购入库 PD）
    //    借方 = 减少应付（采购退货 PR + 付款单 Payment）
    let tx_sql = format!(
        r#"
        SELECT * FROM (
            SELECT
                io.IoDate AS TxDate,
                io.IONo AS DocNo,
                CASE io.Kind
                    WHEN 'PD' THEN '采购入库'
                    WHEN 'PR' THEN '采购退货'
                END AS TxType,
                io.Kind AS Kind,
                CAST(CASE WHEN io.Kind = 'PR' THEN io.SumAmt ELSE 0 END AS FLOAT) AS Debit,
                CAST(CASE WHEN io.Kind = 'PD' THEN io.SumAmt ELSE 0 END AS FLOAT) AS Credit,
                io.Note AS Note,
                io.State AS State
            FROM tStk_IO io
            WHERE io.State IN {}
              AND io.SuppID = @p1
              AND io.Kind IN ('PD','PR')
              AND (@p2 = '' OR io.IoDate >= @p2)
              AND (@p3 = '' OR io.IoDate < DATEADD(DAY, 1, @p3))
            UNION ALL
            SELECT
                p.PayDate AS TxDate,
                p.PayNO AS DocNo,
                CASE WHEN p.PayType = 'cash' THEN '现金付款'
                     WHEN p.PayType = 'bank' THEN '银行付款'
                     ELSE '付款' END AS TxType,
                'PAYMENT' AS Kind,
                CAST(p.PayAmt AS FLOAT) AS Debit,
                CAST(0 AS FLOAT) AS Credit,
                ISNULL(p.BankName,'') + ' ' + ISNULL(p.Remark,'') AS Note,
                p.State AS State
            FROM tFin_Payment p
            WHERE p.State IN {}
              AND p.SuppID = @p1
              AND (@p2 = '' OR p.PayDate >= @p2)
              AND (@p3 = '' OR p.PayDate < DATEADD(DAY, 1, @p3))
        ) t
        ORDER BY TxDate ASC, DocNo ASC
    "#,
        state_filter, state_filter
    );
    let stream = conn
        .query(&tx_sql, &[&supp_id.as_str(), &start_date, &end_date])
        .await?;
    let rows: Vec<Row> = stream.into_first_result().await?;
    let mut transactions: Vec<serde_json::Value> = Vec::with_capacity(rows.len());
    let mut running = opening;
    for r in &rows {
        let debit: f64 = r.get::<f64, _>("Debit").unwrap_or(0.0);
        let credit: f64 = r.get::<f64, _>("Credit").unwrap_or(0.0);
        // 应付方向：贷方 - 借方（正数代表我方欠供应商）
        running = running + credit - debit;
        let mut obj = row_to_json(r);
        if let Some(obj_map) = obj.as_object_mut() {
            obj_map.insert("OpeningBalance".to_string(), serde_json::json!(opening));
            obj_map.insert("RunningBalance".to_string(), serde_json::json!(running));
        }
        transactions.push(obj);
    }

    // 3) 汇总
    let period_debit: f64 = transactions
        .iter()
        .map(|v| v.get("Debit").and_then(|x| x.as_f64()).unwrap_or(0.0))
        .sum();
    let period_credit: f64 = transactions
        .iter()
        .map(|v| v.get("Credit").and_then(|x| x.as_f64()).unwrap_or(0.0))
        .sum();
    let ending_balance = opening + period_credit - period_debit;

    // 4) 供应商信息
    let mut supp_name = String::new();
    {
        let nrow = conn
            .query(
                "SELECT SuppName FROM tBas_Supp WHERE SuppID=@p1",
                &[&supp_id.as_str()],
            )
            .await?
            .into_row()
            .await?;
        if let Some(r) = nrow {
            supp_name = r.get::<&str, _>("SuppName").unwrap_or("").to_string();
        }
    }

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "party_id": supp_id,
        "party_name": supp_name,
        "party_type": "supplier",
        "start_date": start_date,
        "end_date": end_date,
        "opening_balance": opening,
        "period_debit": period_debit,
        "period_credit": period_credit,
        "ending_balance": ending_balance,
        "transactions": transactions,
    }))))
}

// ============================================================================
// 核销明细查询（编辑模式回显用）
// 查询某收款单/付款单已有的核销明细行
// ============================================================================

#[derive(Deserialize)]
pub struct WriteoffDetailQuery {
    pub table: Option<String>, // 'tFin_ReceiptDtl' | 'tFin_PaymentDtl'，不传则按 doc_type 推断
    pub doc_type: Option<String>, // 'receipt' | 'payment'，与 table 二选一
    pub master_id: String,     // 主表 ID（RecID 或 PayID）
}

pub async fn get_writeoff_details(
    State(_config): State<Config>,
    Json(params): Json<WriteoffDetailQuery>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let master_id = params.master_id.clone();

    // 推断明细表 + 外键字段 + 源单 JOIN 表
    let (detail_table, fk_field, _doc_type): (&str, &str, &str) = match (
        params.table.as_deref(),
        params.doc_type.as_deref(),
    ) {
        (Some("tFin_ReceiptDtl"), _) => ("tFin_ReceiptDtl", "RecID", "receipt"),
        (Some("tFin_PaymentDtl"), _) => ("tFin_PaymentDtl", "PayID", "payment"),
        (_, Some("receipt")) => ("tFin_ReceiptDtl", "RecID", "receipt"),
        (_, Some("payment")) => ("tFin_PaymentDtl", "PayID", "payment"),
        _ => {
            return Ok(Json(ApiResponse::err(
                "请指定 table（tFin_ReceiptDtl 或 tFin_PaymentDtl）或 doc_type（receipt 或 payment）",
            )));
        }
    };

    // 查询明细行 + JOIN 源单获取 IoDate/Kind/SumAmt，并计算 OpenAmt
    // OpenAmt = 源单金额 - 当前明细行的 Amt - 其他收款单/付款单对同一源单的核销合计
    // 编辑模式下，当前单据的核销明细尚未审核，故已审核合计不含当前单据本身
    //
    // 注意：明细表主键（ReceiptDtlID/PaymentDtlID）只存在于明细表，
    // 主表（tFin_Receipt/tFin_Payment）的主键是 RecID/PayID，
    // JOIN 条件必须用主表主键，否则 SQL Server 报 "Invalid column name"
    let (detail_pk, master_table, master_pk): (&str, &str, &str) = match detail_table {
        "tFin_ReceiptDtl" => ("ReceiptDtlID", "tFin_Receipt", "RecID"),
        "tFin_PaymentDtl" => ("PaymentDtlID", "tFin_Payment", "PayID"),
        _ => ("ReceiptDtlID", "tFin_Receipt", "RecID"),
    };
    let sql = format!(
        r#"
        SELECT
            d.{detail_pk} AS DetailID,
            d.SourceDocID,
            d.SourceDocNo,
            d.Amt,
            d.Note,
            d.RowNO,
            io.IoDate AS SourceDate,
            io.Kind AS SourceKind,
            io.SumQty AS SourceSumQty,
            io.SumAmt AS SourceSumAmt,
            ISNULL(other.AlreadyWriteoff, 0) AS OtherWriteoff,
            (io.SumAmt - ISNULL(other.AlreadyWriteoff, 0)) AS OpenAmt
        FROM {tbl} d
        LEFT JOIN tStk_IO io ON io.IOID = d.SourceDocID
        LEFT JOIN (
            SELECT x.SourceDocID, SUM(x.Amt) AS AlreadyWriteoff
            FROM {tbl} x
            INNER JOIN {master_table} m ON m.{master_pk} = x.{fk}
            WHERE m.State IN ('S','Y')
              AND x.SourceDocID IS NOT NULL
            GROUP BY x.SourceDocID
        ) other ON other.SourceDocID = d.SourceDocID
        WHERE d.{fk} = @p1
        ORDER BY d.RowNO ASC
        "#,
        detail_pk = detail_pk,
        tbl = detail_table,
        master_table = master_table,
        master_pk = master_pk,
        fk = fk_field,
    );

    let stream = conn.query(&sql, &[&master_id.as_str()]).await?;
    let rows: Vec<Row> = stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
    Ok(Json(ApiResponse::ok(data)))
}
