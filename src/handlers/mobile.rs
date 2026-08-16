use crate::config::Config;
use crate::db::get_pool;
use crate::error::Result;
use crate::handlers::base_data::{row_to_json, try_get_value};
use crate::middleware::auth::Claims;
use crate::utils::jwt::{create_token, make_claims};
use crate::utils::password::{hash_password, needs_upgrade, verify_password};
use crate::utils::{ApiResponse, build_pagination_sql_with_sort};
use axum::{Extension, Json, extract::State};
use chrono::Datelike;
use serde::{Deserialize, Serialize};
use tiberius::Row;

/// 读取字符串字段，兼容 uniqueidentifier(GUID) 类型字段（如 EmpID/StkID/DeptID 等）
/// 直接用 row.get::<&str,_> 读取 Guid 字段会 panic，故统一通过 try_get_value 兜底
pub fn get_str(row: &Row, col_name: &str) -> String {
    let val = try_get_value(row, col_name);
    match val {
        serde_json::Value::String(s) => s,
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

#[derive(Deserialize)]
pub struct MobileLoginRequest {
    pub EmpNo: String,
    pub Password: String,
}

#[derive(Serialize)]
pub struct MobileLoginResponse {
    pub token: String,
    pub user: MobileUserInfo,
}

#[derive(Serialize, Clone)]
pub struct MobileUserInfo {
    pub id: String,
    pub code: String,
    pub name: String,
    pub dept_name: String,
    pub stk_name: String,
    pub stk_id: String,
}

/// 公开接口：获取门店列表（登录页用，无需 token）
pub async fn list_stores(
    State(_config): State<Config>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let sql = "SELECT [StkID], [StkCode], [StkName] FROM [tBas_Stock] WHERE [Used] = @P1 ORDER BY [StkName] ASC";
    let stream = conn.query(sql, &[&"Y"]).await?;
    let rows: Vec<Row> = stream.into_first_result().await?;
    let list = rows
        .iter()
        .map(|r| {
            serde_json::json!({
                "StkID": get_str(r, "StkID"),
                "StkCode": get_str(r, "StkCode"),
                "StkName": get_str(r, "StkName"),
            })
        })
        .collect();
    Ok(Json(ApiResponse::ok(list)))
}

pub async fn mobile_login(
    State(config): State<Config>,
    Json(body): Json<MobileLoginRequest>,
) -> Result<Json<ApiResponse<MobileLoginResponse>>> {
    let mut conn = get_pool().get().await?;

    let sql = r#"SELECT TOP 1 e.*, d.[DeptName], s.[StkName]
        FROM [tBas_Emp] e
        LEFT JOIN [tBas_Dept] d ON e.[DeptID] = d.[DeptID]
        LEFT JOIN [tBas_Stock] s ON e.[StkID] = s.[StkID]
        WHERE e.[EmpNo] = @p1 AND e.[AllowLogin] = 'Y'"#;
    let stream = conn.query(sql, &[&body.EmpNo.as_str()]).await?;

    if let Some(row) = stream.into_row().await? {
        let emp_no = get_str(&row, "EmpNo");
        let emp_name = get_str(&row, "EmpName");
        let dept_name = get_str(&row, "DeptName");
        let stk_name = get_str(&row, "StkName");
        let stk_id = get_str(&row, "StkID");

        let mut stored_password = String::new();
        let columns = row.columns();
        for col in columns {
            let name = col.name();
            if name.eq_ignore_ascii_case("passwordstr") {
                if let Some(v) = row.try_get::<&str, _>(name).ok().flatten() {
                    stored_password = v.to_string();
                    break;
                }
            }
        }

        if stored_password.is_empty() || !verify_password(&body.Password, &stored_password) {
            return Ok(Json(ApiResponse::<MobileLoginResponse>::err("密码错误")));
        }

        let emp_id = get_str(&row, "EmpID");

        if needs_upgrade(&stored_password) {
            if let Some(hashed) = hash_password(&body.Password) {
                if !emp_id.is_empty() {
                    let _ = conn
                        .execute(
                            "UPDATE tBas_Emp SET PassWordStr = @p1 WHERE EmpID = @p2",
                            &[&hashed.as_str(), &emp_id.as_str()],
                        )
                        .await;
                }
            }
        }

        let claims = make_claims(&emp_no, &emp_name, &emp_id);

        let token = create_token(&config.jwt_secret, &claims)?;

        let resp = MobileLoginResponse {
            token,
            user: MobileUserInfo {
                id: emp_no.clone(),
                code: emp_no,
                name: emp_name,
                dept_name,
                stk_name,
                stk_id,
            },
        };
        Ok(Json(ApiResponse::ok(resp)))
    } else {
        Ok(Json(ApiResponse::<MobileLoginResponse>::err(
            "未找到该工号或无移动端登录权限",
        )))
    }
}

#[derive(Deserialize)]
pub struct MobileRegisterRequest {
    pub EmpNo: String,
    pub EmpName: String,
    pub Password: String,
    pub DeptID: Option<String>,
    pub StkID: Option<String>,
    pub Phone: Option<String>,
}

pub async fn mobile_register(
    State(config): State<Config>,
    Json(body): Json<MobileRegisterRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    // P0 安全修复：公开端点默认禁用匿名注册，防止批量创建可登录账号。
    // 需要开放时设置 ALLOW_MOBILE_REGISTER=true（见 config.rs / .env.example）。
    if !config.allow_mobile_register {
        tracing::warn!(emp_no = %body.EmpNo, "移动端自助注册被拒绝：ALLOW_MOBILE_REGISTER 未开启");
        return Ok(Json(ApiResponse::err(
            "注册功能未开放，请联系管理员创建账号",
        )));
    }

    let mut conn = get_pool().get().await?;

    let check_sql = "SELECT TOP 1 1 FROM [tBas_Emp] WHERE [EmpNo] = @p1";
    let check_stream = conn.query(check_sql, &[&body.EmpNo.as_str()]).await?;
    if check_stream.into_row().await?.is_some() {
        return Ok(Json(ApiResponse::err("该工号已存在")));
    }

    let hashed = match hash_password(&body.Password) {
        Some(h) => h,
        None => {
            return Ok(Json(ApiResponse::err(
                "密码哈希失败，可能密码过长（>72字节）",
            )));
        }
    };
    let emp_id = format!("{}", uuid::Uuid::new_v4());
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let dept_id = body
        .DeptID
        .as_deref()
        .unwrap_or("00000000-0000-0000-0000-000000000000");
    let stk_id = body
        .StkID
        .as_deref()
        .unwrap_or("00000000-0000-0000-0000-000000000000");
    let tel = body.Phone.as_deref().unwrap_or("");
    let zero_uuid = "00000000-0000-0000-0000-000000000000";
    let emp_sd: i32 = 0;

    let sql = r#"INSERT INTO [tBas_Emp] ([EmpID], [EmpNo], [EmpName], [PassWordStr], [DeptID], [StkID], [Tel], [AllowLogin], [State], [EDate], [EUser], [empSD], [OnlyLogin], [AndroidPassWord], [AndroidPower])
        VALUES (@p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10, @p11, @p12, 'N', '3', 'A')"#;
    let params: Vec<&dyn tiberius::ToSql> = vec![
        &emp_id,
        &body.EmpNo,
        &body.EmpName,
        &hashed,
        &dept_id,
        &stk_id,
        &tel,
        &"Y",
        &"Y",
        &now,
        &zero_uuid,
        &emp_sd,
    ];
    conn.execute(sql, &params).await?;

    Ok(Json(ApiResponse::msg("注册成功")))
}

#[derive(Deserialize)]
pub struct MobileChangePasswordRequest {
    pub old_password: String,
    pub new_password: String,
}

pub async fn mobile_change_password(
    Extension(claims): Extension<Claims>,
    Json(body): Json<MobileChangePasswordRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    if body.new_password.len() < 6 {
        return Ok(Json(ApiResponse::err("新密码长度不能少于6位")));
    }

    let mut conn = get_pool().get().await?;
    let sql = "SELECT TOP 1 [EmpNo], [PassWordStr] FROM [tBas_Emp] WHERE [EmpNo] = @p1";
    let stream = conn.query(sql, &[&claims.user_code.as_str()]).await?;

    if let Some(row) = stream.into_row().await? {
        let stored_password: &str = row.get::<&str, _>("PassWordStr").unwrap_or("");
        if !verify_password(&body.old_password, stored_password) {
            return Ok(Json(ApiResponse::err("旧密码错误")));
        }
        let hashed = match hash_password(&body.new_password) {
            Some(h) => h,
            None => {
                return Ok(Json(ApiResponse::err(
                    "密码哈希失败，可能密码过长（>72字节）",
                )));
            }
        };
        let update_sql = "UPDATE [tBas_Emp] SET [PassWordStr] = @p1 WHERE [EmpNo] = @p2";
        conn.execute(update_sql, &[&hashed.as_str(), &claims.user_code.as_str()])
            .await?;
    } else {
        return Ok(Json(ApiResponse::err("未找到该用户")));
    }

    Ok(Json(ApiResponse::msg("密码修改成功")))
}

pub async fn sync_base_data(
    Extension(_claims): Extension<Claims>,
    State(_config): State<Config>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let goods_sql = r#"SELECT TOP 500 [GDSID], [GDSNO], [GDSDesc], [GDSSpec], [BarCode],
        [UnitNO], [AInPrice], [SPrice], [BrandID], [GDSTypeID]
        FROM [tBas_Goods] WHERE [State] = 'Y' ORDER BY [GDSDesc]"#;
    let goods_stream = conn.query(goods_sql, &[]).await?;
    let goods_rows: Vec<Row> = goods_stream.into_first_result().await?;
    let goods: Vec<serde_json::Value> = goods_rows.iter().map(row_to_json).collect();

    let stock_sql = r#"SELECT TOP 200 [StkID], [StkName], [StkType] FROM [tBas_Stock] WHERE [Used] <> 'N' ORDER BY [StkName]"#;
    let stock_stream = conn.query(stock_sql, &[]).await?;
    let stock_rows: Vec<Row> = stock_stream.into_first_result().await?;
    let warehouses: Vec<serde_json::Value> = stock_rows.iter().map(row_to_json).collect();

    let cust_sql = r#"SELECT TOP 200 [CustID], [CustNo], [CustName], [CustTypeID], [AreaID]
        FROM [tBas_Cust] WHERE [State] <> 'D' ORDER BY [CustName]"#;
    let cust_stream = conn.query(cust_sql, &[]).await?;
    let cust_rows: Vec<Row> = cust_stream.into_first_result().await?;
    let customers: Vec<serde_json::Value> = cust_rows.iter().map(row_to_json).collect();

    let qty_sql = r#"SELECT TOP 1000 q.[GDSID], q.[StkID], q.[Qty],
        g.[GDSNO], g.[GDSDesc], s.[StkName]
        FROM [tStk_Qty] q
        LEFT JOIN [tBas_Goods] g ON q.[GDSID] = g.[GDSID]
        LEFT JOIN [tBas_Stock] s ON q.[StkID] = s.[StkID]"#;
    let qty_stream = conn.query(qty_sql, &[]).await?;
    let qty_rows: Vec<Row> = qty_stream.into_first_result().await?;
    let stock_qty: Vec<serde_json::Value> = qty_rows.iter().map(row_to_json).collect();

    let data = serde_json::json!({
        "goods": goods,
        "warehouses": warehouses,
        "customers": customers,
        "stock_qty": stock_qty,
    });

    Ok(Json(ApiResponse::ok(data)))
}

#[derive(Deserialize)]
pub struct SubmitReplenishmentRequest {
    pub StkID: String,
    pub Remark: Option<String>,
    #[serde(default)]
    pub SaleDate: Option<String>, // 补货日期 YYYY-MM-DD（默认今天）
    pub details: Vec<ReplenishmentDetailItem>,
}

#[derive(Deserialize)]
pub struct ReplenishmentDetailItem {
    pub GDSID: String,
    pub Qty: f64,
    pub Remark: Option<String>,
}

pub async fn submit_replenishment(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<SubmitReplenishmentRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    if body.details.is_empty() {
        return Ok(Json(ApiResponse::err("补货明细不能为空")));
    }

    let mut conn = get_pool().get().await?;

    // 补货日期（YYYY-MM-DD），默认今天
    let date_str = body
        .SaleDate
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| chrono::Local::now().format("%Y-%m-%d").to_string());
    let date_key = chrono::NaiveDate::parse_from_str(&date_str, "%Y-%m-%d")
        .map(|d| d.format("%Y%m%d").to_string())
        .unwrap_or_else(|_| date_str.replace("-", ""));

    // 同一天同一仓库的补货 = 同一单：有则追加明细，无则新建
    // 检查当天该仓库是否已有补货记录，有则沿用其 EDate
    let check_sql = "SELECT TOP 1 [EDate] FROM [tArd_AR] WHERE [StkID] = @p1 AND CONVERT(varchar(8),[EDate],112) = @p2";
    let check_row = conn
        .query(check_sql, &[&body.StkID, &date_key])
        .await?
        .into_row()
        .await?;
    let batch_time = if let Some(row) = check_row {
        let edate = get_str(&row, "EDate");
        if edate.is_empty() {
            format!("{} 00:00:00", date_str)
        } else {
            edate
        }
    } else {
        format!("{} 00:00:00", date_str)
    };

    let used = "Y";
    let zero_price: f64 = 0.0;

    for detail in body.details.iter() {
        let detail_sql = r#"INSERT INTO [tArd_AR] ([RowID], [StkID], [EmpID], [EDate], [SaleDate], [GDSID], [Qty], [Price], [Amt], [Used])
            VALUES (NEWID(), @p1, @p2, @p3, @p3, @p4, @p5, @p6, @p7, @p8)"#;
        let detail_params: Vec<&dyn tiberius::ToSql> = vec![
            &body.StkID,
            &claims.emp_id,
            &batch_time,
            &detail.GDSID,
            &detail.Qty,
            &zero_price,
            &zero_price,
            &used,
        ];
        conn.execute(detail_sql, &detail_params).await?;
    }

    // 生成显示用单号：仓库编码-YYYYMMDD
    let code_sql = "SELECT TOP 1 [StkCode] FROM [tBas_Stock] WHERE [StkID] = @p1";
    let code_row = conn
        .query(code_sql, &[&body.StkID])
        .await?
        .into_row()
        .await?;
    let stk_code = code_row
        .as_ref()
        .map(|r| get_str(r, "StkCode"))
        .unwrap_or_default();
    let apply_no = format!("{}-{}", stk_code, date_key);

    let count = body.details.len();
    Ok(Json(ApiResponse::ok(serde_json::json!({
        "count": count,
        "ApplyNo": apply_no,
        "message": format!("成功上传 {} 条补货", count)
    }))))
}

#[derive(Deserialize)]
pub struct ReplenishmentHistoryParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
}

