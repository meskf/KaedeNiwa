//! 登录会话 / 设备管理。
//!
//! 当前架构仍是**单设备在线**：同账号新 WS 会顶替旧连接。
//! 这里提供“查看登录设备 + 撤销会话”，撤销当前会话会同时断开 WS。

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{delete, get, post},
};
use serde::Serialize;

use crate::{db, error::ApiError, http::authenticate_session, state::AppState};

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/sessions", get(list_sessions))
        .route("/api/sessions/revoke-others", post(revoke_others))
        .route("/api/sessions/{id}", delete(revoke_session))
        .route("/api/logout", post(logout))
        .with_state(state)
}

#[derive(Serialize)]
struct SessionsResp {
    sessions: Vec<SessionInfo>,
}

#[derive(Serialize)]
struct SessionInfo {
    id: String,
    device_name: String,
    created_at: i64,
    last_seen_at: i64,
    current: bool,
}

async fn list_sessions(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<SessionsResp>, ApiError> {
    let (uid, current_sid) = authenticate_session(&state, &headers).await?;
    let now = db::now_ms();

    let rows: Vec<(String, String, i64, i64)> = sqlx::query_as(
        "SELECT session_id, device_name, created_at, last_seen_at \
         FROM sessions WHERE login_id = ? AND expires_at > ? \
         ORDER BY last_seen_at DESC, created_at DESC",
    )
    .bind(uid.as_str())
    .bind(now)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(SessionsResp {
        sessions: rows
            .into_iter()
            .map(|(id, device_name, created_at, last_seen_at)| SessionInfo {
                current: id == current_sid,
                id,
                device_name,
                created_at,
                last_seen_at,
            })
            .collect(),
    }))
}

async fn revoke_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let (uid, current_sid) = authenticate_session(&state, &headers).await?;

    // 只能撤销自己的会话；不存在/不属于自己都返回 404，避免枚举。
    let owned: Option<(String,)> =
        sqlx::query_as("SELECT session_id FROM sessions WHERE session_id = ? AND login_id = ?")
            .bind(&session_id)
            .bind(uid.as_str())
            .fetch_optional(&state.pool)
            .await?;
    if owned.is_none() {
        return Err(ApiError::NotFound("session".into()));
    }

    sqlx::query("DELETE FROM sessions WHERE session_id = ? AND login_id = ?")
        .bind(&session_id)
        .bind(uid.as_str())
        .execute(&state.pool)
        .await?;

    // 断开与被撤销会话对应的在线 WS：
    // - 撤销当前会话：直接踢掉本 uid 的连接；
    // - 撤销其他会话：只有在线连接确实属于那个 session 时才踢。
    let peer = if session_id == current_sid {
        state.take_peer(&uid)
    } else {
        state.take_peer_if_http_session(&uid, &session_id)
    };
    if let Some(peer) = peer {
        let _ = peer.tx.try_send(kaede_protocol::ServerMessage::Error {
            code: kaede_protocol::ErrorCode::Unauthenticated,
            message: "session revoked".into(),
        });
    }

    Ok(StatusCode::NO_CONTENT)
}

async fn revoke_others(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let (uid, current_sid) = authenticate_session(&state, &headers).await?;
    sqlx::query("DELETE FROM sessions WHERE login_id = ? AND session_id != ?")
        .bind(uid.as_str())
        .bind(&current_sid)
        .execute(&state.pool)
        .await?;
    // 单设备在线：若当前 WS 属于被撤销的“其他 session”，立即断开，
    // 不再等 ≤5 分钟的 token 重验窗口。
    if let Some(peer) = state.take_peer_if_other_http_session(&uid, &current_sid) {
        let _ = peer.tx.try_send(kaede_protocol::ServerMessage::Error {
            code: kaede_protocol::ErrorCode::Unauthenticated,
            message: "session revoked".into(),
        });
    }
    Ok(StatusCode::NO_CONTENT)
}

/// 登出：撤销当前 HTTP session，并立即断开对应 WS。
async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Result<StatusCode, ApiError> {
    let (uid, current_sid) = authenticate_session(&state, &headers).await?;
    sqlx::query("DELETE FROM sessions WHERE login_id = ? AND session_id = ?")
        .bind(uid.as_str())
        .bind(&current_sid)
        .execute(&state.pool)
        .await?;
    if let Some(peer) = state.take_peer_if_http_session(&uid, &current_sid) {
        let _ = peer.tx.try_send(kaede_protocol::ServerMessage::Error {
            code: kaede_protocol::ErrorCode::Unauthenticated,
            message: "logged out".into(),
        });
    }
    Ok(StatusCode::NO_CONTENT)
}
