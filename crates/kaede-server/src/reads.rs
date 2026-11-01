//! 已读位置与未读计数。
//!
//! - `POST /api/reads`：上报某个频道/DM 的最后已读 ts
//! - `GET /api/unread`：返回每个可见频道/DM 的未读条数
//!
//! 服务端只存密文，因此未读数只是“条数”，无法知道 @ 等语义信息。

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use crate::{db, error::ApiError, http::authenticate, state::AppState};

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/reads", post(mark_read))
        .route("/api/unread", get(unread))
        .layer(DefaultBodyLimit::max(16 * 1024))
        .with_state(state)
}

#[derive(Deserialize)]
struct MarkReadReq {
    #[serde(default)]
    channel_id: Option<String>,
    #[serde(default)]
    conv_id: Option<String>,
    ts: i64,
}

async fn mark_read(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<MarkReadReq>,
) -> Result<StatusCode, ApiError> {
    let uid = authenticate(&state, &headers).await?;
    let me = uid.as_str();

    // 不允许把已读位置推到未来，否则会永久吞掉之后的新消息。
    let ts = req.ts.max(0).min(db::now_ms());

    match (&req.channel_id, &req.conv_id) {
        (Some(channel_id), _) => {
            let meta: Option<(i64,)> = sqlx::query_as("SELECT is_private FROM nodes WHERE id = ?")
                .bind(channel_id)
                .fetch_optional(&state.pool)
                .await?;
            let Some((is_private,)) = meta else {
                return Err(ApiError::NotFound("channel".into()));
            };
            if is_private != 0 {
                let member: Option<(String,)> = sqlx::query_as(
                    "SELECT login_id FROM channel_members WHERE channel_id = ? AND login_id = ?",
                )
                .bind(channel_id)
                .bind(me)
                .fetch_optional(&state.pool)
                .await?;
                if member.is_none() {
                    return Err(ApiError::Forbidden("not a channel member".into()));
                }
            }
            sqlx::query(
                "INSERT INTO channel_reads (login_id, channel_id, last_read_ts) VALUES (?, ?, ?) \
                 ON CONFLICT(login_id, channel_id) DO UPDATE SET \
                 last_read_ts = MAX(last_read_ts, excluded.last_read_ts)",
            )
            .bind(me)
            .bind(channel_id)
            .bind(ts)
            .execute(&state.pool)
            .await?;
        }
        (None, Some(conv_id)) => {
            let member: Option<(String,)> = sqlx::query_as(
                "SELECT conv_id FROM dm_conversations WHERE conv_id = ? AND (a = ? OR b = ?)",
            )
            .bind(conv_id)
            .bind(me)
            .bind(me)
            .fetch_optional(&state.pool)
            .await?;
            if member.is_none() {
                return Err(ApiError::Forbidden("not a dm participant".into()));
            }
            sqlx::query(
                "INSERT INTO dm_reads (login_id, conv_id, last_read_ts) VALUES (?, ?, ?) \
                 ON CONFLICT(login_id, conv_id) DO UPDATE SET \
                 last_read_ts = MAX(last_read_ts, excluded.last_read_ts)",
            )
            .bind(me)
            .bind(conv_id)
            .bind(ts)
            .execute(&state.pool)
            .await?;
        }
        (None, None) => {
            return Err(ApiError::BadRequest(
                "channel_id or conv_id required".into(),
            ));
        }
    }

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct UnreadResp {
    channels: Vec<UnreadChannel>,
    dms: Vec<UnreadDm>,
}

#[derive(Serialize)]
struct UnreadChannel {
    channel_id: String,
    count: i64,
}

#[derive(Serialize)]
struct UnreadDm {
    conv_id: String,
    count: i64,
}

async fn unread(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<UnreadResp>, ApiError> {
    let uid = authenticate(&state, &headers).await?;
    let me = uid.as_str();

    // 频道：只统计当前用户可见的频道（公开频道 + 自己已加入的私密频道）。
    let channel_rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT m.channel_id, COUNT(*) \
         FROM messages m \
         LEFT JOIN channel_reads r ON r.channel_id = m.channel_id AND r.login_id = ? \
         WHERE m.sender != ? \
           AND m.ts > COALESCE(r.last_read_ts, 0) \
           AND m.channel_id NOT LIKE 'dm:%' \
           AND m.channel_id IN ( \
               SELECT id FROM nodes \
               WHERE is_private = 0 \
                  OR id IN (SELECT channel_id FROM channel_members WHERE login_id = ?)) \
         GROUP BY m.channel_id",
    )
    .bind(me)
    .bind(me)
    .bind(me)
    .fetch_all(&state.pool)
    .await?;

    // DM：只统计自己参与的会话。
    let dm_rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT m.channel_id, COUNT(*) \
         FROM messages m \
         JOIN dm_conversations c ON c.conv_id = m.channel_id \
         LEFT JOIN dm_reads r ON r.conv_id = m.channel_id AND r.login_id = ? \
         WHERE (c.a = ? OR c.b = ?) \
           AND m.sender != ? \
           AND m.ts > COALESCE(r.last_read_ts, 0) \
           AND substr(m.ciphertext, 1, 1) != X'02' \
         GROUP BY m.channel_id",
    )
    .bind(me)
    .bind(me)
    .bind(me)
    .bind(me)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(UnreadResp {
        channels: channel_rows
            .into_iter()
            .map(|(channel_id, count)| UnreadChannel { channel_id, count })
            .collect(),
        dms: dm_rows
            .into_iter()
            .map(|(conv_id, count)| UnreadDm { conv_id, count })
            .collect(),
    }))
}
