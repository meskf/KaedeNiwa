//! 成员 HTTP API。
//!
//! - 所有成员可读
//! - **op4+ 才能改他人**（改 op、踢人）
//! - 任何人可以改自己的 nickname / bio / title / avatar_url

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::get,
};
use serde::{Deserialize, Serialize};

use kaede_protocol::{ErrorCode, Member, ServerMessage, UserId};

use crate::db;
use crate::error::ApiError;
use crate::http::authenticate;
use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/members", get(list_members))
        .route(
            "/api/members/{login_id}",
            axum::routing::patch(update_member).delete(remove_member),
        )
        .with_state(state)
}

#[derive(Deserialize)]
struct UpdateMemberReq {
    #[serde(default)]
    nickname: Option<String>,
    #[serde(default)]
    bio: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    avatar_url: Option<String>,
    /// 只有 op4+ 能改
    #[serde(default)]
    op: Option<String>,
}

#[derive(Serialize)]
struct MembersResp {
    members: Vec<Member>,
}

async fn list_members(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<MembersResp>, ApiError> {
    let _uid = authenticate(&state, &headers).await?;

    let rows: Vec<(String, String, String, String, String, String, i64)> = sqlx::query_as(
        "SELECT login_id, nickname, op, bio, title, avatar_url, joined_at \
         FROM members ORDER BY joined_at",
    )
    .fetch_all(&state.pool)
    .await?;

    let members = rows
        .into_iter()
        .map(
            |(login_id, nickname, op, bio, title, avatar_url, joined_at)| Member {
                login_id,
                nickname,
                op,
                bio,
                title,
                avatar_url,
                joined_at,
            },
        )
        .collect();

    Ok(Json(MembersResp { members }))
}

async fn update_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(target): Path<String>,
    Json(req): Json<UpdateMemberReq>,
) -> Result<Json<Member>, ApiError> {
    let uid = authenticate(&state, &headers).await?;

    let is_self = uid.as_str() == target;
    let my_op = get_op(&state, uid.as_str()).await?;
    let is_admin = matches!(my_op.as_str(), "op4" | "op5");

    // 权限：改自己 OR 是 op4+
    if !is_self && !is_admin {
        return Err(ApiError::Unauthorized);
    }

    // S-5: 字段长度 + 控制字符 + URL scheme 校验
    if let Some(n) = &req.nickname
        && (n.chars().count() > 32 || n.trim().is_empty())
    {
        return Err(ApiError::BadRequest("nickname 1..=32 chars".into()));
    }
    if let Some(b) = &req.bio
        && b.chars().count() > 2048
    {
        return Err(ApiError::BadRequest("bio too long (max 2048)".into()));
    }
    if let Some(t) = &req.title
        && t.chars().count() > 64
    {
        return Err(ApiError::BadRequest("title too long (max 64)".into()));
    }
    if let Some(a) = &req.avatar_url {
        if a.chars().count() > 512 {
            return Err(ApiError::BadRequest("avatar_url too long (max 512)".into()));
        }
        if !a.is_empty() {
            let lower = a.to_ascii_lowercase();
            // P1-AD：显式禁止 SVG（可含 <script>，某些渲染器会执行）
            if lower.starts_with("data:image/svg") {
                return Err(ApiError::BadRequest(
                    "svg avatars are not allowed (XSS risk); use png/jpeg/webp".into(),
                ));
            }
            let allowed = lower.starts_with("http://")
                || lower.starts_with("https://")
                || lower.starts_with("data:image/png")
                || lower.starts_with("data:image/jpeg")
                || lower.starts_with("data:image/gif")
                || lower.starts_with("data:image/webp");
            if !allowed {
                return Err(ApiError::BadRequest(
                    "avatar_url must be http(s) or data:image/{png,jpeg,gif,webp}".into(),
                ));
            }
        }
    }

    // S-8: op 升级限制——不能升到比自己高，也不能降级 op5
    if req.op.is_some() {
        let rank = |s: &str| -> u8 {
            match s {
                "op1" => 1,
                "op2" => 2,
                "op3" => 3,
                "op4" => 4,
                "op5" => 5,
                _ => 0,
            }
        };
        let new_op_str = req.op.as_deref().unwrap_or("op2");
        if rank(new_op_str) > rank(&my_op) {
            return Err(ApiError::Unauthorized);
        }
        let target_op = get_op(&state, &target).await?;
        if target_op == "op5" && my_op != "op5" {
            return Err(ApiError::Unauthorized);
        }
    }

    // op 只能 op4+ 改
    if req.op.is_some() && !is_admin {
        return Err(ApiError::Unauthorized);
    }

    // 校验 op 值
    if let Some(ref op) = req.op
        && !matches!(op.as_str(), "op1" | "op2" | "op3" | "op4" | "op5")
    {
        return Err(ApiError::BadRequest("invalid op".into()));
    }

    // 拼动态 UPDATE
    let now = db::now_ms();
    let mut sets = Vec::new();
    if req.nickname.is_some() {
        sets.push("nickname = ?");
    }
    if req.bio.is_some() {
        sets.push("bio = ?");
    }
    if req.title.is_some() {
        sets.push("title = ?");
    }
    if req.avatar_url.is_some() {
        sets.push("avatar_url = ?");
    }
    if req.op.is_some() {
        sets.push("op = ?");
    }
    if sets.is_empty() {
        return Err(ApiError::BadRequest("nothing to update".into()));
    }
    sets.push("updated_at = ?");
    let sql = format!("UPDATE members SET {} WHERE login_id = ?", sets.join(", "));

    let mut q = sqlx::query(&sql);
    if let Some(n) = &req.nickname {
        q = q.bind(n);
    }
    if let Some(b) = &req.bio {
        q = q.bind(b);
    }
    if let Some(t) = &req.title {
        q = q.bind(t);
    }
    if let Some(a) = &req.avatar_url {
        q = q.bind(a);
    }
    if let Some(o) = &req.op {
        q = q.bind(o);
    }
    q = q.bind(now);
    q = q.bind(&target);

    let affected = q.execute(&state.pool).await?.rows_affected();
    if affected == 0 {
        return Err(ApiError::NotFound("member".into()));
    }

    // 返回更新后的记录
    let (login_id, nickname, op, bio, title, avatar_url, joined_at): (
        String,
        String,
        String,
        String,
        String,
        String,
        i64,
    ) = sqlx::query_as(
        "SELECT login_id, nickname, op, bio, title, avatar_url, joined_at \
         FROM members WHERE login_id = ?",
    )
    .bind(&target)
    .fetch_one(&state.pool)
    .await?;

    let member = Member {
        login_id,
        nickname,
        op,
        bio,
        title,
        avatar_url,
        joined_at,
    };
    state.broadcast_all(ServerMessage::MemberUpdated {
        member: member.clone(),
    });
    Ok(Json(member))
}

