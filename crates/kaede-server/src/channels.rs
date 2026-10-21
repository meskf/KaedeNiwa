//! 频道 HTTP API。
//!
//! 频道在服务器端真存（不是硬编码）。所有成员都能读；**op3+ 才能写**。

use argon2::Argon2;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::get,
};
use serde::{Deserialize, Serialize};

use kaede_protocol::{Channel, ServerMessage, UserId};

use crate::db;
use crate::error::ApiError;
use crate::http::authenticate;
use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/channels", get(list_channels).post(create_channel))
        .route(
            "/api/channels/{id}",
            axum::routing::patch(update_channel).delete(delete_channel),
        )
        .route("/api/channels/{id}/join", axum::routing::post(join_channel))
        .route(
            "/api/channels/{id}/leave",
            axum::routing::delete(leave_channel),
        )
        .with_state(state)
}

#[derive(Deserialize)]
struct CreateChannelReq {
    name: String,
    kind: String,
    #[serde(default)]
    is_private: bool,
    /// 私密频道必填。明文只在 create_channel 一处落库前 hash。
    #[serde(default)]
    password: Option<String>,
    /// 哈希强度档位："19"（默认）/ "64" / "256"。
    #[serde(default)]
    hash_tier: Option<String>,
}

#[derive(Deserialize)]
struct UpdateChannelReq {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    position: Option<i64>,
}

#[derive(Serialize)]
struct ChannelsResp {
    channels: Vec<Channel>,
}

async fn list_channels(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ChannelsResp>, ApiError> {
    let uid = authenticate(&state, &headers).await?;

    // 私密频道只对已加入的成员可见。
    type ChannelRow = (String, String, String, i64, i64, i64, Option<String>);
    let rows: Vec<ChannelRow> = sqlx::query_as(
        "SELECT id, name, kind, position, created_at, is_private, hash_tier \
         FROM channels \
         WHERE is_private = 0 \
            OR id IN (SELECT channel_id FROM channel_members WHERE login_id = ?) \
         ORDER BY position, id",
    )
    .bind(uid.as_str())
    .fetch_all(&state.pool)
    .await?;

    let channels = rows
        .into_iter()
        .map(
            |(id, name, kind, position, created_at, is_private, hash_tier)| Channel {
                id,
                name,
                kind,
                position,
                created_at,
                is_private: is_private != 0,
                hash_tier,
            },
        )
        .collect();

    Ok(Json(ChannelsResp { channels }))
}

async fn create_channel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateChannelReq>,
) -> Result<(StatusCode, Json<Channel>), ApiError> {
    let uid = authenticate(&state, &headers).await?;
    check_write_perm(&state, uid.as_str()).await?;

    let name = req.name.trim().to_string();
    if name.is_empty() || name.len() > 64 {
        return Err(ApiError::BadRequest("channel name 1..=64".into()));
    }
    if req.kind != "text" && req.kind != "voice" {
        return Err(ApiError::BadRequest("kind must be text|voice".into()));
    }

    // 私密频道必须提供 password，且长度 4..=128。
    let password_hash = if req.is_private {
        // 创建私密频道会跑 Argon2（最高 256 MiB）。必须和 login 一样限流 + 限并发，
        // 否则 op3+ 用户可用并发建频道打爆内存。
        if !state.check_rate(&uid, 0.2, 3.0) {
            return Err(ApiError::BadRequest("create_channel rate limited".into()));
        }
        let _permit = state
            .login_semaphore
            .acquire()
            .await
            .map_err(|_| ApiError::Internal("create_channel queue closed".into()))?;

        let pw = req
            .password
            .as_deref()
            .ok_or_else(|| ApiError::BadRequest("is_private requires password".into()))?;
        if pw.len() < 4 || pw.len() > 128 {
            return Err(ApiError::BadRequest("password length 4..=128".into()));
        }
        let (hash, tier_str) = hash_channel_password(pw, req.hash_tier.as_deref())?;
        Some((hash, tier_str))
    } else {
        None
    };
    let (password_hash, hash_tier) = match password_hash {
        Some((h, t)) => (Some(h), Some(t)),
        None => (None, None),
    };

    let id = format!("ch_{}", &db::random_token()[..10]);
    let now = db::now_ms();

    // 计算新 position
    let max_pos: Option<(i64,)> =
        sqlx::query_as("SELECT COALESCE(MAX(position), -1) FROM channels")
            .fetch_optional(&state.pool)
            .await?;
    let position = max_pos.map(|(p,)| p + 1).unwrap_or(0);

    // 事务：INSERT channel + 创建者自动加入 channel_members
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "INSERT INTO channels (id, name, kind, position, created_at, is_private, password_hash, hash_tier) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&name)
    .bind(&req.kind)
    .bind(position)
    .bind(now)
    .bind(if req.is_private { 1i64 } else { 0i64 })
    .bind(password_hash.as_deref())
    .bind(hash_tier.as_deref())
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT OR IGNORE INTO channel_members (channel_id, login_id, joined_at) \
         VALUES (?, ?, ?)",
    )
    .bind(&id)
    .bind(uid.as_str())
    .bind(now)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    let channel = Channel {
        id,
        name,
        kind: req.kind,
        position,
        created_at: now,
        is_private: req.is_private,
        hash_tier: hash_tier.clone(),
    };

    // 私密频道只广播给成员，避免向全服泄露频道名/存在性。
    broadcast_channel_scoped(
        &state,
        &channel.id,
        req.is_private,
        ServerMessage::ChannelCreated {
            channel: channel.clone(),
        },
    )
    .await;

    Ok((StatusCode::CREATED, Json(channel)))
}

