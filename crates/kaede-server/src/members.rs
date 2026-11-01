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
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024))
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
        && (n.chars().count() > 32 || n.trim().is_empty() || n.chars().any(|c| c.is_control()))
    {
        return Err(ApiError::BadRequest(
            "nickname 1..=32 chars, no control characters".into(),
        ));
    }
    if let Some(b) = &req.bio
        && (b.chars().count() > 2048 || b.chars().any(|c| c.is_control() && c != '\n' && c != '\t'))
    {
        return Err(ApiError::BadRequest(
            "bio too long or contains control characters".into(),
        ));
    }
    if let Some(t) = &req.title
        && (t.chars().count() > 64 || t.chars().any(|c| c.is_control()))
    {
        return Err(ApiError::BadRequest(
            "title too long or contains control characters".into(),
        ));
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

    // S-8: op 变更限制。
    //
    // 除了“不能升到比自己高”，还要求操作者**严格高于**目标（改自己除外）。
    // 否则 op4 可以先把同级 op4 降为 op2，再走删除接口绕过
    // “op4 不可删除同级账号”的约束。
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
        let my_rank = rank(&my_op);

        // S-8.1: 禁止 op5 降级自己（唯一服主自杀后只能靠 CLI promote 恢复）
        if is_self && my_op == "op5" && new_op_str != "op5" {
            return Err(ApiError::Unauthorized);
        }
        if rank(new_op_str) > my_rank {
            return Err(ApiError::Unauthorized);
        }
        let target_op = get_op(&state, &target).await?;
        let target_rank = rank(&target_op);
        if target_rank > my_rank || (!is_self && target_rank >= my_rank) {
            return Err(ApiError::Unauthorized);
        }
        // 唯一性：同一时刻只允许一个 op5（房主）。
        // 与 admin 接口保持一致；两个 op5 会各自持有主群 group state，
        // 后续 Welcome 指向不同 group_id，导致成员永久分裂。
        if new_op_str == "op5" {
            let existing: Option<(String,)> =
                sqlx::query_as("SELECT login_id FROM members WHERE op = 'op5' AND login_id != ?")
                    .bind(&target)
                    .fetch_optional(&state.pool)
                    .await?;
            if let Some((existing,)) = existing {
                return Err(ApiError::Conflict(format!(
                    "another op5 exists ({}); demote it first",
                    existing
                )));
            }
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

    let mut q = sqlx::query(sqlx::AssertSqlSafe(sql));
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

    // members 与 accounts 双写必须一起提交：登录/`/api/me` 读 accounts，
    // 好友/DM 列表读 members，否则改昵称后两边会长期不一致。
    let mut tx = state.pool.begin().await?;
    let affected = q.execute(&mut *tx).await?.rows_affected();
    if affected == 0 {
        tx.rollback().await?;
        return Err(ApiError::NotFound("member".into()));
    }
    if let Some(n) = &req.nickname {
        sqlx::query("UPDATE accounts SET nickname = ? WHERE login_id = ?")
            .bind(n)
            .bind(&target)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;

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

    // 必须严格高于目标 rank：op4 不能删 op5（服主）或其他 op4。
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
    let exists: Option<(String,)> = sqlx::query_as("SELECT op FROM members WHERE login_id = ?")
        .bind(&target)
        .fetch_optional(&state.pool)
        .await?;
    let target_op = exists.ok_or_else(|| ApiError::NotFound("member".into()))?.0;
    if rank(&my_op) <= rank(&target_op) {
        return Err(ApiError::Unauthorized);
    }

    // H15: 顺序改为「先 DB 事务（必须成功），后踢 WS」。
    //
    // 原顺序「先 take_peer 后 DB」在 DB 失败时会导致：
    //   用户被踢出内存（WS 断开）但账号仍然存在 → 重连成功 → 踢人失败。
    // 新顺序保证：DB 成功后才踢 WS；DB 失败则整个操作失败，peer 仍在。
    let mut tx = state.pool.begin().await?;

    // 删除前写墓碑：否则 accounts 行删除后 login_id 重新变成可注册，
    // 客户端“登录失败即自动注册”会把被删账号直接重建回来。
    sqlx::query(
        "INSERT INTO retired_logins (login_id, retired_at) VALUES (?, ?) \
         ON CONFLICT(login_id) DO UPDATE SET retired_at = excluded.retired_at",
    )
    .bind(&target)
    .bind(db::now_ms())
    .execute(&mut *tx)
    .await?;

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
    sqlx::query("DELETE FROM message_usage WHERE login_id = ?")
        .bind(&target)
        .execute(&mut *tx)
        .await?;

    // P1-14：清理 friend 双向关系，防止孤儿记录。
    sqlx::query("DELETE FROM friends WHERE login_id = ? OR friend_login_id = ?")
        .bind(&target)
        .bind(&target)
        .execute(&mut *tx)
        .await?;

    // R2-08：删除账号后不能继续留在频道成员表/离线信封/DM 索引里，
    // 否则私密频道的“权威成员”选择会命中已删除用户。
    sqlx::query("DELETE FROM channel_members WHERE login_id = ?")
        .bind(&target)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM pending_envelopes WHERE to_uid = ? OR from_uid = ?")
        .bind(&target)
        .bind(&target)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM dm_conversations WHERE a = ? OR b = ?")
        .bind(&target)
        .bind(&target)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    // DB 成功后再踢 WS。若 DB 已成功但踢 WS 失败，用户会短暂在线但
    // 下次 API 调用会因 session 已删而 401，不影响安全语义。
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
        }
    }

    // 踢人后必须吊销媒体访问：只删账号/断 WS 不会中断已经建立的 LiveKit
    // 连接，被踢者仍能在 token 有效期内继续收听语音/屏幕共享。
    for room in state.media_revoke_user(&target) {
        crate::livekit::spawn_remove_participant(state.clone(), room, target.clone());
    }

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
