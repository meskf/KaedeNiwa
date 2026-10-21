//! LiveKit token 生成 + HTTP API。

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::error::ApiError;
use crate::http::authenticate;
use crate::state::AppState;

type HmacSha256 = Hmac<Sha256>;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/livekit/token", post(issue_token))
        .route("/api/livekit/leave", post(leave_room))
        .route("/api/livekit/e2ee-key", post(issue_e2ee_key))
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024))
        .with_state(state)
}

#[derive(Deserialize)]
struct TokenReq {
    channel_id: String,
}

#[derive(Serialize)]
struct TokenResp {
    token: String,
    ws_url: String,
    room: String,
    identity: String,
    /// P0-2: 房间级 e2ee seed（base64 URL-safe no-pad）。
    /// 客户端用 `MLS exporter(channel_id, "livekit-e2ee", seed)` 派生真正的
    /// e2ee key。服务端不知道 exporter 输出（仍是 E2EE）；两次媒体会话
    /// seed 不同 → key 不同 → 防 LiveKit SDK ratchet 重启导致的 nonce 复用。
    e2ee_seed: String,
}

async fn issue_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<TokenReq>,
) -> Result<(StatusCode, Json<TokenResp>), ApiError> {
    let uid = authenticate(&state, &headers).await?;

    if req.channel_id.is_empty() {
        return Err(ApiError::BadRequest("channel_id required".into()));
    }
    // 该端点会创建 room seed / participant 状态；必须限流，防无限虚构房间。
    if !state.check_rate_scoped(&uid, "livekit_token", 2.0, 10.0) {
        return Err(ApiError::BadRequest("livekit token rate limited".into()));
    }

    check_channel_access(&state, uid.as_str(), &req.channel_id).await?;
    let api_key = state.cfg.livekit_api_key.clone();
    let api_secret = state.cfg.livekit_api_secret.clone();
    let ws_url = state.cfg.livekit_ws_url.clone();
    if api_key.is_empty() || api_secret.is_empty() {
        return Err(ApiError::Internal(
            "LIVEKIT_API_KEY/LIVEKIT_API_SECRET must be set (no default for security)".into(),
        ));
    }

    let room = format!("kaede-{}", req.channel_id);
    let identity = uid.as_str().to_string();

    // P0-2: 房间级 e2ee seed。
    //
    // seed 在“同一个媒体会话”内共享；最后一个参与者 leave 后条目被删除，
    // 下一次进入会生成全新 seed。这保证：
    // - 同一次通话内所有人 key 一致（只要 MLS epoch 也一致）；
    // - 跨通话 key 不同 → LiveKit SDK ratchet 从 0 重启也不会 nonce 复用。
    let seed: [u8; 32] = {
        use rand::RngCore;
        use std::collections::HashMap;
        let mut seeds = state.room_media_seeds.lock().unwrap_or_else(|e| e.into_inner());
        let now = std::time::Instant::now();
        let entry = seeds.entry(room.clone()).or_insert_with(|| {
            let mut s = [0u8; 32];
            rand::thread_rng().fill_bytes(&mut s);
            tracing::info!("[livekit] generated fresh e2ee seed for room={}", room);
            crate::state::RoomMedia {
                seed: s,
                participants: HashMap::new(),
                last_used: now,
            }
        });
        entry.participants.insert(uid.as_str().to_string(), now);
        entry.last_used = now;
        entry.seed
    };
    let e2ee_seed = URL_SAFE_NO_PAD.encode(seed);

    let token =
        make_jwt(&api_key, &api_secret, &room, &identity, &identity).map_err(ApiError::Internal)?;

    Ok((
        StatusCode::OK,
        Json(TokenResp {
            token,
            ws_url,
            room,
            identity,
            e2ee_seed,
        }),
    ))
}

pub fn make_jwt(
    api_key: &str,
    api_secret: &str,
    room: &str,
    identity: &str,
    display_name: &str,
) -> Result<String, String> {
    let now = now_secs();
    let exp = now + 6 * 3600;

    let header = serde_json::json!({ "alg": "HS256", "typ": "JWT" });
    let payload = serde_json::json!({
        "exp": exp,
        "iss": api_key,
        "nbf": now - 10,
        "sub": identity,
        "name": display_name,
        "video": {
            "room": room,
            "roomJoin": true,
            "canPublish": true,
            "canSubscribe": true,
            "canPublishData": true,
        }
    });

    let h = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).map_err(|e| e.to_string())?);
    let p = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).map_err(|e| e.to_string())?);
    let signing_input = format!("{}.{}", h, p);

    let mut mac = HmacSha256::new_from_slice(api_secret.as_bytes()).map_err(|e| e.to_string())?;
    mac.update(signing_input.as_bytes());
    let sig = mac.finalize().into_bytes();
    let s = URL_SAFE_NO_PAD.encode(sig);

    Ok(format!("{}.{}", signing_input, s))
}

// ============================================================
// E2EE key（确定性派生）
// ============================================================

/// 离开媒体房间。
///
/// 最后一个参与者离开时删除 seed，下次进入重新生成（防 nonce 复用）。
#[derive(Deserialize)]
struct LeaveReq {
    channel_id: String,
}

