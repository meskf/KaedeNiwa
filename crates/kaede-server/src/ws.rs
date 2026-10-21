//! WebSocket：实时信令 + MLS 转发。

use axum::{
    Router,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
    routing::get,
};
use futures_util::{SinkExt, Stream, StreamExt};
use kaede_protocol::{ClientMessage, ErrorCode, ServerMessage, UserId, WireMessage};
use tokio::sync::mpsc;

use crate::state::{AppState, PeerHandle};

/// WS 空闲超时（P1-M）。
///
/// 5 分钟无任何消息（含 ping）则断开，防 5000 空闲连接耗尽 semaphore。
pub(crate) const WS_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// P1-L: 定期重验 session token 的间隔。
///
/// 连接建立后，即使 token 已过期，WS 也不会自动断开——
/// 攻击者拿到泄漏的 token 能在 TTL 窗口内持续使用。
/// 每 5 分钟重验一次，把暴露窗口从"TTL 全长"缩短到"≤5 分钟"。
///
/// 性能：2000 在线用户 = 每 5 分钟 2000 次 SELECT（6.7 QPS），可忽略。
pub(crate) const SESSION_REVALIDATE_INTERVAL: std::time::Duration =
    std::time::Duration::from_secs(300);

/// P1-L: 距离上次检查是否到了该重验的时机（`>=`，含边界）。
pub(crate) fn should_revalidate(
    last_check: std::time::Instant,
    interval: std::time::Duration,
) -> bool {
    last_check.elapsed() >= interval
}

/// 单个 MLS 信封上限。离线队列按同样大小限额，在线投递也必须一致，
/// 否则成员之间可以直接推送 16 MiB 的 ChannelSync/Welcome。
const MAX_MLS_ENVELOPE_BYTES: usize = 256 * 1024;

/// Commit / Proposal 上限（含 group info，正常远小于此值）。
const MAX_MLS_HANDSHAKE_BYTES: usize = 2 * 1024 * 1024;

/// 主群应用消息上限（与 HTTP post_message 的 4 MiB 对齐）。
const MAX_MLS_APP_BYTES: usize = 4 * 1024 * 1024;

/// `recv_with_idle_timeout` 的返回值。
#[derive(Debug)]
pub(crate) enum RecvOutcome {
    Message(Message),
    Timeout,
    Closed,
    Error(String),
}

/// 从 WS stream 读下一条消息，带 idle timeout。
///
/// 抽成独立函数便于测试（用短 timeout 验证三态：Timeout / Message / Closed）。
/// `run_socket` 用它替换内联 `tokio::time::timeout`。
async fn recv_with_idle_timeout<S>(stream: &mut S, timeout: std::time::Duration) -> RecvOutcome
where
    S: Stream<Item = Result<Message, axum::Error>> + Unpin,
{
    match tokio::time::timeout(timeout, stream.next()).await {
        Err(_) => RecvOutcome::Timeout,
        Ok(None) => RecvOutcome::Closed,
        Ok(Some(Err(e))) => RecvOutcome::Error(format!("{}", e)),
        Ok(Some(Ok(m))) => RecvOutcome::Message(m),
    }
}

/// 简单的令牌桶限流器（每连接一个实例）。
///
/// rate = 每秒补充的令牌数；capacity = 桶容量（突发上限）。
/// 超过限制的消息会被拒绝（Error{code: RateLimited}），连接不断。
struct RateLimiter {
    tokens: f64,
    last: std::time::Instant,
    rate: f64,
    capacity: f64,
}

impl RateLimiter {
    fn new(rate: f64, capacity: f64) -> Self {
        Self {
            tokens: capacity,
            last: std::time::Instant::now(),
            rate,
            capacity,
        }
    }

    fn try_consume(&mut self, n: f64) -> bool {
        let now = std::time::Instant::now();
        let elapsed = now.duration_since(self.last).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.rate).min(self.capacity);
        self.last = now;
        if self.tokens >= n {
            self.tokens -= n;
            true
        } else {
            false
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/ws", get(ws_handler))
        .with_state(state)
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    // P1-10：总连接数限制。
    let permit = match state.ws_semaphore.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => {
            return axum::response::Response::builder()
                .status(axum::http::StatusCode::SERVICE_UNAVAILABLE)
                .body(axum::body::Body::from("server connection limit reached"))
                .unwrap();
        }
    };
    let max = kaede_protocol::MAX_MESSAGE_BYTES;
    ws.max_message_size(max)
        .max_frame_size(max)
        .on_upgrade(move |socket| async move {
            handle_socket(socket, state).await;
            drop(permit);
        })
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    if let Err(e) = run_socket(socket, state).await {
        tracing::debug!("ws closed: {}", e);
    }
}