pub async fn get_replenishment_history(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<ReplenishmentHistoryParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 1000);

    // 分组键 = StkID + EDate(到天)，ApplyNo 格式 = 仓库编码-YYYYMMDD
    // 只看当前员工自己的补货单
    let mut base_query = r#"SELECT t.[ApplyNo], t.[EDate], t.[StkID], t.[EmpID], t.[DetailCount], t.[SumQty], t.[SumAmt],
        sk.[StkName], e.[EmpName]
        FROM (
          SELECT ISNULL(sk_code,'') + '-' + ISNULL(CONVERT(varchar(8),[EDate],112),'') AS [ApplyNo],
            MIN([EDate]) AS [EDate], [StkID], MAX([EmpID]) AS [EmpID],
            COUNT(*) AS [DetailCount], SUM([Qty]) AS [SumQty], SUM([Amt]) AS [SumAmt]
          FROM (
            SELECT [EDate], [StkID], [EmpID], [Qty], [Amt],
              (SELECT TOP 1 [StkCode] FROM [tBas_Stock] WHERE [StkID] = a.[StkID]) AS sk_code
            FROM [tArd_AR] a
            WHERE [EmpID] = @p1
          ) b
          GROUP BY [StkID], CONVERT(varchar(8),[EDate],112), sk_code
        ) t
        LEFT JOIN [tBas_Stock] sk ON t.[StkID] = sk.[StkID]
        LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID]"#.to_string();
    let mut query_params: Vec<Option<String>> = vec![Some(claims.emp_id.clone())];
    let pidx = 2;

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(&format!(" WHERE t.[ApplyNo] LIKE @p{}", pidx));
            query_params.push(Some(format!("%{}%", kw)));
        }
    }

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, None, None);
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
pub struct ReplenishmentDetailParams {
    pub ApplyNo: String, // 仓库编码-YYYYMMDD
}

