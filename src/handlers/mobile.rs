use axum::{
    extract::State,
    Json,
    Extension,
};
use chrono::Datelike;
use serde::{Deserialize, Serialize};
use tiberius::Row;
use crate::config::Config;
use crate::db::get_pool;
use crate::error::Result;
use crate::utils::{ApiResponse, build_pagination_sql_with_sort};
use crate::handlers::base_data::try_get_value;
use crate::middleware::auth::Claims;

const PASSWORD_SALT: &str = "erp_shenhuihui_2024";
const HASH_PREFIX: &str = "SHA256:";

fn hash_password(password: &str) -> String {
    use sha2::{Sha256, Digest};
    let mut hasher = Sha256::new();
    hasher.update(format!("{}{}", password, PASSWORD_SALT).as_bytes());
    let result = hasher.finalize();
    format!("{}{}", HASH_PREFIX, hex::encode(result))
}

const LEGACY_XOR_KEY: [u8; 8] = [0x36, 0x5B, 0xAC, 0xCD, 0xE1, 0x29, 0x0B, 0xAD];

fn is_legacy_encrypted_password(s: &str) -> bool {
    s.len() == 16 && s.chars().all(|c| c.is_ascii_hexdigit())
}

fn decrypt_legacy_password(stored: &str) -> Option<String> {
    let bytes: Vec<u8> = (0..stored.len())
        .step_by(2)
        .filter_map(|i| u8::from_str_radix(&stored[i..i + 2], 16).ok())
        .collect();
    if bytes.len() != 8 {
        return None;
    }
    let decrypted: Vec<u8> = bytes.iter()
        .enumerate()
        .map(|(i, &b)| b ^ LEGACY_XOR_KEY[i])
        .collect();
    let trimmed = decrypted.iter()
        .take_while(|&&b| b != 0)
        .copied()
        .collect::<Vec<u8>>();
    String::from_utf8(trimmed).ok()
}

fn verify_password(password: &str, stored: &str) -> bool {
    // 方式1: SHA256加密
    if stored.starts_with(HASH_PREFIX) {
        let hash = hash_password(password);
        return hash == stored;
    }
    
    // 方式2: XOR加密（16位十六进制）
    // 兼容老ERP：自动把字母O/o替换为数字0
    let normalized_stored = stored.replace('O', "0").replace('o', "0");
    
    if normalized_stored.len() == 16 && normalized_stored.chars().all(|c| c.is_ascii_hexdigit()) {
        // 使用规范化后的值进行解密
        if let Some(decrypted) = decrypt_legacy_password(&normalized_stored) {
            return password == decrypted;
        }
    }
    
    // 方式3: 空密码
    if stored.is_empty() {
        return false;
    }
    
    // 方式4: 明文比较
    password == stored
}

fn needs_upgrade(stored: &str) -> bool {
    !stored.starts_with(HASH_PREFIX)
}

fn row_to_json(row: &Row) -> serde_json::Value {
    let columns = row.columns();
    let mut map = serde_json::Map::new();
    for col in columns {
        let name = col.name().to_string();
        if name == "_rn" {
            continue;
        }
        let val = try_get_value(row, &name);
        map.insert(name, val);
    }
    serde_json::Value::Object(map)
}