async fn update_channel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<UpdateChannelReq>,
) -> Result<Json<Channel>, ApiError> {
    let uid = authenticate(&state, &headers).await?;
    check_write_perm(&state, uid.as_str()).await?;

    let row: Option<(String, String, i64, i64, i64, Option<String>)> = sqlx::query_as(
        "SELECT name, kind, position, created_at, is_private, hash_tier FROM channels WHERE id = ?",
    )
    .bind(&id)
    .fetch_optional(&state.pool)
    .await?;
    let (mut name, kind, mut position, created_at, is_private_raw, hash_tier) =
        row.ok_or_else(|| ApiError::NotFound("channel".into()))?;
    let is_private = is_private_raw != 0;

    if let Some(n) = req.name {
        let n = n.trim().to_string();
        if n.is_empty() || n.len() > 64 {
            return Err(ApiError::BadRequest("channel name 1..=64".into()));
        }
        name = n;
        sqlx::query("UPDATE channels SET name = ? WHERE id = ?")
            .bind(&name)
            .bind(&id)
            .execute(&state.pool)
            .await?;
    }

    if let Some(p) = req.position {
        // M28: 上限校验，防 create_channel 的 MAX(position)+1 溢出
        if !(0..=i32::MAX as i64).contains(&p) {
            return Err(ApiError::BadRequest("position must be 0..=i32::MAX".into()));
        }
        position = p;
        sqlx::query("UPDATE channels SET position = ? WHERE id = ?")
            .bind(position)
            .bind(&id)
            .execute(&state.pool)
            .await?;
    }

    let channel = Channel {
        id,
        name,
        kind,
        position,
        created_at,
        is_private,
        hash_tier,
    };
    broadcast_channel_scoped(
        &state,
        &channel.id,
        is_private,
        ServerMessage::ChannelUpdated {
            channel: channel.clone(),
        },
    )
    .await;
    Ok(Json(channel))
}

async fn delete_channel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let uid = authenticate(&state, &headers).await?;
    check_write_perm(&state, uid.as_str()).await?;

    // 私密频道：非成员即使是 op3/op4 也不能删（防通过广播泄露的 id 越权删除）。
    // op5（服主）保留兜底删除能力。
    let meta: Option<(i64,)> = sqlx::query_as("SELECT is_private FROM channels WHERE id = ?")
        .bind(&id)
        .fetch_optional(&state.pool)
        .await?;
    let is_private = meta.ok_or_else(|| ApiError::NotFound("channel".into()))?.0 != 0;
    let mut private_members: Vec<String> = Vec::new();
    if is_private {
        let member: Option<(i64,)> =
            sqlx::query_as("SELECT 1 FROM channel_members WHERE channel_id = ? AND login_id = ?")
                .bind(&id)
                .bind(uid.as_str())
                .fetch_optional(&state.pool)
                .await?;
        if member.is_none() {
            let op: Option<(String,)> = sqlx::query_as("SELECT op FROM members WHERE login_id = ?")
                .bind(uid.as_str())
                .fetch_optional(&state.pool)
                .await?;
            if op.map(|(o,)| o).as_deref() != Some("op5") {
                return Err(ApiError::Unauthorized);
            }
        }
        // 必须在删除 channel_members 之前保存成员列表，否则删完就没人可通知了。
        private_members = sqlx::query_as::<_, (String,)>(
            "SELECT login_id FROM channel_members WHERE channel_id = ?",
        )
        .bind(&id)
        .fetch_all(&state.pool)
        .await?
        .into_iter()
        .map(|(m,)| m)
        .collect();
    }

    // P1-O：事务包裹，防"删 channel 成功但删 messages 崩溃"留孤儿
    let mut tx = state.pool.begin().await?;

    let affected = sqlx::query("DELETE FROM channels WHERE id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?
        .rows_affected();

    if affected == 0 {
        tx.rollback().await?;
        return Err(ApiError::NotFound("channel".into()));
    }

    sqlx::query("DELETE FROM messages WHERE channel_id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;

    // M32: 清理 channel_members，防孤儿行
    sqlx::query("DELETE FROM channel_members WHERE channel_id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    // 删除时成员关系已清理：私密频道用事先保存的成员列表通知，
    // 公开频道广播给所有人。
    if is_private {
        for member in private_members {
            state.send_to(
                &UserId::new(member),
                ServerMessage::ChannelDeleted { id: id.clone() },
            );
        }
    } else {
        state.broadcast_all(ServerMessage::ChannelDeleted { id: id.clone() });
    }

    Ok(StatusCode::NO_CONTENT)
}

/// 按频道可见性广播：私密频道只发给 `channel_members`，公开频道广播给所有人。
async fn broadcast_channel_scoped(
    state: &AppState,
    channel_id: &str,
    is_private: bool,
    msg: ServerMessage,
) {
    if !is_private {
        state.broadcast_all(msg);
        return;
    }
    let members: Vec<(String,)> =
        match sqlx::query_as("SELECT login_id FROM channel_members WHERE channel_id = ?")
            .bind(channel_id)
            .fetch_all(&state.pool)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("broadcast_channel_scoped query failed: {}", e);
                return;
            }
        };
    for (login_id,) in members {
        state.send_to(&UserId::new(login_id), msg.clone());
    }
}