pub async fn get_replenishment_detail(
    Extension(_claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<ReplenishmentDetailParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    // 解析 ApplyNo: 仓库编码-YYYYMMDD
    let (stk_code, date_key) = match params.ApplyNo.rsplit_once('-') {
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
    if stk_id.is_empty() {
        return Ok(Json(ApiResponse::err("未找到仓库")));
    }

    let sql = r#"SELECT a.[RowID], a.[GDSID], a.[Qty], a.[EDate],
        g.[GDSNO], g.[GDSDesc], g.[GDSSpec], g.[UnitNO]
        FROM [tArd_AR] a
        LEFT JOIN [tBas_Goods] g ON a.[GDSID] = g.[GDSID]
        WHERE a.[StkID] = @p1 AND CONVERT(varchar(8), a.[EDate], 112) = @p2
        ORDER BY g.[GDSNO]"#;
    let stream = conn.query(sql, &[&stk_id, &date_key]).await?;
    let rows: Vec<Row> = stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
    Ok(Json(ApiResponse::ok(data)))
}

#[derive(Deserialize)]
pub struct StockCheckDetailParams {
    pub MoveID: String,
}

pub async fn get_stock_check_detail(
    Extension(_claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<StockCheckDetailParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    // tStk_MoveDetail: Qty=DiffQty, Price=SysQty(账面), Amt=RealQty(实盘)
    let sql = r#"SELECT d.[MoveDetailID] AS [RowID], d.[GDSID], d.[Qty] AS [DiffQty], d.[Price] AS [SysQty], d.[Amt] AS [RealQty],
        g.[GDSNO], g.[GDSDesc], g.[GDSSpec], g.[UnitNO]
        FROM [tStk_MoveDetail] d
        LEFT JOIN [tBas_Goods] g ON d.[GDSID] = g.[GDSID]
        WHERE d.[MoveID] = @p1
        ORDER BY d.[RowNO]"#;
    let stream = conn.query(sql, &[&params.MoveID]).await?;
    let rows: Vec<Row> = stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
    Ok(Json(ApiResponse::ok(data)))
}

#[derive(Deserialize)]
pub struct ReplenishmentTransferParams {
    pub StkID: Option<String>,
}

pub async fn get_replenishment_for_transfer(
    Extension(_claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<ReplenishmentTransferParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;

    let mut base_query = r#"SELECT r.[ReplenishApplyNo] AS ApplyNo, r.[ReplenishApplyDate] AS ApplyDate, r.[StkID], sk.[StkName],
        d.[RowNO] AS LineNo, d.[GDSID], g.[GDSNO], g.[GDSDesc], d.[Qty], d.[Note] AS Remark
        FROM [tStk_ReplenishApply] r
        INNER JOIN [tStk_ReplenishApplyDtl] d ON r.[ReplenishApplyID] = d.[ReplenishApplyID]
        LEFT JOIN [tBas_Stock] sk ON r.[StkID] = sk.[StkID]
        LEFT JOIN [tBas_Goods] g ON d.[GDSID] = g.[GDSID]
        WHERE r.[State] = 'S'"#.to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let pidx = 1;

    if let Some(stk_id) = &params.StkID {
        if !stk_id.is_empty() {
            base_query.push_str(&format!(" AND r.[StkID] = @p{}", pidx));
            query_params.push(Some(stk_id.clone()));
        }
    }

    base_query.push_str(" ORDER BY r.[ReplenishApplyDate] DESC");

    let param_refs: Vec<&dyn tiberius::ToSql> = query_params
        .iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();
    let data_stream = conn.query(&base_query, &param_refs).await?;
    let rows: Vec<Row> = data_stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok(data)))
}

#[derive(Deserialize)]
pub struct ReplenishmentSalesParams {
    pub StkID: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

pub async fn get_replenishment_for_sales(
    Extension(_claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<ReplenishmentSalesParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;

    let mut base_query = r#"SELECT q.[GDSID], g.[GDSNO], g.[GDSDesc], g.[GDSSpec],
        q.[StkID], s.[StkName], q.[Qty],
        ISNULL(sale.[SalesQty], 0) as SalesQty,
        ISNULL(sale.[SalesAmt], 0) as SalesAmt
        FROM [tStk_Qty] q
        LEFT JOIN [tBas_Goods] g ON q.[GDSID] = g.[GDSID]
        LEFT JOIN [tBas_Stock] s ON q.[StkID] = s.[StkID]
        LEFT JOIN (
            SELECT d.[GDSID], h.[StkID], SUM(d.[Qty]) as SalesQty, SUM(d.[Amt]) as SalesAmt
            FROM [tSal_InvDetail] d
            INNER JOIN [tSal_Inv] h ON d.[SIID] = h.[SIID]
            WHERE h.[State] <> 'D'"#
        .to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    if let Some(start) = &params.start_date {
        if !start.is_empty() {
            base_query.push_str(&format!(" AND h.[SIDate] >= @p{}", pidx));
            pidx += 1;
            query_params.push(Some(start.clone()));
        }
    }
    if let Some(end) = &params.end_date {
        if !end.is_empty() {
            base_query.push_str(&format!(" AND h.[SIDate] <= @p{}", pidx));
            pidx += 1;
            query_params.push(Some(end.clone()));
        }
    }

    base_query.push_str(" GROUP BY d.[GDSID], h.[StkID]) sale ON q.[GDSID] = sale.[GDSID] AND q.[StkID] = sale.[StkID]");
    base_query.push_str(" WHERE g.[State] <> 'D'");

    if let Some(stk_id) = &params.StkID {
        if !stk_id.is_empty() {
            base_query.push_str(&format!(" AND q.[StkID] = @p{}", pidx));
            query_params.push(Some(stk_id.clone()));
        }
    }

    base_query.push_str(" ORDER BY g.[GDSDesc]");

    let param_refs: Vec<&dyn tiberius::ToSql> = query_params
        .iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();
    let data_stream = conn.query(&base_query, &param_refs).await?;
    let rows: Vec<Row> = data_stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok(data)))
}

#[derive(Deserialize)]
pub struct SubmitStockCheckRequest {
    pub StkID: String,
    pub Remark: Option<String>,
    pub details: Vec<StockCheckDetailItem>,
}

#[derive(Deserialize)]
pub struct StockCheckDetailItem {
    pub GDSID: String,
    pub SysQty: f64,
    pub RealQty: f64,
    pub DiffQty: f64,
}

pub async fn submit_stock_check(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<SubmitStockCheckRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    if body.details.is_empty() {
        return Ok(Json(ApiResponse::err("盘点明细不能为空")));
    }

    let mut conn = get_pool().get().await?;

    // 统一单据号生成：使用 tSys_DocNoSeq 原子分配，格式 PD{YYMM}{NNNN}
    // 替换旧的 PD{YYYYMMDD}-{NNN} 格式，避免并发冲突和跳号
    let move_no = crate::utils::doc_no::generate_via_docnoseq(&mut conn, "PD").await?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let remark = body.Remark.as_deref().unwrap_or("");
    let move_id = format!("{}", uuid::Uuid::new_v4());

    // 事务包裹：INSERT 主表 + INSERT 明细 原子化，任一明细失败回滚
    let tx_result: std::result::Result<(), String> = async {
        crate::services::inventory_ledger::begin_tran(&mut conn).await.map_err(|e| e.to_string())?;

        let header_sql = r#"INSERT INTO [tStk_Move] ([MoveID], [MoveNO], [MoveDate], [FromStkID], [ToStkID], [Kind], [RSumAmt], [State], [Note], [EDate], [EUser])
            VALUES (@p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p3, @p10)"#;
        // P5 修复：tStk_Move.EUser 是 NOT NULL uniqueidentifier，旧 token 的 claims.emp_id 可能为空，
        //   空字符串会导致 "Conversion failed when converting from a character string to uniqueidentifier"
        //   对齐 inventory.rs:249 的写法，加 ZERO_UUID 回退
        const ZERO_UUID_STR: &str = "00000000-0000-0000-0000-000000000000";
        let euser = if claims.emp_id.is_empty() { ZERO_UUID_STR.to_string() } else { claims.emp_id.clone() };
        let header_params: Vec<&dyn tiberius::ToSql> = vec![
            &move_id, &move_no, &now, &body.StkID, &body.StkID, &"PD", &0.0f64, &"N", &remark, &euser,
        ];
        conn.execute(header_sql, &header_params).await.map_err(|e| format!("保存盘点主表失败: {}", e))?;

        for (i, detail) in body.details.iter().enumerate() {
            let row_no = format!("{:03}", i + 1);

            // 盘点明细：用 Qty 存 DiffQty（调整数量），Price 存 SysQty（账面），Amt 存 RealQty（实盘）
            // 这样后续查询盘点历史时仍可还原账面/实盘数据，避免 SysQty/RealQty 丢失
            let detail_sql = r#"INSERT INTO [tStk_MoveDetail] ([MoveID], [MoveDetailID], [RowNO], [GDSID], [Qty], [CNVQty], [StdQty], [Price], [Amt])
                VALUES (@p1, NEWID(), @p2, @p3, @p4, @p4, @p4, @p5, @p6)"#;
            let detail_params: Vec<&dyn tiberius::ToSql> = vec![
                &move_id, &row_no, &detail.GDSID, &detail.DiffQty, &detail.SysQty, &detail.RealQty,
            ];
            conn.execute(detail_sql, &detail_params).await.map_err(|e| format!("保存明细(行{})失败: {}", i + 1, e))?;
        }

        crate::services::inventory_ledger::commit_tran(&mut conn).await.map_err(|e| e.to_string())?;
        Ok(())
    }.await;
    if let Err(e) = tx_result {
        crate::services::inventory_ledger::rollback_tran(&mut conn).await;
        return Ok(Json(ApiResponse::err(&format!("盘点单保存失败: {}", e))));
    }

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "MoveID": move_id,
        "MoveNo": move_no
    }))))
}

