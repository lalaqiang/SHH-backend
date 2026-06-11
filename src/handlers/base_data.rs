use axum::{
    extract::{Query, State},
    Json,
};
use serde::Deserialize;
use tiberius::Row;
use tiberius::ColumnType;
use crate::config::Config;
use crate::db::get_pool;
use crate::utils::{ApiResponse, build_pagination_sql, build_pagination_sql_with_sort};

#[derive(Deserialize)]
pub struct PaginationParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
    pub sort_prop: Option<String>,
    pub sort_order: Option<String>,
    pub include_deleted: Option<bool>,
}

#[derive(Deserialize)]
pub struct TableDataParams {
    pub table_name: String,
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
}

#[derive(serde::Serialize, Clone)]
pub struct TableInfo {
    #[serde(rename = "TABLE_NAME")]
    pub table_name: String,
}

pub fn row_to_json(row: &Row) -> serde_json::Value {
    let columns = row.columns();
    let mut map = serde_json::Map::new();
    for col in columns {
        let name = col.name().to_string();
        let val = try_get_value(row, &name);
        map.insert(name, val);
    }
    serde_json::Value::Object(map)
}

pub fn try_get_value(row: &Row, col_name: &str) -> serde_json::Value {
    // 优先按列类型精确匹配，避免 decimal 被错误解析
    if let Some(col) = row.columns().iter().find(|c| c.name() == col_name) {
        match col.column_type() {
            ColumnType::Decimaln | ColumnType::Numericn | ColumnType::Money | ColumnType::Money4 => {
                if let Ok(Some(n)) = row.try_get::<tiberius::numeric::Numeric, _>(col_name) {
                    let scale = n.scale() as i32;
                    let v = n.value() as f64 / 10f64.powi(scale);
                    return serde_json::Value::Number(
                        serde_json::Number::from_f64(v).unwrap_or(serde_json::Number::from(0)),
                    );
                }
                if let Ok(Some(v)) = row.try_get::<f64, _>(col_name) {
                    return serde_json::Value::Number(
                        serde_json::Number::from_f64(v).unwrap_or(serde_json::Number::from(0)),
                    );
                }
                return serde_json::Value::Null;
            }
            ColumnType::Int4 => {
                if let Ok(Some(v)) = row.try_get::<i32, _>(col_name) {
                    return serde_json::Value::Number(serde_json::Number::from(v));
                }
            }
            ColumnType::Int8 => {
                if let Ok(Some(v)) = row.try_get::<i64, _>(col_name) {
                    return serde_json::Value::Number(serde_json::Number::from(v));
                }
            }
            ColumnType::Int2 => {
                if let Ok(Some(v)) = row.try_get::<i16, _>(col_name) {
                    return serde_json::Value::Number(serde_json::Number::from(v as i64));
                }
            }
            ColumnType::Int1 => {
                if let Ok(Some(v)) = row.try_get::<u8, _>(col_name) {
                    return serde_json::Value::Number(serde_json::Number::from(v as i64));
                }
            }
            ColumnType::Float4 => {
                if let Ok(Some(v)) = row.try_get::<f32, _>(col_name) {
                    return serde_json::Value::Number(
                        serde_json::Number::from_f64(v as f64).unwrap_or(serde_json::Number::from(0)),
                    );
                }
            }
            ColumnType::Float8 | ColumnType::Floatn => {
                if let Ok(Some(v)) = row.try_get::<f64, _>(col_name) {
                    return serde_json::Value::Number(
                        serde_json::Number::from_f64(v).unwrap_or(serde_json::Number::from(0)),
                    );
                }
            }
            ColumnType::Bit | ColumnType::Bitn => {
                if let Ok(Some(v)) = row.try_get::<bool, _>(col_name) {
                    return serde_json::Value::Bool(v);
                }
            }
            ColumnType::Datetime | ColumnType::Datetime2 | ColumnType::Datetime4 | ColumnType::Datetimen => {
                if let Ok(Some(v)) = row.try_get::<chrono::NaiveDateTime, _>(col_name) {
                    return serde_json::Value::String(v.format("%Y-%m-%d %H:%M:%S").to_string());
                }
            }
            ColumnType::Daten => {
                if let Ok(Some(v)) = row.try_get::<chrono::NaiveDate, _>(col_name) {
                    return serde_json::Value::String(v.format("%Y-%m-%d").to_string());
                }
            }
            ColumnType::Guid => {
                if let Ok(Some(v)) = row.try_get::<uuid::Uuid, _>(col_name) {
                    return serde_json::Value::String(v.to_string());
                }
            }
            ColumnType::Image => {
                if let Ok(Some(v)) = row.try_get::<&[u8], _>(col_name) {
                    return serde_json::Value::String(String::from_utf8_lossy(v).to_string());
                }
            }
            _ => {}
        }
    }

    // 兜底：按常用类型依次尝试
    if let Ok(Some(v)) = row.try_get::<&str, _>(col_name) {
        return serde_json::Value::String(v.to_string());
    }
    if let Ok(Some(n)) = row.try_get::<tiberius::numeric::Numeric, _>(col_name) {
        let scale = n.scale() as i32;
        let v = n.value() as f64 / 10f64.powi(scale);
        return serde_json::Value::Number(
            serde_json::Number::from_f64(v).unwrap_or(serde_json::Number::from(0)),
        );
    }
    if let Ok(Some(v)) = row.try_get::<i32, _>(col_name) {
        return serde_json::Value::Number(serde_json::Number::from(v));
    }
    if let Ok(Some(v)) = row.try_get::<i64, _>(col_name) {
        return serde_json::Value::Number(serde_json::Number::from(v));
    }
    if let Ok(Some(v)) = row.try_get::<f32, _>(col_name) {
        return serde_json::Value::Number(
            serde_json::Number::from_f64(v as f64).unwrap_or(serde_json::Number::from(0)),
        );
    }
    if let Ok(Some(v)) = row.try_get::<f64, _>(col_name) {
        return serde_json::Value::Number(
            serde_json::Number::from_f64(v).unwrap_or(serde_json::Number::from(0)),
        );
    }
    if let Ok(Some(v)) = row.try_get::<bool, _>(col_name) {
        return serde_json::Value::Bool(v);
    }
    if let Ok(Some(v)) = row.try_get::<chrono::NaiveDateTime, _>(col_name) {
        return serde_json::Value::String(v.format("%Y-%m-%d %H:%M:%S").to_string());
    }
    if let Ok(Some(v)) = row.try_get::<chrono::NaiveDate, _>(col_name) {
        return serde_json::Value::String(v.format("%Y-%m-%d").to_string());
    }
    if let Ok(Some(v)) = row.try_get::<uuid::Uuid, _>(col_name) {
        return serde_json::Value::String(v.to_string());
    }
    if let Ok(Some(v)) = row.try_get::<&[u8], _>(col_name) {
        let hex: Vec<String> = v.iter().map(|b| format!("{:02X}", b)).collect();
        return serde_json::Value::String(hex.join(""));
    }
    serde_json::Value::Null
}

pub async fn get_tables(
    State(_config): State<Config>,
) -> Json<ApiResponse<Vec<TableInfo>>> {
    let mut conn = match get_pool().get().await {
        Ok(conn) => conn,
        Err(e) => return Json(ApiResponse::err(&format!("获取数据库连接失败: {}", e))),
    };

    let stream = match conn.query(
        "SELECT TABLE_NAME FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_TYPE = 'BASE TABLE' ORDER BY TABLE_NAME",
        &[],
    ).await {
        Ok(stream) => stream,
        Err(e) => return Json(ApiResponse::err(&format!("查询表列表失败: {}", e))),
    };

    let rows: Vec<Row> = match stream.into_first_result().await {
        Ok(rows) => rows,
        Err(e) => return Json(ApiResponse::err(&format!("获取查询结果失败: {}", e))),
    };
    let tables: Vec<TableInfo> = rows
        .iter()
        .map(|row| TableInfo { table_name: row.get::<&str, _>("TABLE_NAME").unwrap_or("").to_string() })
        .collect();

    Json(ApiResponse::ok(tables))
}

pub async fn get_table_data(
    State(_config): State<Config>,
    Json(params): Json<TableDataParams>,
) -> Json<ApiResponse<Vec<serde_json::Value>>> {
    let mut conn = match get_pool().get().await {
        Ok(conn) => conn,
        Err(e) => return Json(ApiResponse::err(&format!("获取数据库连接失败: {}", e))),
    };

    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);

    let base_query = format!("SELECT * FROM [{}]", params.table_name);

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql(&base_query, page, page_size);

    let mut total: i32 = 0;
    let count_stream = match conn.query(&count_sql, &[]).await {
        Ok(stream) => stream,
        Err(e) => return Json(ApiResponse::err(&format!("查询总数失败: {}", e))),
    };
    match count_stream.into_row().await {
        Ok(Some(row)) => { total = row.get::<i32, _>("cnt").unwrap_or(0); }
        Ok(None) => {}
        Err(e) => return Json(ApiResponse::err(&format!("获取总数行失败: {}", e))),
    }

    let data_stream = match conn.query(&paginated_sql, &[]).await {
        Ok(stream) => stream,
        Err(e) => return Json(ApiResponse::err(&format!("查询数据失败: {}", e))),
    };
    let rows: Vec<Row> = match data_stream.into_first_result().await {
        Ok(rows) => rows,
        Err(e) => return Json(ApiResponse::err(&format!("获取数据结果失败: {}", e))),
    };
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Json(ApiResponse::ok_paginated(data, total as u64, page, page_size))
}

