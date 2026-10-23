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
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::error::ApiError;
use crate::http::authenticate;
use crate::state::AppState;

type HmacSha256 = Hmac<Sha256>;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/livekit/token", post(issue_token))
        .route("/api/livekit/leave", post(leave_room))
        .route("/api/livekit/e2ee-key", post(issue_e2ee_key))
        // LiveKit 服务端回调（participant_left / room_finished）。body 可能包含
        // 完整 room 信息，上限放宽到 1 MiB；认证由 JWT 签名保证。
        .route(
            "/api/livekit/webhook",
            post(webhook).layer(axum::extract::DefaultBodyLimit::max(1024 * 1024)),
        )
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

    let room = room_name_for(&req.channel_id);
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
        let mut seeds = state
            .room_media_seeds
            .lock()
            .unwrap_or_else(|e| e.into_inner());
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

    sign_jwt(
        api_secret,
        &serde_json::json!({
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
        }),
    )
}

/// 用 HS256 对 payload 签名，返回 compact JWT。
fn sign_jwt(api_secret: &str, payload: &serde_json::Value) -> Result<String, String> {
    let header = serde_json::json!({ "alg": "HS256", "typ": "JWT" });
    let h = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).map_err(|e| e.to_string())?);
    let p = URL_SAFE_NO_PAD.encode(serde_json::to_vec(payload).map_err(|e| e.to_string())?);
    let signing_input = format!("{}.{}", h, p);

    let mut mac = HmacSha256::new_from_slice(api_secret.as_bytes()).map_err(|e| e.to_string())?;
    mac.update(signing_input.as_bytes());
    let sig = mac.finalize().into_bytes();
    let s = URL_SAFE_NO_PAD.encode(sig);

    Ok(format!("{}.{}", signing_input, s))
}

// ============================================================
// LiveKit 管理 API（踢人 / 回收媒体会话）
// ============================================================

/// LiveKit 房间名与频道 id 的映射（单点定义，避免各处手写前缀）。
pub fn room_name_for(channel_id: &str) -> String {
    format!("kaede-{}", channel_id)
}

/// 纯函数：从 WS URL 推导 HTTP 管理 API 的 base。
///
/// `wss://host:port/anything` → `https://host:port`；`ws://` → `http://`。
fn admin_http_base_from_ws(ws_url: &str) -> Option<String> {
    let (scheme, rest) = if let Some(r) = ws_url.strip_prefix("wss://") {
        ("https", r)
    } else {
        ("http", ws_url.strip_prefix("ws://")?)
    };
    let host = rest.split('/').next().unwrap_or_default();
    if host.is_empty() {
        return None;
    }
    Some(format!("{scheme}://{host}"))
}

/// LiveKit HTTP 管理 API base。`KAEDE_LIVEKIT_HTTP_URL` 显式覆盖（端口不同场景）。
fn admin_http_base(ws_url: &str) -> Option<String> {
    if let Ok(v) = std::env::var("KAEDE_LIVEKIT_HTTP_URL") {
        let v = v.trim().trim_end_matches('/');
        if !v.is_empty() {
            return Some(v.to_string());
        }
    }
    admin_http_base_from_ws(ws_url)
}

fn http_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

/// 把某个 identity 从 LiveKit 房间踢出。
///
/// 用于封禁 / 踢人 / 私密频道移除成员：只删 DB 行与 MLS 成员不会中断
/// 已建立的媒体连接（token 有效期 6 小时），被移除者仍能听到该房间的音视频。
///
/// 参与者已经不在房间时 LiveKit 返回 404，视为成功。
pub async fn remove_participant(
    state: &AppState,
    room: &str,
    identity: &str,
) -> Result<(), String> {
    let api_key = state.cfg.livekit_api_key.clone();
    let api_secret = state.cfg.livekit_api_secret.clone();
    if api_key.is_empty() || api_secret.is_empty() {
        return Err("LIVEKIT_API_KEY/LIVEKIT_API_SECRET not configured".into());
    }
    let base = admin_http_base(&state.cfg.livekit_ws_url).ok_or_else(|| {
        format!(
            "cannot derive livekit http url from {}",
            state.cfg.livekit_ws_url
        )
    })?;

    let now = now_secs();
    let token = sign_jwt(
        &api_secret,
        &serde_json::json!({
            "exp": now + 300,
            "nbf": now - 10,
            "iss": api_key,
            "sub": api_key,
            "video": { "room": room, "roomAdmin": true, "roomJoin": false }
        }),
    )?;

    let resp = http_client()
        .post(format!(
            "{base}/twirp/livekit.RoomService/RemoveParticipant"
        ))
        .bearer_auth(token)
        .json(&serde_json::json!({ "room": room, "identity": identity }))
        .send()
        .await
        .map_err(|e| format!("livekit remove_participant request: {e}"))?;

    if resp.status().is_success() || resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(());
    }
    let status = resp.status();
    let body: String = resp
        .text()
        .await
        .unwrap_or_default()
        .chars()
        .take(200)
        .collect();
    Err(format!("livekit remove_participant {status}: {body}"))
}

