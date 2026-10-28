use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, patch},
    Json, Router,
};
use kaede_protocol::Node;
use serde::Deserialize;

use crate::{error::ApiError, http::authenticate, state::AppState};
use axum::http::HeaderMap;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/nodes", get(list_nodes).post(create_node))
        .route("/api/nodes/{id}", patch(update_node).delete(delete_node))
        }

#[derive(Deserialize)]
pub struct CreateNodeReq {
    pub name: String,
    pub kind: String,
    pub parent_id: Option<String>,
    pub is_private: Option<bool>,
    pub _password: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateNodeReq {
    pub name: Option<String>,
    pub parent_id: Option<String>,
    pub _position: Option<i64>,
}

async fn list_nodes(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Vec<Node>>, ApiError> {
    let _uid = authenticate(&state, &headers).await?;
    let rows: Vec<(String, String, Option<String>, String, i64, i64, i64)> = sqlx::query_as(
        "SELECT id, name, parent_id, kind, position, is_private, created_at FROM nodes ORDER BY position ASC"
    ).fetch_all(&state.pool).await?;

    let nodes = rows.into_iter().map(|(id, name, parent_id, kind, position, is_private, created_at)| Node {
        id, name, parent_id, kind, position, is_private: is_private != 0, created_at
    }).collect();

    Ok(Json(nodes))
}

async fn create_node(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateNodeReq>,
) -> Result<Json<Node>, ApiError> {
    let _uid = authenticate(&state, &headers).await?;
    let now_ns = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let id = format!("node_{}", now_ns);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
    let is_private = req.is_private.unwrap_or(false);

    sqlx::query(
        "INSERT INTO nodes (id, name, parent_id, kind, position, is_private, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(&id)
    .bind(&req.name)
    .bind(&req.parent_id)
    .bind(&req.kind)
    .bind(0) // position
    .bind(if is_private { 1 } else { 0 })
    .bind(now)
    .execute(&state.pool).await?;

    Ok(Json(Node {
        id, name: req.name, parent_id: req.parent_id, kind: req.kind, position: 0, is_private, created_at: now
    }))
}

async fn update_node(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<UpdateNodeReq>,
) -> Result<StatusCode, ApiError> {
    let _uid = authenticate(&state, &headers).await?;
    if let Some(name) = req.name {
        sqlx::query("UPDATE nodes SET name = ? WHERE id = ?").bind(name).bind(&id).execute(&state.pool).await?;
    }
    if let Some(pid) = req.parent_id {
        sqlx::query("UPDATE nodes SET parent_id = ? WHERE id = ?").bind(pid).bind(&id).execute(&state.pool).await?;
    }
    Ok(StatusCode::OK)
}

async fn delete_node(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let _uid = authenticate(&state, &headers).await?;
    sqlx::query("DELETE FROM nodes WHERE id = ?").bind(&id).execute(&state.pool).await?;
    Ok(StatusCode::NO_CONTENT)
}