#[derive(Deserialize)]
struct JoinChannelReq {
    password: String,
    /// base64 URL-safe 编码的 KeyPackage。B2：加入时提交，等待已有成员 add_member。
    #[serde(default)]
    key_package: Option<String>,
}

async fn join_channel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<JoinChannelReq>,
) -> Result<StatusCode, ApiError> {
    let uid = authenticate(&state, &headers).await?;

    // C10: per-uid 限流，防频道密码暴破
    if !state.check_rate(&uid, 0.5, 3.0) {
        return Err(ApiError::BadRequest("join_channel rate limited".into()));
    }
    // C10: 并发 Argon2 上限（同 login），防 OOM DoS
    let _permit = state
        .login_semaphore
        .acquire()
        .await
        .map_err(|_| ApiError::Internal("join_channel queue closed".into()))?;

    // C8: 若提交了 KP，先校验 credential == auth uid。
    // 无论首次加入还是重广播，KP 都会流到 channel group 的 add_member，
    // 恶意 credential 会污染 group state。
    if let Some(kp_b64) = &req.key_package {
        let kp_bytes = base64_decode(kp_b64)?;
        let cred = kaede_mls::extract_credential_from_kp(&kp_bytes)
            .map_err(|e| ApiError::BadRequest(format!("invalid keypackage: {}", e)))?;
        if cred != uid.as_str() {
            return Err(ApiError::Unauthorized);
        }
    }

    let row: Option<(i64, Option<String>)> =
        sqlx::query_as("SELECT is_private, password_hash FROM channels WHERE id = ?")
            .bind(&id)
            .fetch_optional(&state.pool)
            .await?;
    let (is_private_raw, pw_hash) = row.ok_or_else(|| ApiError::NotFound("channel".into()))?;

    // A1: 总是先验证密码（不再"已加入直接放行"）
    if is_private_raw != 0 {
        let stored = pw_hash
            .ok_or_else(|| ApiError::BadRequest("private channel without password_hash".into()))?;
        if !verify_channel_password(&req.password, &stored) {
            return Err(ApiError::Unauthorized);
        }
    }

    // 密码通过后：检查是否已在成员表
    let already: Option<(i64,)> =
        sqlx::query_as("SELECT 1 FROM channel_members WHERE channel_id = ? AND login_id = ?")
            .bind(&id)
            .bind(uid.as_str())
            .fetch_optional(&state.pool)
            .await?;
    if already.is_some() {
        // P1: 已加入成员再次带 KP 请求 → 可能是 group 丢失后重连
        if req.key_package.is_some() {
            let others: Vec<(String,)> = sqlx::query_as(
                "SELECT login_id FROM channel_members WHERE channel_id = ? AND login_id != ?",
            )
            .bind(&id)
            .bind(uid.as_str())
            .fetch_all(&state.pool)
            .await?;

            if others.is_empty() {
                // 单人频道：没人能补发 Welcome → 返回 409，client 自己重建 group
                tracing::info!(
                    "join_channel: solo channel {} rebuilt by {}",
                    id,
                    uid.as_str()
                );
                return Err(ApiError::Conflict(
                    "solo channel: local group lost, rebuild required".into(),
                ));
            }

            // 有其他成员 → 只请求**权威成员**（最早加入者）补发 Welcome。
            // 发给所有人会导致多人同时 add → group split。
            if let Some(kp_b64) = req.key_package.as_ref() {
                tracing::info!(
                    "join_channel: re-broadcast welcome request for {} to authority",
                    id
                );
                notify_authority(&state, &id, &uid, kp_b64, &others).await;
            }
        }
        return Ok(StatusCode::NO_CONTENT);
    }

    let kp_blob: Option<Vec<u8>> = match &req.key_package {
        Some(s) => Some(base64_decode(s)?),
        None => None,
    };

    // H18: INSERT + SELECT others 必须同一事务。
    // 原顺序（先 SELECT 再 INSERT）在并发 join 下：两个请求都看不到对方
    // → 各自广播 ChannelMemberJoined → 接收方 MLS group 并发 add 失败。
    //
    // 事务内 INSERT 先落地，SELECT 才能看到刚加入的自己 + 并发者（如已提交）。
    // **注**：SQLite WAL 下，BEGIN 在 T1 COMMIT 前开始的事务仍可能看不到
    // T1 的 INSERT。要完全串行化需 per-channel mutex 或 SELECT FOR UPDATE
    // （SQLite 不支持）。当前事务至少消除"自己 INSERT 失败但已广播"的问题。
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "INSERT OR IGNORE INTO channel_members (channel_id, login_id, joined_at, key_package) \
         VALUES (?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(uid.as_str())
    .bind(db::now_ms())
    .bind(kp_blob.as_deref())
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    // B2：只通知权威成员（最早加入者）执行 add + Welcome（事务外）。
    // 发给所有成员会触发多次独立 add → epoch 分叉。
    if let Some(kp_bytes) = &kp_blob {
        let kp_b64 = crate::db::base64_url(kp_bytes);
        let members: Vec<(String,)> = sqlx::query_as(
            "SELECT login_id FROM channel_members WHERE channel_id = ? AND login_id != ? \
             ORDER BY joined_at ASC",
        )
        .bind(&id)
        .bind(uid.as_str())
        .fetch_all(&state.pool)
        .await?;
        notify_authority(&state, &id, &uid, &kp_b64, &members).await;
    }

    Ok(StatusCode::NO_CONTENT)
}