async fn run_socket(socket: WebSocket, state: AppState) -> anyhow::Result<()> {
    let (mut sink, mut stream) = socket.split();

    // 出站 channel
    let (tx, mut rx) = mpsc::channel::<ServerMessage>(1024);

    // per-connection 限流器：30 msg/s，突发上限 60。
    // 握手前不消耗令牌。
    let mut limiter = RateLimiter::new(30.0, 60.0);

    // P0-X：本连接的 session_id。握手前为 0（未分配）。
    let my_session_id: std::sync::Arc<std::sync::atomic::AtomicU64> =
        std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));

    // ---- 握手 ----
    let first = tokio::time::timeout(std::time::Duration::from_secs(10), stream.next())
        .await
        .map_err(|_| anyhow::anyhow!("hello timeout (10s)"))?
        .ok_or_else(|| anyhow::anyhow!("client closed before hello"))??;
    let bytes = extract_bytes(first)?;
    let hello = ClientMessage::decode(&bytes)?;

    let (client_name, client_version, session_token) = match hello {
        ClientMessage::ClientHello {
            protocol_version,
            client_name,
            client_version,
            session_token,
        } => {
            // P1-20：token 长度上限 128，防 DB 查询放大。
            if let Some(t) = &session_token
                && t.len() > 128
            {
                let err = ServerMessage::Error {
                    code: ErrorCode::MalformedMessage,
                    message: "session_token too long (max 128)".into(),
                };
                let _ = sink.send(Message::Binary(err.encode()?.into())).await;
                return Ok(());
            }
            if protocol_version < kaede_protocol::MIN_SUPPORTED_VERSION
                || protocol_version > kaede_protocol::PROTOCOL_VERSION
            {
                let err = ServerMessage::Error {
                    code: ErrorCode::UnsupportedVersion,
                    message: format!(
                        "server supports {}-{}",
                        kaede_protocol::MIN_SUPPORTED_VERSION,
                        kaede_protocol::PROTOCOL_VERSION
                    ),
                };
                let _ = sink.send(Message::Binary(err.encode()?.into())).await;
                return Ok(());
            }
            (client_name, client_version, session_token)
        }
        _ => anyhow::bail!("expected ClientHello, got {:?}", hello),
    };

    tracing::info!("ws hello from {} v{}", client_name, client_version);

    let hello_reply = ServerMessage::ServerHello {
        protocol_version: kaede_protocol::PROTOCOL_VERSION,
        server_id: "kaede-local".into(),
        server_name: "KaedeNiwa".into(),
    };
    sink.send(Message::Binary(hello_reply.encode()?.into()))
        .await?;

    // 若客户端提供了 session_token，则在此验证并绑定 uid。
    // 未提供 token 的会话可继续连接，但调用 `Join` 会被拒。
    let mut my_uid: Option<UserId> = if let Some(tok) = &session_token {
        match validate_session(&state, tok).await {
            Ok(uid) => {
                tracing::info!("ws auth ok uid={}", uid);
                Some(uid)
            }
            Err(e) => {
                // C-6: token 无效时立即关闭连接，防止恶意连接占满 ws_semaphore
                tracing::warn!("ws auth failed, closing: {:?}", e);
                let err = ServerMessage::Error {
                    code: ErrorCode::MalformedMessage,
                    message: "invalid or expired session token".into(),
                };
                let _ = sink.send(Message::Binary(err.encode()?.into())).await;
                return Ok(());
            }
        }
    } else {
        None
    };

    // ---- Writer task ----
    let writer = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            match msg.encode() {
                Ok(bytes) => {
                    if sink.send(Message::Binary(bytes.into())).await.is_err() {
                        break;
                    }
                }
                Err(e) => {
                    tracing::warn!("encode failed: {}", e);
                    break;
                }
            }
        }
    });

    // ---- 主循环 ----

    // P1-L: 定期重验 session token 的时间戳
    let mut last_auth_check = std::time::Instant::now();
    let mut last_activity = std::time::Instant::now();
    'outer: loop {
        // P1-L: 每 5 分钟重验一次 session token（防 token 泄漏后长时间滥用）
        if let Some(tok) = &session_token
            && should_revalidate(last_auth_check, SESSION_REVALIDATE_INTERVAL)
        {
            if validate_session(&state, tok).await.is_err() {
                tracing::info!("ws session expired, closing");
                break;
            }
            last_auth_check = std::time::Instant::now();
        }

        // 每条消息处理前都确认当前连接仍是 peers 表里的那一个。
        // 被踢/被顶替/账号删除后即使持续发消息也会立刻断开。
        if let Some(uid) = &my_uid {
            let sid = my_session_id.load(std::sync::atomic::Ordering::Relaxed);
            if sid != 0 && state.peer_session_id(uid) != Some(sid) {
                tracing::info!("ws peer superseded/removed, closing");
                break 'outer;
            }
        }

        // P1-M：5 分钟无任何消息（含 ping）则断开，防 5000 空闲连接耗尽 semaphore。
        // 用 1s 切片轮询，同时检查“当前连接是否仍是 peers 表里的那一个”：
        // 被踢/被顶替/账号删除后 1s 内关闭 socket，而不是等 5 分钟重验。
        let msg = loop {
            match recv_with_idle_timeout(&mut stream, std::time::Duration::from_secs(1)).await {
                RecvOutcome::Timeout => {
                    if last_activity.elapsed() >= WS_IDLE_TIMEOUT {
                        tracing::debug!("ws idle timeout ({}s)", WS_IDLE_TIMEOUT.as_secs());
                        break 'outer;
                    }
                    if let Some(uid) = &my_uid {
                        let sid = my_session_id.load(std::sync::atomic::Ordering::Relaxed);
                        if sid != 0 && state.peer_session_id(uid) != Some(sid) {
                            tracing::info!("ws peer superseded/removed, closing");
                            break 'outer;
                        }
                    }
                    continue;
                }
                RecvOutcome::Closed => break 'outer,
                RecvOutcome::Error(e) => {
                    tracing::debug!("ws read error: {}", e);
                    break 'outer;
                }
                RecvOutcome::Message(m) => {
                    last_activity = std::time::Instant::now();
                    break m;
                }
            }
        };

        let bytes = match msg {
            Message::Binary(b) => b.to_vec(),
            Message::Text(_) => {
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::MalformedMessage,
                    message: "text frames not supported; use binary CBOR".into(),
                });
                continue;
            }
            Message::Close(_) => break,
            _ => continue,
        };

        let msg = match ClientMessage::decode(&bytes) {
            Ok(m) => m,
            Err(e) => {
                // C13: 详细错误进 log，客户端只收固定字符串（防协议结构泄露）
                tracing::debug!("ws decode failed: {}", e);
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::MalformedMessage,
                    message: "malformed message".into(),
                });
                continue;
            }
        };

        // 限流：已认证走 per-uid（断线重连不重置速率），未认证走 per-connection。
        // 超限时给客户端一条 RateLimited，不断连接。
        let allowed = match &my_uid {
            Some(uid) => state.check_rate_scoped(uid, "ws", 30.0, 60.0),
            None => limiter.try_consume(1.0),
        };
        if !allowed {
            let _ = tx.try_send(ServerMessage::Error {
                code: ErrorCode::RateLimited,
                message: "too many messages, slow down".into(),
            });
            continue;
        }

        // 被踢/账号删除后，旧连接手里的 token 已失效；Join 必须重新校验，
        // 不能只信任 Hello 时缓存的 my_uid。
        if matches!(&msg, ClientMessage::Join)
            && let Some(tok) = &session_token
            && validate_session(&state, tok).await.is_err()
        {
            let _ = tx.try_send(ServerMessage::Error {
                code: ErrorCode::Kicked,
                message: "session revoked; please log in again".into(),
            });
            break 'outer;
        }

        if let Err(e) = handle_message(msg, &state, &mut my_uid, &my_session_id, &tx).await {
            tracing::warn!("handle message: {}", e);
            // 不把内部细节发给客户端，但必须让它知道请求没有被接受，
            // 否则客户端会一直以为自己是“已加入”的僵尸状态。
            let _ = tx.try_send(ServerMessage::Error {
                code: ErrorCode::NotJoined,
                message: "request rejected: not joined or session superseded".into(),
            });
        }
    }

    // ---- 清理 ----
    // P0-X：只有本 session 仍是 peers 表里的那一个才清理和广播。
    // 若已被新连接顶替（多设备 / 重连），不动新连接的 handle。
    if let Some(uid) = my_uid {
        let sid = my_session_id.load(std::sync::atomic::Ordering::Relaxed);
        if sid != 0 && state.remove_peer_if_session(&uid, sid) {
            state.broadcast_except(&uid, ServerMessage::PeerLeft { uid: uid.clone() });
            let snapshot = state.snapshot_presence();
            state.broadcast_all(ServerMessage::PresenceUpdated { presence: snapshot });
        }
    }
    drop(tx);
    let _ = writer.await;
    Ok(())
}

