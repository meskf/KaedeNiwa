//! 管理面板的 handler。

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::admin::auth::AdminAuth;
use crate::admin::logs::LogEntry;
use crate::admin::service::ServiceStatus;
use crate::db;
use crate::error::ApiError;
use crate::state::AppState;
use kaede_protocol::UserId;

/// 记录一次管理写操作。`result` 用 `ok` / `not_found` / `rejected` 区分结果。
async fn audit(state: &AppState, action: &str, target: &str, result: &str) {
    if let Err(e) = sqlx::query(
        "INSERT INTO admin_audit (ts, admin, action, target, result) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(db::now_ms())
    .bind("token")
    .bind(action)
    .bind(target)
    .bind(result)
    .execute(&state.pool)
    .await
    {
        tracing::warn!("admin audit write failed: {}", e);
    }
}

// ---------- dashboard ----------

#[derive(Serialize)]
pub struct Dashboard {
    pub version: &'static str,
    pub uptime_secs: u64,
    pub users_online: usize,
    pub channels: i64,
    pub messages: i64,
    pub accounts: i64,
    pub banned: i64,
}

pub async fn dashboard(
    _: AdminAuth,
    State(state): State<AppState>,
) -> Result<Json<Dashboard>, ApiError> {
    let uptime = state.started_at.elapsed().as_secs();
    let users_online = state.peer_uids().len();
    let (channels,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM channels")
        .fetch_one(&state.pool)
        .await?;
    let (messages,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM messages")
        .fetch_one(&state.pool)
        .await?;
    let (accounts,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM accounts")
        .fetch_one(&state.pool)
        .await?;
    let (banned,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM accounts WHERE banned != 0")
        .fetch_one(&state.pool)
        .await?;
    Ok(Json(Dashboard {
        version: env!("CARGO_PKG_VERSION"),
        uptime_secs: uptime,
        users_online,
        channels,
        messages,
        accounts,
        banned,
    }))
}

// ---------- users ----------

#[derive(Deserialize)]
pub struct UsersQuery {
    #[serde(default)]
    pub q: String,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    50
}

#[derive(Serialize)]
pub struct UserRow {
    pub login_id: String,
    pub nickname: String,
    pub created_at: i64,
    pub banned: bool,
    pub banned_at: Option<i64>,
    pub banned_reason: Option<String>,
}

pub async fn list_users(
    _: AdminAuth,
    State(state): State<AppState>,
    Query(q): Query<UsersQuery>,
) -> Result<Json<Vec<UserRow>>, ApiError> {
    let query = q.q.chars().take(64).collect::<String>();
    let pattern = format!("%{}%", query);
    let rows = sqlx::query(
        "SELECT login_id, nickname, created_at, banned, banned_at, banned_reason \
         FROM accounts \
         WHERE (? = '' OR login_id LIKE ? OR nickname LIKE ?) \
         ORDER BY created_at DESC LIMIT ? OFFSET ?",
    )
    .bind(&query)
    .bind(&pattern)
    .bind(&pattern)
    .bind(q.limit.clamp(1, 200))
    .bind(q.offset.max(0))
    .fetch_all(&state.pool)
    .await?;
    let users: Vec<UserRow> = rows
        .into_iter()
        .map(|r| UserRow {
            login_id: r.get("login_id"),
            nickname: r.get("nickname"),
            created_at: r.get("created_at"),
            banned: r
                .try_get::<i64, _>("banned")
                .map(|v| v != 0)
                .unwrap_or(false),
            banned_at: r.try_get("banned_at").ok().flatten(),
            banned_reason: r.try_get("banned_reason").ok().flatten(),
        })
        .collect();
    Ok(Json(users))
}

#[derive(Deserialize, Default)]
pub struct BanReq {
    #[serde(default)]
    pub reason: Option<String>,
}

/// P1-12：admin 直接设置任意账号的 op 等级。
///
/// 权限模型：admin token 是独立于用户 op 体系的最高权限，不受
/// members::update_member 的"不能改同级/上级"约束。用途：恢复误操作、
/// 转移房主（op5）、临时指派 op4 代理人。所有变更强制写 admin_audit。
#[derive(Deserialize)]
pub struct SetOpReq {
    pub op: String,
}

pub async fn set_op(
    _: AdminAuth,
    State(state): State<AppState>,
    Path(login_id): Path<String>,
    Json(req): Json<SetOpReq>,
) -> Result<StatusCode, ApiError> {
    const VALID_OPS: &[&str] = &["op1", "op2", "op3", "op4", "op5"];
    if !VALID_OPS.contains(&req.op.as_str()) {
        audit(&state, "set_op", &login_id, "rejected_invalid_op").await;
        return Err(ApiError::BadRequest(format!(
            "invalid op '{}' (must be op1..op5)",
            req.op
        )));
    }
    // 唯一性：同一时刻只允许一个 op5（房主）。
    //
    // 理由：两个 op5 可能各自持有一个主群 group state，导致后续 Welcome
    // 指向不同 group_id，成员分裂到两个群。这是当前"单主群"模型的固有
    // 约束，**不是产品设计意图**——Phase 1（每频道 group 化）后主群只
    // 做身份目录，这个约束自然消失。
    //
    // 转移房主：先降级旧 op5，再升级新 op5（顺序不能反，否则会撞
    // rejected_op5_conflict）。
    if req.op == "op5" {
        let current_op5: Option<(String,)> =
            sqlx::query_as("SELECT login_id FROM members WHERE op = 'op5' AND login_id != ?")
                .bind(&login_id)
                .fetch_optional(&state.pool)
                .await?;
        if let Some((existing,)) = current_op5 {
            audit(&state, "set_op", &login_id, "rejected_op5_conflict").await;
            return Err(ApiError::Conflict(format!(
                "another op5 exists ({}); demote it first",
                existing
            )));
        }
    }
    let affected = sqlx::query("UPDATE members SET op = ?, updated_at = ? WHERE login_id = ?")
        .bind(&req.op)
        .bind(db::now_ms())
        .bind(&login_id)
        .execute(&state.pool)
        .await?
        .rows_affected();
    if affected == 0 {
        audit(&state, "set_op", &login_id, "not_found").await;
        return Err(ApiError::NotFound("user".into()));
    }
    // 广播：所有在线客户端立即看到新 op。
    if let Ok(row) = sqlx::query_as::<_, (String, String, String, String, String, String, i64)>(
        "SELECT login_id, nickname, op, bio, title, avatar_url, joined_at \
         FROM members WHERE login_id = ?",
    )
    .bind(&login_id)
    .fetch_one(&state.pool)
    .await
    {
        state.broadcast_all(kaede_protocol::ServerMessage::MemberUpdated {
            member: kaede_protocol::Member {
                login_id: row.0,
                nickname: row.1,
                op: row.2,
                bio: row.3,
                title: row.4,
                avatar_url: row.5,
                joined_at: row.6,
            },
        });
    }
    audit(&state, "set_op", &format!("{}={}", login_id, req.op), "ok").await;
    tracing::info!("[admin] set_op {} -> {}", login_id, req.op);
    Ok(StatusCode::NO_CONTENT)
}

/// 清理可选封禁原因：去控制字符、限长。
fn clean_reason(raw: Option<String>) -> Option<String> {
    raw.map(|s| {
        s.chars()
            .filter(|c| !c.is_control())
            .take(200)
            .collect::<String>()
    })
    .filter(|s| !s.trim().is_empty())
}

pub async fn ban_user(
    _: AdminAuth,
    State(state): State<AppState>,
    Path(login_id): Path<String>,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    let reason = clean_reason(
        serde_json::from_slice::<BanReq>(&body)
            .ok()
            .and_then(|r| r.reason),
    );
    let affected = sqlx::query(
        "UPDATE accounts SET banned = 1, banned_at = ?, banned_reason = ? WHERE login_id = ?",
    )
    .bind(db::now_ms())
    .bind(&reason)
    .bind(&login_id)
    .execute(&state.pool)
    .await?
    .rows_affected();
    if affected == 0 {
        audit(&state, "ban_user", &login_id, "not_found").await;
        return Err(ApiError::NotFound("user".into()));
    }
    sqlx::query("DELETE FROM sessions WHERE login_id = ?")
        .bind(&login_id)
        .execute(&state.pool)
        .await?;
    let uid = UserId::new(login_id.clone());
    if let Some(peer) = state.take_peer(&uid) {
        let _ = peer.tx.try_send(kaede_protocol::ServerMessage::Error {
            code: kaede_protocol::ErrorCode::Unauthenticated,
            message: "account banned".into(),
        });
    }
    // 封禁同样要吊销已建立的媒体连接（token 6 小时内仍然可用）。
    for room in state.media_revoke_user(&login_id) {
        crate::livekit::spawn_remove_participant(state.clone(), room, login_id.clone());
    }
    audit(&state, "ban_user", &login_id, "ok").await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn unban_user(
    _: AdminAuth,
    State(state): State<AppState>,
    Path(login_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let affected = sqlx::query(
        "UPDATE accounts SET banned = 0, banned_at = NULL, banned_reason = NULL \
         WHERE login_id = ?",
    )
    .bind(&login_id)
    .execute(&state.pool)
    .await?
    .rows_affected();
    if affected == 0 {
        audit(&state, "unban_user", &login_id, "not_found").await;
        return Err(ApiError::NotFound("user".into()));
    }
    audit(&state, "unban_user", &login_id, "ok").await;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- channels ----------

#[derive(Serialize)]
pub struct ChannelRow {
    pub id: String,
    pub name: String,
    pub is_private: bool,
    pub members: i64,
}

pub async fn list_channels(
    _: AdminAuth,
    State(state): State<AppState>,
) -> Result<Json<Vec<ChannelRow>>, ApiError> {
    let rows = sqlx::query(
        "SELECT c.id, c.name, c.is_private, \
                (SELECT COUNT(*) FROM channel_members cm WHERE cm.channel_id = c.id) AS members \
         FROM channels c ORDER BY c.id",
    )
    .fetch_all(&state.pool)
    .await?;
    let list: Vec<ChannelRow> = rows
        .into_iter()
        .map(|r| ChannelRow {
            id: r.get("id"),
            name: r.get("name"),
            is_private: r
                .try_get::<i64, _>("is_private")
                .map(|v| v != 0)
                .unwrap_or(false),
            members: r.try_get("members").unwrap_or(0),
        })
        .collect();
    Ok(Json(list))
}

pub async fn delete_channel(
    _: AdminAuth,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    if id == "general" || id == "voice" {
        return Err(ApiError::BadRequest(
            "cannot delete the built-in 'general' / 'voice' channel".into(),
        ));
    }

    let meta: Option<(i64,)> = sqlx::query_as("SELECT is_private FROM channels WHERE id = ?")
        .bind(&id)
        .fetch_optional(&state.pool)
        .await?;
    let Some((is_private,)) = meta else {
        audit(&state, "delete_channel", &id, "not_found").await;
        return Err(ApiError::NotFound("channel".into()));
    };
    let members: Vec<(String,)> =
        sqlx::query_as("SELECT login_id FROM channel_members WHERE channel_id = ?")
            .bind(&id)
            .fetch_all(&state.pool)
            .await?;

    let mut tx = state.pool.begin().await?;
    let affected = sqlx::query("DELETE FROM channels WHERE id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if affected == 0 {
        tx.rollback().await?;
        audit(&state, "delete_channel", &id, "not_found").await;
        return Err(ApiError::NotFound("channel".into()));
    }
    sqlx::query("DELETE FROM messages WHERE channel_id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM channel_members WHERE channel_id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM channel_reads WHERE channel_id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    crate::attachments::purge_target(&state.pool, &id).await;

    if is_private != 0 {
        for (member,) in members {
            state.send_to(
                &UserId::new(member),
                kaede_protocol::ServerMessage::ChannelDeleted { id: id.clone() },
            );
        }
    } else {
        state.broadcast_all(kaede_protocol::ServerMessage::ChannelDeleted { id: id.clone() });
    }

    audit(&state, "delete_channel", &id, "ok").await;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- logs ----------

#[derive(Deserialize)]
pub struct LogsQuery {
    #[serde(default = "default_log_limit")]
    pub limit: usize,
}

fn default_log_limit() -> usize {
    100
}

pub async fn list_logs(
    _: AdminAuth,
    State(state): State<AppState>,
    Query(q): Query<LogsQuery>,
) -> Result<Json<Vec<LogEntry>>, ApiError> {
    let limit = q.limit.clamp(1, 500);
    Ok(Json(state.log_buffer.snapshot(limit)))
}

// ---------- service ----------

pub async fn get_service(
    _: AdminAuth,
    State(_state): State<AppState>,
) -> Result<Json<ServiceStatus>, ApiError> {
    Ok(Json(crate::admin::service::detect()))
}

#[derive(Deserialize)]
pub struct ModeReq {
    pub mode: String,
}

#[derive(Serialize)]
pub struct ModeResp {
    pub mode: String,
    pub command: String,
    pub note: String,
}

pub async fn set_mode(
    _: AdminAuth,
    State(state): State<AppState>,
    Json(req): Json<ModeReq>,
) -> Result<Json<ModeResp>, ApiError> {
    match crate::admin::service::command_for_mode(&req.mode) {
        Ok((cmd, note)) => {
            audit(&state, "set_mode", &req.mode, "ok").await;
            Ok(Json(ModeResp {
                mode: req.mode,
                command: cmd,
                note,
            }))
        }
        Err(e) => {
            audit(&state, "set_mode", &req.mode, "rejected").await;
            Err(e)
        }
    }
}