/// 把“加入申请”按 `joined_at` 顺序投给第一个在线的成员（频道 group 权威）。
///
/// 权威成员的客户端负责 `add_member` → 发 Welcome → 广播 Commit。
async fn notify_authority(
    state: &AppState,
    channel_id: &str,
    requester: &UserId,
    key_package_b64: &str,
    members: &[(String,)],
) {
    for (member_id,) in members {
        let target = UserId::new(member_id);
        let sent = state.send_to(
            &target,
            ServerMessage::ChannelMemberJoined {
                channel_id: channel_id.to_string(),
                requester: requester.clone(),
                key_package: key_package_b64.to_string(),
            },
        );
        if sent {
            tracing::info!(
                "join_channel: authority {} handles requester {} for {}",
                member_id,
                requester,
                channel_id
            );
            return;
        }
    }

    // 没有在线权威成员：把“加入申请”持久化，等第一个成员上线 Join 时补投，
    // 否则创建者离线期间新成员永远无法入群。
    if let Some((authority,)) = members.first() {
        let payload = match serde_json::to_vec(&serde_json::json!({
            "requester": requester.as_str(),
            "key_package": key_package_b64,
        })) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("join_channel: encode pending authority request: {}", e);
                return;
            }
        };
        if let Err(e) = crate::db::enqueue_envelope(
            &state.pool,
            authority,
            requester.as_str(),
            "channel_member_joined",
            Some(channel_id),
            &payload,
        )
        .await
        {
            tracing::warn!("join_channel: enqueue authority request failed: {}", e);
        } else {
            tracing::info!(
                "join_channel: queued authority request for {} (authority {} offline)",
                channel_id,
                authority
            );
        }
    } else {
        tracing::warn!(
            "join_channel: no authority for {} (requester {})",
            channel_id,
            requester
        );
    }
}

