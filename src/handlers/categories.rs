use axum::extract::{Json, State};
use serde::Deserialize;
use tiberius::Row;
use crate::config::Config;
use crate::db::get_pool;
use crate::error::Result;
use crate::utils::{ApiResponse, build_pagination_sql_with_sort};
use crate::handlers::base_data::{try_get_value, row_to_json};

#[derive(Deserialize)]
pub struct CategoryListParams {
    pub table: Option<String>,
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
    pub sort_prop: Option<String>,
    pub sort_order: Option<String>,
    pub include_deleted: Option<bool>,
}

#[derive(Deserialize)]
pub struct CategoryGetParams {
    pub table: Option<String>,
    pub id: String,
}

#[derive(Deserialize)]
pub struct CategoryCreateParams {
    pub table: Option<String>,
    pub GDSTypeCode: Option<String>,
    pub GDSTypeName: Option<String>,
    pub Flg: Option<String>,
    pub Note: Option<String>,
    pub Used: Option<String>,
    pub gdsTypeSD: Option<i32>,
}

#[derive(Deserialize)]
pub struct CategoryUpdateParams {
    pub table: Option<String>,
    pub id: String,
    pub GDSTypeCode: Option<String>,
    pub GDSTypeName: Option<String>,
    pub Flg: Option<String>,
    pub Note: Option<String>,
    pub Used: Option<String>,
    pub gdsTypeSD: Option<i32>,
}

#[derive(Deserialize)]
pub struct CategoryDeleteParams {
    pub table: Option<String>,
    pub ids: Vec<String>,
}

#[derive(Deserialize)]
pub struct CategoryRestoreParams {
    pub table: Option<String>,
    pub ids: Vec<String>,
}

#[derive(Deserialize)]
pub struct CategoryImportParams {
    pub table: Option<String>,
    pub data: Vec<serde_json::Map<String, serde_json::Value>>,
}

#[derive(Deserialize)]
pub struct CategoryTreeParams {
    pub table: Option<String>,
}

fn resolve_table(table: Option<&str>) -> &'static str {
    match table.unwrap_or("") {
        "tBas_GDSProperty" => "tBas_GDSProperty",
        "tBas_GDSKind" => "tBas_GDSKind",
        _ => "tBas_GDSType",
    }
}

fn resolve_pk(table: &str) -> &'static str {
    match table {
        "tBas_GDSProperty" => "GDSPropertyID",
        "tBas_GDSKind" => "GDSKindID",
        _ => "GDSTypeID",
    }
}

fn resolve_name_col(table: &str) -> &'static str {
    match table {
        "tBas_GDSProperty" => "GDSPropertyName",
        "tBas_GDSKind" => "GDSKindName",
        _ => "GDSTypeName",
    }
}

fn resolve_code_col(table: &str) -> &'static str {
    match table {
        "tBas_GDSProperty" => "GDSPropertyCode",
        "tBas_GDSKind" => "GDSKindCode",
        _ => "GDSTypeCode",
    }
}

fn resolve_sd_col(table: &str) -> &'static str {
    match table {
        "tBas_GDSProperty" => "gdsPropertySD",
        "tBas_GDSKind" => "gdsKindSD",
        _ => "gdsTypeSD",
    }
}