pub async fn delete_supplier(
    State(_config): State<Config>,
    Json(body): Json<DeleteRequest>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(conn) => conn,
        Err(e) => return Json(ApiResponse::err(&format!("获取数据库连接失败: {}", e))),
    };

    if body.ids.is_empty() {
        return Json(ApiResponse::err("请选择要操作的记录"));
    }

    let physical_delete = body.permanent.unwrap_or(false);

    if physical_delete {
        // 物理删除前：引用检查
        let references: Vec<(&str, &str, &str)> = vec![
            ("tBas_Goods", "SuppID", "商品资料"),
            ("tPur_Order", "SuppID", "采购订单"),
            ("tPur_Quote", "SuppID", "采购报价"),
            ("tPur_AdjPrice", "SuppID", "采购调价"),
            ("tFin_Payable", "SuppID", "应付款"),
            ("tFin_Payment", "SuppID", "付款单"),
            ("tStk_IO", "SuppID", "出入库单"),
        ];

        let mut ref_hits: Vec<String> = Vec::new();
        for (ref_table, ref_col, ref_label) in &references {
            let in_list = body.ids.iter()
                .map(|s| format!("'{}'", s.replace('\'', "''")))
                .collect::<Vec<_>>()
                .join(",");
            let total_sql = format!(
                "SELECT COUNT(*) AS cnt FROM [{}] WHERE [{}] IN ({})",
                ref_table, ref_col, in_list
            );
            match conn.query(&total_sql, &[]).await {
                Ok(mut stream) => {
                    if let Ok(Some(row)) = stream.into_row().await {
                        let v = try_get_value(&row, "cnt");
                        let cnt = match v {
                            serde_json::Value::Number(n) => n.as_i64().unwrap_or(0),
                            serde_json::Value::String(s) => s.parse::<i64>().unwrap_or(0),
                            _ => 0,
                        };
                        if cnt > 0 {
                            ref_hits.push(format!("  · {} ({}): {} 条", ref_label, ref_table, cnt));
                        }
                    }
                }
                Err(_) => {}
            }
        }
        if !ref_hits.is_empty() {
            return Json(ApiResponse::err(&format!(
                "该供应商已被以下数据引用，无法彻底删除：\n{}\n请先清理引用数据后再试。",
                ref_hits.join("\n")
            )));
        }

        // 引用检查通过，执行物理删除
        for id in &body.ids {
            let sql = "DELETE FROM tBas_Supp WHERE SuppID = @p1";
            let id_str = id.as_str();
            if let Err(e) = conn.execute(sql, &[&id_str]).await {
                return Json(ApiResponse::err(&format!("彻底删除供应商失败: {}", e)));
            }
        }
        return Json(ApiResponse::msg(&format!("成功彻底删除 {} 条供应商", body.ids.len())));
    }

    // 软删除
    for id in &body.ids {
        let sql = "UPDATE tBas_Supp SET State = 'D' WHERE SuppID = @p1";
        let id_str = id.as_str();
        if let Err(e) = conn.execute(sql, &[&id_str]).await {
            return Json(ApiResponse::err(&format!("作废供应商失败: {}", e)));
        }
    }

    Json(ApiResponse::msg(&format!("成功作废{}条供应商", body.ids.len())))
}

pub async fn delete_warehouse(
    State(_config): State<Config>,
    Json(body): Json<DeleteRequest>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(conn) => conn,
        Err(e) => return Json(ApiResponse::err(&format!("获取数据库连接失败: {}", e))),
    };

    if body.ids.is_empty() {
        return Json(ApiResponse::err("请选择要操作的记录"));
    }

    let physical_delete = body.permanent.unwrap_or(false);

    if physical_delete {
        // 物理删除前：引用检查
        let references: Vec<(&str, &str, &str)> = vec![
            ("tStk_Stock", "StkID", "商品库存余额"),
            ("tStk_Qty", "StkID", "商品即时库存"),
            ("tStk_IO", "StkID", "出入库单"),
            ("tStk_Move", "FromStkID", "调拨单(发出)"),
            ("tStk_Move", "ToStkID", "调拨单(接收)"),
            ("tBas_Emp", "StkID", "员工"),
            ("tBas_Goods", "StkID", "商品资料"),
        ];

        let mut ref_hits: Vec<String> = Vec::new();
        for (ref_table, ref_col, ref_label) in &references {
            let in_list = body.ids.iter()
                .map(|s| format!("'{}'", s.replace('\'', "''")))
                .collect::<Vec<_>>()
                .join(",");
            let total_sql = format!(
                "SELECT COUNT(*) AS cnt FROM [{}] WHERE [{}] IN ({})",
                ref_table, ref_col, in_list
            );
            match conn.query(&total_sql, &[]).await {
                Ok(mut stream) => {
                    if let Ok(Some(row)) = stream.into_row().await {
                        let v = try_get_value(&row, "cnt");
                        let cnt = match v {
                            serde_json::Value::Number(n) => n.as_i64().unwrap_or(0),
                            serde_json::Value::String(s) => s.parse::<i64>().unwrap_or(0),
                            _ => 0,
                        };
                        if cnt > 0 {
                            ref_hits.push(format!("  · {} ({}): {} 条", ref_label, ref_table, cnt));
                        }
                    }
                }
                Err(_) => {}
            }
        }
        if !ref_hits.is_empty() {
            return Json(ApiResponse::err(&format!(
                "该仓库已被以下数据引用，无法彻底删除：\n{}\n请先清理引用数据后再试。",
                ref_hits.join("\n")
            )));
        }

        // 引用检查通过，执行物理删除
        for id in &body.ids {
            let sql = "DELETE FROM tBas_Stock WHERE StkID = @p1";
            let id_str = id.as_str();
            if let Err(e) = conn.execute(sql, &[&id_str]).await {
                return Json(ApiResponse::err(&format!("彻底删除仓库失败: {}", e)));
            }
        }
        return Json(ApiResponse::msg(&format!("成功彻底删除 {} 条仓库", body.ids.len())));
    }

    // 软删除
    for id in &body.ids {
        let sql = "UPDATE tBas_Stock SET Used = 'N' WHERE StkID = @p1";
        let id_str = id.as_str();
        if let Err(e) = conn.execute(sql, &[&id_str]).await {
            return Json(ApiResponse::err(&format!("停用仓库失败: {}", e)));
        }
    }

    Json(ApiResponse::msg(&format!("成功停用{}条仓库", body.ids.len())))
}

#[derive(Deserialize)]
pub struct CustomerCreateRequest {
    pub CustNo: String,
    pub CustName: String,
    pub CustTypeID: Option<String>,
    pub AreaID: Option<String>,
    pub LinkMan: Option<String>,
    pub Tel: Option<String>,
    pub Addr: Option<String>,
    pub State: Option<String>,
}

pub async fn create_customer(
    State(_config): State<Config>,
    Json(body): Json<CustomerCreateRequest>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(conn) => conn,
        Err(e) => return Json(ApiResponse::err(&format!("获取数据库连接失败: {}", e))),
    };

    let custid = format!("{}", uuid::Uuid::new_v4());
    let sql = r#"INSERT INTO tBas_Cust (CustID, CustNo, CustName, CustTypeID, AreaID, LinkMan, Tel, Addr, State)
              VALUES (@p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9)"#;

    let state = body.State.as_deref().unwrap_or("S");
    let custtypeid = body.CustTypeID.as_deref().unwrap_or("");
    let areaid = body.AreaID.as_deref().unwrap_or("");
    let linkman = body.LinkMan.as_deref().unwrap_or("");
    let tel = body.Tel.as_deref().unwrap_or("");
    let addr = body.Addr.as_deref().unwrap_or("");

    if let Err(e) = conn.execute(sql, &[
        &custid, &body.CustNo, &body.CustName, &custtypeid, &areaid,
        &linkman, &tel, &addr, &state,
    ]).await {
        return Json(ApiResponse::err(&format!("新增客户失败: {}", e)));
    }

    Json(ApiResponse::msg("客户新增成功"))
}

#[derive(Deserialize)]
pub struct CustomerUpdateRequest {
    pub CustID: String,
    pub CustNo: String,
    pub CustName: String,
    pub CustTypeID: Option<String>,
    pub AreaID: Option<String>,
    pub LinkMan: Option<String>,
    pub Tel: Option<String>,
    pub Addr: Option<String>,
    pub State: Option<String>,
}

pub async fn update_customer(
    State(_config): State<Config>,
    Json(body): Json<CustomerUpdateRequest>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(conn) => conn,
        Err(e) => return Json(ApiResponse::err(&format!("获取数据库连接失败: {}", e))),
    };

    let sql = r#"UPDATE tBas_Cust SET CustNo=@p1, CustName=@p2, CustTypeID=@p3, AreaID=@p4,
              LinkMan=@p5, Tel=@p6, Addr=@p7, State=@p8 WHERE CustID=@p9"#;

    let state = body.State.as_deref().unwrap_or("S");
    let custtypeid = body.CustTypeID.as_deref().unwrap_or("");
    let areaid = body.AreaID.as_deref().unwrap_or("");
    let linkman = body.LinkMan.as_deref().unwrap_or("");
    let tel = body.Tel.as_deref().unwrap_or("");
    let addr = body.Addr.as_deref().unwrap_or("");

    if let Err(e) = conn.execute(sql, &[
        &body.CustNo, &body.CustName, &custtypeid, &areaid,
        &linkman, &tel, &addr, &state, &body.CustID,
    ]).await {
        return Json(ApiResponse::err(&format!("更新客户失败: {}", e)));
    }

    Json(ApiResponse::msg("客户更新成功"))
}