/// 把离线暂存的 MLS 信封补投给刚 Join 的客户端。
///
/// 顺序严格按入队顺序（Welcome 先于 Commit）。取出的同时从 DB 删除，
/// 避免下次 Join 重复投递。
async fn deliver_pending(state: &AppState, uid: &UserId, tx: &mpsc::Sender<ServerMessage>) {
    let envelopes = match crate::db::list_envelopes(&state.pool, uid.as_str()).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("list_envelopes for {} failed: {}", uid, e);
            return;
        }
    };
    if envelopes.is_empty() {
        return;
    }
    tracing::info!(
        "delivering {} pending envelope(s) to {}",
        envelopes.len(),
        uid
    );
    let mut delivered: Vec<i64> = Vec::new();
    for (id, from, kind, channel_id, payload) in envelopes {
        let msg = match kind.as_str() {
            "mls_welcome" => ServerMessage::MlsWelcome {
                from: UserId::new(from),
                payload,
            },
            "channel_member_left" => ServerMessage::ChannelMemberLeft {
                channel_id: channel_id.unwrap_or_default(),
                member: UserId::new(String::from_utf8_lossy(&payload).to_string()),
            },
            "channel_member_joined" => {
                use serde::Deserialize;
                #[derive(Deserialize)]
                struct PendingJoin {
                    requester: String,
                    key_package: String,
                }
                match serde_json::from_slice::<PendingJoin>(&payload) {
                    Ok(p) => ServerMessage::ChannelMemberJoined {
                        channel_id: channel_id.unwrap_or_default(),
                        requester: UserId::new(p.requester),
                        key_package: p.key_package,
                    },
                    Err(e) => {
                        tracing::warn!("bad pending channel_member_joined: {}", e);
                        // 毒数据直接删除，避免每次 Join 都卡住队列
                        delivered.push(id);
                        continue;
                    }
                }
            }
            _ => ServerMessage::ChannelSync {
                channel_id: channel_id.unwrap_or_default(),
                payload,
            },
        };
        match tokio::time::timeout(std::time::Duration::from_secs(5), tx.send(msg)).await {
            Ok(Ok(())) => delivered.push(id),
            Ok(Err(_)) | Err(_) => {
                // 连接关闭/队列满：保留未投递的信封，下次 Join 再试。
                tracing::warn!(
                    "pending envelope {} not delivered to {}; keep for retry",
                    id,
                    uid
                );
                break;
            }
        }
    }
    if !delivered.is_empty()
        && let Err(e) = crate::db::delete_envelopes(&state.pool, &delivered).await
    {
        tracing::warn!("delete delivered envelopes failed: {}", e);
    }
}