#[derive(Deserialize)]
pub struct StockCheckHistoryParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
}

pub async fn get_stock_check_history(
    Extension(_claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<StockCheckHistoryParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 1000);

    let mut base_query = r#"SELECT m.[MoveID], m.[MoveNO] AS [MoveNo], m.[MoveDate], m.[State], m.[Note], m.[EDate], m.[EUser],
        fs.[StkName] AS [FromStkName], e.[EmpName]
        FROM [tStk_Move] m
        LEFT JOIN [tBas_Stock] fs ON m.[FromStkID] = fs.[StkID]
        LEFT JOIN [tBas_Emp] e ON m.[EUser] = e.[EmpID]
        WHERE m.[State] <> 'D' AND m.[Kind] = 'PD'"#.to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let pidx = 1;

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(&format!(
                " AND (m.[MoveNO] LIKE @p{} OR m.[Note] LIKE @p{} OR fs.[StkName] LIKE @p{})",
                pidx,
                pidx + 1,
                pidx + 2
            ));
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
        }
    }

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, None, None);
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
pub struct MobileStockQueryParams {
    pub keyword: Option<String>,
    pub StkID: Option<String>,
    pub page: Option<u32>,
    pub page_size: Option<u32>,
}

pub async fn get_mobile_stock_query(
    Extension(_claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<MobileStockQueryParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 1000);

    let mut base_query = r#"SELECT q.[GDSID], q.[StkID], q.[Qty],
        g.[GDSNO], g.[GDSDesc], g.[GDSSpec], g.[BarCode], g.[UnitNO], g.[SPrice],
        s.[StkName], u.[UnitName]
        FROM [tStk_Qty] q
        LEFT JOIN [tBas_Goods] g ON q.[GDSID] = g.[GDSID]
        LEFT JOIN [tBas_Stock] s ON q.[StkID] = s.[StkID]
        LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO]
        WHERE g.[State] <> 'D'"#
        .to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(&format!(
                " AND (g.[GDSNO] LIKE @p{} OR g.[GDSDesc] LIKE @p{} OR g.[BarCode] LIKE @p{})",
                pidx,
                pidx + 1,
                pidx + 2
            ));
            pidx += 3;
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
        }
    }

    if let Some(stk_id) = &params.StkID {
        if !stk_id.is_empty() {
            base_query.push_str(&format!(" AND q.[StkID] = @p{}", pidx));
            query_params.push(Some(stk_id.clone()));
        }
    }

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, None, None);
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
pub struct SubmitSpecialPriceRequest {
    pub CustID: Option<String>,
    pub GDSID: String,
    pub OrigPrice: Option<f64>,
    pub NewPrice: f64,
    pub StartDate: Option<String>,
    pub EndDate: Option<String>,
    pub Remark: Option<String>,
}