pub async fn delete_customer(
    State(_config): State<Config>,
    Json(body): Json<DeleteRequest>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(conn) => conn,
        Err(e) => return Json(ApiResponse::err(&format!("获取数据库连接失败: {}", e))),
    };

    if body.ids.is_empty() {
        return Json(ApiResponse::err("请选择要操作的记录"));
    }

    let physical_delete = body.permanent.unwrap_or(false);

    if physical_delete {
        // 物理删除前：引用检查
        let references: Vec<(&str, &str, &str)> = vec![
            ("tSal_Inv", "CustID", "销售发票"),
            ("tSal_InvDetail", "CustID", "销售发票明细"),
            ("tSal_Order", "CustID", "销售订单"),
            ("tSal_Quote", "CustID", "销售报价"),
            ("tArd_AR", "CustID", "应收款"),
            ("tAcc_PayIn", "CustID", "收款单"),
            ("tOnline_Order", "CustID", "线上订单"),
            ("tBas_Goods", "CustID", "商品资料"),
        ];

        let mut ref_hits: Vec<String> = Vec::new();
        for (ref_table, ref_col, ref_label) in &references {
            let in_list = body.ids.iter()
                .map(|s| format!("'{}'", s.replace('\'', "''")))
                .collect::<Vec<_>>()
                .join(",");
            let total_sql = format!(
                "SELECT COUNT(*) AS cnt FROM [{}] WHERE [{}] IN ({})",
                ref_table, ref_col, in_list
            );
            match conn.query(&total_sql, &[]).await {
                Ok(mut stream) => {
                    if let Ok(Some(row)) = stream.into_row().await {
                        let v = try_get_value(&row, "cnt");
                        let cnt = match v {
                            serde_json::Value::Number(n) => n.as_i64().unwrap_or(0),
                            serde_json::Value::String(s) => s.parse::<i64>().unwrap_or(0),
                            _ => 0,
                        };
                        if cnt > 0 {
                            ref_hits.push(format!("  · {} ({}): {} 条", ref_label, ref_table, cnt));
                        }
                    }
                }
                Err(_) => {}
            }
        }
        if !ref_hits.is_empty() {
            return Json(ApiResponse::err(&format!(
                "该客户已被以下数据引用，无法彻底删除：\n{}\n请先清理引用数据后再试。",
                ref_hits.join("\n")
            )));
        }

        // 引用检查通过，执行物理删除
        for id in &body.ids {
            let sql = "DELETE FROM tBas_Cust WHERE CustID = @p1";
            let id_str = id.as_str();
            if let Err(e) = conn.execute(sql, &[&id_str]).await {
                return Json(ApiResponse::err(&format!("彻底删除客户失败: {}", e)));
            }
        }
        return Json(ApiResponse::msg(&format!("成功彻底删除 {} 条客户", body.ids.len())));
    }

    // 软删除
    for id in &body.ids {
        let sql = "UPDATE tBas_Cust SET State = 'D' WHERE CustID = @p1";
        let id_str = id.as_str();
        if let Err(e) = conn.execute(sql, &[&id_str]).await {
            return Json(ApiResponse::err(&format!("作废客户失败: {}", e)));
        }
    }

    Json(ApiResponse::msg(&format!("成功作废{}条客户", body.ids.len())))
}

pub async fn get_base_versions(
    State(_config): State<Config>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(conn) => conn,
        Err(e) => return Json(ApiResponse::err(&format!("获取数据库连接失败: {}", e))),
    };

    let mut goods_version: String = String::new();
    let sql = "SELECT MAX(CONVERT(varchar(19), EDate, 120)) as ver FROM tBas_Goods WHERE State <> 'D'";
    match conn.query(sql, &[]).await {
        Ok(stream) => {
            if let Ok(Some(row)) = stream.into_row().await {
                goods_version = row.get::<&str, _>("ver").unwrap_or("").to_string();
            }
        }
        Err(_) => {}
    }

    let mut cust_version: String = String::new();
    let sql = "SELECT MAX(CONVERT(varchar(19), EDate, 120)) as ver FROM tBas_Cust WHERE State <> 'D'";
    match conn.query(sql, &[]).await {
        Ok(stream) => {
            if let Ok(Some(row)) = stream.into_row().await {
                cust_version = row.get::<&str, _>("ver").unwrap_or("").to_string();
            }
        }
        Err(_) => {}
    }

    let mut supp_version: String = String::new();
    let sql = "SELECT MAX(CONVERT(varchar(19), EDate, 120)) as ver FROM tBas_Supp WHERE State <> 'D'";
    match conn.query(sql, &[]).await {
        Ok(stream) => {
            if let Ok(Some(row)) = stream.into_row().await {
                supp_version = row.get::<&str, _>("ver").unwrap_or("").to_string();
            }
        }
        Err(_) => {}
    }

    let data = serde_json::json!({
        "goodsVersion": goods_version,
        "custVersion": cust_version,
        "suppVersion": supp_version
    });

    Json(ApiResponse::ok(data))
}

pub async fn get_suppliers(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Json<ApiResponse<Vec<serde_json::Value>>> {
    let mut conn = match get_pool().get().await {
        Ok(conn) => conn,
        Err(e) => return Json(ApiResponse::err(&format!("获取数据库连接失败: {}", e))),
    };
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);

    let mut base_query = "SELECT * FROM tBas_Supp WHERE State <> 'D'".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(&format!(" AND (SuppNo LIKE @p{} OR SuppName LIKE @p{})", pidx, pidx + 1));
            pidx += 2;
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
        }
    }

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, params.sort_prop.as_deref(), params.sort_order.as_deref());
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();

    let mut total: i32 = 0;
    let count_stream = match conn.query(&count_sql, &param_refs).await {
        Ok(stream) => stream,
        Err(e) => return Json(ApiResponse::err(&format!("查询供应商总数失败: {}", e))),
    };
    match count_stream.into_row().await {
        Ok(Some(row)) => { total = row.get::<i32, _>("cnt").unwrap_or(0); }
        Ok(None) => {}
        Err(e) => return Json(ApiResponse::err(&format!("获取供应商总数行失败: {}", e))),
    }

    let data_stream = match conn.query(&paginated_sql, &param_refs).await {
        Ok(stream) => stream,
        Err(e) => return Json(ApiResponse::err(&format!("查询供应商数据失败: {}", e))),
    };
    let rows: Vec<Row> = match data_stream.into_first_result().await {
        Ok(rows) => rows,
        Err(e) => return Json(ApiResponse::err(&format!("获取供应商数据结果失败: {}", e))),
    };
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Json(ApiResponse::ok_paginated(data, total as u64, page, page_size))
}

#[derive(Deserialize)]
pub struct RetailSearchParams {
    pub keyword: String,
}

pub async fn retail_goods_search(
    State(_config): State<Config>,
    Json(params): Json<RetailSearchParams>,
) -> Json<ApiResponse<Vec<serde_json::Value>>> {
    let mut conn = match get_pool().get().await {
        Ok(conn) => conn,
        Err(e) => return Json(ApiResponse::err(&format!("获取数据库连接失败: {}", e))),
    };

    let kw_pattern = format!("%{}%", params.keyword);
    let sql = "SELECT TOP 50 GDSID, GDSNO, GDSDesc, BarCode, UnitNO, SPrice as Price, 999 as StockQty FROM tBas_Goods WHERE (GDSNO LIKE @p1 OR GDSDesc LIKE @p2 OR BarCode LIKE @p3) AND State IN ('S', '1')";

    let stream = match conn.query(sql, &[&kw_pattern.as_str(), &kw_pattern.as_str(), &kw_pattern.as_str()]).await {
        Ok(stream) => stream,
        Err(e) => return Json(ApiResponse::err(&format!("搜索商品失败: {}", e))),
    };
    let rows: Vec<Row> = match stream.into_first_result().await {
        Ok(rows) => rows,
        Err(e) => return Json(ApiResponse::err(&format!("获取搜索结果失败: {}", e))),
    };
    let data: Vec<serde_json::Value> = rows.iter().map(|row| {
        let mut map = serde_json::Map::new();
        for col in row.columns() {
            map.insert(col.name().to_string(), try_get_value(row, col.name()));
        }
        serde_json::Value::Object(map)
    }).collect();

    Json(ApiResponse::ok(data))
}

#[derive(Deserialize)]
pub struct RetailSettleParams {
    pub items: Vec<RetailSettleItem>,
    pub total_amt: Option<f64>,
}

#[derive(Deserialize)]
pub struct RetailSettleItem {
    pub gdsno: Option<String>,
    pub qty: Option<f64>,
    pub price: Option<f64>,
}

pub async fn retail_sales_settle(
    State(_config): State<Config>,
    Json(params): Json<RetailSettleParams>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(conn) => conn,
        Err(e) => return Json(ApiResponse::err(&format!("获取数据库连接失败: {}", e))),
    };

    let inv_no = format!("RT{}", chrono::Local::now().format("%Y%m%d%H%M%S"));
    let total_amt = params.total_amt.unwrap_or(0.0);

    let inv_sql = "INSERT INTO tSal_Inv (InvNo, InvDate, CustID, TotalAmt, State, EDate, EUser) VALUES (@p1, @p2, @p3, @p4, @p5, @p6, @p7)";
    let now = chrono::Local::now().naive_local();
    if let Err(e) = conn.execute(inv_sql, &[
        &inv_no.as_str(),
        &now,
        &"RETAIL",
        &total_amt,
        &"A",
        &now,
        &"system",
    ]).await {
        return Json(ApiResponse::err(&format!("插入销售单失败: {}", e)));
    }

    for (i, item) in params.items.iter().enumerate() {
        let gdsno = item.gdsno.as_deref().unwrap_or("");
        let qty = item.qty.unwrap_or(0.0);
        let price = item.price.unwrap_or(0.0);
        let amt = qty * price;
        let line_no = format!("{}", i + 1);

        let detail_sql = "INSERT INTO tSal_InvDetail (InvNo, LineNo, GDSNO, Qty, Price, Amt) VALUES (@p1, @p2, @p3, @p4, @p5, @p6)";
        if let Err(e) = conn.execute(detail_sql, &[
            &inv_no.as_str(),
            &line_no.as_str(),
            &gdsno,
            &qty,
            &price,
            &amt,
        ]).await {
            return Json(ApiResponse::err(&format!("插入销售明细失败: {}", e)));
        }
    }

    Json(ApiResponse::msg(&format!("结算成功，共{}件商品", params.items.len())))
}