async fn leave_room(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<LeaveReq>,
) -> Result<StatusCode, ApiError> {
    let uid = authenticate(&state, &headers).await?;
    if req.channel_id.is_empty() {
        return Err(ApiError::BadRequest("channel_id required".into()));
    }
    let room = format!("kaede-{}", req.channel_id);
    {
        let mut seeds = state.room_media_seeds.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = seeds.get_mut(&room) {
            entry.participants.remove(uid.as_str());
            entry.last_used = std::time::Instant::now();
            if entry.participants.is_empty() {
                seeds.remove(&room);
                tracing::info!("[livekit] media session ended, seed dropped room={}", room);
            }
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct E2eeKeyReq {
    channel_id: String,
}

#[derive(Serialize)]
#[allow(dead_code)]
struct E2eeKeyResp {
    key: String,
    algorithm: String,
    key_size: u32,
}

/// 派生 E2EE 共享密钥。
///
/// **确定性**：`HMAC-SHA256(LIVEKIT_API_SECRET, "kaede-e2ee-v1:" || channel_id)`
/// → 32 字节。同一 channel_id 永远得到同一个 key，不同 channel_id 得到不同 key。
///
/// 这样 server 无需为每个房间存储 key 状态，房间内所有成员拿到的 key 一致。
async fn issue_e2ee_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<E2eeKeyReq>,
) -> Result<(StatusCode, Json<E2eeKeyResp>), ApiError> {
    let uid = authenticate(&state, &headers).await?;

    if req.channel_id.is_empty() {
        return Err(ApiError::BadRequest("channel_id required".into()));
    }

    // P0-Z (2026-10-19): 本端点已下线。key 派生是确定性的，配合
    // LiveKit SDK ratchet 重启归零会造成 nonce 复用（密码学灾难）。
    // 客户端请改用 MLS exporter: RoomSession::begin_media_session
    let _ = check_channel_access(&state, uid.as_str(), &req.channel_id).await;
    tracing::warn!(
        "deprecated endpoint /api/livekit/e2ee-key called by uid={} channel={}; returning 410 Gone",
        uid,
        req.channel_id
    );
    Err(ApiError::Gone(
        "server-side e2ee-key endpoint has been removed (P0-Z security fix: deterministic key derivation + LiveKit SDK ratchet restart -> nonce reuse). Upgrade client to use MLS exporter: RoomSession::begin_media_session".into(),
    ))
}

/// 与 client / example 共享的派生算法。
///
/// ```text
/// key = HMAC-SHA256(api_secret, "kaede-e2ee-v1:" || channel_id)
/// ```
#[allow(dead_code)]
pub fn derive_e2ee_key(api_secret: &str, channel_id: &str) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(api_secret.as_bytes()).expect("hmac init");
    mac.update(b"kaede-e2ee-v1:");
    mac.update(channel_id.as_bytes());
    let out = mac.finalize().into_bytes();
    let mut key = [0u8; 32];
    key.copy_from_slice(&out);
    key
}

fn now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// 读必需的环境变量，缺失则返回 500（绝不用弱默认值）。
/// 检查 uid 是否有权访问 channel_id。
///
/// - DM 格式 (`dm:user1:user2`)：仅双方可访问
/// - 频道 id：必须存在于 channels 表
async fn check_channel_access(
    state: &AppState,
    uid: &str,
    channel_id: &str,
) -> Result<(), ApiError> {
    if let Some((a, b)) = crate::dm::parse_conv_id(channel_id) {
        if !crate::dm::is_member(channel_id, uid) {
            return Err(ApiError::Unauthorized);
        }
        // 不能只做字符串校验：否则 `dm:自己:随机` 也能创建 seed/participant。
        let exists: Option<(i64,)> = sqlx::query_as(
            "SELECT 1 FROM dm_conversations WHERE \
             (a = ? AND b = ?) OR (a = ? AND b = ?)",
        )
        .bind(a)
        .bind(b)
        .bind(b)
        .bind(a)
        .fetch_optional(&state.pool)
        .await
        .map_err(|e| ApiError::Internal(format!("db: {e}")))?;
        if exists.is_none() {
            // 新会话在发送第一条消息前还没有索引行；要求双方账号都真实存在。
            let accounts: (i64,) =
                sqlx::query_as("SELECT COUNT(*) FROM accounts WHERE login_id IN (?, ?)")
                    .bind(a)
                    .bind(b)
                    .fetch_one(&state.pool)
                    .await
                    .map_err(|e| ApiError::Internal(format!("db: {e}")))?;
            if accounts.0 < 2 {
                return Err(ApiError::NotFound("dm conversation".into()));
            }
        }
        return Ok(());
    }
    let exists: Option<(String,)> = sqlx::query_as("SELECT id FROM channels WHERE id = ?")
        .bind(channel_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(|e| ApiError::Internal(format!("db: {e}")))?;
    if exists.is_none() {
        return Err(ApiError::NotFound(format!("channel {channel_id}")));
    }

    // B2 Phase 1：私密频道媒体 token 必须校验 membership
    let is_private: (i64,) = sqlx::query_as("SELECT is_private FROM channels WHERE id = ?")
        .bind(channel_id)
        .fetch_one(&state.pool)
        .await
        .map_err(|e| ApiError::Internal(format!("db: {e}")))?;
    if is_private.0 != 0 {
        let member: Option<(i64,)> =
            sqlx::query_as("SELECT 1 FROM channel_members WHERE channel_id = ? AND login_id = ?")
                .bind(channel_id)
                .bind(uid)
                .fetch_optional(&state.pool)
                .await
                .map_err(|e| ApiError::Internal(format!("db: {e}")))?;
        if member.is_none() {
            return Err(ApiError::Unauthorized);
        }
    }

    Ok(())
}