pub async fn submit_special_price(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<SubmitSpecialPriceRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let cust_id = body.CustID.as_deref().unwrap_or("");
    let orig_price = body.OrigPrice.unwrap_or(0.0);
    let start_date = body.StartDate.as_deref().unwrap_or("");
    let end_date = body.EndDate.as_deref().unwrap_or("");
    let remark = body.Remark.as_deref().unwrap_or("");

    let sql = r#"INSERT INTO [tSys_Parameters] ([ParametersID], [PCode], [PName], [PKind], [PHelp], [PValue], [EUser], [EDate])
        VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7)"#;

    let p_value = serde_json::json!({
        "CustID": cust_id,
        "GDSID": body.GDSID,
        "OrigPrice": orig_price,
        "NewPrice": body.NewPrice,
        "StartDate": start_date,
        "EndDate": end_date,
    })
    .to_string();

    let p_kind = "special_price";
    let p_desc = "特价申请";
    // PCode 加时间戳后缀避免唯一索引 (PCode, PTerm) 冲突
    let p_code = format!("SP{}", chrono::Local::now().format("%Y%m%d%H%M%S"));
    let p_value_str = p_value.as_str();
    let params: Vec<&dyn tiberius::ToSql> = vec![
        &p_code,
        &p_desc,
        &p_kind,
        &remark,
        &p_value_str,
        &claims.emp_id,
        &now,
    ];
    conn.execute(sql, &params).await?;

    Ok(Json(ApiResponse::msg("特价申请提交成功")))
}

#[derive(Deserialize)]
pub struct SpecialPriceHistoryParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
}

pub async fn get_special_price_history(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<SpecialPriceHistoryParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 1000);

    let base_query =
        r#"SELECT * FROM [tSys_Parameters] WHERE [PKind] = 'special_price' AND [EUser] = @p1"#
            .to_string();
    let query_params: Vec<Option<String>> = vec![Some(claims.emp_id.clone())];

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, None, None);
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
pub struct SubmitRewardProductRequest {
    pub CustID: Option<String>,
    pub GDSID: String,
    pub Qty: f64,
    pub Reason: Option<String>,
    pub Remark: Option<String>,
}

pub async fn submit_reward_product(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<SubmitRewardProductRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let cust_id = body.CustID.as_deref().unwrap_or("");
    let reason = body.Reason.as_deref().unwrap_or("");
    let remark = body.Remark.as_deref().unwrap_or("");

    let sql = r#"INSERT INTO [tSys_Parameters] ([ParametersID], [PCode], [PName], [PKind], [PHelp], [PValue], [EUser], [EDate])
        VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7)"#;

    let p_value = serde_json::json!({
        "CustID": cust_id,
        "GDSID": body.GDSID,
        "Qty": body.Qty,
        "Reason": reason,
    })
    .to_string();

    let p_kind = "reward_product";
    let p_desc = "奖励产品申请";
    let p_code = format!("RP{}", chrono::Local::now().format("%Y%m%d%H%M%S"));
    let p_value_str = p_value.as_str();
    let params: Vec<&dyn tiberius::ToSql> = vec![
        &p_code,
        &p_desc,
        &p_kind,
        &remark,
        &p_value_str,
        &claims.emp_id,
        &now,
    ];
    conn.execute(sql, &params).await?;

    Ok(Json(ApiResponse::msg("奖励产品申请提交成功")))
}

#[derive(Deserialize)]
pub struct RewardProductHistoryParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
}

pub async fn get_reward_product_history(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<RewardProductHistoryParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 1000);

    let base_query =
        r#"SELECT * FROM [tSys_Parameters] WHERE [PKind] = 'reward_product' AND [EUser] = @p1"#
            .to_string();
    let query_params: Vec<Option<String>> = vec![Some(claims.emp_id.clone())];

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, None, None);
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
pub struct SubmitGiftGivingRequest {
    pub CustID: Option<String>,
    pub GDSID: String,
    pub Qty: f64,
    pub Reason: Option<String>,
    pub Remark: Option<String>,
}

pub async fn submit_gift_giving(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<SubmitGiftGivingRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let cust_id = body.CustID.as_deref().unwrap_or("");
    let reason = body.Reason.as_deref().unwrap_or("");
    let remark = body.Remark.as_deref().unwrap_or("");

    let sql = r#"INSERT INTO [tSys_Parameters] ([ParametersID], [PCode], [PName], [PKind], [PHelp], [PValue], [EUser], [EDate])
        VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7)"#;

    let p_value = serde_json::json!({
        "CustID": cust_id,
        "GDSID": body.GDSID,
        "Qty": body.Qty,
        "Reason": reason,
    })
    .to_string();

    let p_kind = "gift_giving";
    let p_desc = "赠品赠送申请";
    let p_code = format!("GG{}", chrono::Local::now().format("%Y%m%d%H%M%S"));
    let p_value_str = p_value.as_str();
    let params: Vec<&dyn tiberius::ToSql> = vec![
        &p_code,
        &p_desc,
        &p_kind,
        &remark,
        &p_value_str,
        &claims.emp_id,
        &now,
    ];
    conn.execute(sql, &params).await?;

    Ok(Json(ApiResponse::msg("赠品赠送申请提交成功")))
}

#[derive(Deserialize)]
pub struct GiftGivingHistoryParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
}

pub async fn get_gift_giving_history(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<GiftGivingHistoryParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 1000);

    let base_query =
        r#"SELECT * FROM [tSys_Parameters] WHERE [PKind] = 'gift_giving' AND [EUser] = @p1"#
            .to_string();
    let query_params: Vec<Option<String>> = vec![Some(claims.emp_id.clone())];

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, None, None);
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

// ==================== 批量提交（特价/奖励/赠品统一接口） ====================

#[derive(Deserialize, Debug)]
pub struct SubmitBatchRequest {
    /// 业务类型：special_price / reward_product / gift_giving
    pub kind: String,
    /// 客户ID（选填，批次级公共字段）
    #[serde(default)]
    pub CustID: Option<String>,
    /// 备注（批次级公共字段，写入 PHelp）
    #[serde(default)]
    pub Remark: Option<String>,
    /// 奖励/赠品原因（批次级公共字段）
    #[serde(default)]
    pub Reason: Option<String>,
    /// 明细列表，每项含 GDSID + 各类型专属字段
    pub details: Vec<serde_json::Value>,
}

pub async fn submit_batch(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<SubmitBatchRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    if body.details.is_empty() {
        return Ok(Json(ApiResponse::err("明细不能为空")));
    }

    let (p_kind, p_name, p_code_prefix) = match body.kind.as_str() {
        "special_price" => ("special_price", "特价申请", "SP"),
        "reward_product" => ("reward_product", "奖励产品申请", "RP"),
        "gift_giving" => ("gift_giving", "赠品赠送申请", "GG"),
        _ => return Ok(Json(ApiResponse::err(&format!("未知类型: {}", body.kind)))),
    };

    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    // 生成共享批次号 PCode（同一批次所有明细共用）
    let p_code = format!(
        "{}{}",
        p_code_prefix,
        chrono::Local::now().format("%Y%m%d%H%M%S")
    );
    let cust_id = body.CustID.as_deref().unwrap_or("");
    let remark = body.Remark.as_deref().unwrap_or("");
    let reason = body.Reason.as_deref().unwrap_or("");
    let emp_id = if claims.emp_id.is_empty() {
        "00000000-0000-0000-0000-000000000000".to_string()
    } else {
        claims.emp_id.clone()
    };

    // ★ PTerm 用行号保证唯一索引 idx_Parameters_CodeTerm(PCode, PTerm) 不冲突
    let sql = r#"INSERT INTO [tSys_Parameters] ([ParametersID], [PCode], [PTerm], [PName], [PKind], [PHelp], [PValue], [EUser], [EDate])
        VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8)"#;

    for (idx, d) in body.details.iter().enumerate() {
        let gds_id = d.get("GDSID").and_then(|v| v.as_str()).unwrap_or("");
        let p_term = format!("{}", idx + 1);
        let p_value = match body.kind.as_str() {
            "special_price" => {
                let orig = d.get("OrigPrice").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let new_p = d.get("NewPrice").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let start = d.get("StartDate").and_then(|v| v.as_str()).unwrap_or("");
                let end = d.get("EndDate").and_then(|v| v.as_str()).unwrap_or("");
                serde_json::json!({
                    "CustID": cust_id, "GDSID": gds_id,
                    "OrigPrice": orig, "NewPrice": new_p,
                    "StartDate": start, "EndDate": end,
                })
            }
            "reward_product" | "gift_giving" => {
                let qty = d.get("Qty").and_then(|v| v.as_f64()).unwrap_or(0.0);
                serde_json::json!({
                    "CustID": cust_id, "GDSID": gds_id,
                    "Qty": qty, "Reason": reason,
                })
            }
            _ => serde_json::json!({ "GDSID": gds_id }),
        };
        let p_value_str = p_value.to_string();
        let params: Vec<&dyn tiberius::ToSql> = vec![
            &p_code,
            &p_term,
            &p_name,
            &p_kind,
            &remark,
            &p_value_str,
            &emp_id,
            &now,
        ];
        conn.execute(sql, &params).await?;
    }

    Ok(Json(ApiResponse::ok(
        serde_json::json!({ "PCode": p_code, "count": body.details.len() }),
    )))
}