pub async fn get_categories(
    State(_config): State<Config>,
    Json(params): Json<CategoryListParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let table = resolve_table(params.table.as_deref());
    let name_col = resolve_name_col(table);
    let code_col = resolve_code_col(table);
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(50), 1000);

    let mut base_query = format!("SELECT t.* FROM [{}] t", table);
    let mut conditions = Vec::new();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let pidx = 1;

    if !params.include_deleted.unwrap_or(false) {
        conditions.push("t.[Used] <> 'N'".to_string());
    }

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            conditions.push(format!(
                "(CAST(t.[{}] AS varchar(max)) LIKE @p{} OR CAST(t.[{}] AS varchar(max)) LIKE @p{})",
                code_col, pidx, name_col, pidx + 1
            ));
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
        }
    }

    if !conditions.is_empty() {
        base_query.push_str(&format!(" WHERE {}", conditions.join(" AND ")));
    }

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let default_sort = format!("[{}]", name_col);
    let sort_prop = params.sort_prop.as_deref().or(Some(&default_sort));
    let paginated_sql = build_pagination_sql_with_sort(
        &base_query,
        page,
        page_size,
        sort_prop,
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

pub async fn get_category_tree(
    State(_config): State<Config>,
    Json(params): Json<CategoryTreeParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let table = resolve_table(params.table.as_deref());
    let pk = resolve_pk(table);
    let name_col = resolve_name_col(table);

    let sql = format!(
        "SELECT [{}], [{}], [Flg], [Used] FROM [{}] WHERE [Used] <> 'N' ORDER BY [Flg], [{}]",
        pk, name_col, table, name_col
    );

    let stream = conn.query(&sql, &[]).await?;
    let rows: Vec<Row> = stream.into_first_result().await?;

    let all_items: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let id = try_get_value(row, pk);
            let name = try_get_value(row, name_col);
            let flg = try_get_value(row, "Flg");
            let used = try_get_value(row, "Used");
            serde_json::json!({
                "id": id,
                "label": name,
                "Flg": flg,
                "Used": used,
                "children": []
            })
        })
        .collect();

    let id_map: std::collections::HashMap<String, usize> = all_items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let id = item.get("id")?.as_str()?;
            Some((id.to_string(), i))
        })
        .collect();

    let mut root_indices: Vec<usize> = Vec::new();
    let mut children_map: std::collections::HashMap<String, Vec<usize>> =
        std::collections::HashMap::new();

    for (i, item) in all_items.iter().enumerate() {
        let flg_val = item.get("Flg").and_then(|v| v.as_str()).unwrap_or("");
        if flg_val.is_empty() || flg_val == "0" || !id_map.contains_key(flg_val) {
            root_indices.push(i);
        } else {
            children_map
                .entry(flg_val.to_string())
                .or_default()
                .push(i);
        }
    }

    fn build_tree(
        all_items: &[serde_json::Value],
        indices: &[usize],
        children_map: &std::collections::HashMap<String, Vec<usize>>,
    ) -> Vec<serde_json::Value> {
        indices
            .iter()
            .map(|&i| {
                let item = &all_items[i];
                let id_str = item
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let children = if let Some(child_indices) = children_map.get(&id_str) {
                    build_tree(all_items, child_indices, children_map)
                } else {
                    vec![]
                };
                let mut obj = item.clone();
                if children.is_empty() {
                    obj.as_object_mut()
                        .map(|m| m.remove("children"));
                } else {
                    obj.as_object_mut().map(|m| {
                        m.insert("children".to_string(), serde_json::Value::Array(children));
                    });
                }
                obj
            })
            .collect()
    }

    let tree = build_tree(&all_items, &root_indices, &children_map);

    Ok(Json(ApiResponse::ok(tree)))
}

pub async fn get_category(
    State(_config): State<Config>,
    Json(params): Json<CategoryGetParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let table = resolve_table(params.table.as_deref());
    let pk = resolve_pk(table);

    let sql = format!("SELECT t.* FROM [{}] t WHERE t.[{}] = @p1", table, pk);
    let id_str = params.id.as_str();
    let stream = conn.query(&sql, &[&id_str]).await?;

    if let Some(row) = stream.into_row().await? {
        let data = row_to_json(&row);
        Ok(Json(ApiResponse::ok(data)))
    } else {
        Ok(Json(ApiResponse::err("记录不存在")))
    }
}

pub async fn create_category(
    State(_config): State<Config>,
    Json(params): Json<CategoryCreateParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let table = resolve_table(params.table.as_deref());
    let pk = resolve_pk(table);

    let new_id = uuid::Uuid::new_v4().to_string();
    let code = params.GDSTypeCode.as_deref().unwrap_or("");
    let name = params.GDSTypeName.as_deref().unwrap_or("");
    let flg = params.Flg.as_deref().unwrap_or("");
    let note = params.Note.as_deref().unwrap_or("");
    let used = params.Used.as_deref().unwrap_or("Y");
    let gds_type_sd = params.gdsTypeSD.unwrap_or(0);
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let sql = format!(
        "INSERT INTO [{}] ([{}], [{}], [{}], [Flg], [Note], [Used], [LUTime], [{}]) VALUES (@p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8)",
        table, pk, resolve_code_col(table), resolve_name_col(table), resolve_sd_col(table)
    );

    conn.execute(
        &sql,
        &[
            &new_id.as_str(),
            &code,
            &name,
            &flg,
            &note,
            &used,
            &now,
            &gds_type_sd,
        ],
    )
    .await?;

    Ok(Json(ApiResponse::msg("新增成功")))
}