fn extract_bytes(m: Message) -> anyhow::Result<Vec<u8>> {
    match m {
        Message::Binary(b) => Ok(b.to_vec()),
        Message::Text(_) => anyhow::bail!("text frames not supported; use binary CBOR"),
        _ => anyhow::bail!("expected binary/text"),
    }
}

/// 查询某个成员的权限等级。
async fn sender_op(state: &AppState, uid: &UserId) -> anyhow::Result<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as("SELECT op FROM members WHERE login_id = ?")
        .bind(uid.as_str())
        .fetch_optional(&state.pool)
        .await?;
    Ok(row.map(|(op,)| op))
}

/// 当前频道权威成员：`joined_at` 最早且**在线**的成员。
///
/// 与 `channels::notify_authority` / `notify_member_left` 的投递规则一致。
/// 私密频道只允许权威成员发送 Welcome / Commit，防止任意成员污染 epoch。
async fn channel_authority(state: &AppState, channel_id: &str) -> anyhow::Result<Option<String>> {
    let members: Vec<(String,)> = sqlx::query_as(
        "SELECT login_id FROM channel_members WHERE channel_id = ? \
         ORDER BY joined_at ASC, login_id ASC",
    )
    .bind(channel_id)
    .fetch_all(&state.pool)
    .await?;
    for (login_id,) in members {
        if state
            .peer_session_id(&UserId::new(login_id.clone()))
            .is_some()
        {
            return Ok(Some(login_id));
        }
    }
    Ok(None)
}

fn ensure_joined(
    state: &AppState,
    my_uid: &Option<UserId>,
    my_session_id: &std::sync::atomic::AtomicU64,
) -> anyhow::Result<UserId> {
    let uid = my_uid
        .clone()
        .ok_or_else(|| anyhow::anyhow!("not authenticated"))?;
    let sid = my_session_id.load(std::sync::atomic::Ordering::Relaxed);
    if sid == 0 {
        anyhow::bail!("not joined; send Join first");
    }
    let room = state.room.lock().unwrap();
    match room.peers.get(&uid) {
        Some(p) if p.session_id == sid => Ok(uid),
        _ => anyhow::bail!("session superseded by another connection"),
    }
}