#[derive(Deserialize)]
pub struct GoodsCreateRequest {
    pub GDSNO: String,
    pub GDSDesc: String,
    pub GDSSpec: Option<String>,
    pub BarCode: Option<String>,
    pub UnitNO: Option<String>,
    pub AInPrice: Option<f64>,
    pub SPrice: Option<f64>,
    pub State: Option<String>,
}

#[derive(Deserialize)]
pub struct GoodsUpdateRequest {
    pub GDSID: String,
    pub GDSNO: String,
    pub GDSDesc: String,
    pub GDSSpec: Option<String>,
    pub BarCode: Option<String>,
    pub UnitNO: Option<String>,
    pub AInPrice: Option<f64>,
    pub SPrice: Option<f64>,
    pub State: Option<String>,
}

pub async fn create_goods(
    State(_config): State<Config>,
    Json(body): Json<GoodsCreateRequest>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(conn) => conn,
        Err(e) => return Json(ApiResponse::err(&format!("获取数据库连接失败: {}", e))),
    };

    let gdsid = format!("{}", uuid::Uuid::new_v4());
    let sql = r#"INSERT INTO tBas_Goods (GDSID, GDSNO, GDSDesc, GDSSpec, BarCode, UnitNO, AInPrice, SPrice, State)
              VALUES (@p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9)"#;

    let gdsno = body.GDSNO.as_str();
    let gdsdesc = body.GDSDesc.as_str();
    let gdsspec = body.GDSSpec.as_deref().unwrap_or("");
    let barcode = body.BarCode.as_deref().unwrap_or("");
    let unitno = body.UnitNO.as_deref().unwrap_or("");
    let ainprice = body.AInPrice.unwrap_or(0.0);
    let sprice = body.SPrice.unwrap_or(0.0);
    let state = body.State.as_deref().unwrap_or("S");

    if let Err(e) = conn.execute(sql, &[
        &gdsid,
        &gdsno,
        &gdsdesc,
        &gdsspec,
        &barcode,
        &unitno,
        &ainprice,
        &sprice,
        &state,
    ]).await {
        return Json(ApiResponse::err(&format!("新增商品失败: {}", e)));
    }

    Json(ApiResponse::msg("商品新增成功"))
}

pub async fn update_goods(
    State(_config): State<Config>,
    Json(body): Json<GoodsUpdateRequest>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(conn) => conn,
        Err(e) => return Json(ApiResponse::err(&format!("获取数据库连接失败: {}", e))),
    };

    let sql = r#"UPDATE tBas_Goods SET GDSNO=@p1, GDSDesc=@p2, GDSSpec=@p3, BarCode=@p4,
              UnitNO=@p5, AInPrice=@p6, SPrice=@p7, State=@p8 WHERE GDSID=@p9"#;

    let gdsno = body.GDSNO.as_str();
    let gdsdesc = body.GDSDesc.as_str();
    let gdsspec = body.GDSSpec.as_deref().unwrap_or("");
    let barcode = body.BarCode.as_deref().unwrap_or("");
    let unitno = body.UnitNO.as_deref().unwrap_or("");
    let ainprice = body.AInPrice.unwrap_or(0.0);
    let sprice = body.SPrice.unwrap_or(0.0);
    let state = body.State.as_deref().unwrap_or("1");
    let gdsid = body.GDSID.as_str();

    if let Err(e) = conn.execute(sql, &[
        &gdsno,
        &gdsdesc,
        &gdsspec,
        &barcode,
        &unitno,
        &ainprice,
        &sprice,
        &state,
        &gdsid,
    ]).await {
        return Json(ApiResponse::err(&format!("更新商品失败: {}", e)));
    }

    Json(ApiResponse::msg("商品更新成功"))
}

#[derive(Deserialize)]
pub struct DeleteRequest {
    pub ids: Vec<String>,
    /// true = 物理删除（DELETE FROM），false = 软删除（更新状态字段）
    pub permanent: Option<bool>,
}

pub async fn delete_goods(
    State(_config): State<Config>,
    Json(body): Json<DeleteRequest>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(conn) => conn,
        Err(e) => return Json(ApiResponse::err(&format!("获取数据库连接失败: {}", e))),
    };

    if body.ids.is_empty() {
        return Json(ApiResponse::err("请选择要操作的记录"));
    }

    let physical_delete = body.permanent.unwrap_or(false);

    if physical_delete {
        // 物理删除前：引用检查
        let references: Vec<(&str, &str, &str)> = vec![
            ("tStk_Stock", "GDSID", "商品库存余额"),
            ("tStk_Qty", "GDSID", "商品即时库存"),
            ("tStk_IODetail", "GDSID", "出入库明细"),
            ("tStk_MoveDetail", "GDSID", "调拨明细"),
            ("tSal_InvDetail", "GDSID", "销售发票明细"),
            ("tPur_OrderDetail", "GDSID", "采购订单明细"),
            ("tOnline_Goods", "GDSID", "线上商城商品"),
            ("tOnline_OrderDetail", "GDSID", "线上订单明细"),
        ];

        let mut ref_hits: Vec<String> = Vec::new();
        for (ref_table, ref_col, ref_label) in &references {
            let in_list = body.ids.iter()
                .map(|s| format!("'{}'", s.replace('\'', "''")))
                .collect::<Vec<_>>()
                .join(",");
            let total_sql = format!(
                "SELECT COUNT(*) AS cnt FROM [{}] WHERE [{}] IN ({})",
                ref_table, ref_col, in_list
            );
            match conn.query(&total_sql, &[]).await {
                Ok(mut stream) => {
                    if let Ok(Some(row)) = stream.into_row().await {
                        let v = try_get_value(&row, "cnt");
                        let cnt = match v {
                            serde_json::Value::Number(n) => n.as_i64().unwrap_or(0),
                            serde_json::Value::String(s) => s.parse::<i64>().unwrap_or(0),
                            _ => 0,
                        };
                        if cnt > 0 {
                            ref_hits.push(format!("  · {} ({}): {} 条", ref_label, ref_table, cnt));
                        }
                    }
                }
                Err(_) => {}
            }
        }
        if !ref_hits.is_empty() {
            return Json(ApiResponse::err(&format!(
                "该商品已被以下数据引用，无法彻底删除：\n{}\n请先清理引用数据后再试。",
                ref_hits.join("\n")
            )));
        }

        // 引用检查通过，执行物理删除
        for id in &body.ids {
            let sql = "DELETE FROM tBas_Goods WHERE GDSID = @p1";
            let id_str = id.as_str();
            if let Err(e) = conn.execute(sql, &[&id_str]).await {
                return Json(ApiResponse::err(&format!("彻底删除商品失败: {}", e)));
            }
        }
        return Json(ApiResponse::msg(&format!("成功彻底删除 {} 条商品", body.ids.len())));
    }

    // 软删除
    for id in &body.ids {
        let sql = "UPDATE tBas_Goods SET State = 'D' WHERE GDSID = @p1";
        let id_str = id.as_str();
        if let Err(e) = conn.execute(sql, &[&id_str]).await {
            return Json(ApiResponse::err(&format!("作废商品失败: {}", e)));
        }
    }

    Json(ApiResponse::msg(&format!("成功作废{}条商品", body.ids.len())))
}

pub async fn get_customers(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Json<ApiResponse<Vec<serde_json::Value>>> {
    let mut conn = match get_pool().get().await {
        Ok(conn) => conn,
        Err(e) => return Json(ApiResponse::err(&format!("获取数据库连接失败: {}", e))),
    };
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);

    let mut base_query = "SELECT * FROM tBas_Cust WHERE State <> 'D'".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(&format!(" AND (CustNo LIKE @p{} OR CustName LIKE @p{})", pidx, pidx + 1));
            pidx += 2;
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
        }
    }

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, params.sort_prop.as_deref(), params.sort_order.as_deref());
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();

    let mut total: i32 = 0;
    let count_stream = match conn.query(&count_sql, &param_refs).await {
        Ok(stream) => stream,
        Err(e) => return Json(ApiResponse::err(&format!("查询客户总数失败: {}", e))),
    };
    match count_stream.into_row().await {
        Ok(Some(row)) => { total = row.get::<i32, _>("cnt").unwrap_or(0); }
        Ok(None) => {}
        Err(e) => return Json(ApiResponse::err(&format!("获取客户总数行失败: {}", e))),
    }

    let data_stream = match conn.query(&paginated_sql, &param_refs).await {
        Ok(stream) => stream,
        Err(e) => return Json(ApiResponse::err(&format!("查询客户数据失败: {}", e))),
    };
    let rows: Vec<Row> = match data_stream.into_first_result().await {
        Ok(rows) => rows,
        Err(e) => return Json(ApiResponse::err(&format!("获取客户数据结果失败: {}", e))),
    };
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Json(ApiResponse::ok_paginated(data, total as u64, page, page_size))
}