#[derive(Deserialize)]
pub struct MobileShortageParams {
    pub StkID: Option<String>,
    pub threshold: Option<f64>,
    pub page: Option<u32>,
    pub page_size: Option<u32>,
}

pub async fn get_mobile_shortages(
    Extension(_claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<MobileShortageParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 1000);
    let threshold = params.threshold.unwrap_or(0.0);

    let mut base_query = r#"SELECT q.[GDSID], q.[StkID], q.[Qty],
        g.[GDSNO], g.[GDSDesc], g.[GDSSpec], g.[BarCode], g.[SPrice],
        s.[StkName]
        FROM [tStk_Qty] q
        LEFT JOIN [tBas_Goods] g ON q.[GDSID] = g.[GDSID]
        LEFT JOIN [tBas_Stock] s ON q.[StkID] = s.[StkID]
        WHERE g.[State] <> 'D' AND q.[Qty] <= @p1"#
        .to_string();
    let mut query_params: Vec<Option<String>> = vec![Some(threshold.to_string())];
    let pidx = 2;

    if let Some(stk_id) = &params.StkID {
        if !stk_id.is_empty() {
            base_query.push_str(&format!(" AND q.[StkID] = @p{}", pidx));
            query_params.push(Some(stk_id.clone()));
        }
    }

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, None, None);
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
pub struct MobileCommissionParams {
    pub StkID: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

/// POST /api/mobile/commission
/// 移动端门店提成：按品牌分组聚合 tSal_InvDetail.Commission（已由 recalc_invoice_commission 计算好）
/// 对齐 88 项目 GetMobileCommission 实现，返回 list + total_sales + total_commission
pub async fn get_mobile_commission(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<MobileCommissionParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    // 通过 EmpNo 找到 EmpID + 关联仓库（门店身份）
    // 移动端登录身份为门店仓库时，claims.user_code 存的是仓库编码或员工工号
    // 先尝试匹配 tBas_Emp，若未找到则按 claims 中的仓库 ID 查询
    let emp_sql = "SELECT TOP 1 e.[EmpID], e.[EmpName], e.[StkID] FROM [tBas_Emp] e WHERE e.[EmpNo] = @p1 AND e.[State] <> 'D'";
    let emp_stream = conn.query(emp_sql, &[&claims.user_code.as_str()]).await?;
    let (emp_id, _emp_name, emp_stk_id) = if let Some(row) = emp_stream.into_row().await? {
        (
            get_str(&row, "EmpID"),
            get_str(&row, "EmpName"),
            get_str(&row, "StkID"),
        )
    } else {
        (String::new(), String::new(), String::new())
    };

    // 优先使用参数 StkID，其次使用员工关联仓库（tBas_Emp.StkID）
    let stk_id = params
        .StkID
        .clone()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            if !emp_stk_id.is_empty() {
                Some(emp_stk_id.clone())
            } else {
                None
            }
        })
        .filter(|s| !s.is_empty());

    if stk_id.is_none() {
        return Ok(Json(ApiResponse::ok(serde_json::json!({
            "list": Vec::<serde_json::Value>::new(),
            "total_sales": 0.0,
            "total_commission": 0.0,
            "start_date": params.start_date.clone().unwrap_or_default(),
            "end_date": params.end_date.clone().unwrap_or_default(),
        }))));
    }
    let stk_id = stk_id.unwrap();

    // 默认本月
    let now = chrono::Utc::now();
    let default_start = format!("{:04}-{:02}-01", now.year(), now.month());
    let default_end = format!("{:04}-{:02}-{:02}", now.year(), now.month(), now.day());
    let start_date = params
        .start_date
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or(default_start);
    let end_date = params
        .end_date
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or(default_end);

    // 按品牌分组聚合销售单明细的提成数据
    // 数据源：tSal_Inv + tSal_InvDetail（State IN ('S','Y')），已包含 Commission/CommissionRate/CommissionType
    let sql = r#"
        SELECT
            CONVERT(varchar(40), ISNULL(g.BrandID, '00000000-0000-0000-0000-000000000000')) AS BrandID,
            ISNULL(b.BrandName, '未分类') AS BrandName,
            ISNULL(b.Level, '') AS BrandLevel,
            SUM(ISNULL(d.Amt, 0)) AS SalesAmount,
            SUM(ISNULL(d.Commission, 0)) AS Commission,
            AVG(NULLIF(d.CommissionRate, 0)) AS CommissionRate,
            COUNT(DISTINCT d.GDSID) AS ProductCount
        FROM tSal_Inv i
        INNER JOIN tSal_InvDetail d ON i.SIID = d.SIID
        LEFT JOIN tBas_Goods g ON d.GDSID = g.GDSID
        LEFT JOIN tBas_Brand b ON g.BrandID = b.BrandID
        WHERE i.State IN ('S', 'Y')
          AND i.StkID = @p1
          AND CONVERT(date, i.SIDate) >= @p2
          AND CONVERT(date, i.SIDate) <= @p3
        GROUP BY ISNULL(g.BrandID, '00000000-0000-0000-0000-000000000000'),
                 ISNULL(b.BrandName, '未分类'),
                 ISNULL(b.Level, '')
        ORDER BY Commission DESC
    "#;

    let stream = conn
        .query(
            sql,
            &[&stk_id.as_str(), &start_date.as_str(), &end_date.as_str()],
        )
        .await?;
    let rows: Vec<Row> = stream.into_first_result().await?;

    let mut total_sales = 0.0f64;
    let mut total_commission = 0.0f64;
    let list: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let brand_id = get_str(row, "BrandID");
            let brand_name = get_str(row, "BrandName");
            let brand_level = get_str(row, "BrandLevel");
            let sales_amount = row_try_f64(row, "SalesAmount");
            let commission = row_try_f64(row, "Commission");
            let commission_rate = row_try_f64(row, "CommissionRate");
            let product_count = row
                .try_get::<i32, _>("ProductCount")
                .ok()
                .flatten()
                .unwrap_or(0);

            total_sales += sales_amount;
            total_commission += commission;

            serde_json::json!({
                "BrandID": brand_id,
                "BrandName": brand_name,
                "BrandLevel": brand_level,
                "SalesAmount": sales_amount,
                "Commission": commission,
                "CommissionRate": commission_rate,
                "CommissionAmount": commission,
                "ProductCount": product_count,
            })
        })
        .collect();

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "list": list,
        "total_sales": (total_sales * 100.0).round() / 100.0,
        "total_commission": (total_commission * 100.0).round() / 100.0,
        "start_date": start_date,
        "end_date": end_date,
        "EmpID": emp_id,
    }))))
}

