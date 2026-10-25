//! DM（1v1 私聊）HTTP API。
//!
//! # 数据模型
//!
//! DM 会话 id = `dm:<min_uid>:<max_uid>`（uid 字典序，保证唯一）。
//! 消息存进 `messages` 表，channel_id = 该会话 id。
//!
//! # 权限
//!
//! 只有会话双方能读/写。

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::HeaderMap,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::http::authenticate;
use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/dm", get(list_conversations))
        .route("/api/dm/{peer}", post(open_conversation))
        .route(
            "/api/dm/{conv_id}/messages",
            get(get_messages).post(post_message),
        )
        .layer(axum::extract::DefaultBodyLimit::max(4 * 1024 * 1024))
        .with_state(state)
}

/// 会话 id 生成。
pub fn conv_id_for(a: &str, b: &str) -> String {
    let (x, y) = if a < b { (a, b) } else { (b, a) };
    format!("dm:{}:{}", x, y)
}

/// 解析会话 id 里的两个成员。失败返回 None。
pub fn parse_conv_id(conv_id: &str) -> Option<(&str, &str)> {
    let rest = conv_id.strip_prefix("dm:")?;
    let (a, b) = rest.split_once(':')?;
    if a.is_empty() || b.is_empty() {
        return None;
    }
    Some((a, b))
}

pub(crate) fn is_member(conv_id: &str, uid: &str) -> bool {
    match parse_conv_id(conv_id) {
        Some((a, b)) => a == uid || b == uid,
        None => false,
    }
}

// ---------------------------------------------------------------------------
// 会话
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct Conversation {
    conv_id: String,
    peer_login_id: String,
    peer_nickname: String,
}

#[derive(Serialize)]
struct ConversationsResp {
    conversations: Vec<Conversation>,
}

async fn list_conversations(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ConversationsResp>, ApiError> {
    let uid = authenticate(&state, &headers).await?;
    let me = uid.as_str();

    // 从 dm_conversations 索引表查（有 a/b 索引），不再全表扫描 messages。
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT conv_id FROM dm_conversations WHERE a = ? OR b = ? \
         ORDER BY last_ts DESC",
    )
    .bind(me)
    .bind(me)
    .fetch_all(&state.pool)
    .await?;

    let mut conversations = Vec::new();
    for (cid,) in rows {
        let peer: String = match parse_conv_id(&cid) {
            Some((a, b)) => (if a == me { b } else { a }).to_string(),
            None => continue,
        };
        let nick = get_nickname(&state, &peer).await;
        conversations.push(Conversation {
            conv_id: cid,
            peer_login_id: peer,
            peer_nickname: nick,
        });
    }

    Ok(Json(ConversationsResp { conversations }))
}

#[derive(Serialize)]
struct OpenConversationResp {
    conv_id: String,
    peer_login_id: String,
    peer_nickname: String,
}

async fn open_conversation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(peer): Path<String>,
) -> Result<Json<OpenConversationResp>, ApiError> {
    let uid = authenticate(&state, &headers).await?;
    let me = uid.as_str();

    if peer == me {
        return Err(ApiError::BadRequest("cannot dm yourself".into()));
    }

    // 检查 peer 存在
    let exists: Option<(String,)> =
        sqlx::query_as("SELECT login_id FROM accounts WHERE login_id = ?")
            .bind(&peer)
            .fetch_optional(&state.pool)
            .await?;
    if exists.is_none() {
        return Err(ApiError::NotFound("user".into()));
    }

    let conv_id = conv_id_for(me, &peer);
    let nick = get_nickname(&state, &peer).await;

    // P1-dm: open_conversation 也要写 dm_conversations 表，
    // 否则发起方 list_conversations 看不到自己刚打开的会话
    // （peer 侧在 send 消息时由 post_message 写表，反方向不对称）
    let (a, b) = if me < peer.as_str() { (me.to_string(), peer.clone()) } else { (peer.clone(), me.to_string()) };
    if let Err(e) = sqlx::query(
        "INSERT OR IGNORE INTO dm_conversations (conv_id, a, b, last_ts) \
         VALUES (?, ?, ?, 0)"
    )
    .bind(&conv_id)
    .bind(&a)
    .bind(&b)
    .execute(&state.pool)
    .await
    {
        tracing::warn!("open_conversation insert dm_conversations failed: {}", e);
    }

    Ok(Json(OpenConversationResp {
        conv_id,
        peer_login_id: peer,
        peer_nickname: nick,
    }))
}