pub async fn get_goods(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Json<ApiResponse<Vec<serde_json::Value>>> {
    let mut conn = match get_pool().get().await {
        Ok(conn) => conn,
        Err(e) => return Json(ApiResponse::err(&format!("获取数据库连接失败: {}", e))),
    };
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);

    let mut base_query = "SELECT * FROM tBas_Goods WHERE State <> 'D'".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(&format!(" AND (GDSNO LIKE @p{} OR GDSDesc LIKE @p{} OR BarCode LIKE @p{})", pidx, pidx + 1, pidx + 2));
            pidx += 3;
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
        }
    }

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, params.sort_prop.as_deref(), params.sort_order.as_deref());
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();

    let mut total: i32 = 0;
    let count_stream = match conn.query(&count_sql, &param_refs).await {
        Ok(stream) => stream,
        Err(e) => return Json(ApiResponse::err(&format!("查询商品总数失败: {}", e))),
    };
    match count_stream.into_row().await {
        Ok(Some(row)) => { total = row.get::<i32, _>("cnt").unwrap_or(0); }
        Ok(None) => {}
        Err(e) => return Json(ApiResponse::err(&format!("获取商品总数行失败: {}", e))),
    }

    let data_stream = match conn.query(&paginated_sql, &param_refs).await {
        Ok(stream) => stream,
        Err(e) => return Json(ApiResponse::err(&format!("查询商品数据失败: {}", e))),
    };
    let rows: Vec<Row> = match data_stream.into_first_result().await {
        Ok(rows) => rows,
        Err(e) => return Json(ApiResponse::err(&format!("获取商品数据结果失败: {}", e))),
    };
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Json(ApiResponse::ok_paginated(data, total as u64, page, page_size))
}

pub async fn get_dashboard_stats(
    State(_config): State<Config>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(conn) => conn,
        Err(e) => return Json(ApiResponse::err(&format!("获取数据库连接失败: {}", e))),
    };

    let mut goods_count: i32 = 0;
    let stream = match conn.query("SELECT COUNT(*) as cnt FROM tBas_Goods WHERE State <> 'D'", &[]).await {
        Ok(stream) => stream,
        Err(e) => return Json(ApiResponse::err(&format!("查询商品数量失败: {}", e))),
    };
    match stream.into_row().await {
        Ok(Some(row)) => { goods_count = row.get::<i32, _>("cnt").unwrap_or(0); }
        Ok(None) => {}
        Err(e) => return Json(ApiResponse::err(&format!("获取商品数量行失败: {}", e))),
    }

    let mut supplier_count: i32 = 0;
    let stream = match conn.query("SELECT COUNT(*) as cnt FROM tBas_Supp WHERE State <> 'D'", &[]).await {
        Ok(stream) => stream,
        Err(e) => return Json(ApiResponse::err(&format!("查询供应商数量失败: {}", e))),
    };
    match stream.into_row().await {
        Ok(Some(row)) => { supplier_count = row.get::<i32, _>("cnt").unwrap_or(0); }
        Ok(None) => {}
        Err(e) => return Json(ApiResponse::err(&format!("获取供应商数量行失败: {}", e))),
    }

    let mut customer_count: i32 = 0;
    let stream = match conn.query("SELECT COUNT(*) as cnt FROM tBas_Cust WHERE State <> 'D'", &[]).await {
        Ok(stream) => stream,
        Err(e) => return Json(ApiResponse::err(&format!("查询客户数量失败: {}", e))),
    };
    match stream.into_row().await {
        Ok(Some(row)) => { customer_count = row.get::<i32, _>("cnt").unwrap_or(0); }
        Ok(None) => {}
        Err(e) => return Json(ApiResponse::err(&format!("获取客户数量行失败: {}", e))),
    }

    let mut purchase_order_count: i32 = 0;
    match conn.query("SELECT COUNT(*) as cnt FROM tPur_Order WHERE State <> 'D'", &[]).await {
        Ok(stream) => {
            match stream.into_row().await {
                Ok(Some(row)) => { purchase_order_count = row.get::<i32, _>("cnt").unwrap_or(0); }
                Ok(None) => {}
                Err(_) => {}
            }
        }
        Err(_) => {}
    }

    let mut active_goods_count: i32 = 0;
    let stream = match conn.query("SELECT COUNT(*) as cnt FROM tBas_Goods WHERE State='S'", &[]).await {
        Ok(stream) => stream,
        Err(e) => return Json(ApiResponse::err(&format!("查询活跃商品数量失败: {}", e))),
    };
    match stream.into_row().await {
        Ok(Some(row)) => { active_goods_count = row.get::<i32, _>("cnt").unwrap_or(0); }
        Ok(None) => {}
        Err(e) => return Json(ApiResponse::err(&format!("获取活跃商品数量行失败: {}", e))),
    }

    let mut sales_trend = serde_json::json!([]);
    let sales_sql = r#"SELECT TOP 5 CONVERT(varchar(7), InvDate, 120) as month, SUM(TotalAmt) as amount
        FROM tSal_Inv WHERE State <> 'D' AND InvDate >= DATEADD(month, -5, GETDATE())
        GROUP BY CONVERT(varchar(7), InvDate, 120) ORDER BY month"#;
    match conn.query(sales_sql, &[]).await {
        Ok(stream) => {
            match stream.into_first_result().await {
                Ok(rows) => {
                    let items: Vec<serde_json::Value> = rows.iter().map(|row| {
                        serde_json::json!({
                            "month": row.get::<&str, _>("month").unwrap_or(""),
                            "amount": try_get_value(&row, "amount").as_f64().unwrap_or(0.0) as i64
                        })
                    }).collect();
                    if !items.is_empty() {
                        sales_trend = serde_json::json!(items);
                    }
                }
                Err(_) => {}
            }
        }
        Err(_) => {}
    }

    let mut category_stats = serde_json::json!([]);
    let cat_sql = r#"SELECT TOP 6 b.GDSType as name, COUNT(*) as value
        FROM tBas_Goods b WHERE b.State = 'S'
        GROUP BY b.GDSType ORDER BY COUNT(*) DESC"#;
    match conn.query(cat_sql, &[]).await {
        Ok(stream) => {
            match stream.into_first_result().await {
                Ok(rows) => {
                    let items: Vec<serde_json::Value> = rows.iter().map(|row| {
                        serde_json::json!({
                            "name": row.get::<&str, _>("name").unwrap_or("未分类"),
                            "value": row.get::<i32, _>("value").unwrap_or(0)
                        })
                    }).collect();
                    if !items.is_empty() { category_stats = serde_json::json!(items); }
                }
                Err(_) => {}
            }
        }
        Err(_) => {}
    }

    let data = serde_json::json!({
        "goodsCount": goods_count,
        "supplierCount": supplier_count,
        "customerCount": customer_count,
        "purchaseOrderCount": purchase_order_count,
        "activeGoodsCount": active_goods_count,
        "salesTrend": sales_trend,
        "categoryStats": category_stats
    });

    Json(ApiResponse::ok(data))
}

#[derive(Deserialize)]
pub struct StockQueryParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
    pub sort_prop: Option<String>,
    pub sort_order: Option<String>,
    pub warehouse_id: Option<String>,
    pub category_id: Option<String>,
    pub brand_id: Option<String>,
    pub supplier_id: Option<String>,
    /// 是否显示 0 库存（默认 false，只显示有库存）
    pub show_zero: Option<bool>,
}

pub async fn get_inventory_stock(
    State(_config): State<Config>,
    Json(params): Json<StockQueryParams>,
) -> Json<ApiResponse<Vec<serde_json::Value>>> {
    let mut conn = match get_pool().get().await {
        Ok(conn) => conn,
        Err(e) => return Json(ApiResponse::err(&format!("获取数据库连接失败: {}", e))),
    };

    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 5000);
    let show_zero = params.show_zero.unwrap_or(false);

    // 按商品×仓库展开：tStk_Stock LEFT JOIN 商品/仓库/分类/品牌
    let mut base_query = "SELECT \
                          s.[GDSStockID], s.[GDSID], s.[StkID] AS [StkID], \
                          sk.[StkCode], sk.[StkName], \
                          g.[GDSNO], g.[GDSDesc], g.[GDSSpec], g.[BarCode], \
                          g.[UnitNO], g.[AInPrice], g.[BPrice], g.[SPrice], g.[VPrice], g.[CPrice], \
                          g.[BrandID], b.[BrandName], g.[GDSTypeID], gt.[GDSTypeName], \
                          g.[GDSStateNO], g.[State], \
                          g.[SuppID], sp.[SuppName], g.[DeaTypeID], \
                          ISNULL(s.[Qty], 0) AS [Qty], ISNULL(s.[QQty], 0) AS [QQty] \
                          FROM [tStk_Stock] s \
                          INNER JOIN [tBas_Goods] g ON s.[GDSID] = g.[GDSID] \
                          LEFT JOIN [tBas_Stock] sk ON s.[StkID] = sk.[StkID] \
                          LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID] \
                          LEFT JOIN [tBas_GDSType] gt ON g.[GDSTypeID] = gt.[GDSTypeID] \
                          LEFT JOIN [tBas_Supp] sp ON g.[SuppID] = sp.[SuppID] \
                          WHERE g.[State] IN ('S', '1', 'N')".to_string();
    if !show_zero {
        base_query.push_str(" AND ISNULL(s.[Qty], 0) <> 0");
    }
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(&format!(" AND (g.[GDSNO] LIKE @p{} OR g.[GDSDesc] LIKE @p{} OR g.[BarCode] LIKE @p{})", pidx, pidx + 1, pidx + 2));
            pidx += 3;
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
        }
    }
    if let Some(wid) = &params.warehouse_id {
        if !wid.is_empty() {
            base_query.push_str(&format!(" AND s.[StkID] = @p{}", pidx));
            pidx += 1;
            query_params.push(Some(wid.clone()));
        }
    }
    if let Some(cid) = &params.category_id {
        if !cid.is_empty() {
            base_query.push_str(&format!(" AND g.[GDSTypeID] = @p{}", pidx));
            pidx += 1;
            query_params.push(Some(cid.clone()));
        }
    }
    if let Some(bid) = &params.brand_id {
        if !bid.is_empty() {
            base_query.push_str(&format!(" AND g.[BrandID] = @p{}", pidx));
            pidx += 1;
            query_params.push(Some(bid.clone()));
        }
    }
    if let Some(sid) = &params.supplier_id {
        if !sid.is_empty() {
            base_query.push_str(&format!(" AND g.[SuppID] = @p{}", pidx));
            pidx += 1;
            query_params.push(Some(sid.clone()));
        }
    }

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, params.sort_prop.as_deref(), params.sort_order.as_deref());
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();

    let mut total: i32 = 0;
    let count_stream = match conn.query(&count_sql, &param_refs).await {
        Ok(stream) => stream,
        Err(e) => return Json(ApiResponse::err(&format!("查询库存总数失败: {}", e))),
    };
    match count_stream.into_row().await {
        Ok(Some(row)) => { total = row.get::<i32, _>("cnt").unwrap_or(0); }
        Ok(None) => {}
        Err(e) => return Json(ApiResponse::err(&format!("获取库存总数行失败: {}", e))),
    }

    let data_stream = match conn.query(&paginated_sql, &param_refs).await {
        Ok(stream) => stream,
        Err(e) => return Json(ApiResponse::err(&format!("查询库存数据失败: {}", e))),
    };
    let rows: Vec<Row> = match data_stream.into_first_result().await {
        Ok(rows) => rows,
        Err(e) => return Json(ApiResponse::err(&format!("获取库存数据结果失败: {}", e))),
    };
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Json(ApiResponse::ok_paginated(data, total as u64, page, page_size))
}