pub async fn update_category(
    State(_config): State<Config>,
    Json(params): Json<CategoryUpdateParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let table = resolve_table(params.table.as_deref());
    let pk = resolve_pk(table);
    let code_col = resolve_code_col(table);
    let name_col = resolve_name_col(table);

    let sql = format!(
        "UPDATE [{}] SET [{}] = @p1, [{}] = @p2, [Flg] = @p3, [Note] = @p4, [Used] = @p5, [LUTime] = @p6, [{}] = @p7 WHERE [{}] = @p8",
        table, code_col, name_col, resolve_sd_col(table), pk
    );

    let code = params.GDSTypeCode.as_deref().unwrap_or("");
    let name = params.GDSTypeName.as_deref().unwrap_or("");
    let flg = params.Flg.as_deref().unwrap_or("");
    let note = params.Note.as_deref().unwrap_or("");
    let used = params.Used.as_deref().unwrap_or("Y");
    let gds_type_sd = params.gdsTypeSD.unwrap_or(0);
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let id_str = params.id.as_str();

    conn.execute(
        &sql,
        &[
            &code,
            &name,
            &flg,
            &note,
            &used,
            &now,
            &gds_type_sd,
            &id_str,
        ],
    )
    .await?;

    Ok(Json(ApiResponse::msg("更新成功")))
}

pub async fn delete_category(
    State(_config): State<Config>,
    Json(params): Json<CategoryDeleteParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let table = resolve_table(params.table.as_deref());
    let pk = resolve_pk(table);

    if params.ids.is_empty() {
        return Ok(Json(ApiResponse::err("请选择要删除的记录")));
    }

    for id in &params.ids {
        let sql = format!("DELETE FROM [{}] WHERE [{}] = @p1", table, pk);
        let id_str = id.as_str();
        conn.execute(&sql, &[&id_str]).await?;
    }

    Ok(Json(ApiResponse::msg(&format!(
        "成功删除{}条记录",
        params.ids.len()
    ))))
}

pub async fn restore_category(
    State(_config): State<Config>,
    Json(params): Json<CategoryRestoreParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let table = resolve_table(params.table.as_deref());
    let pk = resolve_pk(table);

    if params.ids.is_empty() {
        return Ok(Json(ApiResponse::err("请选择要恢复的记录")));
    }

    for id in &params.ids {
        let sql = format!(
            "UPDATE [{}] SET [Used] = 'Y' WHERE [{}] = @p1",
            table, pk
        );
        let id_str = id.as_str();
        conn.execute(&sql, &[&id_str]).await?;
    }

    Ok(Json(ApiResponse::msg(&format!(
        "成功恢复{}条记录",
        params.ids.len()
    ))))
}

pub async fn import_categories(
    State(_config): State<Config>,
    Json(params): Json<CategoryImportParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let table = resolve_table(params.table.as_deref());
    let pk = resolve_pk(table);

    if params.data.is_empty() {
        return Ok(Json(ApiResponse::err("没有提供导入数据")));
    }

    let mut success_count = 0u32;
    for row in &params.data {
        let mut columns = Vec::new();
        let mut placeholders = Vec::new();
        let mut values: Vec<Option<String>> = Vec::new();

        let has_pk = row.keys().any(|k| k == pk);
        if !has_pk {
            columns.push(format!("[{}]", pk));
            placeholders.push(format!("@p{}", columns.len()));
            values.push(Some(uuid::Uuid::new_v4().to_string()));
        }

        let now_str = chrono::Local::now().naive_local().format("%Y-%m-%d %H:%M:%S").to_string();
        let has_lutime = row.keys().any(|k| k.eq_ignore_ascii_case("LUTime"));
        if !has_lutime {
            columns.push("[LUTime]".to_string());
            placeholders.push(format!("@p{}", columns.len()));
            values.push(Some(now_str.clone()));
        }

        for (key, val) in row.iter() {
            if key == pk && !has_pk {
                continue;
            }
            columns.push(format!("[{}]", key));
            placeholders.push(format!("@p{}", columns.len()));
            values.push(json_to_sql_value(val));
        }

        let sql = format!(
            "INSERT INTO [{}] ({}) VALUES ({})",
            table,
            columns.join(", "),
            placeholders.join(", ")
        );

        let param_refs: Vec<&dyn tiberius::ToSql> = values
            .iter()
            .map(|v| v as &dyn tiberius::ToSql)
            .collect();

        match conn.execute(&sql, &param_refs).await {
            Ok(_) => success_count += 1,
            Err(e) => {
                tracing::warn!("导入行失败: {:?}", e);
            }
        }
    }

    Ok(Json(ApiResponse::msg(&format!(
        "成功导入{}条记录",
        success_count
    ))))
}

fn json_to_sql_value(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::Null => None,
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(if *b {
            "1".to_string()
        } else {
            "0".to_string()
        }),
        _ => Some(v.to_string()),
    }
}
