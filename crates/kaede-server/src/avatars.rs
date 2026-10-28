//! 用户头像与服务器图标（明文存储）。
//!
//! 存储：\`<data_dir>/avatars/<login_id>.jpg\` 与 \`<data_dir>/avatars/server.jpg\`。
//!
//! **为什么明文**：头像是公开元数据（成员列表 / 服务器卡片上所有用户可见），
//! 与 E2EE 消息内容无关。明文存储让 GET 可以直接被 \`<img src>\` 加载，
//! 不需要客户端解密再创建 blob URL。
//!
//! **格式**：只接受 JPEG（magic bytes \`FF D8 FF\`）。前端压缩到 256×256 JPEG。
//! **大小**：MAX_AVATAR_BYTES = 200 KiB。

use std::path::PathBuf;

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
    Router,
};

use crate::db;
use crate::error::ApiError;
use crate::http::authenticate;
use crate::state::AppState;

pub const MAX_AVATAR_BYTES: usize = 200 * 1024;

fn avatars_dir() -> PathBuf {
    let base = match std::env::var("KAEDE_DATA_DIR") {
        Ok(v) if !v.is_empty() => PathBuf::from(v),
        _ => std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("data"),
    };
    base.join("avatars")
}

fn is_safe_name(s: &str) -> bool {
    !s.is_empty() && !s.contains('/') && !s.contains('\\') && !s.contains("..") && s.len() <= 128
}

fn is_jpeg(b: &[u8]) -> bool {
    b.len() >= 3 && b[0] == 0xFF && b[1] == 0xD8 && b[2] == 0xFF
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route(
            "/api/me/avatar",
            post(upload_my_avatar).delete(delete_my_avatar),
        )
        .route("/api/avatars/{login_id}", get(get_avatar))
        .route(
            "/api/server/icon",
            post(upload_server_icon).get(get_server_icon),
        )
        .layer(axum::extract::DefaultBodyLimit::max(MAX_AVATAR_BYTES))
        .with_state(state)
}

async fn upload_my_avatar(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    let uid = authenticate(&state, &headers).await?;
    if !state.check_rate_scoped(&uid, "avatar_upload", 0.2, 2.0) {
        return Err(ApiError::TooManyRequests("avatar upload rate limited".into()));
    }
    if body.len() > MAX_AVATAR_BYTES {
        return Err(ApiError::BadRequest(format!(
            "avatar too large (max {} bytes)",
            MAX_AVATAR_BYTES
        )));
    }
    if !is_jpeg(&body) {
        return Err(ApiError::BadRequest("avatar must be JPEG".into()));
    }
    let dir = avatars_dir();
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| ApiError::Internal(format!("mkdir: {e}")))?;
    let path = dir.join(format!("{}.jpg", uid.as_str()));
    tokio::fs::write(&path, &body)
        .await
        .map_err(|e| ApiError::Internal(format!("write: {e}")))?;

    let url = format!("/api/avatars/{}", uid.as_str());
    let now = db::now_ms();
    sqlx::query("UPDATE members SET avatar_url = ?, updated_at = ? WHERE login_id = ?")
        .bind(&url)
        .bind(now)
        .bind(uid.as_str())
        .execute(&state.pool)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

async fn delete_my_avatar(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let uid = authenticate(&state, &headers).await?;
    let path = avatars_dir().join(format!("{}.jpg", uid.as_str()));
    let _ = tokio::fs::remove_file(&path).await;

    let now = db::now_ms();
    sqlx::query("UPDATE members SET avatar_url = '', updated_at = ? WHERE login_id = ?")
        .bind(now)
        .bind(uid.as_str())
        .execute(&state.pool)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

async fn get_avatar(Path(login_id): Path<String>) -> Result<impl IntoResponse, ApiError> {
    if !is_safe_name(&login_id) {
        return Err(ApiError::BadRequest("invalid login_id".into()));
    }
    let path = avatars_dir().join(format!("{login_id}.jpg"));
    match tokio::fs::read(&path).await {
        Ok(bytes) => Ok((
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "image/jpeg"),
                (header::CACHE_CONTROL, "public, max-age=60"),
            ],
            bytes,
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(ApiError::NotFound("avatar".into()))
        }
        Err(e) => Err(ApiError::Internal(format!("read avatar: {e}"))),
    }
}

async fn upload_server_icon(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    let uid = authenticate(&state, &headers).await?;

    let op: (String,) = sqlx::query_as("SELECT op FROM members WHERE login_id = ?")
        .bind(uid.as_str())
        .fetch_one(&state.pool)
        .await?;
    let level = match op.0.as_str() {
        "op5" => 5,
        "op4" => 4,
        "op3" => 3,
        _ => 1,
    };
    if level < 3 {
        return Err(ApiError::Forbidden("requires op3 or higher".into()));
    }

    if body.len() > MAX_AVATAR_BYTES {
        return Err(ApiError::BadRequest(format!(
            "icon too large (max {} bytes)",
            MAX_AVATAR_BYTES
        )));
    }
    if !is_jpeg(&body) {
        return Err(ApiError::BadRequest("icon must be JPEG".into()));
    }

    let dir = avatars_dir();
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| ApiError::Internal(format!("mkdir: {e}")))?;
    let path = dir.join("server.jpg");
    tokio::fs::write(&path, &body)
        .await
        .map_err(|e| ApiError::Internal(format!("write: {e}")))?;

    let now = db::now_ms();
    sqlx::query("UPDATE server_profile SET icon_url = '/api/server/icon', updated_at = ? WHERE id = 1")
        .bind(now)
        .execute(&state.pool)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

async fn get_server_icon() -> Result<impl IntoResponse, ApiError> {
    let path = avatars_dir().join("server.jpg");
    match tokio::fs::read(&path).await {
        Ok(bytes) => Ok((
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "image/jpeg"),
                (header::CACHE_CONTROL, "public, max-age=60"),
            ],
            bytes,
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(ApiError::NotFound("server icon".into()))
        }
        Err(e) => Err(ApiError::Internal(format!("read icon: {e}"))),
    }
}
