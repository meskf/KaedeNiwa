//! 好友 HTTP API。
//!
//! - 双向关系（A 请求 → B 接受）
//! - 状态：`pending` | `accepted` | `blocked`

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use kaede_protocol::{ServerMessage, UserId};

use crate::db;
use crate::error::ApiError;
use crate::http::authenticate;
use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/friends", get(list_friends))
        .route(
            "/api/friends/{login_id}",
            post(send_request).delete(remove_friend),
        )
        .route("/api/friends/{login_id}/accept", post(accept_request))
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024))
        .with_state(state)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Friend {
    pub login_id: String,
    pub nickname: String,
    /// `"outgoing"` | `"incoming"` | `"friend"`
    pub direction: String,
    pub created_at: i64,
}

#[derive(Serialize)]
struct FriendsResp {
    friends: Vec<Friend>,
}

async fn list_friends(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<FriendsResp>, ApiError> {
    let uid = authenticate(&state, &headers).await?;
    let me = uid.as_str();

    // 出站（我主动请求的 pending）
    let out_rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT friend_login_id, created_at FROM friends \
         WHERE login_id = ? AND status = 'pending'",
    )
    .bind(me)
    .fetch_all(&state.pool)
    .await?;

    // 入站（别人请求我 + 已接受的朋友）
    let in_rows: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT login_id, status, created_at FROM friends \
         WHERE friend_login_id = ?",
    )
    .bind(me)
    .fetch_all(&state.pool)
    .await?;

    let mut friends = Vec::new();

    for (flid, ts) in out_rows {
        let nick = get_nickname(&state, &flid).await;
        friends.push(Friend {
            login_id: flid,
            nickname: nick,
            direction: "outgoing".into(),
            created_at: ts,
        });
    }

    for (flid, status, ts) in in_rows {
        let nick = get_nickname(&state, &flid).await;
        let direction = if status == "accepted" {
            "friend".to_string()
        } else {
            "incoming".to_string()
        };
        friends.push(Friend {
            login_id: flid,
            nickname: nick,
            direction,
            created_at: ts,
        });
    }

    Ok(Json(FriendsResp { friends }))
}

async fn send_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(target): Path<String>,
) -> Result<StatusCode, ApiError> {
    let uid = authenticate(&state, &headers).await?;
    let me = uid.as_str().to_string();

    // 好友请求会触发对方实时通知：限流防通知轰炸与关系表灌水。
    if !state.check_rate_scoped(&uid, "friend_request", 0.2, 5.0) {
        state.metrics.rate_limited();
        return Err(ApiError::BadRequest("friend request rate limited".into()));
    }

    if me == target {
        return Err(ApiError::BadRequest("cannot friend yourself".into()));
    }

    // 检查目标存在
    let exists: Option<(String,)> =
        sqlx::query_as("SELECT login_id FROM accounts WHERE login_id = ?")
            .bind(&target)
            .fetch_optional(&state.pool)
            .await?;
    if exists.is_none() {
        return Err(ApiError::NotFound("user".into()));
    }

    // 若对方已请求我 → 自动成为朋友
    let reverse: Option<(String,)> =
        sqlx::query_as("SELECT status FROM friends WHERE login_id = ? AND friend_login_id = ?")
            .bind(&target)
            .bind(&me)
            .fetch_optional(&state.pool)
            .await?;

    let now = db::now_ms();

    if let Some((s,)) = reverse {
        if s == "pending" {
            // 双向 accepted
            sqlx::query(
                "UPDATE friends SET status = 'accepted' \
                 WHERE login_id = ? AND friend_login_id = ?",
            )
            .bind(&target)
            .bind(&me)
            .execute(&state.pool)
            .await?;
            sqlx::query(
                "INSERT OR REPLACE INTO friends (login_id, friend_login_id, status, created_at) \
                 VALUES (?, ?, 'accepted', ?)",
            )
            .bind(&me)
            .bind(&target)
            .bind(now)
            .execute(&state.pool)
            .await?;
            notify_friends_changed(&state, &[&me, &target]);
            return Ok(StatusCode::OK);
        }
        if s == "accepted" {
            return Ok(StatusCode::OK); // 已是朋友
        }
    }

    // 普通请求
    sqlx::query(
        "INSERT OR REPLACE INTO friends (login_id, friend_login_id, status, created_at) \
         VALUES (?, ?, 'pending', ?)",
    )
    .bind(&me)
    .bind(&target)
    .bind(now)
    .execute(&state.pool)
    .await?;

    notify_friends_changed(&state, &[&me, &target]);

    Ok(StatusCode::CREATED)
}