pub async fn get_stock_summary(
    State(_config): State<Config>,
    Json(params): Json<StockQueryParams>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(conn) => conn,
        Err(e) => return Json(ApiResponse::err(&format!("获取数据库连接失败: {}", e))),
    };

    let mut base_query = "SELECT \
                          COUNT(DISTINCT s.[GDSID]) AS [product_types], \
                          COUNT(*) AS [record_count], \
                          SUM(ISNULL(s.[Qty], 0) * ISNULL(g.[AInPrice], 0)) AS [total_value], \
                          SUM(ISNULL(s.[Qty], 0)) AS [total_quantity] \
                          FROM [tStk_Stock] s \
                          INNER JOIN [tBas_Goods] g ON s.[GDSID] = g.[GDSID] \
                          WHERE g.[State] IN ('S', '1', 'N')".to_string();
    if !params.show_zero.unwrap_or(false) {
        base_query.push_str(" AND ISNULL(s.[Qty], 0) <> 0");
    }
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(&format!(" AND (g.[GDSNO] LIKE @p{} OR g.[GDSDesc] LIKE @p{} OR g.[BarCode] LIKE @p{})", pidx, pidx + 1, pidx + 2));
            pidx += 3;
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
        }
    }
    if let Some(wid) = &params.warehouse_id {
        if !wid.is_empty() {
            base_query.push_str(&format!(" AND s.[StkID] = @p{}", pidx));
            pidx += 1;
            query_params.push(Some(wid.clone()));
        }
    }
    if let Some(cid) = &params.category_id {
        if !cid.is_empty() {
            base_query.push_str(&format!(" AND g.[GDSTypeID] = @p{}", pidx));
            pidx += 1;
            query_params.push(Some(cid.clone()));
        }
    }
    if let Some(bid) = &params.brand_id {
        if !bid.is_empty() {
            base_query.push_str(&format!(" AND g.[BrandID] = @p{}", pidx));
            pidx += 1;
            query_params.push(Some(bid.clone()));
        }
    }
    if let Some(sid) = &params.supplier_id {
        if !sid.is_empty() {
            base_query.push_str(&format!(" AND g.[SuppID] = @p{}", pidx));
            pidx += 1;
            query_params.push(Some(sid.clone()));
        }
    }

    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();
    let stream = match conn.query(&base_query, &param_refs).await {
        Ok(s) => s,
        Err(e) => return Json(ApiResponse::err(&format!("查询库存汇总失败: {}", e))),
    };
    let row = match stream.into_row().await {
        Ok(Some(r)) => r,
        Ok(None) => return Json(ApiResponse::ok(serde_json::json!({ "product_types": 0, "record_count": 0, "total_value": 0.0, "total_quantity": 0 }))),
        Err(e) => return Json(ApiResponse::err(&format!("获取库存汇总行失败: {}", e))),
    };
    let summary = serde_json::json!({
        "product_types": row.get::<i32, _>("product_types").unwrap_or(0),
        "record_count": row.get::<i32, _>("record_count").unwrap_or(0),
        "total_value": try_get_value(&row, "total_value").as_f64().unwrap_or(0.0),
        "total_quantity": try_get_value(&row, "total_quantity").as_f64().unwrap_or(0.0),
    });
    Json(ApiResponse::ok(summary))
}

pub async fn get_sales_analysis(
    State(_config): State<Config>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(conn) => conn,
        Err(e) => return Json(ApiResponse::err(&format!("获取数据库连接失败: {}", e))),
    };

    let mut monthly_sales = serde_json::json!([]);
    let sql = r#"SELECT TOP 6 CONVERT(varchar(7), InvDate, 120) as month,
        SUM(TotalAmt) as sales, SUM(CostAmt) as cost
        FROM tSal_Inv WHERE State <> 'D' AND InvDate >= DATEADD(month, -6, GETDATE())
        GROUP BY CONVERT(varchar(7), InvDate, 120) ORDER BY month"#;
    match conn.query(sql, &[]).await {
        Ok(stream) => {
            match stream.into_first_result().await {
                Ok(rows) => {
                    let items: Vec<serde_json::Value> = rows.iter().map(|row| {
                        let sales = try_get_value(&row, "sales").as_f64().unwrap_or(0.0);
                        let cost = try_get_value(&row, "cost").as_f64().unwrap_or(0.0);
                        serde_json::json!({
                            "month": row.get::<&str, _>("month").unwrap_or(""),
                            "sales": sales as i64,
                            "cost": cost as i64,
                            "profit": (sales - cost) as i64
                        })
                    }).collect();
                    if !items.is_empty() {
                        monthly_sales = serde_json::json!(items);
                    }
                }
                Err(_) => {}
            }
        }
        Err(_) => {}
    }

    let mut top_products = serde_json::json!([]);
    let tp_sql = r#"SELECT TOP 8 d.GDSNO, d.GDSDesc as name, SUM(d.Qty) as quantity, SUM(d.Amt) as sales
        FROM tSal_InvDetail d INNER JOIN tSal_Inv h ON d.InvNo = h.InvNo
        WHERE h.State <> 'D' AND h.InvDate >= DATEADD(month, -6, GETDATE())
        GROUP BY d.GDSNO, d.GDSDesc ORDER BY SUM(d.Amt) DESC"#;
    match conn.query(tp_sql, &[]).await {
        Ok(stream) => {
            match stream.into_first_result().await {
                Ok(rows) => {
                    let items: Vec<serde_json::Value> = rows.iter().map(|row| {
                        serde_json::json!({
                            "name": row.get::<&str, _>("name").unwrap_or(""),
                            "sales": try_get_value(&row, "sales").as_f64().unwrap_or(0.0) as i64,
                            "quantity": try_get_value(&row, "quantity").as_f64().unwrap_or(0.0) as i64
                        })
                    }).collect();
                    if !items.is_empty() { top_products = serde_json::json!(items); }
                }
                Err(_) => {}
            }
        }
        Err(_) => {}
    }

    let mut customer_ranking = serde_json::json!([]);
    let cr_sql = r#"SELECT TOP 10 CustID as name, CustName, SUM(TotalAmt) as amount
        FROM tSal_Inv WHERE State <> 'D' AND InvDate >= DATEADD(month, -6, GETDATE())
        GROUP BY CustID, CustName ORDER BY SUM(TotalAmt) DESC"#;
    match conn.query(cr_sql, &[]).await {
        Ok(stream) => {
            match stream.into_first_result().await {
                Ok(rows) => {
                    let items: Vec<serde_json::Value> = rows.iter().map(|row| {
                        serde_json::json!({
                            "name": row.get::<&str, _>("CustName").unwrap_or(row.get::<&str, _>("name").unwrap_or("")),
                            "amount": try_get_value(&row, "amount").as_f64().unwrap_or(0.0) as i64
                        })
                    }).collect();
                    if !items.is_empty() { customer_ranking = serde_json::json!(items); }
                }
                Err(_) => {}
            }
        }
        Err(_) => {}
    }

    let data = serde_json::json!({
        "monthlySales": monthly_sales,
        "topProducts": top_products,
        "customerRanking": customer_ranking
    });

    Json(ApiResponse::ok(data))
}