async fn leave_channel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let uid = authenticate(&state, &headers).await?;

    let meta: Option<(i64,)> = sqlx::query_as("SELECT is_private FROM channels WHERE id = ?")
        .bind(&id)
        .fetch_optional(&state.pool)
        .await?;
    let is_private = meta.map(|(v,)| v != 0).unwrap_or(false);

    // 先保存其余成员（删除后查不到），用于通知权威成员轮换频道密钥。
    let others: Vec<(String,)> = if is_private {
        sqlx::query_as(
            "SELECT login_id FROM channel_members WHERE channel_id = ? AND login_id != ? \
             ORDER BY joined_at ASC",
        )
        .bind(&id)
        .bind(uid.as_str())
        .fetch_all(&state.pool)
        .await?
    } else {
        Vec::new()
    };

    sqlx::query("DELETE FROM channel_members WHERE channel_id = ? AND login_id = ?")
        .bind(&id)
        .bind(uid.as_str())
        .execute(&state.pool)
        .await?;

    if is_private {
        let fallback = others.first().map(|(m,)| m.clone());
        let mut delivered = false;
        for (member,) in others {
            let sent = state.send_to(
                &UserId::new(member),
                ServerMessage::ChannelMemberLeft {
                    channel_id: id.clone(),
                    member: uid.clone(),
                },
            );
            if sent {
                delivered = true;
                break;
            }
        }
        // 权威成员不在线：暂存事件，等它下次 Join 时补投并轮换频道密钥。
        if !delivered
            && let Some(authority) = fallback
            && let Err(e) = crate::db::enqueue_envelope(
                &state.pool,
                &authority,
                uid.as_str(),
                "channel_member_left",
                Some(&id),
                uid.as_str().as_bytes(),
            )
            .await
        {
            tracing::warn!("enqueue channel_member_left failed: {}", e);
        }
    }

    Ok(StatusCode::NO_CONTENT)
}

fn base64_decode(s: &str) -> Result<Vec<u8>, ApiError> {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    URL_SAFE_NO_PAD
        .decode(s)
        .map_err(|e| ApiError::BadRequest(format!("base64: {}", e)))
}

/// 按档位选 Argon2 参数。返回 (argon2 实例, 归一化后的档位字符串)。
///
/// 档位常量：
/// - "19"  Standard  19 MiB / 2 iter（OWASP 最低合规，≈20ms）
/// - "64"  Strong    64 MiB / 3 iter（OWASP 推荐，≈80ms）
/// - "256" Paranoid  256 MiB / 4 iter（高安全，≈400ms）
fn resolve_argon2(tier: Option<&str>) -> Result<(Argon2<'static>, &'static str), ApiError> {
    use argon2::{Algorithm, Params, Version};
    let tier_str = match tier {
        None | Some("") | Some("19") => "19",
        Some("64") => "64",
        Some("256") => "256",
        Some(other) => {
            return Err(ApiError::BadRequest(format!(
                "hash_tier must be 19/64/256, got {}",
                other
            )));
        }
    };
    let (m_kib, t, p) = match tier_str {
        "19" => (19 * 1024u32, 2u32, 1u32),
        "64" => (64 * 1024, 3, 1),
        "256" => (256 * 1024, 4, 1),
        _ => unreachable!(),
    };
    let params = Params::new(m_kib, t, p, None)
        .map_err(|e| ApiError::Internal(format!("argon2 params: {e}")))?;
    let a2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    Ok((a2, tier_str))
}

fn hash_channel_password(password: &str, tier: Option<&str>) -> Result<(String, String), ApiError> {
    use argon2::password_hash::{PasswordHasher, SaltString, rand_core::OsRng};
    let (a2, tier_str) = resolve_argon2(tier)?;
    let salt = SaltString::generate(&mut OsRng);
    let hash = a2
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| ApiError::BadRequest(format!("argon2 hash: {e}")))?;
    Ok((hash, tier_str.to_string()))
}

fn verify_channel_password(password: &str, hash: &str) -> bool {
    use argon2::password_hash::{PasswordHash, PasswordVerifier};
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

/// 检查用户是否有写入权限（op3 = 普管 及以上）。
///
/// **临时策略**：第一个注册用户自动 op5（服主），其余默认 op2（成员）。
/// 严格权限系统见后续阶段。
async fn check_write_perm(state: &AppState, uid: &str) -> Result<(), ApiError> {
    let row: Option<(String,)> = sqlx::query_as("SELECT op FROM members WHERE login_id = ?")
        .bind(uid)
        .fetch_optional(&state.pool)
        .await?;
    let (op,) = row.ok_or_else(|| ApiError::NotFound("member".into()))?;
    // op1=访客 op2=成员 op3=普管 op4=超管 op5=服主
    let allowed = matches!(op.as_str(), "op3" | "op4" | "op5");
    if !allowed {
        return Err(ApiError::Unauthorized);
    }
    Ok(())
}