async fn handle_message(
    msg: ClientMessage,
    state: &AppState,
    my_uid: &mut Option<UserId>,
    my_session_id: &std::sync::atomic::AtomicU64,
    tx: &mpsc::Sender<ServerMessage>,
) -> anyhow::Result<()> {
    match msg {
        ClientMessage::ClientHello { .. } => {
            // 已握手，忽略重复
        }

        ClientMessage::Join => {
            let uid = my_uid
                .clone()
                .ok_or_else(|| anyhow::anyhow!("not authenticated"))?;

            // P1-8：幂等只针对**同一条连接**的重复 Join。
            // 不同连接（断线重连 / 同账号第二个客户端）必须走 add_peer 顶替旧连接，
            // 否则新连接会变成永远无法发消息的僵尸。
            let current_sid = my_session_id.load(std::sync::atomic::Ordering::Relaxed);
            let same_session = current_sid != 0 && state.peer_session_id(&uid) == Some(current_sid);
            if same_session {
                let existing = state.peer_uids();
                let _ = tx.try_send(ServerMessage::Joined {
                    peers: existing.iter().filter(|u| *u != &uid).cloned().collect(),
                });
                deliver_pending(state, &uid, tx).await;
                return Ok(());
            }

            let existing = state.peer_uids();

            let session_id = state
                .next_session_id
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            my_session_id.store(session_id, std::sync::atomic::Ordering::Relaxed);

            let _prev = state.add_peer(PeerHandle {
                uid: uid.clone(),
                tx: tx.clone(),
                session_id,
            });

            let _ = tx.try_send(ServerMessage::Joined {
                peers: existing.iter().filter(|u| *u != &uid).cloned().collect(),
            });

            let evt = ServerMessage::PeerJoined { uid: uid.clone() };
            state.broadcast_except(&uid, evt);

            // 广播完整的在线列表。broadcast_all 覆盖所有 peer（含刚加入的自己），
            // 所以新成员也会收到 —— 不要再额外 tx.send 一次。
            let snapshot = state.snapshot_presence();
            state.broadcast_all(ServerMessage::PresenceUpdated { presence: snapshot });

            // 离线期间暂存的 Welcome / ChannelSync 在这里补投。
            deliver_pending(state, &uid, tx).await;
        }

        ClientMessage::Leave => {
            // P0-4: 保留 my_uid（认证态），只清 session_id（房间态）。
            // 这样 Leave 之后可以再次 Join 到其它房间。
            if let Some(uid) = my_uid.clone() {
                let sid = my_session_id.load(std::sync::atomic::Ordering::Relaxed);
                if sid != 0 && state.remove_peer_if_session(&uid, sid) {
                    let evt = ServerMessage::PeerLeft { uid: uid.clone() };
                    state.broadcast_except(&uid, evt);
                    let snapshot = state.snapshot_presence();
                    state.broadcast_all(ServerMessage::PresenceUpdated { presence: snapshot });
                }
            }
            my_session_id.store(0, std::sync::atomic::Ordering::Relaxed);
        }

        ClientMessage::PublishKeyPackages { .. } => {
            let _ = tx.try_send(ServerMessage::Error {
                code: ErrorCode::MalformedMessage,
                message: "publish_keypackages must go via HTTP /api/keypackages".into(),
            });
        }

        ClientMessage::FetchKeyPackage { target } => {
            let _ = tx.try_send(ServerMessage::Error {
                code: ErrorCode::MalformedMessage,
                message: format!(
                    "fetch_key_package({}) must go via HTTP /api/keypackages/{}",
                    target, target
                ),
            });
        }

        ClientMessage::ChannelSync {
            to,
            channel_id,
            payload,
        } => {
            let from = ensure_joined(state, my_uid, my_session_id)?;
            if payload.len() > MAX_MLS_ENVELOPE_BYTES {
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::MalformedMessage,
                    message: "channel sync payload too large".into(),
                });
                return Ok(());
            }
            // 安全校验：发送者必须是频道成员
            let is_member: Option<(i64,)> = sqlx::query_as(
                "SELECT 1 FROM channel_members WHERE channel_id = ? AND login_id = ?",
            )
            .bind(&channel_id)
            .bind(from.as_str())
            .fetch_optional(&state.pool)
            .await?;
            if is_member.is_none() {
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::TargetNotFound,
                    message: format!("not a member of {}", channel_id),
                });
                return Ok(());
            }
            // 收件人必须是同频道成员，否则任意成员都能向陌生人投递任意密文。
            let target_member: Option<(i64,)> = sqlx::query_as(
                "SELECT 1 FROM channel_members WHERE channel_id = ? AND login_id = ?",
            )
            .bind(&channel_id)
            .bind(to.as_str())
            .fetch_optional(&state.pool)
            .await?;
            if target_member.is_none() {
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::TargetNotFound,
                    message: format!("recipient is not a member of {}", channel_id),
                });
                return Ok(());
            }
            // 只有当前权威成员（最早加入且在线）能下发 Welcome / Commit，
            // 任意成员都能发送会让所有人被拉入攻击者构造的 channel group。
            let authority = channel_authority(state, &channel_id).await?;
            if authority.as_deref() != Some(from.as_str()) {
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::TargetNotFound,
                    message: format!("sender is not the channel authority of {}", channel_id),
                });
                return Ok(());
            }
            if !state.send_to(
                &to,
                ServerMessage::ChannelSync {
                    channel_id: channel_id.clone(),
                    payload: payload.clone(),
                },
            ) {
                // 目标离线：暂存，等它下次 Join 时补投（否则私密频道永远进不去）。
                if let Err(e) = crate::db::enqueue_envelope(
                    &state.pool,
                    to.as_str(),
                    from.as_str(),
                    "channel_sync",
                    Some(&channel_id),
                    &payload,
                )
                .await
                {
                    tracing::warn!("enqueue channel_sync failed: {}", e);
                }
            }
        }

        ClientMessage::ChannelCommit {
            channel_id,
            exclude,
            payload,
        } => {
            let from = ensure_joined(state, my_uid, my_session_id)?;
            let is_member: Option<(i64,)> = sqlx::query_as(
                "SELECT 1 FROM channel_members WHERE channel_id = ? AND login_id = ?",
            )
            .bind(&channel_id)
            .bind(from.as_str())
            .fetch_optional(&state.pool)
            .await?;
            if is_member.is_none() {
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::TargetNotFound,
                    message: format!("not a member of {}", channel_id),
                });
                return Ok(());
            }
            if payload.len() > MAX_MLS_HANDSHAKE_BYTES {
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::MalformedMessage,
                    message: "channel commit payload too large".into(),
                });
                return Ok(());
            }
            // 与 ChannelSync 一致：只有当前权威成员能推进频道 epoch。
            let authority = channel_authority(state, &channel_id).await?;
            if authority.as_deref() != Some(from.as_str()) {
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::TargetNotFound,
                    message: format!("sender is not the channel authority of {}", channel_id),
                });
                return Ok(());
            }

            // 频道内广播 Commit（0x05 前缀由服务端统一加，和 ChannelSync 一致）。
            let mut framed = Vec::with_capacity(1 + payload.len());
            framed.push(0x05u8);
            framed.extend_from_slice(&payload);

            let members: Vec<(String,)> = sqlx::query_as(
                "SELECT login_id FROM channel_members WHERE channel_id = ? AND login_id != ?",
            )
            .bind(&channel_id)
            .bind(from.as_str())
            .fetch_all(&state.pool)
            .await?;

            for (member,) in members {
                if exclude.as_ref().is_some_and(|e| e.as_str() == member) {
                    continue;
                }
                let target = UserId::new(member);
                let msg = ServerMessage::ChannelSync {
                    channel_id: channel_id.clone(),
                    payload: framed.clone(),
                };
                if !state.send_to(&target, msg)
                    && let Err(e) = crate::db::enqueue_envelope(
                        &state.pool,
                        target.as_str(),
                        from.as_str(),
                        "channel_sync",
                        Some(&channel_id),
                        &framed,
                    )
                    .await
                {
                    tracing::warn!("enqueue channel_commit failed: {}", e);
                }
            }
        }

        ClientMessage::MlsWelcome { to, payload } => {
            let from = ensure_joined(state, my_uid, my_session_id)?;
            if payload.len() > MAX_MLS_ENVELOPE_BYTES {
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::MalformedMessage,
                    message: "welcome payload too large".into(),
                });
                return Ok(());
            }
            // 主群 Welcome 只能由 host（op5）发送。
            //
            // 客户端在本地已有主群时会用 Welcome 里的 group_id 判断是否重建；
            // 若任意成员都能发 Welcome，攻击者只需拉取目标 KP + 自建 group，
            // 就能把目标拉进攻击者控制的群（读走其后续所有公开频道消息）。
            if sender_op(state, &from).await?.as_deref() != Some("op5") {
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::TargetNotFound,
                    message: "only the room host (op5) may send MLS welcomes".into(),
                });
                return Ok(());
            }
            // 目标必须是真实存在的成员；避免向不存在/任意 uid 塞离线信封。
            let target_exists: Option<(i64,)> =
                sqlx::query_as("SELECT 1 FROM members WHERE login_id = ?")
                    .bind(to.as_str())
                    .fetch_optional(&state.pool)
                    .await?;
            if target_exists.is_none() {
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::TargetNotFound,
                    message: to.0,
                });
                return Ok(());
            }
            if !state.send_to(
                &to,
                ServerMessage::MlsWelcome {
                    from: from.clone(),
                    payload: payload.clone(),
                },
            ) {
                // Welcome 是入组唯一凭证，绝不能静默丢弃。离线时暂存补投。
                if let Err(e) = crate::db::enqueue_envelope(
                    &state.pool,
                    to.as_str(),
                    from.as_str(),
                    "mls_welcome",
                    None,
                    &payload,
                )
                .await
                {
                    tracing::warn!("enqueue mls_welcome failed: {}", e);
                }
            }
        }

        ClientMessage::MlsCommit { epoch, payload } => {
            let from = ensure_joined(state, my_uid, my_session_id)?;
            // 只有管理员（op4+）能推进主群 epoch：普通成员构造的 Commit
            // 可以把别人踢出群或把群切到攻击者控制的分支。
            if !matches!(
                sender_op(state, &from).await?.as_deref(),
                Some("op4" | "op5")
            ) {
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::TargetNotFound,
                    message: "only moderators (op4+) may send MLS commits".into(),
                });
                return Ok(());
            }
            if payload.len() > MAX_MLS_HANDSHAKE_BYTES {
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::MalformedMessage,
                    message: "commit payload too large".into(),
                });
                return Ok(());
            }
            // epoch 是客户端可控值。限制在 u32 范围内，避免 i64::MAX
            // 污染 mls_commits 的 MAX(epoch) 使所有合法 Commit 被清理。
            if epoch > u32::MAX as u64 {
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::MalformedMessage,
                    message: "mls commit epoch out of range".into(),
                });
                return Ok(());
            }
            // P1-7：关键消息用有背压的广播，避免 try_send 静默丢弃。
            state
                .broadcast_except_important(
                    &from,
                    ServerMessage::MlsCommit {
                        from: from.clone(),
                        epoch,
                        payload: payload.clone(),
                    },
                )
                .await;

            // 持久化最近 Commit，供离线成员重连后追赶。
            if epoch > 0 {
                // 同一 epoch 只保留第一份 Commit：后到的同 epoch commit
                // 属于并发分支，覆盖会让已跟随前一份的客户端更混乱。
                if let Err(e) = sqlx::query(
                    "INSERT OR IGNORE INTO mls_commits (epoch, from_uid, payload, created_at) \
                     VALUES (?, ?, ?, ?)",
                )
                .bind(epoch as i64)
                .bind(from.as_str())
                .bind(&payload)
                .bind(crate::db::now_ms())
                .execute(&state.pool)
                .await
                {
                    tracing::warn!("store mls commit failed: {}", e);
                }
                // 只保留最近 256 条 Commit。
                // 注意：不能按 MAX(epoch) 保留——epoch 受客户端控制，
                // 一个超大值会永久清空所有合法 commit。按插入顺序(rowid)保留。
                let _ = sqlx::query(
                    "DELETE FROM mls_commits WHERE rowid NOT IN \
                     (SELECT rowid FROM mls_commits ORDER BY rowid DESC LIMIT 256)",
                )
                .execute(&state.pool)
                .await;
            }
        }

        ClientMessage::ReportEpoch { epoch } => {
            let _uid = ensure_joined(state, my_uid, my_session_id)?;
            if epoch > u32::MAX as u64 {
                return Ok(());
            }
            let rows: Vec<(i64, String, Vec<u8>)> = sqlx::query_as(
                "SELECT epoch, from_uid, payload FROM mls_commits \
                 WHERE epoch > ? ORDER BY epoch ASC LIMIT 256",
            )
            .bind(epoch as i64)
            .fetch_all(&state.pool)
            .await?;

            for (commit_epoch, from_uid, payload) in rows {
                let _ = tx.try_send(ServerMessage::MlsCommit {
                    from: UserId::new(from_uid),
                    epoch: commit_epoch as u64,
                    payload,
                });
            }
        }

        ClientMessage::MlsProposal { payload } => {
            let from = ensure_joined(state, my_uid, my_session_id)?;
            if !matches!(
                sender_op(state, &from).await?.as_deref(),
                Some("op4" | "op5")
            ) {
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::TargetNotFound,
                    message: "only moderators (op4+) may send MLS proposals".into(),
                });
                return Ok(());
            }
            if payload.len() > MAX_MLS_HANDSHAKE_BYTES {
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::MalformedMessage,
                    message: "proposal payload too large".into(),
                });
                return Ok(());
            }
            state
                .broadcast_except_important(
                    &from,
                    ServerMessage::MlsProposal {
                        from: from.clone(),
                        payload,
                    },
                )
                .await;
        }

        ClientMessage::MlsAppMessage { payload } => {
            let from = ensure_joined(state, my_uid, my_session_id)?;
            if payload.len() > MAX_MLS_APP_BYTES {
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::MalformedMessage,
                    message: "application message too large".into(),
                });
                return Ok(());
            }
            // H14: 关键消息（聊天/媒体信令）不能静默丢，走 important broadcast
            state
                .broadcast_except_important(
                    &from,
                    ServerMessage::MlsAppMessage {
                        from: from.clone(),
                        payload,
                    },
                )
                .await;
        }

        ClientMessage::SetPresence { status } => {
            let uid = ensure_joined(state, my_uid, my_session_id)?;
            let valid = matches!(status.as_str(), "online" | "idle" | "dnd");
            if !valid {
                return Err(anyhow::anyhow!("invalid status"));
            }
            state.set_presence(&uid, status);
            let snapshot = state.snapshot_presence();
            state.broadcast_all(ServerMessage::PresenceUpdated { presence: snapshot });
        }

        ClientMessage::Ping { nonce } => {
            let _ = tx.try_send(ServerMessage::Pong { nonce });
        }
    }
    Ok(())
}