/// 后台尽力踢人：不阻塞 HTTP 响应，失败只记日志。
///
/// 媒体访问的主要吊销手段是 seed 回收（同步完成）；这里只是把已经建立的
/// LiveKit 连接也断掉，失败不影响安全语义（对方拿不到新 key）。
pub fn spawn_remove_participant(state: AppState, room: String, identity: String) {
    tokio::spawn(async move {
        match remove_participant(&state, &room, &identity).await {
            Ok(()) => tracing::info!(
                "[livekit] removed participant identity={} room={}",
                identity,
                room
            ),
            Err(e) => tracing::warn!(
                "[livekit] remove_participant identity={} room={} failed: {}",
                identity,
                room,
                e
            ),
        }
    });
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
    let room = room_name_for(&req.channel_id);
    state.media_participant_left(&room, uid.as_str());
    Ok(StatusCode::NO_CONTENT)
}

// ============================================================
// LiveKit webhook
// ============================================================

#[derive(Deserialize)]
struct WebhookEvent {
    event: String,
    #[serde(default)]
    room: Option<WebhookRoom>,
    #[serde(default)]
    participant: Option<WebhookParticipant>,
}

#[derive(Deserialize)]
struct WebhookRoom {
    name: String,
}

#[derive(Deserialize)]
struct WebhookParticipant {
    identity: String,
}

/// LiveKit 服务端回调。
///
/// 崩溃/断网的参与者不会调用 `/leave`；依赖 LiveKit 的 `participant_left` /
/// `room_finished` 事件可以立即回收 seed，而不是等 8 小时 TTL。这样同一房间
/// 下一次会话一定得到新 E2EE key，避免 LiveKit ratchet 归零后的 nonce 复用。
async fn webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<StatusCode, ApiError> {
    let auth = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    verify_webhook_signature(
        &state.cfg.livekit_api_key,
        &state.cfg.livekit_api_secret,
        auth,
        &body,
    )?;

    let event: WebhookEvent = serde_json::from_slice(&body)
        .map_err(|e| ApiError::BadRequest(format!("bad webhook body: {e}")))?;
    let Some(room) = event.room.map(|r| r.name) else {
        return Ok(StatusCode::OK);
    };
    match event.event.as_str() {
        "participant_left" => {
            if let Some(p) = event.participant {
                state.media_participant_left(&room, &p.identity);
            }
        }
        "room_finished" => state.media_room_finished(&room),
        _ => {}
    }
    Ok(StatusCode::OK)
}

