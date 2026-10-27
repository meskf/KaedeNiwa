//! 频道分类 HTTP API。
//!
//! 分类只是 UI 分组，本身不含消息；权限管理统一走 `check_write_perm`（op3+）。

use axum::{Json, Router, extract::{Path, State}, http::{HeaderMap, StatusCode}, routing::get};
use serde::{Deserialize, Serialize};

use kaede_protocol::Category;

use crate::db;
use crate::error::ApiError;
use crate::http::authenticate;
use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/categories", get(list_categories).post(create_category))
        .route(
            "/api/categories/{id}",
            axum::routing::patch(update_category).delete(delete_category),
        )
        .with_state(state)
}

#[derive(Deserialize)]
struct CreateReq {
    name: String,
    #[serde(default)]
    position: Option<i64>,
}

#[derive(Deserialize)]
struct UpdateReq {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    position: Option<i64>,
}

#[derive(Serialize)]
struct ListResp {
    categories: Vec<Category>,
}

async fn list_categories(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ListResp>, ApiError> {
    let _uid = authenticate(&state, &headers).await?;
    let rows: Vec<(String, String, i64, i64)> = sqlx::query_as(
        "SELECT id, name, position, created_at FROM categories ORDER BY position, id",
    )
    .fetch_all(&state.pool)
    .await?;
    let categories = rows
        .into_iter()
        .map(|(id, name, position, created_at)| Category { id, name, position, created_at })
        .collect();
    Ok(Json(ListResp { categories }))
}

async fn create_category(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateReq>,
) -> Result<(StatusCode, Json<Category>), ApiError> {
    let uid = authenticate(&state, &headers).await?;
    check_write_perm(&state, uid.as_str()).await?;

    let name = req.name.trim().to_string();
    if name.is_empty() || name.len() > 64 {
        return Err(ApiError::BadRequest("category name 1..=64".into()));
    }

    let id = format!("cat_{}", &db::random_token()[..10]);
    let now = db::now_ms();
    let position = match req.position {
        Some(p) => {
            if !(0..=i32::MAX as i64).contains(&p) {
                return Err(ApiError::BadRequest("position 0..=i32::MAX".into()));
            }
            p
        }
        None => {
            let (max,): (Option<i64>,) =
                sqlx::query_as("SELECT MAX(position) FROM categories")
                    .fetch_one(&state.pool)
                    .await?;
            max.map(|m| m + 1).unwrap_or(0)
        }
    };

    sqlx::query(
        "INSERT INTO categories (id, name, position, created_at) VALUES (?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&name)
    .bind(position)
    .bind(now)
    .execute(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(Category { id, name, position, created_at: now }),
    ))
}

async fn update_category(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<UpdateReq>,
) -> Result<Json<Category>, ApiError> {
    let uid = authenticate(&state, &headers).await?;
    check_write_perm(&state, uid.as_str()).await?;

    let row: Option<(String, i64, i64)> =
        sqlx::query_as("SELECT name, position, created_at FROM categories WHERE id = ?")
            .bind(&id)
            .fetch_optional(&state.pool)
            .await?;
    let (mut name, mut position, created_at) =
        row.ok_or_else(|| ApiError::NotFound("category".into()))?;

    if let Some(n) = req.name {
        let n = n.trim().to_string();
        if n.is_empty() || n.len() > 64 {
            return Err(ApiError::BadRequest("category name 1..=64".into()));
        }
        sqlx::query("UPDATE categories SET name = ? WHERE id = ?")
            .bind(&n)
            .bind(&id)
            .execute(&state.pool)
            .await?;
        name = n;
    }

    if let Some(p) = req.position {
        if !(0..=i32::MAX as i64).contains(&p) {
            return Err(ApiError::BadRequest("position 0..=i32::MAX".into()));
        }
        sqlx::query("UPDATE categories SET position = ? WHERE id = ?")
            .bind(p)
            .bind(&id)
            .execute(&state.pool)
            .await?;
        position = p;
    }

    Ok(Json(Category { id, name, position, created_at }))
}

async fn delete_category(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let uid = authenticate(&state, &headers).await?;
    check_write_perm(&state, uid.as_str()).await?;

    let mut tx = state.pool.begin().await?;
    // Detach channels first so they fall into the uncategorized bucket.
    sqlx::query("UPDATE channels SET category_id = NULL WHERE category_id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    let affected = sqlx::query("DELETE FROM categories WHERE id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    if affected.rows_affected() == 0 {
        return Err(ApiError::NotFound("category".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn check_write_perm(state: &AppState, uid: &str) -> Result<(), ApiError> {
    let row: Option<(String,)> = sqlx::query_as("SELECT op FROM members WHERE login_id = ?")
        .bind(uid)
        .fetch_optional(&state.pool)
        .await?;
    let (op,) = row.ok_or_else(|| ApiError::NotFound("member".into()))?;
    let allowed = matches!(op.as_str(), "op3" | "op4" | "op5");
    if !allowed {
        return Err(ApiError::Unauthorized);
    }
    Ok(())
}