// ---------------------------------------------------------------------------
// 消息
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct PostMessageReq {
    /// S-3: 客户端自报 ts，服务器用 `db::now_ms()` 覆盖；仅为反序列化兼容保留。
    #[allow(dead_code)]
    ts: i64,
    /// 客户端自报的 sender。
    ///
    /// **服务器故意不读此字段**——用 auth uid 覆盖，防止 DM 冒充。
    /// 保留字段是为了兼容客户端旧版本的 CBOR/JSON 反序列化。
    #[allow(dead_code)]
    sender: String,
    ciphertext: String,
}

#[derive(Serialize)]
struct MessageRecord {
    ts: i64,
    sender: String,
    ciphertext: String,
}

/// DM 发送响应：返回服务器权威 ts，供发送方本地回显使用。
///
/// 发送方不再用客户端时钟回显，否则重新打开会话时服务器历史
/// （服务器 ts）与本地回显（客户端 ts）无法按 ts 去重。
#[derive(Serialize)]
struct PostMessageResp {
    ts: i64,
}

#[derive(Serialize)]
struct MessagesResp {
    messages: Vec<MessageRecord>,
}

#[derive(Deserialize)]
struct GetMessagesQuery {
    before_ts: Option<i64>,
    limit: Option<i64>,
}