pub async fn get_purchase_analysis(
    State(_config): State<Config>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(conn) => conn,
        Err(e) => return Json(ApiResponse::err(&format!("获取数据库连接失败: {}", e))),
    };

    let mut monthly_purchase = serde_json::json!([]);
    let sql = r#"SELECT TOP 6 CONVERT(varchar(7), PoDate, 120) as month,
        SUM(TotalAmt) as amount, COUNT(*) as orders
        FROM tPur_Order WHERE State <> 'D' AND PoDate >= DATEADD(month, -6, GETDATE())
        GROUP BY CONVERT(varchar(7), PoDate, 120) ORDER BY month"#;
    match conn.query(sql, &[]).await {
        Ok(stream) => {
            match stream.into_first_result().await {
                Ok(rows) => {
                    let items: Vec<serde_json::Value> = rows.iter().map(|row| {
                        serde_json::json!({
                            "month": row.get::<&str, _>("month").unwrap_or(""),
                            "amount": try_get_value(&row, "amount").as_f64().unwrap_or(0.0) as i64,
                            "orders": row.get::<i32, _>("orders").unwrap_or(0)
                        })
                    }).collect();
                    if !items.is_empty() {
                        monthly_purchase = serde_json::json!(items);
                    }
                }
                Err(_) => {}
            }
        }
        Err(_) => {}
    }

    let data = serde_json::json!({
        "monthlyPurchase": monthly_purchase
    });

    Json(ApiResponse::ok(data))
}

// ===== P2-3 补全 基础数据 create/update handler =====
// 字段名基于通用 ERP 模式：sqlcmd 核查后可一键调整

#[derive(Deserialize)]
pub struct SupplierCreateRequest {
    pub SuppNo: String,
    pub SuppName: String,
    pub SuppTypeID: Option<String>,
    pub AreaID: Option<String>,
    pub LinkMan: Option<String>,
    pub Tel: Option<String>,
    pub Addr: Option<String>,
    pub State: Option<String>,
}

pub async fn create_supplier(
    State(_config): State<Config>,
    Json(body): Json<SupplierCreateRequest>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("DB: {}", e))),
    };
    if body.SuppNo.is_empty() || body.SuppName.is_empty() {
        return Json(ApiResponse::err("供应商编码和名称不能为空"));
    }
    let suppid = format!("{}", uuid::Uuid::new_v4());
    let sql = r#"INSERT INTO tBas_Supp (SuppID, SuppNo, SuppName, SuppTypeID, AreaID, LinkMan, Tel, Addr, State)
              VALUES (@p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9)"#;
    let state = body.State.as_deref().unwrap_or("S");
    let supptypeid = body.SuppTypeID.as_deref().unwrap_or("");
    let areaid = body.AreaID.as_deref().unwrap_or("");
    let linkman = body.LinkMan.as_deref().unwrap_or("");
    let tel = body.Tel.as_deref().unwrap_or("");
    let addr = body.Addr.as_deref().unwrap_or("");
    if let Err(e) = conn.execute(sql, &[
        &suppid, &body.SuppNo, &body.SuppName, &supptypeid, &areaid,
        &linkman, &tel, &addr, &state,
    ]).await {
        return Json(ApiResponse::err(&format!("新增供应商失败: {}", e)));
    }
    Json(ApiResponse::msg("供应商新增成功"))
}

#[derive(Deserialize)]
pub struct SupplierUpdateRequest {
    pub SuppID: String,
    pub SuppNo: String,
    pub SuppName: String,
    pub SuppTypeID: Option<String>,
    pub AreaID: Option<String>,
    pub LinkMan: Option<String>,
    pub Tel: Option<String>,
    pub Addr: Option<String>,
    pub State: Option<String>,
}

pub async fn update_supplier(
    State(_config): State<Config>,
    Json(body): Json<SupplierUpdateRequest>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("DB: {}", e))),
    };
    let sql = r#"UPDATE tBas_Supp SET SuppNo=@p1, SuppName=@p2, SuppTypeID=@p3, AreaID=@p4,
              LinkMan=@p5, Tel=@p6, Addr=@p7, State=@p8 WHERE SuppID=@p9"#;
    let state = body.State.as_deref().unwrap_or("S");
    let supptypeid = body.SuppTypeID.as_deref().unwrap_or("");
    let areaid = body.AreaID.as_deref().unwrap_or("");
    let linkman = body.LinkMan.as_deref().unwrap_or("");
    let tel = body.Tel.as_deref().unwrap_or("");
    let addr = body.Addr.as_deref().unwrap_or("");
    if let Err(e) = conn.execute(sql, &[
        &body.SuppNo, &body.SuppName, &supptypeid, &areaid,
        &linkman, &tel, &addr, &state, &body.SuppID,
    ]).await {
        return Json(ApiResponse::err(&format!("更新供应商失败: {}", e)));
    }
    Json(ApiResponse::msg("供应商更新成功"))
}

#[derive(Deserialize)]
pub struct WarehouseCreateRequest {
    pub StkCode: String,
    pub StkName: String,
    pub StkType: Option<String>,
    pub StkPID: Option<String>,
    pub NodeKind: Option<String>,
    pub CostCalc: Option<String>,
    pub Used: Option<String>,
}

pub async fn create_warehouse(
    State(_config): State<Config>,
    Json(body): Json<WarehouseCreateRequest>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("DB: {}", e))),
    };
    if body.StkCode.is_empty() || body.StkName.is_empty() {
        return Json(ApiResponse::err("仓库编码和名称不能为空"));
    }
    let stkid = format!("{}", uuid::Uuid::new_v4());
    let sql = r#"INSERT INTO tBas_Stock (StkID, StkCode, StkName, StkType, StkPID, NodeKind, CostCalc, Used)
              VALUES (@p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8)"#;
    let stktype = body.StkType.as_deref().unwrap_or("");
    let stkpid = body.StkPID.as_deref().unwrap_or("00000000-0000-0000-0000-000000000000");
    let nodekind = body.NodeKind.as_deref().unwrap_or("C");
    let costcalc = body.CostCalc.as_deref().unwrap_or("Y");
    let used = body.Used.as_deref().unwrap_or("Y");
    if let Err(e) = conn.execute(sql, &[
        &stkid, &body.StkCode, &body.StkName, &stktype, &stkpid, &nodekind, &costcalc, &used,
    ]).await {
        return Json(ApiResponse::err(&format!("新增仓库失败: {}", e)));
    }
    Json(ApiResponse::msg("仓库新增成功"))
}

#[derive(Deserialize)]
pub struct WarehouseUpdateRequest {
    pub StkID: String,
    pub StkCode: String,
    pub StkName: String,
    pub StkType: Option<String>,
    pub StkPID: Option<String>,
    pub NodeKind: Option<String>,
    pub CostCalc: Option<String>,
    pub Used: Option<String>,
}

pub async fn update_warehouse(
    State(_config): State<Config>,
    Json(body): Json<WarehouseUpdateRequest>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("DB: {}", e))),
    };
    let sql = r#"UPDATE tBas_Stock SET StkCode=@p1, StkName=@p2, StkType=@p3, StkPID=@p4,
              NodeKind=@p5, CostCalc=@p6, Used=@p7 WHERE StkID=@p8"#;
    let stktype = body.StkType.as_deref().unwrap_or("");
    let stkpid = body.StkPID.as_deref().unwrap_or("00000000-0000-0000-0000-000000000000");
    let nodekind = body.NodeKind.as_deref().unwrap_or("C");
    let costcalc = body.CostCalc.as_deref().unwrap_or("Y");
    let used = body.Used.as_deref().unwrap_or("Y");
    if let Err(e) = conn.execute(sql, &[
        &body.StkCode, &body.StkName, &stktype, &stkpid, &nodekind, &costcalc, &used, &body.StkID,
    ]).await {
        return Json(ApiResponse::err(&format!("更新仓库失败: {}", e)));
    }
    Json(ApiResponse::msg("仓库更新成功"))
}

#[derive(Deserialize)]
pub struct BrandCreateRequest {
    pub BrandCode: String,
    pub BrandName: String,
    pub BrandEN: Option<String>,
    pub Used: Option<String>,
}

pub async fn create_brand(
    State(_config): State<Config>,
    Json(body): Json<BrandCreateRequest>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("DB: {}", e))),
    };
    if body.BrandCode.is_empty() || body.BrandName.is_empty() {
        return Json(ApiResponse::err("品牌编码和名称不能为空"));
    }
    let brandid = format!("{}", uuid::Uuid::new_v4());
    let sql = r#"INSERT INTO tBas_Brand (BrandID, BrandCode, BrandName, BrandEN, Used)
              VALUES (@p1, @p2, @p3, @p4, @p5)"#;
    let branden = body.BrandEN.as_deref().unwrap_or("");
    let used = body.Used.as_deref().unwrap_or("Y");
    if let Err(e) = conn.execute(sql, &[
        &brandid, &body.BrandCode, &body.BrandName, &branden, &used,
    ]).await {
        return Json(ApiResponse::err(&format!("新增品牌失败: {}", e)));
    }
    Json(ApiResponse::msg("品牌新增成功"))
}

#[derive(Deserialize)]
pub struct BrandUpdateRequest {
    pub BrandID: String,
    pub BrandCode: String,
    pub BrandName: String,
    pub BrandEN: Option<String>,
    pub Used: Option<String>,
}

pub async fn update_brand(
    State(_config): State<Config>,
    Json(body): Json<BrandUpdateRequest>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("DB: {}", e))),
    };
    let sql = r#"UPDATE tBas_Brand SET BrandCode=@p1, BrandName=@p2, BrandEN=@p3, Used=@p4 WHERE BrandID=@p5"#;
    let branden = body.BrandEN.as_deref().unwrap_or("");
    let used = body.Used.as_deref().unwrap_or("Y");
    if let Err(e) = conn.execute(sql, &[
        &body.BrandCode, &body.BrandName, &branden, &used, &body.BrandID,
    ]).await {
        return Json(ApiResponse::err(&format!("更新品牌失败: {}", e)));
    }
    Json(ApiResponse::msg("品牌更新成功"))
}