async fn accept_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(from): Path<String>,
) -> Result<StatusCode, ApiError> {
    let uid = authenticate(&state, &headers).await?;
    let me = uid.as_str().to_string();

    // from → me 必须是 pending
    let row: Option<(String,)> =
        sqlx::query_as("SELECT status FROM friends WHERE login_id = ? AND friend_login_id = ?")
            .bind(&from)
            .bind(&me)
            .fetch_optional(&state.pool)
            .await?;

    let Some((s,)) = row else {
        return Err(ApiError::NotFound("friend request".into()));
    };
    if s != "pending" {
        return Err(ApiError::BadRequest("not a pending request".into()));
    }

    let now = db::now_ms();
    sqlx::query(
        "UPDATE friends SET status = 'accepted' \
         WHERE login_id = ? AND friend_login_id = ?",
    )
    .bind(&from)
    .bind(&me)
    .execute(&state.pool)
    .await?;
    sqlx::query(
        "INSERT OR REPLACE INTO friends (login_id, friend_login_id, status, created_at) \
         VALUES (?, ?, 'accepted', ?)",
    )
    .bind(&me)
    .bind(&from)
    .bind(now)
    .execute(&state.pool)
    .await?;

    notify_friends_changed(&state, &[&me, &from]);

    Ok(StatusCode::OK)
}

async fn remove_friend(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(target): Path<String>,
) -> Result<StatusCode, ApiError> {
    let uid = authenticate(&state, &headers).await?;
    let me = uid.as_str().to_string();

    sqlx::query("DELETE FROM friends WHERE login_id = ? AND friend_login_id = ?")
        .bind(&me)
        .bind(&target)
        .execute(&state.pool)
        .await?;
    sqlx::query("DELETE FROM friends WHERE login_id = ? AND friend_login_id = ?")
        .bind(&target)
        .bind(&me)
        .execute(&state.pool)
        .await?;

    notify_friends_changed(&state, &[&me, &target]);

    Ok(StatusCode::NO_CONTENT)
}

/// 是否为已接受的好友关系（双向关系里任一方向 accepted 即可）。
///
/// 用于 KeyPackage 拉取的信任判定：好友不受目标队列保留量限制。
pub async fn is_friend(pool: &sqlx::SqlitePool, a: &str, b: &str) -> Result<bool, sqlx::Error> {
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT 1 FROM friends \
         WHERE login_id = ? AND friend_login_id = ? AND status = 'accepted'",
    )
    .bind(a)
    .bind(b)
    .fetch_optional(pool)
    .await?;
    Ok(row.is_some())
}

/// 通知在线的相关用户重新拉取好友列表；离线用户忽略（DB 已持久化）。
fn notify_friends_changed(state: &AppState, uids: &[&String]) {
    for uid in uids {
        state.send_to(&UserId::new(uid.as_str()), ServerMessage::FriendsChanged);
    }
}

async fn get_nickname(state: &AppState, login_id: &str) -> String {
    let r: Option<(String,)> = sqlx::query_as("SELECT nickname FROM members WHERE login_id = ?")
        .bind(login_id)
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten();
    r.map(|(n,)| n).unwrap_or_else(|| login_id.to_string())
}