async fn post_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(conv_id): Path<String>,
    Json(req): Json<PostMessageReq>,
) -> Result<Json<PostMessageResp>, ApiError> {
    // C3：用 auth uid 覆盖客户端自报的 sender，防止 DM 冒充
    let uid = authenticate(&state, &headers).await?;

    // C11: per-uid 限流，防 DM flood
    if !state.check_rate_scoped(&uid, "dm_messages", 10.0, 30.0) {
        return Err(ApiError::BadRequest("dm message rate limited".into()));
    }

    if !is_member(&conv_id, uid.as_str()) {
        return Err(ApiError::Unauthorized);
    }

    let ct = base64_decode(&req.ciphertext)?;
    if ct.len() > 4 * 1024 * 1024 {
        return Err(ApiError::BadRequest("message too large".into()));
    }
    // 实时通道只接受两种封装：
    //   0x02 = DM group Welcome 信封（首次建会话时通过本端点投递给对方）
    //   0x05 = v2 per-conv AEAD 密文（AAD 绑定 conv_id / from / to / msg_id）
    // 旧 v1（0x01）只允许用于读取历史：允许重新写入会让失陷服务器能把
    // 旧格式密文当成新消息注入（v1 的 AAD 不绑定发送者）。
    match ct.first() {
        Some(&0x02) | Some(&0x05) => {}
        _ => {
            return Err(ApiError::BadRequest(
                "dm ciphertext must start with 0x02 (welcome) or 0x05 (v2)".into(),
            ));
        }
    }
    if !state.check_rate_scoped_cost(
        &uid,
        "dm-bytes",
        1024.0 * 1024.0,
        8.0 * 1024.0 * 1024.0,
        ct.len() as f64,
    ) {
        state.metrics.rate_limited();
        return Err(ApiError::BadRequest(
            "dm message byte budget exceeded".into(),
        ));
    }

    // S-3: 忽略客户端 ts，用服务器时间。
    //
    // 同时保证同一会话内 ts **严格递增**：否则同毫秒的多条消息会让
    // `ts < before_ts` 分页丢消息。用单条 INSERT ... SELECT 原子计算。
    let now = crate::db::now_ms();
    let mut tx = state.pool.begin().await?;
    if !crate::db::reserve_message_bytes(&mut tx, uid.as_str(), ct.len() as i64).await? {
        tx.rollback().await?;
        return Err(ApiError::BadRequest(format!(
            "message quota exceeded (max {} MiB)",
            crate::db::MAX_USER_MESSAGE_BYTES / 1024 / 1024
        )));
    }
    let (ts,): (i64,) = sqlx::query_as(
        "INSERT INTO messages (channel_id, ts, sender, ciphertext) \
         SELECT ?, CASE WHEN ? > COALESCE(MAX(ts), 0) THEN ? ELSE COALESCE(MAX(ts), 0) + 1 END, ?, ? \
         FROM messages WHERE channel_id = ? \
         RETURNING ts",
    )
    .bind(&conv_id)
    .bind(now)
    .bind(now)
    .bind(uid.as_str()) // ← 服务器权威
    .bind(&ct)
    .bind(&conv_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;

    // 更新会话索引（供 list_conversations 使用）。
    if let Some((a, b)) = parse_conv_id(&conv_id) {
        sqlx::query(
            "INSERT INTO dm_conversations (conv_id, a, b, last_ts) VALUES (?, ?, ?, ?) \
             ON CONFLICT(conv_id) DO UPDATE SET last_ts = excluded.last_ts",
        )
        .bind(&conv_id)
        .bind(a)
        .bind(b)
        .bind(ts)
        .execute(&state.pool)
        .await?;
    }
    // B3：WS 推送给对端（sender 靠本地回显，server 不推回 sender）
    if let Some((a, b)) = parse_conv_id(&conv_id) {
        let peer = if a == uid.as_str() { b } else { a };
        let peer_uid = kaede_protocol::UserId::new(peer);
        let pushed = state.send_to(
            &peer_uid,
            kaede_protocol::ServerMessage::DmReceived {
                conv_id: conv_id.clone(),
                from: uid.clone(),
                ts,
                ciphertext: ct,
            },
        );
        if !pushed {
            tracing::debug!("dm: peer {} offline, message persisted only", peer);
        }
    }

    Ok(Json(PostMessageResp { ts }))
}

async fn get_messages(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(conv_id): Path<String>,
    Query(q): Query<GetMessagesQuery>,
) -> Result<Json<MessagesResp>, ApiError> {
    let uid = authenticate(&state, &headers).await?;

    if !is_member(&conv_id, uid.as_str()) {
        return Err(ApiError::Unauthorized);
    }

    // 历史读取限流 + 单次密文字节封顶（同公开频道）。
    if !state.check_rate_scoped(&uid, "dm_history_read", 2.0, 10.0) {
        state.metrics.rate_limited();
        return Err(ApiError::TooManyRequests(
            "dm history read rate limited".into(),
        ));
    }

    let before = q.before_ts.unwrap_or(i64::MAX);
    let limit = q.limit.unwrap_or(200).clamp(1, 500); // C-8: clamp 防 limit=-1 全表扫描

    let rows: Vec<(i64, String, Vec<u8>)> = sqlx::query_as(
        "WITH recent AS ( \
           SELECT ts, sender, ciphertext, \
                  SUM(LENGTH(ciphertext)) OVER (ORDER BY ts DESC) AS run_bytes \
           FROM ( \
             SELECT ts, sender, ciphertext FROM messages \
             WHERE channel_id = ? AND ts < ? \
             ORDER BY ts DESC LIMIT ? \
           ) \
         ) \
         SELECT ts, sender, ciphertext FROM recent \
         WHERE run_bytes <= ? ORDER BY ts ASC",
    )
    .bind(&conv_id)
    .bind(before)
    .bind(limit)
    .bind(crate::db::MAX_HISTORY_RESPONSE_BYTES)
    .fetch_all(&state.pool)
    .await?;

    // SQL 已按时间升序返回
    let out: Vec<MessageRecord> = rows
        .into_iter()
        .map(|(ts, sender, ct)| MessageRecord {
            ts,
            sender,
            ciphertext: crate::db::base64_url(&ct),
        })
        .collect();

    Ok(Json(MessagesResp { messages: out }))
}

// ---------------------------------------------------------------------------

async fn get_nickname(state: &AppState, login_id: &str) -> String {
    let r: Option<(String,)> = sqlx::query_as("SELECT nickname FROM members WHERE login_id = ?")
        .bind(login_id)
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten();
    r.map(|(n,)| n).unwrap_or_else(|| login_id.to_string())
}

fn base64_decode(s: &str) -> Result<Vec<u8>, ApiError> {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    URL_SAFE_NO_PAD
        .decode(s)
        .map_err(|e| ApiError::BadRequest(format!("base64: {}", e)))
}