fn parse_naive_datetime(s: &str) -> Option<chrono::NaiveDateTime> {
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").ok().or_else(|| {
        chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .ok()
            .and_then(|d| d.and_hms_opt(0, 0, 0))
    })
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
        WHERE e.[EmpNo] = @p1 AND e.[AllowLogin] = 1"#;
    let stream = conn.query(sql, &[&body.EmpNo.as_str()]).await?;

    if let Some(row) = stream.into_row().await? {
        let emp_no: &str = row.get::<&str, _>("EmpNo").unwrap_or("");
        let emp_name: &str = row.get::<&str, _>("EmpName").unwrap_or("");
        let dept_name: &str = row.get::<&str, _>("DeptName").unwrap_or("");
        let stk_name: &str = row.get::<&str, _>("StkName").unwrap_or("");

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

        if needs_upgrade(&stored_password) {
            let hashed = hash_password(&body.Password);
            let emp_id: &str = row.get::<&str, _>("EmpID").unwrap_or("");
            if !emp_id.is_empty() {
                let _ = conn.execute(
                    "UPDATE tBas_Emp SET PassWordStr = @p1 WHERE EmpID = @p2",
                    &[&hashed.as_str(), &emp_id],
                ).await;
            }
        }

        let claims = Claims {
            sub: emp_no.to_string(),
            user_code: emp_no.to_string(),
            user_name: emp_name.to_string(),
            exp: (chrono::Utc::now() + chrono::Duration::hours(24)).timestamp() as usize,
        };

        let token = jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret(config.jwt_secret.as_ref()),
        )?;

        let resp = MobileLoginResponse {
            token,
            user: MobileUserInfo {
                id: emp_no.to_string(),
                code: emp_no.to_string(),
                name: emp_name.to_string(),
                dept_name: dept_name.to_string(),
                stk_name: stk_name.to_string(),
            },
        };
        Ok(Json(ApiResponse::ok(resp)))
    } else {
        Ok(Json(ApiResponse::<MobileLoginResponse>::err("未找到该工号或无移动端登录权限")))
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
    State(_config): State<Config>,
    Json(body): Json<MobileRegisterRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let check_sql = "SELECT TOP 1 1 FROM [tBas_Emp] WHERE [EmpNo] = @p1";
    let check_stream = conn.query(check_sql, &[&body.EmpNo.as_str()]).await?;
    if check_stream.into_row().await?.is_some() {
        return Ok(Json(ApiResponse::err("该工号已存在")));
    }

    let hashed = hash_password(&body.Password);
    let emp_id = format!("{}", uuid::Uuid::new_v4());
    let now = chrono::Local::now().naive_local();
    let dept_id = body.DeptID.as_deref().unwrap_or("");
    let stk_id = body.StkID.as_deref().unwrap_or("");
    let phone = body.Phone.as_deref().unwrap_or("");

    let sql = r#"INSERT INTO [tBas_Emp] ([EmpID], [EmpNo], [EmpName], [PassWordStr], [DeptID], [StkID], [Phone], [AllowLogin], [State], [EDate], [EUser])
        VALUES (@p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10, @p11)"#;
    let params: Vec<&dyn tiberius::ToSql> = vec![
        &emp_id, &body.EmpNo, &body.EmpName, &hashed, &dept_id, &stk_id,
        &phone, &1i32, &"S", &now, &"mobile",
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
        let hashed = hash_password(&body.new_password);
        let update_sql = "UPDATE [tBas_Emp] SET [PassWordStr] = @p1 WHERE [EmpNo] = @p2";
        conn.execute(update_sql, &[&hashed.as_str(), &claims.user_code.as_str()]).await?;
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
        FROM [tBas_Goods] WHERE [State] IN ('S', '1') ORDER BY [GDSDesc]"#;
    let goods_stream = conn.query(goods_sql, &[]).await?;
    let goods_rows: Vec<Row> = goods_stream.into_first_result().await?;
    let goods: Vec<serde_json::Value> = goods_rows.iter().map(row_to_json).collect();

    let stock_sql = r#"SELECT [StkID], [StkName], [StkType] FROM [tBas_Stock] WHERE [Used] <> 'N'"#;
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
    conn.execute("BEGIN TRANSACTION", &[]).await?;

    let today = chrono::Local::now().format("%Y%m%d").to_string();
    let prefix_pattern = format!("RP{}-%", today);

    let seq_sql = "SELECT MAX([ApplyNo]) as max_no FROM [tStk_ReplenishApply] WHERE [ApplyNo] LIKE @p1";
    let seq_stream = conn.query(seq_sql, &[&prefix_pattern.as_str()]).await?;
    let seq_row = seq_stream.into_row().await?;

    let next_seq = if let Some(row) = seq_row {
        let max_no: Option<&str> = row.get("max_no");
        if let Some(max) = max_no {
            if let Some(seq_part) = max.rsplit('-').next() {
                seq_part.parse::<u32>().unwrap_or(0) + 1
            } else {
                1
            }
        } else {
            1
        }
    } else {
        1
    };

    let apply_no = format!("RP{}-{:03}", today, next_seq);
    let now = chrono::Local::now().naive_local();
    let remark = body.Remark.as_deref().unwrap_or("");

    let header_sql = r#"INSERT INTO [tStk_ReplenishApply] ([ApplyNo], [ApplyDate], [StkID], [EmpID], [State], [Remark], [EDate], [EUser])
        VALUES (@p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8)"#;
    let header_params: Vec<&dyn tiberius::ToSql> = vec![
        &apply_no, &now, &body.StkID, &claims.user_code, &"N", &remark, &now, &claims.user_code,
    ];
    conn.execute(header_sql, &header_params).await?;

    for (i, detail) in body.details.iter().enumerate() {
        let line_no = (i + 1) as i32;
        let detail_remark = detail.Remark.as_deref().unwrap_or("");

        let detail_sql = r#"INSERT INTO [tStk_ReplenishApplyDtl] ([ApplyNo], [LineNo], [GDSID], [Qty], [Remark])
            VALUES (@p1, @p2, @p3, @p4, @p5)"#;
        let detail_params: Vec<&dyn tiberius::ToSql> = vec![
            &apply_no, &line_no, &detail.GDSID, &detail.Qty, &detail_remark,
        ];
        conn.execute(detail_sql, &detail_params).await?;
    }

    conn.execute("COMMIT TRANSACTION", &[]).await?;

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "ApplyNo": apply_no
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
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);

    let mut base_query = r#"SELECT r.*, sk.[StkName]
        FROM [tStk_ReplenishApply] r
        LEFT JOIN [tBas_Stock] sk ON r.[StkID] = sk.[StkID]
        WHERE r.[State] <> 'D' AND r.[EmpID] = @p1"#.to_string();
    let mut query_params: Vec<Option<String>> = vec![Some(claims.user_code.clone())];
    let mut pidx = 2;

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(&format!(
                " AND (r.[ApplyNo] LIKE @p{} OR r.[Remark] LIKE @p{})",
                pidx, pidx + 1
            ));
            pidx += 2;
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
        }
    }

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(
        &base_query,
        page,
        page_size,
        None,
        None,
    );
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();

    let mut total: i32 = 0;
    let count_stream = conn.query(&count_sql, &param_refs).await?;
    if let Some(row) = count_stream.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }

    let data_stream = conn.query(&paginated_sql, &param_refs).await?;
    let rows: Vec<Row> = data_stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok_paginated(data, total as u64, page, page_size)))
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

    let mut base_query = r#"SELECT r.[ApplyNo], r.[ApplyDate], r.[StkID], sk.[StkName],
        d.[LineNo], d.[GDSID], g.[GDSNO], g.[GDSDesc], d.[Qty], d.[Remark]
        FROM [tStk_ReplenishApply] r
        INNER JOIN [tStk_ReplenishApplyDtl] d ON r.[ApplyNo] = d.[ApplyNo]
        LEFT JOIN [tBas_Stock] sk ON r.[StkID] = sk.[StkID]
        LEFT JOIN [tBas_Goods] g ON d.[GDSID] = g.[GDSID]
        WHERE r.[State] = 'S'"#.to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    if let Some(stk_id) = &params.StkID {
        if !stk_id.is_empty() {
            base_query.push_str(&format!(" AND r.[StkID] = @p{}", pidx));
            pidx += 1;
            query_params.push(Some(stk_id.clone()));
        }
    }

    base_query.push_str(" ORDER BY r.[ApplyDate] DESC");

    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();
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
            INNER JOIN [tSal_Inv] h ON d.[InvNo] = h.[InvNo]
            WHERE h.[State] <> 'D'"#.to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    if let Some(start) = &params.start_date {
        if !start.is_empty() {
            base_query.push_str(&format!(" AND h.[InvDate] >= @p{}", pidx));
            pidx += 1;
            query_params.push(Some(start.clone()));
        }
    }
    if let Some(end) = &params.end_date {
        if !end.is_empty() {
            base_query.push_str(&format!(" AND h.[InvDate] <= @p{}", pidx));
            pidx += 1;
            query_params.push(Some(end.clone()));
        }
    }

    base_query.push_str(" GROUP BY d.[GDSID], h.[StkID]) sale ON q.[GDSID] = sale.[GDSID] AND q.[StkID] = sale.[StkID]");
    base_query.push_str(" WHERE g.[State] <> 'D'");

    if let Some(stk_id) = &params.StkID {
        if !stk_id.is_empty() {
            base_query.push_str(&format!(" AND q.[StkID] = @p{}", pidx));
            pidx += 1;
            query_params.push(Some(stk_id.clone()));
        }
    }

    base_query.push_str(" ORDER BY g.[GDSDesc]");

    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();
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
    conn.execute("BEGIN TRANSACTION", &[]).await?;

    let today = chrono::Local::now().format("%Y%m%d").to_string();
    let prefix_pattern = format!("PD{}-%", today);

    let seq_sql = "SELECT MAX([MoveNo]) as max_no FROM [tStk_Move] WHERE [MoveNo] LIKE @p1";
    let seq_stream = conn.query(seq_sql, &[&prefix_pattern.as_str()]).await?;
    let seq_row = seq_stream.into_row().await?;

    let next_seq = if let Some(row) = seq_row {
        let max_no: Option<&str> = row.get("max_no");
        if let Some(max) = max_no {
            if let Some(seq_part) = max.rsplit('-').next() {
                seq_part.parse::<u32>().unwrap_or(0) + 1
            } else {
                1
            }
        } else {
            1
        }
    } else {
        1
    };

    let move_no = format!("PD{}-{:03}", today, next_seq);
    let now = chrono::Local::now().naive_local();
    let remark = body.Remark.as_deref().unwrap_or("");

    let header_sql = r#"INSERT INTO [tStk_Move] ([MoveNo], [MoveDate], [FromStkID], [ToStkID], [Kind], [TotalAmt], [State], [Remark], [EDate], [EUser])
        VALUES (@p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10)"#;
    let header_params: Vec<&dyn tiberius::ToSql> = vec![
        &move_no, &now, &body.StkID, &body.StkID, &"PD", &0.0f64, &"N", &remark, &now, &claims.user_code,
    ];
    conn.execute(header_sql, &header_params).await?;

    for (i, detail) in body.details.iter().enumerate() {
        let line_no = (i + 1) as i32;

        let detail_sql = r#"INSERT INTO [tStk_MoveDetail] ([MoveNo], [LineNo], [GDSID], [Qty], [Price], [Amt])
            VALUES (@p1, @p2, @p3, @p4, @p5, @p6)"#;
        let detail_params: Vec<&dyn tiberius::ToSql> = vec![
            &move_no, &line_no, &detail.GDSID, &detail.DiffQty, &0.0f64, &0.0f64,
        ];
        conn.execute(detail_sql, &detail_params).await?;
    }

    conn.execute("COMMIT TRANSACTION", &[]).await?;

    Ok(Json(ApiResponse::ok(serde_json::json!({
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
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<StockCheckHistoryParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);

    let mut base_query = r#"SELECT m.*, fs.[StkName] AS [FromStkName]
        FROM [tStk_Move] m
        LEFT JOIN [tBas_Stock] fs ON m.[FromStkID] = fs.[StkID]
        WHERE m.[State] <> 'D' AND m.[Kind] = 'PD'"#.to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(&format!(
                " AND (m.[MoveNo] LIKE @p{} OR m.[Remark] LIKE @p{})",
                pidx, pidx + 1
            ));
            pidx += 2;
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
        }
    }

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(
        &base_query,
        page,
        page_size,
        None,
        None,
    );
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();

    let mut total: i32 = 0;
    let count_stream = conn.query(&count_sql, &param_refs).await?;
    if let Some(row) = count_stream.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }

    let data_stream = conn.query(&paginated_sql, &param_refs).await?;
    let rows: Vec<Row> = data_stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok_paginated(data, total as u64, page, page_size)))
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
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);

    let mut base_query = r#"SELECT q.[GDSID], q.[StkID], q.[Qty],
        g.[GDSNO], g.[GDSDesc], g.[GDSSpec], g.[BarCode], g.[UnitNO], g.[SPrice],
        s.[StkName], u.[UnitName]
        FROM [tStk_Qty] q
        LEFT JOIN [tBas_Goods] g ON q.[GDSID] = g.[GDSID]
        LEFT JOIN [tBas_Stock] s ON q.[StkID] = s.[StkID]
        LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO]
        WHERE g.[State] <> 'D'"#.to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(&format!(
                " AND (g.[GDSNO] LIKE @p{} OR g.[GDSDesc] LIKE @p{} OR g.[BarCode] LIKE @p{})",
                pidx, pidx + 1, pidx + 2
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
            pidx += 1;
            query_params.push(Some(stk_id.clone()));
        }
    }

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(
        &base_query,
        page,
        page_size,
        None,
        None,
    );
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();

    let mut total: i32 = 0;
    let count_stream = conn.query(&count_sql, &param_refs).await?;
    if let Some(row) = count_stream.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }

    let data_stream = conn.query(&paginated_sql, &param_refs).await?;
    let rows: Vec<Row> = data_stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok_paginated(data, total as u64, page, page_size)))
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

    let param_id = format!("{}", uuid::Uuid::new_v4());
    let now = chrono::Local::now().naive_local();
    let cust_id = body.CustID.as_deref().unwrap_or("");
    let orig_price = body.OrigPrice.unwrap_or(0.0);
    let start_date = body.StartDate.as_deref().unwrap_or("");
    let end_date = body.EndDate.as_deref().unwrap_or("");
    let remark = body.Remark.as_deref().unwrap_or("");

    let sql = r#"INSERT INTO [tSys_Parameters] ([PKind], [PKey], [PValue], [PDesc], [Remark], [EDate], [EUser])
        VALUES (@p1, @p2, @p3, @p4, @p5, @p6, @p7)"#;

    let p_value = serde_json::json!({
        "CustID": cust_id,
        "GDSID": body.GDSID,
        "OrigPrice": orig_price,
        "NewPrice": body.NewPrice,
        "StartDate": start_date,
        "EndDate": end_date,
    }).to_string();

    let p_kind = "special_price";
    let p_desc = "特价申请";
    let p_value_str = p_value.as_str();
    let params: Vec<&dyn tiberius::ToSql> = vec![
        &p_kind, &param_id, &p_value_str, &p_desc, &remark, &now, &claims.user_code,
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
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);

    let base_query = r#"SELECT * FROM [tSys_Parameters] WHERE [PKind] = 'special_price' AND [EUser] = @p1"#.to_string();
    let query_params: Vec<Option<String>> = vec![Some(claims.user_code.clone())];

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, None, None);
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();

    let mut total: i32 = 0;
    let count_stream = conn.query(&count_sql, &param_refs).await?;
    if let Some(row) = count_stream.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }

    let data_stream = conn.query(&paginated_sql, &param_refs).await?;
    let rows: Vec<Row> = data_stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok_paginated(data, total as u64, page, page_size)))
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

    let param_id = format!("{}", uuid::Uuid::new_v4());
    let now = chrono::Local::now().naive_local();
    let cust_id = body.CustID.as_deref().unwrap_or("");
    let reason = body.Reason.as_deref().unwrap_or("");
    let remark = body.Remark.as_deref().unwrap_or("");

    let sql = r#"INSERT INTO [tSys_Parameters] ([PKind], [PKey], [PValue], [PDesc], [Remark], [EDate], [EUser])
        VALUES (@p1, @p2, @p3, @p4, @p5, @p6, @p7)"#;

    let p_value = serde_json::json!({
        "CustID": cust_id,
        "GDSID": body.GDSID,
        "Qty": body.Qty,
        "Reason": reason,
    }).to_string();

    let p_kind = "reward_product";
    let p_desc = "奖励产品申请";
    let p_value_str = p_value.as_str();
    let params: Vec<&dyn tiberius::ToSql> = vec![
        &p_kind, &param_id, &p_value_str, &p_desc, &remark, &now, &claims.user_code,
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
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);

    let base_query = r#"SELECT * FROM [tSys_Parameters] WHERE [PKind] = 'reward_product' AND [EUser] = @p1"#.to_string();
    let query_params: Vec<Option<String>> = vec![Some(claims.user_code.clone())];

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, None, None);
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();

    let mut total: i32 = 0;
    let count_stream = conn.query(&count_sql, &param_refs).await?;
    if let Some(row) = count_stream.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }

    let data_stream = conn.query(&paginated_sql, &param_refs).await?;
    let rows: Vec<Row> = data_stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok_paginated(data, total as u64, page, page_size)))
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

    let param_id = format!("{}", uuid::Uuid::new_v4());
    let now = chrono::Local::now().naive_local();
    let cust_id = body.CustID.as_deref().unwrap_or("");
    let reason = body.Reason.as_deref().unwrap_or("");
    let remark = body.Remark.as_deref().unwrap_or("");

    let sql = r#"INSERT INTO [tSys_Parameters] ([PKind], [PKey], [PValue], [PDesc], [Remark], [EDate], [EUser])
        VALUES (@p1, @p2, @p3, @p4, @p5, @p6, @p7)"#;

    let p_value = serde_json::json!({
        "CustID": cust_id,
        "GDSID": body.GDSID,
        "Qty": body.Qty,
        "Reason": reason,
    }).to_string();

    let p_kind = "gift_giving";
    let p_desc = "赠品赠送申请";
    let p_value_str = p_value.as_str();
    let params: Vec<&dyn tiberius::ToSql> = vec![
        &p_kind, &param_id, &p_value_str, &p_desc, &remark, &now, &claims.user_code,
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
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);

    let base_query = r#"SELECT * FROM [tSys_Parameters] WHERE [PKind] = 'gift_giving' AND [EUser] = @p1"#.to_string();
    let query_params: Vec<Option<String>> = vec![Some(claims.user_code.clone())];

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, None, None);
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();

    let mut total: i32 = 0;
    let count_stream = conn.query(&count_sql, &param_refs).await?;
    if let Some(row) = count_stream.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }

    let data_stream = conn.query(&paginated_sql, &param_refs).await?;
    let rows: Vec<Row> = data_stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok_paginated(data, total as u64, page, page_size)))
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
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);
    let threshold = params.threshold.unwrap_or(0.0);

    let mut base_query = r#"SELECT q.[GDSID], q.[StkID], q.[Qty],
        g.[GDSNO], g.[GDSDesc], g.[GDSSpec], g.[BarCode], g.[SPrice],
        s.[StkName]
        FROM [tStk_Qty] q
        LEFT JOIN [tBas_Goods] g ON q.[GDSID] = g.[GDSID]
        LEFT JOIN [tBas_Stock] s ON q.[StkID] = s.[StkID]
        WHERE g.[State] <> 'D' AND q.[Qty] <= @p1"#.to_string();
    let mut query_params: Vec<Option<String>> = vec![Some(threshold.to_string())];
    let mut pidx = 2;

    if let Some(stk_id) = &params.StkID {
        if !stk_id.is_empty() {
            base_query.push_str(&format!(" AND q.[StkID] = @p{}", pidx));
            pidx += 1;
            query_params.push(Some(stk_id.clone()));
        }
    }

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, None, None);
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();

    let mut total: i32 = 0;
    let count_stream = conn.query(&count_sql, &param_refs).await?;
    if let Some(row) = count_stream.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }

    let data_stream = conn.query(&paginated_sql, &param_refs).await?;
    let rows: Vec<Row> = data_stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok_paginated(data, total as u64, page, page_size)))
}