/// 用 HTTP session token 换 uid，并刷新 last_seen_at。
async fn validate_session(state: &AppState, token: &str) -> anyhow::Result<UserId> {
    let token_hash = crate::db::hash_token(token);
    let row: Option<(String, i64)> =
        sqlx::query_as("SELECT login_id, expires_at FROM sessions WHERE token = ?")
            .bind(&token_hash)
            .fetch_optional(&state.pool)
            .await?;
    let (login_id, expires_at) = row.ok_or_else(|| anyhow::anyhow!("no session"))?;
    if expires_at < crate::db::now_ms() {
        anyhow::bail!("session expired");
    }
    let _ = sqlx::query("UPDATE sessions SET last_seen_at = ? WHERE token = ?")
        .bind(crate::db::now_ms())
        .bind(&token_hash)
        .execute(&state.pool)
        .await;
    Ok(UserId::new(login_id))
}

// ============================================================================
// Batch 6：P1-M（WS idle timeout）回归测试
// ============================================================================
#[cfg(test)]
mod p1m_tests {
    use super::*;
    use std::time::Duration;

    /// P1-M #1：pending stream 无消息 → 100ms 后 Timeout。
    #[tokio::test]
    async fn p1m_idle_timeout_fires_on_no_messages() {
        let mut stream = futures_util::stream::pending::<Result<Message, axum::Error>>();
        let start = std::time::Instant::now();
        let outcome = recv_with_idle_timeout(&mut stream, Duration::from_millis(100)).await;
        let elapsed = start.elapsed();

        assert!(
            matches!(outcome, RecvOutcome::Timeout),
            "expected Timeout, got {:?}",
            outcome
        );
        assert!(
            elapsed >= Duration::from_millis(90),
            "expected ~100ms, got {:?} (timeout was too short / skipped)",
            elapsed
        );
        assert!(
            elapsed < Duration::from_millis(500),
            "expected <500ms, got {:?} (timeout was too long)",
            elapsed
        );
    }