/// 校验 LiveKit webhook JWT：HS256 签名 + iss == API key + sha256(body) 匹配。
fn verify_webhook_signature(
    api_key: &str,
    api_secret: &str,
    auth_header: Option<&str>,
    body: &[u8],
) -> Result<(), ApiError> {
    if api_key.is_empty() || api_secret.is_empty() {
        return Err(ApiError::Internal(
            "LIVEKIT_API_KEY/LIVEKIT_API_SECRET not configured".into(),
        ));
    }
    let token = auth_header
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(ApiError::Unauthorized)?;
    let mut parts = token.split('.');
    let (header_b64, claims_b64, sig_b64) = match (parts.next(), parts.next(), parts.next()) {
        (Some(h), Some(c), Some(s)) if parts.next().is_none() => (h, c, s),
        _ => return Err(ApiError::Unauthorized),
    };

    // 先验签，再解析 claims，避免把未认证内容当可信输入。
    let signing_input = format!("{header_b64}.{claims_b64}");
    let mut mac = HmacSha256::new_from_slice(api_secret.as_bytes())
        .map_err(|_| ApiError::Internal("hmac init".into()))?;
    mac.update(signing_input.as_bytes());
    let signature = URL_SAFE_NO_PAD
        .decode(sig_b64.as_bytes())
        .map_err(|_| ApiError::Unauthorized)?;
    mac.verify_slice(&signature)
        .map_err(|_| ApiError::Unauthorized)?;

    let header: serde_json::Value = serde_json::from_slice(
        &URL_SAFE_NO_PAD
            .decode(header_b64.as_bytes())
            .map_err(|_| ApiError::Unauthorized)?,
    )
    .map_err(|_| ApiError::Unauthorized)?;
    if header.get("alg").and_then(|v| v.as_str()) != Some("HS256") {
        return Err(ApiError::Unauthorized);
    }

    let claims: serde_json::Value = serde_json::from_slice(
        &URL_SAFE_NO_PAD
            .decode(claims_b64.as_bytes())
            .map_err(|_| ApiError::Unauthorized)?,
    )
    .map_err(|_| ApiError::Unauthorized)?;
    if claims.get("iss").and_then(|v| v.as_str()) != Some(api_key) {
        return Err(ApiError::Unauthorized);
    }
    let now = now_secs();
    let exp = claims.get("exp").and_then(|v| v.as_i64()).unwrap_or(0);
    if exp != 0 && exp + 300 < now {
        return Err(ApiError::Unauthorized);
    }

    let Some(claim_hash) = claims.get("sha256").and_then(|v| v.as_str()) else {
        return Err(ApiError::Unauthorized);
    };
    let digest = Sha256::digest(body);
    let expected = base64::engine::general_purpose::STANDARD.encode(digest);
    if claim_hash.as_bytes().ct_eq(expected.as_bytes()).unwrap_u8() != 1 {
        return Err(ApiError::Unauthorized);
    }
    Ok(())
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
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
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

#[cfg(test)]
mod webhook_tests {
    use super::*;

    fn make_token(secret: &str, body: &[u8], iss: &str) -> String {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"HS256","typ":"JWT"}"#);
        let claims = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&serde_json::json!({
                "iss": iss,
                "exp": now_secs() + 60,
                "sha256": base64::engine::general_purpose::STANDARD
                    .encode(Sha256::digest(body)),
            }))
            .unwrap(),
        );
        let input = format!("{header}.{claims}");
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(input.as_bytes());
        let sig = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
        format!("{input}.{sig}")
    }

    #[test]
    fn accepts_valid_signature_and_body_hash() {
        let body = br#"{"event":"participant_left"}"#;
        let token = make_token("secret", body, "key");
        let auth = format!("Bearer {token}");
        assert!(verify_webhook_signature("key", "secret", Some(&auth), body).is_ok());
    }

    #[test]
    fn rejects_body_tamper() {
        let body = br#"{"event":"participant_left"}"#;
        let token = make_token("secret", body, "key");
        let auth = format!("Bearer {token}");
        assert!(
            verify_webhook_signature(
                "key",
                "secret",
                Some(&auth),
                br#"{"event":"room_finished"}"#
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_wrong_secret_or_issuer() {
        let body = b"{}";
        let token = make_token("other", body, "key");
        let auth = format!("Bearer {token}");
        assert!(verify_webhook_signature("key", "secret", Some(&auth), body).is_err());

        let token = make_token("secret", body, "other-key");
        let auth = format!("Bearer {token}");
        assert!(verify_webhook_signature("key", "secret", Some(&auth), body).is_err());
    }

    #[test]
    fn rejects_missing_or_malformed_auth() {
        assert!(verify_webhook_signature("key", "secret", None, b"{}").is_err());
        assert!(verify_webhook_signature("key", "secret", Some("Bearer a.b"), b"{}").is_err());
        assert!(verify_webhook_signature("key", "", Some("Bearer a.b.c"), b"{}").is_err());
    }

    // ---- 管理 API：URL 推导与 admin JWT ----

    #[test]
    fn admin_http_base_maps_ws_scheme() {
        assert_eq!(
            admin_http_base_from_ws("ws://localhost:18082").as_deref(),
            Some("http://localhost:18082")
        );
        assert_eq!(
            admin_http_base_from_ws("wss://lk.example.com").as_deref(),
            Some("https://lk.example.com")
        );
        // 带路径（LiveKit 的 rtc 路由）时只取 host。
        assert_eq!(
            admin_http_base_from_ws("wss://lk.example.com/rtc").as_deref(),
            Some("https://lk.example.com")
        );
        assert!(admin_http_base_from_ws("http://lk.example.com").is_none());
        assert!(admin_http_base_from_ws("").is_none());
    }

    #[test]
    fn room_admin_jwt_has_admin_grant_and_valid_signature() {
        let token = sign_jwt(
            "secret",
            &serde_json::json!({
                "exp": now_secs() + 60,
                "iss": "key",
                "video": { "room": "kaede-general", "roomAdmin": true, "roomJoin": false }
            }),
        )
        .unwrap();

        // 签名必须能被同一 secret 复算出来。
        let mut parts = token.split('.');
        let (h, c, s) = (
            parts.next().unwrap(),
            parts.next().unwrap(),
            parts.next().unwrap(),
        );
        let mut mac = HmacSha256::new_from_slice(b"secret").unwrap();
        mac.update(format!("{h}.{c}").as_bytes());
        assert_eq!(s, URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()));

        let claims: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(c.as_bytes()).unwrap()).unwrap();
        assert_eq!(claims["video"]["roomAdmin"], serde_json::json!(true));
        assert_eq!(claims["video"]["roomJoin"], serde_json::json!(false));
    }

    #[test]
    fn room_name_prefix_is_stable() {
        assert_eq!(room_name_for("general"), "kaede-general");
    }
}