#[derive(Deserialize)]
pub struct MobileCommissionParams {
    pub StkID: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

pub async fn get_mobile_commission(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<MobileCommissionParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;

    let mut base_query = r#"SELECT * FROM [tSys_Parameters] WHERE [PKind] = 'commission' AND [EUser] = @p1"#.to_string();
    let mut query_params: Vec<Option<String>> = vec![Some(claims.user_code.clone())];
    let mut pidx = 2;

    if let Some(stk_id) = &params.StkID {
        if !stk_id.is_empty() {
            base_query.push_str(&format!(" AND [Remark] LIKE @p{}", pidx));
            pidx += 1;
            query_params.push(Some(format!("%{}%", stk_id)));
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
            pidx += 1;
            query_params.push(Some(end.clone()));
        }
    }

    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();
    let data_stream = conn.query(&base_query, &param_refs).await?;
    let rows: Vec<Row> = data_stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok(data)))
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

    let now = chrono::Local::now().naive_local();
    let current_month_start = chrono::NaiveDate::from_ymd_opt(now.year(), now.month(), 1)
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .unwrap_or(now);

    let mut base_query = r#"SELECT * FROM [tSys_Parameters]
        WHERE [PKind] = 'sales_task' AND [EUser] = @p1 AND [EDate] >= @p2"#.to_string();
    let mut query_params: Vec<Option<String>> = vec![
        Some(claims.user_code.clone()),
        Some(current_month_start.format("%Y-%m-%d").to_string()),
    ];

    if let Some(stk_id) = &params.StkID {
        if !stk_id.is_empty() {
            base_query.push_str(" AND [Remark] LIKE @p3");
            query_params.push(Some(format!("%{}%", stk_id)));
        }
    }

    base_query.push_str(" ORDER BY [EDate] DESC");

    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();
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
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);

    let mut base_query = r#"SELECT * FROM [tSys_Parameters]
        WHERE [PKind] = 'sales_task' AND [EUser] = @p1"#.to_string();
    let mut query_params: Vec<Option<String>> = vec![Some(claims.user_code.clone())];
    let mut pidx = 2;

    if let Some(stk_id) = &params.StkID {
        if !stk_id.is_empty() {
            base_query.push_str(&format!(" AND [Remark] LIKE @p{}", pidx));
            pidx += 1;
            query_params.push(Some(format!("%{}%", stk_id)));
        }
    }

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, None, None);
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();

    let mut total: i32 = 0;
    let count_stream = conn.query(&count_sql, &param_refs).await?;
    if let Some(row) = count_stream.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }

    let data_stream = conn.query(&paginated_sql, &param_refs).await?;
    let rows: Vec<Row> = data_stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok_paginated(data, total as u64, page, page_size)))
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
    let now = chrono::Local::now().naive_local();
    let stk_id = body.StkID.as_deref().unwrap_or("");
    let remark = body.Remark.as_deref().unwrap_or("");

    let sql = r#"INSERT INTO [tSys_Parameters] ([PKind], [PKey], [PValue], [PDesc], [Remark], [EDate], [EUser])
        VALUES (@p1, @p2, @p3, @p4, @p5, @p6, @p7)"#;

    let p_value = serde_json::json!({
        "TaskName": body.TaskName,
        "TargetAmt": body.TargetAmt,
        "StartDate": body.StartDate,
        "EndDate": body.EndDate,
        "StkID": stk_id,
    }).to_string();

    let p_kind = "sales_task";
    let p_desc = "销售任务";
    let p_value_str = p_value.as_str();
    let params: Vec<&dyn tiberius::ToSql> = vec![
        &p_kind, &param_id, &p_value_str, &p_desc, &remark, &now, &claims.user_code,
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
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<UpdateSalesTaskRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let check_sql = r#"SELECT TOP 1 [PValue] FROM [tSys_Parameters]
        WHERE [PKind] = 'sales_task' AND [PKey] = @p1 AND [EUser] = @p2"#;
    let check_stream = conn.query(check_sql, &[&body.TaskID.as_str(), &claims.user_code.as_str()]).await?;

    let mut existing_value = String::new();
    if let Some(row) = check_stream.into_row().await? {
        if let Some(v) = row.try_get::<&str, _>("PValue").ok().flatten() {
            existing_value = v.to_string();
        }
    } else {
        return Ok(Json(ApiResponse::err("未找到该销售任务")));
    }

    let mut existing: serde_json::Value = serde_json::from_str(&existing_value).unwrap_or(serde_json::json!({}));
    if let Some(obj) = existing.as_object_mut() {
        if let Some(v) = &body.TaskName { obj.insert("TaskName".to_string(), serde_json::Value::String(v.clone())); }
        if let Some(v) = &body.TargetAmt { obj.insert("TargetAmt".to_string(), serde_json::Value::Number(serde_json::Number::from_f64(*v).unwrap_or(serde_json::Number::from(0)))); }
        if let Some(v) = &body.StartDate { obj.insert("StartDate".to_string(), serde_json::Value::String(v.clone())); }
        if let Some(v) = &body.EndDate { obj.insert("EndDate".to_string(), serde_json::Value::String(v.clone())); }
        if let Some(v) = &body.StkID { obj.insert("StkID".to_string(), serde_json::Value::String(v.clone())); }
    }

    let new_p_value = existing.to_string();
    let new_p_value_str = new_p_value.as_str();
    let remark = body.Remark.as_deref().unwrap_or("");
    let task_id_str = body.TaskID.as_str();
    let user_code_str = claims.user_code.as_str();

    let update_sql = r#"UPDATE [tSys_Parameters] SET [PValue] = @p1, [Remark] = @p2
        WHERE [PKind] = 'sales_task' AND [PKey] = @p3 AND [EUser] = @p4"#;
    let update_params: Vec<&dyn tiberius::ToSql> = vec![
        &new_p_value_str, &remark, &task_id_str, &user_code_str,
    ];
    conn.execute(update_sql, &update_params).await?;

    Ok(Json(ApiResponse::msg("销售任务更新成功")))
}