#[derive(Deserialize)]
pub struct EmployeeCreateRequest {
    pub EmpNo: String,
    pub EmpName: String,
    pub Sex: Option<String>,
    pub DeptID: Option<String>,
    pub DutyID: Option<String>,
    pub Tel: Option<String>,
    pub Used: Option<String>,
}

pub async fn create_employee(
    State(_config): State<Config>,
    Json(body): Json<EmployeeCreateRequest>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("DB: {}", e))),
    };
    if body.EmpNo.is_empty() || body.EmpName.is_empty() {
        return Json(ApiResponse::err("员工编码和姓名不能为空"));
    }
    let empid = format!("{}", uuid::Uuid::new_v4());
    let sql = r#"INSERT INTO tBas_Emp (EmpID, EmpNo, EmpName, Sex, DeptID, DutyID, Tel, Used)
              VALUES (@p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8)"#;
    let sex = body.Sex.as_deref().unwrap_or("M");
    let deptid = body.DeptID.as_deref().unwrap_or("00000000-0000-0000-0000-000000000000");
    let dutyid = body.DutyID.as_deref().unwrap_or("00000000-0000-0000-0000-000000000000");
    let tel = body.Tel.as_deref().unwrap_or("");
    let used = body.Used.as_deref().unwrap_or("Y");
    if let Err(e) = conn.execute(sql, &[
        &empid, &body.EmpNo, &body.EmpName, &sex, &deptid, &dutyid, &tel, &used,
    ]).await {
        return Json(ApiResponse::err(&format!("新增员工失败: {}", e)));
    }
    Json(ApiResponse::msg("员工新增成功"))
}

#[derive(Deserialize)]
pub struct EmployeeUpdateRequest {
    pub EmpID: String,
    pub EmpNo: String,
    pub EmpName: String,
    pub Sex: Option<String>,
    pub DeptID: Option<String>,
    pub DutyID: Option<String>,
    pub Tel: Option<String>,
    pub Used: Option<String>,
}

pub async fn update_employee(
    State(_config): State<Config>,
    Json(body): Json<EmployeeUpdateRequest>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("DB: {}", e))),
    };
    let sql = r#"UPDATE tBas_Emp SET EmpNo=@p1, EmpName=@p2, Sex=@p3, DeptID=@p4,
              DutyID=@p5, Tel=@p6, Used=@p7 WHERE EmpID=@p8"#;
    let sex = body.Sex.as_deref().unwrap_or("M");
    let deptid = body.DeptID.as_deref().unwrap_or("00000000-0000-0000-0000-000000000000");
    let dutyid = body.DutyID.as_deref().unwrap_or("00000000-0000-0000-0000-000000000000");
    let tel = body.Tel.as_deref().unwrap_or("");
    let used = body.Used.as_deref().unwrap_or("Y");
    if let Err(e) = conn.execute(sql, &[
        &body.EmpNo, &body.EmpName, &sex, &deptid, &dutyid, &tel, &used, &body.EmpID,
    ]).await {
        return Json(ApiResponse::err(&format!("更新员工失败: {}", e)));
    }
    Json(ApiResponse::msg("员工更新成功"))
}

pub async fn get_profit_analysis(
    State(_config): State<Config>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(conn) => conn,
        Err(e) => return Json(ApiResponse::err(&format!("获取数据库连接失败: {}", e))),
    };

    let mut monthly_profit = serde_json::json!([]);
    let sql = r#"SELECT TOP 6 CONVERT(varchar(7), InvDate, 120) as month,
        SUM(TotalAmt) as sales, SUM(CostAmt) as cost
        FROM tSal_Inv WHERE State <> 'D' AND InvDate >= DATEADD(month, -6, GETDATE())
        GROUP BY CONVERT(varchar(7), InvDate, 120) ORDER BY month"#;
    match conn.query(sql, &[]).await {
        Ok(stream) => {
            match stream.into_first_result().await {
                Ok(rows) => {
                    let items: Vec<serde_json::Value> = rows.iter().map(|row| {
                        let sales = try_get_value(&row, "sales").as_f64().unwrap_or(0.0);
                        let cost = try_get_value(&row, "cost").as_f64().unwrap_or(0.0);
                        let profit = sales - cost;
                        let margin = if sales > 0.0 { (profit / sales * 100.0 * 10.0).round() / 10.0 } else { 0.0 };
                        serde_json::json!({
                            "month": row.get::<&str, _>("month").unwrap_or(""),
                            "sales": sales as i64,
                            "cost": cost as i64,
                            "profit": profit as i64,
                            "margin": margin
                        })
                    }).collect();
                    if !items.is_empty() {
                        monthly_profit = serde_json::json!(items);
                    }
                }
                Err(_) => {}
            }
        }
        Err(_) => {}
    }

    let data = serde_json::json!({
        "monthlyProfit": monthly_profit
    });

    Json(ApiResponse::ok(data))
}

pub async fn get_warehouses(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Json<ApiResponse<Vec<serde_json::Value>>> {
    let mut conn = match get_pool().get().await {
        Ok(conn) => conn,
        Err(e) => return Json(ApiResponse::err(&format!("获取数据库连接失败: {}", e))),
    };
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);

    let base_query = "SELECT * FROM tBas_Stock WHERE Used <> 'N'".to_string();

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, params.sort_prop.as_deref(), params.sort_order.as_deref());

    let mut total: i32 = 0;
    let count_stream = match conn.query(&count_sql, &[]).await {
        Ok(stream) => stream,
        Err(e) => return Json(ApiResponse::err(&format!("查询仓库总数失败: {}", e))),
    };
    match count_stream.into_row().await {
        Ok(Some(row)) => { total = row.get::<i32, _>("cnt").unwrap_or(0); }
        Ok(None) => {}
        Err(e) => return Json(ApiResponse::err(&format!("获取仓库总数行失败: {}", e))),
    }

    let data_stream = match conn.query(&paginated_sql, &[]).await {
        Ok(stream) => stream,
        Err(e) => return Json(ApiResponse::err(&format!("查询仓库数据失败: {}", e))),
    };
    let rows: Vec<Row> = match data_stream.into_first_result().await {
        Ok(rows) => rows,
        Err(e) => return Json(ApiResponse::err(&format!("获取仓库数据结果失败: {}", e))),
    };
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Json(ApiResponse::ok_paginated(data, total as u64, page, page_size))
}

pub async fn get_employees(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Json<ApiResponse<Vec<serde_json::Value>>> {
    let mut conn = match get_pool().get().await {
        Ok(conn) => conn,
        Err(e) => return Json(ApiResponse::err(&format!("获取数据库连接失败: {}", e))),
    };
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);

    let mut base_query = "SELECT * FROM tBas_Emp WHERE State <> 'D'".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(&format!(" AND (EmpNo LIKE @p{} OR EmpName LIKE @p{})", pidx, pidx + 1));
            pidx += 2;
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
        }
    }

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, params.sort_prop.as_deref(), params.sort_order.as_deref());
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();

    let mut total: i32 = 0;
    let count_stream = match conn.query(&count_sql, &param_refs).await {
        Ok(stream) => stream,
        Err(e) => return Json(ApiResponse::err(&format!("查询员工总数失败: {}", e))),
    };
    match count_stream.into_row().await {
        Ok(Some(row)) => { total = row.get::<i32, _>("cnt").unwrap_or(0); }
        Ok(None) => {}
        Err(e) => return Json(ApiResponse::err(&format!("获取员工总数行失败: {}", e))),
    }

    let data_stream = match conn.query(&paginated_sql, &param_refs).await {
        Ok(stream) => stream,
        Err(e) => return Json(ApiResponse::err(&format!("查询员工数据失败: {}", e))),
    };
    let rows: Vec<Row> = match data_stream.into_first_result().await {
        Ok(rows) => rows,
        Err(e) => return Json(ApiResponse::err(&format!("获取员工数据结果失败: {}", e))),
    };
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Json(ApiResponse::ok_paginated(data, total as u64, page, page_size))
}

pub async fn get_brands(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Json<ApiResponse<Vec<serde_json::Value>>> {
    let mut conn = match get_pool().get().await {
        Ok(conn) => conn,
        Err(e) => return Json(ApiResponse::err(&format!("获取数据库连接失败: {}", e))),
    };
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);

    let base_query = "SELECT * FROM tBas_Brand WHERE Used <> 'N'".to_string();

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, params.sort_prop.as_deref(), params.sort_order.as_deref());

    let mut total: i32 = 0;
    let count_stream = match conn.query(&count_sql, &[]).await {
        Ok(stream) => stream,
        Err(e) => return Json(ApiResponse::err(&format!("查询品牌总数失败: {}", e))),
    };
    match count_stream.into_row().await {
        Ok(Some(row)) => { total = row.get::<i32, _>("cnt").unwrap_or(0); }
        Ok(None) => {}
        Err(e) => return Json(ApiResponse::err(&format!("获取品牌总数行失败: {}", e))),
    }

    let data_stream = match conn.query(&paginated_sql, &[]).await {
        Ok(stream) => stream,
        Err(e) => return Json(ApiResponse::err(&format!("查询品牌数据失败: {}", e))),
    };
    let rows: Vec<Row> = match data_stream.into_first_result().await {
        Ok(rows) => rows,
        Err(e) => return Json(ApiResponse::err(&format!("获取品牌数据结果失败: {}", e))),
    };
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Json(ApiResponse::ok_paginated(data, total as u64, page, page_size))
}