async fn remove_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(target): Path<String>,
) -> Result<StatusCode, ApiError> {
    let uid = authenticate(&state, &headers).await?;
    let my_op = get_op(&state, uid.as_str()).await?;
    let is_admin = matches!(my_op.as_str(), "op4" | "op5");

    if !is_admin {
        return Err(ApiError::Unauthorized);
    }
    if target == uid.as_str() {
        return Err(ApiError::BadRequest("cannot remove self".into()));
    }

    // 先向被踢者发一条 Kicked 错误并断开其 WS。
    // 这一步必须在 DB 删除之前做——之后 take_peer 拿不到 handle。
    {
        let target_uid = UserId::new(target.clone());
        if let Some(peer) = state.take_peer(&target_uid) {
            if peer
                .tx
                .try_send(ServerMessage::Error {
                    code: ErrorCode::Kicked,
                    message: "you have been removed from this server".into(),
                })
                .is_err()
            {
                tracing::warn!("[kick] failed to notify {}", target);
            }
            tracing::info!("[kick] notified and disconnected {}", target);
            // peer 在这里被 drop，tx 随之失效；
            // run_socket 的 writer task 会在 sink 关闭后退出。
            // 被踢者的 run_event_loop 会先收到 fatal Error 主动 close。
        }
    }

    // 一并删除账户（踢人 = 删号，旧项目语义）
    // S-6: 事务包裹所有 DELETE，避免部分提交导致数据不一致
    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM members WHERE login_id = ?")
        .bind(&target)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM accounts WHERE login_id = ?")
        .bind(&target)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE login_id = ?")
        .bind(&target)
        .execute(&mut *tx)
        .await?;
    // 清理该用户的 KeyPackage 队列，避免被误用
    sqlx::query("DELETE FROM key_packages WHERE login_id = ?")
        .bind(&target)
        .execute(&mut *tx)
        .await?;

    // P1-14：清理 friend 双向关系，防止孤儿记录。
    sqlx::query("DELETE FROM friends WHERE login_id = ? OR friend_login_id = ?")
        .bind(&target)
        .bind(&target)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    state.broadcast_all(ServerMessage::MemberRemoved { login_id: target });

    Ok(StatusCode::NO_CONTENT)
}

async fn get_op(state: &AppState, uid: &str) -> Result<String, ApiError> {
    let row: Option<(String,)> = sqlx::query_as("SELECT op FROM members WHERE login_id = ?")
        .bind(uid)
        .fetch_optional(&state.pool)
        .await?;
    Ok(row.map(|(o,)| o).unwrap_or_else(|| "op2".into()))
}