#[derive(Deserialize)]
pub struct DeleteSalesTaskRequest {
    pub TaskID: String,
}

pub async fn delete_sales_task(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<DeleteSalesTaskRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let sql = r#"DELETE FROM [tSys_Parameters]
        WHERE [PKind] = 'sales_task' AND [PKey] = @p1 AND [EUser] = @p2"#;
    conn.execute(sql, &[&body.TaskID.as_str(), &claims.user_code.as_str()]).await?;

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
    let now = chrono::Local::now().naive_local();
    let stk_id = body.StkID.as_deref().unwrap_or("");
    let remark = body.Remark.as_deref().unwrap_or("");

    let sql = r#"INSERT INTO [tSys_Parameters] ([PKind], [PKey], [PValue], [PDesc], [Remark], [EDate], [EUser])
        VALUES (@p1, @p2, @p3, @p4, @p5, @p6, @p7)"#;

    let p_value = serde_json::json!({
        "TaskID": body.TaskID,
        "RecordDate": body.RecordDate,
        "SalesAmt": body.SalesAmt,
        "StkID": stk_id,
    }).to_string();

    let p_kind = "sales_record";
    let p_desc = "销售日报";
    let p_value_str = p_value.as_str();
    let params: Vec<&dyn tiberius::ToSql> = vec![
        &p_kind, &param_id, &p_value_str, &p_desc, &remark, &now, &claims.user_code,
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
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);

    let mut base_query = r#"SELECT * FROM [tSys_Parameters]
        WHERE [PKind] = 'sales_record' AND [EUser] = @p1"#.to_string();
    let mut query_params: Vec<Option<String>> = vec![Some(claims.user_code.clone())];
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
            pidx += 1;
            query_params.push(Some(end.clone()));
        }
    }

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, None, None);
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();

    let mut total: i32 = 0;
    let count_stream = conn.query(&count_sql, &param_refs).await?;
    if let Some(row) = count_stream.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }

    let data_stream = conn.query(&paginated_sql, &param_refs).await?;
    let rows: Vec<Row> = data_stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok_paginated(data, total as u64, page, page_size)))
}