/// 从 Row 中安全读取 f64 字段
fn row_try_f64(row: &Row, col: &str) -> f64 {
    row.try_get::<f64, _>(col).ok().flatten().unwrap_or(0.0)
}

#[derive(Deserialize)]
pub struct SalesTaskQueryParams {
    pub StkID: Option<String>,
}

pub async fn get_current_sales_task(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<SalesTaskQueryParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let mut base_query = r#"SELECT * FROM [tSys_Parameters]
        WHERE [PKind] = 'sales_task' AND [EUser] = @p1 AND [EDate] >= @p2"#
        .to_string();
    let mut query_params: Vec<Option<String>> = vec![
        Some(claims.emp_id.clone()),
        Some(chrono::Local::now().format("%Y-%m-%d").to_string()),
    ];

    if let Some(stk_id) = &params.StkID {
        if !stk_id.is_empty() {
            base_query.push_str(" AND [Remark] LIKE @p3");
            query_params.push(Some(format!("%{}%", stk_id)));
        }
    }

    base_query.push_str(" ORDER BY [EDate] DESC");

    let param_refs: Vec<&dyn tiberius::ToSql> = query_params
        .iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();
    let stream = conn.query(&base_query, &param_refs).await?;

    if let Some(row) = stream.into_row().await? {
        Ok(Json(ApiResponse::ok(row_to_json(&row))))
    } else {
        Ok(Json(ApiResponse::ok(serde_json::Value::Null)))
    }
}

#[derive(Deserialize)]
pub struct SalesTaskListParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub StkID: Option<String>,
}

pub async fn get_sales_task_list(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<SalesTaskListParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 1000);

    let mut base_query = r#"SELECT t.*, e.[EmpName] AS [EUserName] FROM [tSys_Parameters] t
        LEFT JOIN [tBas_Emp] e ON t.[EUser] = e.[EmpID]
        WHERE t.[PKind] = 'sales_task' AND t.[EUser] = @p1"#
        .to_string();
    let mut query_params: Vec<Option<String>> = vec![Some(claims.emp_id.clone())];
    let pidx = 2;

    if let Some(stk_id) = &params.StkID {
        if !stk_id.is_empty() {
            // tSys_Parameters 没有 Remark 字段，用 PHelp 兜底搜索
            base_query.push_str(&format!(" AND t.[PHelp] LIKE @p{}", pidx));
            query_params.push(Some(format!("%{}%", stk_id)));
        }
    }

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, None, None);
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
pub struct CreateSalesTaskRequest {
    pub TaskName: String,
    pub TargetAmt: f64,
    pub StartDate: String,
    pub EndDate: String,
    pub StkID: Option<String>,
    pub Remark: Option<String>,
}

pub async fn create_sales_task(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<CreateSalesTaskRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let param_id = format!("{}", uuid::Uuid::new_v4());
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let stk_id = body.StkID.as_deref().unwrap_or("");
    let remark = body.Remark.as_deref().unwrap_or("");

    let sql = r#"INSERT INTO [tSys_Parameters] ([ParametersID], [PCode], [PName], [PKind], [PHelp], [PValue], [EUser], [EDate])
        VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7)"#;

    let p_value = serde_json::json!({
        "TaskName": body.TaskName,
        "TargetAmt": body.TargetAmt,
        "StartDate": body.StartDate,
        "EndDate": body.EndDate,
        "StkID": stk_id,
    })
    .to_string();

    let p_kind = "sales_task";
    let p_desc = "销售任务";
    let p_value_str = p_value.as_str();
    let params: Vec<&dyn tiberius::ToSql> = vec![
        &param_id,
        &p_desc,
        &p_kind,
        &remark,
        &p_value_str,
        &claims.emp_id,
        &now,
    ];
    conn.execute(sql, &params).await?;

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "TaskID": param_id
    }))))
}

#[derive(Deserialize)]
pub struct UpdateSalesTaskRequest {
    pub TaskID: String,
    pub TaskName: Option<String>,
    pub TargetAmt: Option<f64>,
    pub StartDate: Option<String>,
    pub EndDate: Option<String>,
    pub StkID: Option<String>,
    pub Remark: Option<String>,
}

pub async fn update_sales_task(
    Extension(_claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<UpdateSalesTaskRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let check_sql = r#"SELECT TOP 1 [PValue] FROM [tSys_Parameters]
        WHERE [PKind] = 'sales_task' AND [PCode] = @p1"#;
    let check_stream = conn.query(check_sql, &[&body.TaskID.as_str()]).await?;

    let mut existing_value = String::new();
    if let Some(row) = check_stream.into_row().await? {
        if let Some(v) = row.try_get::<&str, _>("PValue").ok().flatten() {
            existing_value = v.to_string();
        }
    } else {
        return Ok(Json(ApiResponse::err("未找到该销售任务")));
    }

    let mut existing: serde_json::Value =
        serde_json::from_str(&existing_value).unwrap_or(serde_json::json!({}));
    if let Some(obj) = existing.as_object_mut() {
        if let Some(v) = &body.TaskName {
            obj.insert("TaskName".to_string(), serde_json::Value::String(v.clone()));
        }
        if let Some(v) = &body.TargetAmt {
            obj.insert(
                "TargetAmt".to_string(),
                serde_json::Value::Number(
                    serde_json::Number::from_f64(*v).unwrap_or(serde_json::Number::from(0)),
                ),
            );
        }
        if let Some(v) = &body.StartDate {
            obj.insert(
                "StartDate".to_string(),
                serde_json::Value::String(v.clone()),
            );
        }
        if let Some(v) = &body.EndDate {
            obj.insert("EndDate".to_string(), serde_json::Value::String(v.clone()));
        }
        if let Some(v) = &body.StkID {
            obj.insert("StkID".to_string(), serde_json::Value::String(v.clone()));
        }
    }

    let new_p_value = existing.to_string();
    let new_p_value_str = new_p_value.as_str();
    let task_id_str = body.TaskID.as_str();
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let update_sql = r#"UPDATE [tSys_Parameters] SET [PValue] = @p1, [EDate] = @p2
        WHERE [PKind] = 'sales_task' AND [PCode] = @p3"#;
    let update_params: Vec<&dyn tiberius::ToSql> = vec![&new_p_value_str, &now, &task_id_str];
    conn.execute(update_sql, &update_params).await?;

    Ok(Json(ApiResponse::msg("销售任务更新成功")))
}

#[derive(Deserialize)]
pub struct DeleteSalesTaskRequest {
    pub TaskID: String,
}

pub async fn delete_sales_task(
    Extension(_claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<DeleteSalesTaskRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let sql = r#"DELETE FROM [tSys_Parameters]
        WHERE [PKind] = 'sales_task' AND [PCode] = @p1"#;
    conn.execute(sql, &[&body.TaskID.as_str()]).await?;

    Ok(Json(ApiResponse::msg("销售任务删除成功")))
}

#[derive(Deserialize)]
pub struct SubmitDailySalesRecordRequest {
    pub TaskID: String,
    pub RecordDate: String,
    pub SalesAmt: f64,
    pub StkID: Option<String>,
    pub Remark: Option<String>,
}

pub async fn submit_daily_sales_record(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<SubmitDailySalesRecordRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let param_id = format!("{}", uuid::Uuid::new_v4());
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let stk_id = body.StkID.as_deref().unwrap_or("");
    let remark = body.Remark.as_deref().unwrap_or("");

    let sql = r#"INSERT INTO [tSys_Parameters] ([ParametersID], [PCode], [PName], [PKind], [PHelp], [PValue], [EUser], [EDate])
        VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7)"#;

    let p_value = serde_json::json!({
        "TaskID": body.TaskID,
        "RecordDate": body.RecordDate,
        "SalesAmt": body.SalesAmt,
        "StkID": stk_id,
    })
    .to_string();

    let p_kind = "sales_record";
    let p_desc = "销售日报";
    let p_value_str = p_value.as_str();
    let params: Vec<&dyn tiberius::ToSql> = vec![
        &param_id,
        &p_desc,
        &p_kind,
        &remark,
        &p_value_str,
        &claims.emp_id,
        &now,
    ];
    conn.execute(sql, &params).await?;

    Ok(Json(ApiResponse::msg("销售日报提交成功")))
}

#[derive(Deserialize)]
pub struct SalesTaskRecordsParams {
    pub TaskID: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub page: Option<u32>,
    pub page_size: Option<u32>,
}

pub async fn get_sales_task_records(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<SalesTaskRecordsParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 1000);

    let mut base_query = r#"SELECT * FROM [tSys_Parameters]
        WHERE [PKind] = 'sales_record' AND [EUser] = @p1"#
        .to_string();
    let mut query_params: Vec<Option<String>> = vec![Some(claims.emp_id.clone())];
    let mut pidx = 2;

    if let Some(task_id) = &params.TaskID {
        if !task_id.is_empty() {
            base_query.push_str(&format!(" AND [PValue] LIKE @p{}", pidx));
            pidx += 1;
            query_params.push(Some(format!("%{}%", task_id)));
        }
    }

    if let Some(start) = &params.start_date {
        if !start.is_empty() {
            base_query.push_str(&format!(" AND [EDate] >= @p{}", pidx));
            pidx += 1;
            query_params.push(Some(start.clone()));
        }
    }

    if let Some(end) = &params.end_date {
        if !end.is_empty() {
            base_query.push_str(&format!(" AND [EDate] <= @p{}", pidx));
            query_params.push(Some(end.clone()));
        }
    }

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, None, None);
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
pub struct SubmitShortageRequest {
    pub StkID: String,
    pub Priority: Option<String>,
    pub Contact: Option<String>,
    pub Note: Option<String>,
    pub details: Vec<ShortageDetailItem>,
}