    /// P1-M #2：有消息到达 → 不超时，正确返回 Message。
    #[tokio::test]
    async fn p1m_active_stream_does_not_timeout() {
        let mut stream = futures_util::stream::iter(vec![Ok::<Message, axum::Error>(
            Message::Binary(vec![1u8, 2, 3].into()),
        )]);
        let outcome = recv_with_idle_timeout(&mut stream, Duration::from_secs(1)).await;
        match outcome {
            RecvOutcome::Message(Message::Binary(b)) => {
                assert_eq!(&b[..], &[1, 2, 3]);
            }
            other => panic!("expected Binary message, got {:?}", other),
        }
    }

    /// P1-M #3：stream 关闭 → Closed（不超时）。
    #[tokio::test]
    async fn p1m_closed_stream_returns_closed() {
        let mut stream = futures_util::stream::iter(Vec::<Result<Message, axum::Error>>::new());
        let outcome = recv_with_idle_timeout(&mut stream, Duration::from_secs(1)).await;
        assert!(
            matches!(outcome, RecvOutcome::Closed),
            "expected Closed, got {:?}",
            outcome
        );
    }
}

// ============================================================================
// Batch 7：P1-L（session 过期重验）回归测试
// ============================================================================
#[cfg(test)]
mod p1l_tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// P1-L #1：刚检查过 → 不重验。
    #[test]
    fn p1l_should_revalidate_false_when_fresh() {
        let last = Instant::now();
        assert!(!should_revalidate(last, Duration::from_secs(60)));
        assert!(!should_revalidate(last, SESSION_REVALIDATE_INTERVAL));
    }

    /// P1-L #2：超过间隔 → 重验（用短间隔避免等待 5 分钟）。
    #[tokio::test]
    async fn p1l_should_revalidate_true_after_interval() {
        let last = Instant::now();
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert!(should_revalidate(last, Duration::from_millis(50)));
    }

    /// P1-L #3：边界——`elapsed == interval` 应该重验（`>=` 语义）。
    #[test]
    fn p1l_should_revalidate_boundary_inclusive() {
        let last = Instant::now() - Duration::from_millis(100);
        assert!(should_revalidate(last, Duration::from_millis(100)));
    }

    /// P1-L #4：差一点点 → 不重验（防过度检查）。
    #[test]
    fn p1l_should_revalidate_nothing_just_before() {
        let last = Instant::now() - Duration::from_millis(50);
        assert!(!should_revalidate(last, Duration::from_millis(100)));
    }
}