#[derive(Deserialize, Serialize)]
pub struct ShortageDetailItem {
    pub GDSID: String,
    pub Qty: f64,
    pub Reason: Option<String>,
}

pub async fn submit_shortage(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<SubmitShortageRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let param_id = format!("{}", uuid::Uuid::new_v4());
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let p_value = serde_json::json!({
        "StkID": body.StkID,
        "Priority": body.Priority,
        "Contact": body.Contact,
        "Note": body.Note,
        "details": body.details,
    })
    .to_string();

    let p_kind = "shortage_report";
    let p_desc = "缺货上报";
    let p_value_str = p_value.as_str();
    let note_str = body.Note.as_deref().unwrap_or("");
    let user_code = claims.user_code.as_str();
    let params: Vec<&dyn tiberius::ToSql> = vec![
        &param_id,
        &p_desc,
        &p_kind,
        &note_str,
        &p_value_str,
        &user_code,
        &now,
    ];
    let sql = r#"INSERT INTO [tSys_Parameters] ([ParametersID], [PCode], [PName], [PKind], [PHelp], [PValue], [EUser], [EDate])
        VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7)"#;
    conn.execute(sql, &params).await?;

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "ReportID": param_id
    }))))
}

#[derive(Deserialize, Default)]
pub struct MobileHomeStatsParams {}

pub async fn get_mobile_home_stats(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(_params): Json<MobileHomeStatsParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    // 通过 EmpNo 找到 EmpID
    let emp_sql = "SELECT TOP 1 [EmpID] FROM [tBas_Emp] WHERE [EmpNo] = @p1 AND [State] <> 'D'";
    let emp_stream = conn.query(emp_sql, &[&claims.user_code.as_str()]).await?;
    let emp_id = if let Some(row) = emp_stream.into_row().await? {
        get_str(&row, "EmpID")
    } else {
        String::new()
    };

    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let today_start = format!("{} 00:00:00", today);
    let today_end = format!("{} 23:59:59", today);

    let mut today_orders: i64 = 0;
    let mut today_amount: f64 = 0.0;
    let mut pending_tasks: i64 = 0;

    if !emp_id.is_empty() {
        // 今日线上订单数 + 销售额（tOnline_Order 实际字段为 EDate/TotalAmt，不能用 OrderDate/Amount）
        let order_sql = r#"SELECT COUNT(*) as cnt, COALESCE(SUM(COALESCE(o.[TotalAmt], 0)), 0) as amt
            FROM [tOnline_Order] o
            WHERE o.[State] <> 'D' AND o.[EmpID] = @p1
              AND o.[EDate] >= @p2 AND o.[EDate] <= @p3"#;
        let order_stream = conn
            .query(
                order_sql,
                &[&emp_id.as_str(), &today_start.as_str(), &today_end.as_str()],
            )
            .await?;
        if let Some(row) = order_stream.into_row().await? {
            today_orders = row.get::<i32, _>("cnt").unwrap_or(0) as i64;
            today_amount = row.get::<f64, _>("amt").unwrap_or(0.0);
        }

        // 待办：本人通过手机上传的补货记录（tArd_AR 扁平表）
        let rep_sql = r#"SELECT COUNT(*) as cnt FROM [tArd_AR]
            WHERE [EmpID] = @p1"#;
        let rep_stream = conn.query(rep_sql, &[&emp_id.as_str()]).await?;
        if let Some(row) = rep_stream.into_row().await? {
            pending_tasks += row.get::<i32, _>("cnt").unwrap_or(0) as i64;
        }
    }

    // 待办：本人提交的缺货上报未处理数量（EUser 存的是 EmpID GUID）
    let shortage_sql = r#"SELECT COUNT(*) as cnt FROM [tSys_Parameters]
        WHERE [PKind] = 'shortage_report' AND [EUser] = @p1"#;
    let shortage_stream = conn.query(shortage_sql, &[&emp_id.as_str()]).await?;
    if let Some(row) = shortage_stream.into_row().await? {
        pending_tasks += row.get::<i32, _>("cnt").unwrap_or(0) as i64;
    }

    let data = serde_json::json!({
        "todayOrders": today_orders,
        "todayAmount": (today_amount * 100.0).round() / 100.0,
        "pendingTasks": pending_tasks,
    });
    Ok(Json(ApiResponse::ok(data)))
}

#[derive(Deserialize)]
pub struct ShortageReportHistoryParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
}

pub async fn get_shortage_report_history(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<ShortageReportHistoryParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 1000);

    let base_query = r#"SELECT * FROM [tSys_Parameters]
        WHERE [PKind] = 'shortage_report' AND [EUser] = @p1
        ORDER BY [EDate] DESC"#
        .to_string();
    let query_params: Vec<Option<String>> = vec![Some(claims.emp_id.clone())];

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, None, None);
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
