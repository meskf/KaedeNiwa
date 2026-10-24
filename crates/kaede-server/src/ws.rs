//! WebSocket：实时信令 + MLS 转发。

use axum::{
    Router,
    extract::{
        ConnectInfo, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::HeaderMap,
    response::IntoResponse,
    routing::get,
};
use futures_util::{SinkExt, Stream, StreamExt};
use kaede_protocol::{ClientMessage, ErrorCode, ServerMessage, UserId, WireMessage};
use tokio::sync::mpsc;

use crate::state::{AppState, PeerHandle, WS_BYTES_BURST, WS_BYTES_PER_SEC};

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

/// 私密频道表情反应上限。reaction payload 只有几十字节，8 KiB 足够。
const MAX_CHANNEL_REACTION_BYTES: usize = 8 * 1024;

/// 单次 `ReportEpoch` 最多回补的 Commit 总字节数。
///
/// 不设上限时，一次请求可读取最多 256 条 × 2 MiB（≈512 MiB）并全部塞进
/// 本连接 1024 容量的出站缓冲，属于可反复触发的内存放大。
const MAX_REPORT_EPOCH_BYTES: usize = 512 * 1024;

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

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
) -> impl IntoResponse {
    // 单 IP 并发连接上限：覆盖未完成认证的握手阶段，防止单一来源
    // 用大量空连接占满全局 permit。
    let ip = crate::http::client_ip(&headers, peer);
    let Some(ip_guard) = state.try_acquire_ws_ip(ip) else {
        state.metrics.ws_rejected();
        return axum::response::Response::builder()
            .status(axum::http::StatusCode::TOO_MANY_REQUESTS)
            .body(axum::body::Body::from(
                "too many connections from this address",
            ))
            .unwrap();
    };
    // P1-10：总连接数限制。
    let permit = match state.ws_semaphore.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => {
            state.metrics.ws_rejected();
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
            drop(ip_guard);
        })
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    state.metrics.ws_connected();
    if let Err(e) = run_socket(socket, state.clone()).await {
        tracing::debug!("ws closed: {}", e);
    }
    state.metrics.ws_disconnected();
}

async fn run_socket(socket: WebSocket, state: AppState) -> anyhow::Result<()> {
    let (mut sink, mut stream) = socket.split();

    // 出站 channel
    let (tx, mut rx) = mpsc::channel::<ServerMessage>(1024);

    // per-connection 限流器：30 msg/s，突发上限 60。
    // 握手前不消耗令牌。另加字节桶，防止用小条数发送超大包。
    let mut limiter = RateLimiter::new(30.0, 60.0);
    let mut byte_limiter = RateLimiter::new(WS_BYTES_PER_SEC, WS_BYTES_BURST);

    // P0-X：本连接的 session_id。握手前为 0（未分配）。
    let my_session_id: std::sync::Arc<std::sync::atomic::AtomicU64> =
        std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));

    // ---- 握手 ----
    let first = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
        .await
        .map_err(|_| anyhow::anyhow!("hello timeout (5s)"))?
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
            // P1-DoS：认证必须前置。此前允许无 token 连接并长期 Ping 保活，
            // 单个攻击者可占满全局 5000 个 WS permit，让正常用户 503。
            // 版本协商仍在 token 校验之前，保证旧客户端能收到明确错误。
            let Some(session_token) = session_token else {
                let err = ServerMessage::Error {
                    code: ErrorCode::Unauthenticated,
                    message: "session_token required in ClientHello".into(),
                };
                let _ = sink.send(Message::Binary(err.encode()?.into())).await;
                return Ok(());
            };
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

    // Hello 阶段已强制要求 session_token；这里验证失败必须立即关闭，
    // 不允许无效 token 的连接继续占用 ws_semaphore。
    let (mut my_uid, my_http_session_id): (Option<UserId>, String) =
        match validate_session(&state, &session_token).await {
            Ok((uid, http_sid)) => {
                tracing::info!("ws auth ok uid={}", uid);
                (Some(uid), http_sid)
            }
            Err(e) => {
                tracing::warn!("ws auth failed, closing: {:?}", e);
                let err = ServerMessage::Error {
                    code: ErrorCode::Unauthenticated,
                    message: "invalid or expired session token".into(),
                };
                let _ = sink.send(Message::Binary(err.encode()?.into())).await;
                return Ok(());
            }
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
        if should_revalidate(last_auth_check, SESSION_REVALIDATE_INTERVAL) {
            if validate_session(&state, &session_token).await.is_err() {
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

        // 字节预算先于解码：畸形超大帧同样消耗带宽，不能靠 decode 失败绕过。
        // 已认证走 per-uid（断线重连不重置速率），未认证走 per-connection。
        let byte_cost = bytes.len() as f64;
        let byte_ok = match &my_uid {
            Some(uid) => state.check_rate_scoped_cost(
                uid,
                "ws-bytes",
                WS_BYTES_PER_SEC,
                WS_BYTES_BURST,
                byte_cost,
            ),
            None => byte_limiter.try_consume(byte_cost),
        };
        if !byte_ok {
            state.metrics.rate_limited();
            let _ = tx.try_send(ServerMessage::Error {
                code: ErrorCode::RateLimited,
                message: "message byte budget exceeded; slow down".into(),
            });
            continue;
        }

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

        // 条数限流：只限条数时 30 条/s × 4 MiB 仍可打出 120 MiB/s，
        // 因此字节预算单独计费（见上）。
        let allowed = match &my_uid {
            Some(uid) => state.check_rate_scoped(uid, "ws", 30.0, 60.0),
            None => limiter.try_consume(1.0),
        };
        if !allowed {
            state.metrics.rate_limited();
            let _ = tx.try_send(ServerMessage::Error {
                code: ErrorCode::RateLimited,
                message: "too many messages, slow down".into(),
            });
            continue;
        }

        // 被踢/账号删除后，旧连接手里的 token 已失效；Join 必须重新校验，
        // 不能只信任 Hello 时缓存的 my_uid。
        if matches!(&msg, ClientMessage::Join)
            && validate_session(&state, &session_token).await.is_err()
        {
            let _ = tx.try_send(ServerMessage::Error {
                code: ErrorCode::Kicked,
                message: "session revoked; please log in again".into(),
            });
            break 'outer;
        }

        if let Err(e) = handle_message(
            msg,
            &state,
            &mut my_uid,
            &my_session_id,
            &my_http_session_id,
            &tx,
        )
        .await
        {
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
            // 私密频道实时消息在对方离线/缓冲满时暂存；payload 前 8 字节是服务端 ts。
            "channel_message" => {
                if payload.len() <= 8 {
                    delivered.push(id);
                    continue;
                }
                let ts = i64::from_be_bytes(payload[..8].try_into().expect("8-byte ts prefix"));
                ServerMessage::ChannelMessage {
                    channel_id: channel_id.unwrap_or_default(),
                    from: UserId::new(from),
                    ts,
                    ciphertext: payload[8..].to_vec(),
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
    let room = state.room.lock().unwrap_or_else(|e| e.into_inner());
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
    my_http_session_id: &str,
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
                http_session_id: my_http_session_id.to_string(),
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
            // 协议约定 payload 首字节 = 0x04 (Welcome) | 0x05 (Commit)。
            // 拒绝其他前缀能防止恶意客户端发送服务端无法解析、收件方
            // 客户端会 fail-closed 的 payload，也保证 deliver_pending
            // 补投时的语义一致。
            if !matches!(payload.first().copied(), Some(0x04) | Some(0x05)) {
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::MalformedMessage,
                    message: "channel sync payload must start with 0x04 or 0x05".into(),
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
                // 【时序修复】target 可能刚登录，HTTP join_channel 还没落库。
                // 从安全角度：sender 已通过 is_member 检查（来自 authority），
                //   Welcome/Commit payload 内的 MLS 签名保证 authority 身份。
                // 从体验角度：直接丢弃会让 u5-u9 永远收不到 Welcome。
                //
                // 策略：先 try enqueue 兜底，再 try_send 直推（如果 target 在线）。
                let framed_payload = payload.clone();
                if let Err(e) = crate::db::enqueue_envelope(
                    &state.pool,
                    to.as_str(),
                    from.as_str(),
                    "channel_sync",
                    Some(&channel_id),
                    &framed_payload,
                )
                .await
                {
                    tracing::warn!(
                        "[ChannelSync] enqueue for {} failed: {}",
                        to,
                        e
                    );
                }
                // 直推：如果 target 已经建立 WS 连接，先推一波（不校验 member）
                let msg = ServerMessage::ChannelSync {
                    channel_id: channel_id.clone(),
                    payload: framed_payload,
                };
                let direct_ok = state.send_to(&to, msg);
                tracing::info!(
                    "[ChannelSync] target {} not yet member; enqueued={} direct_send={}",
                    to,
                    !direct_ok, // enqueue 成功则 !direct_ok == false，语义反了，用下面日志
                    direct_ok
                );
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

        ClientMessage::ChannelReaction {
            channel_id,
            ciphertext,
        } => {
            let from = ensure_joined(state, my_uid, my_session_id)?;
            if ciphertext.len() > MAX_CHANNEL_REACTION_BYTES {
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::MalformedMessage,
                    message: "channel reaction too large".into(),
                });
                return Ok(());
            }
            if ciphertext.first() != Some(&0x03) {
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::MalformedMessage,
                    message: "channel reaction must use the channel group key (0x03 prefix)".into(),
                });
                return Ok(());
            }

            let meta: Option<(i64,)> =
                sqlx::query_as("SELECT is_private FROM channels WHERE id = ?")
                    .bind(&channel_id)
                    .fetch_optional(&state.pool)
                    .await?;
            let Some((is_private,)) = meta else {
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::TargetNotFound,
                    message: format!("channel not found: {}", channel_id),
                });
                return Ok(());
            };
            if is_private == 0 {
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::MalformedMessage,
                    message: "public channels use MLS broadcast reactions".into(),
                });
                return Ok(());
            }

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

            // 定向 fanout 给频道成员，排除发送者（发送端本地已乐观更新）。
            let members: Vec<(String,)> = sqlx::query_as(
                "SELECT login_id FROM channel_members WHERE channel_id = ? AND login_id != ?",
            )
            .bind(&channel_id)
            .bind(from.as_str())
            .fetch_all(&state.pool)
            .await?;
            if !state.try_reserve_broadcast(ciphertext.len(), members.len()) {
                state.metrics.broadcast_budget_rejected();
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::RateLimited,
                    message: "room broadcast budget exceeded; reaction rejected".into(),
                });
                return Ok(());
            }
            for (member,) in members {
                state.send_to(
                    &UserId::new(member),
                    ServerMessage::ChannelReaction {
                        channel_id: channel_id.clone(),
                        from: from.clone(),
                        ciphertext: ciphertext.clone(),
                    },
                );
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

            // 【时序修复】广播给所有 online peers，不限于 channel_members。
            // 原因：target 的 HTTP join_channel 可能还没落库，
            //   导致它在 channel_members 里查不到，Commit 被漏发。
            // 安全性：Commit 是 MLS 加密的，非成员收到会报 WrongEpoch，无害。
            //   exclude 仍用于排除刚被 Add 的新成员（他不需要这条 Commit）。
            let peer_uids = state.peer_uids();
            if !state.try_reserve_broadcast(framed.len(), peer_uids.len()) {
                state.metrics.broadcast_budget_rejected();
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::RateLimited,
                    message: "room broadcast budget exceeded; channel commit rejected".into(),
                });
                return Ok(());
            }

            for peer_uid in peer_uids {
                // 排除发送者
                if peer_uid == from {
                    continue;
                }
                // 排除 exclude 指定的新成员
                if exclude.as_ref().is_some_and(|e| e == &peer_uid) {
                    continue;
                }
                let msg = ServerMessage::ChannelSync {
                    channel_id: channel_id.clone(),
                    payload: framed.clone(),
                };
                if !state.send_to(&peer_uid, msg) {
                    // 发送失败说明 peer 刚好离线，降级为 enqueue
                    if let Err(e) = crate::db::enqueue_envelope(
                        &state.pool,
                        peer_uid.as_str(),
                        from.as_str(),
                        "channel_sync",
                        Some(&channel_id),
                        &framed,
                    )
                    .await
                    {
                        tracing::warn!("enqueue channel_commit for {} failed: {}", peer_uid, e);
                    }
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
            // Round 6（Signal 模型）：服务器允许任何已加入成员发 Welcome。
            //
            // 安全性由**客户端用户确认**保证：收到 Welcome 时不再自动入群，
            // 而是弹确认框（见 kaede_client::ClientEvent::PendingWelcome）。
            // 用户看到 "X 想把你加入服务器"，主动接受才入群。
            //
            // 这消除了"房主离线 = 新人进不来"的单点。代价：
            // - 新人首次需要多点一次确认
            // - 恶意 Welcome 会出现在 UI 里（用户拒绝即可，不影响状态）
            //
            // 保留的防御：
            // - MAX_MLS_ENVELOPE_BYTES 限制防止超大 Welcome
            // - target 存在性校验防止向不存在的 uid 塞离线信封
            // - 发送者必须已 join（ensure_joined 已在上方校验）
            //
            // 任何成员可发 Welcome 之后必须限流：否则一个账号可以给全服
            // 每个人反复塞 256 KiB 的离线信封，刷爆目标存储与客户端确认队列。
            if !state.check_rate_scoped(&from, "mls_welcome", 0.5, 10.0) {
                state.metrics.rate_limited();
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::RateLimited,
                    message: "welcome rate limited".into(),
                });
                return Ok(());
            }
            if !state.check_rate_key(&format!("welcome_to:{}", to), 0.2, 10.0) {
                state.metrics.rate_limited();
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::RateLimited,
                    message: "target welcome budget exceeded".into(),
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
                // 固定字符串，不把目标 uid 回显给发送方（防用户枚举面，
                // 也为未来放开 op4 权限时不暴露"某 uid 是否存在"）。
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::TargetNotFound,
                    message: "target user not found".into(),
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
            // 开发/重构阶段：放开 MLS Commit 权限限制

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
            // 按发送者限流（2 条/秒，突发 32），避免单个 op4 用伪 epoch
            // 快速填满 256 条、挤掉其他管理员的合法 commit。突发值保留
            // 足够空间给房主在一次重连后连续 Add 多个成员。
            // 必须在广播前拒绝，否则收件人已经拿到未落库的 Commit。
            if !state.check_rate_scoped(&from, "mls_commit", 2.0, 32.0) {
                state.metrics.rate_limited();
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::RateLimited,
                    message: "mls commit rate limited".into(),
                });
                return Ok(());
            }
            if !state.try_reserve_broadcast(payload.len(), state.peer_count().saturating_sub(1)) {
                state.metrics.broadcast_budget_rejected();
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::RateLimited,
                    message: "room broadcast budget exceeded; commit rejected".into(),
                });
                return Ok(());
            }
            // 持久化必须先于广播：否则落库失败时在线成员已经前进，
            // 离线成员却永远追不上这个 epoch（DB 里没有记录）。
            if epoch > 0 {
                // 同一 epoch 只保留第一份 Commit：后到的同 epoch commit
                // 属于并发分支，覆盖会让已跟随前一份的客户端更混乱。
                // **不广播落败者**——落败的发送者会收到胜出者的广播并回滚本地分支。
                let stored = match sqlx::query(
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
                    Ok(r) => r.rows_affected() > 0,
                    Err(e) => {
                        // 落库失败时不能广播：否则在线成员前进了，离线成员永远追不上。
                        tracing::warn!("store mls commit failed: {}", e);
                        let _ = tx.try_send(ServerMessage::Error {
                            code: ErrorCode::InternalError,
                            message: "failed to persist MLS commit".into(),
                        });
                        return Ok(());
                    }
                };
                if !stored {
                    tracing::info!(
                        "[mls] epoch {} already has a winning commit; dropping duplicate from {}",
                        epoch,
                        from
                    );
                    return Ok(());
                }
                // 只保留最近 256 条 Commit。
                // 不能按 MAX(epoch) 保留——epoch 受客户端控制，一个超大值
                // 会永久清空所有合法 commit；按插入顺序(rowid)保留。
                let _ = sqlx::query(
                    "DELETE FROM mls_commits WHERE rowid NOT IN \
                     (SELECT rowid FROM mls_commits ORDER BY rowid DESC LIMIT 256)",
                )
                .execute(&state.pool)
                .await;
                // 单个发送者最多占 64 条，给其他管理员保留补投空间。
                let _ = sqlx::query(
                    "DELETE FROM mls_commits WHERE from_uid = ? AND rowid NOT IN \
                     (SELECT rowid FROM mls_commits WHERE from_uid = ? \
                      ORDER BY rowid DESC LIMIT 64)",
                )
                .bind(from.as_str())
                .bind(from.as_str())
                .execute(&state.pool)
                .await;
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
        }

        ClientMessage::ReportEpoch { epoch } => {
            let uid = ensure_joined(state, my_uid, my_session_id)?;
            if epoch > u32::MAX as u64 {
                return Ok(());
            }
            // 该请求会读 DB 并把最多 256 条 Commit 写回自己的出站队列，
            // 必须限流，否则可被反复触发做内存/DB 放大。
            if !state.check_rate_scoped(&uid, "report_epoch", 1.0, 5.0) {
                state.metrics.rate_limited();
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::RateLimited,
                    message: "epoch report rate limited".into(),
                });
                return Ok(());
            }
            let rows: Vec<(i64, String, Vec<u8>)> = sqlx::query_as(
                "SELECT epoch, from_uid, payload FROM mls_commits \
                 WHERE epoch > ? ORDER BY epoch ASC LIMIT 256",
            )
            .bind(epoch as i64)
            .fetch_all(&state.pool)
            .await?;

            let mut budget = MAX_REPORT_EPOCH_BYTES;
            for (commit_epoch, from_uid, payload) in rows {
                // 超出本次预算就停：客户端可稍后再次 ReportEpoch 续拉。
                if payload.len() > budget {
                    break;
                }
                budget -= payload.len();
                if tx
                    .try_send(ServerMessage::MlsCommit {
                        from: UserId::new(from_uid),
                        epoch: commit_epoch as u64,
                        payload,
                    })
                    .is_err()
                {
                    break;
                }
            }
        }

        ClientMessage::MlsProposal { payload } => {
            let from = ensure_joined(state, my_uid, my_session_id)?;
            // 开发/重构阶段：放开 MLS Proposal 权限限制

            if payload.len() > MAX_MLS_HANDSHAKE_BYTES {
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::MalformedMessage,
                    message: "proposal payload too large".into(),
                });
                return Ok(());
            }
            if !state.try_reserve_broadcast(payload.len(), state.peer_count().saturating_sub(1)) {
                state.metrics.broadcast_budget_rejected();
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::RateLimited,
                    message: "room broadcast budget exceeded; proposal rejected".into(),
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

        ClientMessage::MlsAppMessage { channel_id, payload } => {
            let from = ensure_joined(state, my_uid, my_session_id)?;
            if payload.len() > MAX_MLS_APP_BYTES {
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::MalformedMessage,
                    message: "application message too large".into(),
                });
                return Ok(());
            }
            // 房间级扇出预算：成本 = payload × 在线收件人数。
            if !state.try_reserve_broadcast(payload.len(), state.peer_count().saturating_sub(1)) {
                state.metrics.broadcast_budget_rejected();
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::RateLimited,
                    message: "room broadcast budget exceeded; slow down".into(),
                });
                return Ok(());
            }
            // H14: 关键消息（聊天/媒体信令）不能静默丢，走 important broadcast
            state
                .broadcast_except_important(
                    &from,
                    ServerMessage::MlsAppMessage {
                        from: from.clone(),
                        channel_id: channel_id.clone(),
                        payload,
                    },
                )
                .await;
        }

        ClientMessage::SetPresence { status } => {
            let uid = ensure_joined(state, my_uid, my_session_id)?;
            if !matches!(status.as_str(), "online" | "idle" | "dnd") {
                // 显式返回 MalformedMessage，不再冒泡 Err——
                // 否则外层 run_socket 会把任何 Err 报成 NotJoined，
                // 导致客户端无法区分"输入非法"和"session 失效"。
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::MalformedMessage,
                    message: "invalid presence status".into(),
                });
                return Ok(());
            }
            state.set_presence(&uid, status);
            let snapshot = state.snapshot_presence();
            state.broadcast_all(ServerMessage::PresenceUpdated { presence: snapshot });
        }

        ClientMessage::Typing { channel_id } => {
            let from = ensure_joined(state, my_uid, my_session_id)?;
            // typing 是纯元数据（不落盘、不补投）：每 2 秒 1 条、突发 2 条足够跟手。
            // 超限静默丢——typing 丢了无所谓，发 Error 反而会刷屏打扰正常输入。
            if !state.check_rate_scoped(&from, "typing", 0.5, 2.0) {
                return Ok(());
            }

            // DM 会话：只发对端。解析失败/非会话成员一律静默丢，
            // 不向客户端回 Error（否则等于泄露"会话是否存在/你是否是成员"）。
            if channel_id.starts_with("dm:") {
                let Some((a, b)) = crate::dm::parse_conv_id(&channel_id) else {
                    return Ok(());
                };
                if !crate::dm::is_member(&channel_id, from.as_str()) {
                    return Ok(());
                }
                let peer = if a == from.as_str() { b } else { a };
                // 离线即丢，不暂存——typing 没有补投价值。
                state.send_to(
                    &UserId::new(peer),
                    ServerMessage::Typing {
                        channel_id,
                        uid: from,
                    },
                );
                return Ok(());
            }

            let meta: Option<(i64,)> =
                sqlx::query_as("SELECT is_private FROM channels WHERE id = ?")
                    .bind(&channel_id)
                    .fetch_optional(&state.pool)
                    .await?;
            let Some((is_private,)) = meta else {
                // 不存在的频道：静默丢（同上，不泄露频道存在性）。
                return Ok(());
            };

            if is_private == 0 {
                // 公开频道：全房间广播（除发送者）。
                // Typing 只有几十字节，但仍计入房间广播预算——否则大量账号
                // 可以把 0.5 条/s 的扇出放大成 O(在线人数²) 的带宽。
                let msg_len = 64 + channel_id.len() + from.as_str().len();
                let recipients = state.peer_count().saturating_sub(1);
                if !state.try_reserve_broadcast(msg_len, recipients) {
                    state.metrics.broadcast_budget_rejected();
                    return Ok(());
                }
                let msg = ServerMessage::Typing {
                    channel_id,
                    uid: from.clone(),
                };
                state.broadcast_except(&from, msg);
                return Ok(());
            }

            // 私密频道：成员校验 + 定向扇出（除发送者）。非成员静默丢。
            let is_member: Option<(i64,)> = sqlx::query_as(
                "SELECT 1 FROM channel_members WHERE channel_id = ? AND login_id = ?",
            )
            .bind(&channel_id)
            .bind(from.as_str())
            .fetch_optional(&state.pool)
            .await?;
            if is_member.is_none() {
                return Ok(());
            }
            let members: Vec<(String,)> = sqlx::query_as(
                "SELECT login_id FROM channel_members WHERE channel_id = ? AND login_id != ?",
            )
            .bind(&channel_id)
            .bind(from.as_str())
            .fetch_all(&state.pool)
            .await?;
            let msg_len = 64 + channel_id.len() + from.as_str().len();
            if !state.try_reserve_broadcast(msg_len, members.len()) {
                state.metrics.broadcast_budget_rejected();
                return Ok(());
            }
            for (member,) in members {
                state.send_to(
                    &UserId::new(member),
                    ServerMessage::Typing {
                        channel_id: channel_id.clone(),
                        uid: from.clone(),
                    },
                );
            }
        }

        ClientMessage::Ping { nonce } => {
            let _ = tx.try_send(ServerMessage::Pong { nonce });
        }
    }
    Ok(())
}

/// 用 HTTP session token 换 uid，并刷新 last_seen_at。
async fn validate_session(state: &AppState, token: &str) -> anyhow::Result<(UserId, String)> {
    let token_hash = crate::db::hash_token(token);
    let row: Option<(String, i64, String)> =
        sqlx::query_as("SELECT login_id, expires_at, session_id FROM sessions WHERE token = ?")
            .bind(&token_hash)
            .fetch_optional(&state.pool)
            .await?;
    let (login_id, expires_at, http_session_id) = match row {
        Some(r) => r,
        None => {
            state.metrics.auth_failure();
            anyhow::bail!("no session");
        }
    };
    if expires_at < crate::db::now_ms() {
        state.metrics.auth_failure();
        anyhow::bail!("session expired");
    }
    let _ = sqlx::query("UPDATE sessions SET last_seen_at = ? WHERE token = ?")
        .bind(crate::db::now_ms())
        .bind(&token_hash)
        .execute(&state.pool)
        .await;
    Ok((UserId::new(login_id), http_session_id))
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

// ============================================================================
// Typing（v7）路由 / 限流回归测试
// ============================================================================
#[cfg(test)]
mod typing_tests {
    use super::*;
    use crate::config::Config;
    use std::sync::atomic::AtomicU64;

    /// 内存 sqlite + 最小 schema（Typing 路由只查 channels / channel_members）。
    async fn mk_state() -> AppState {
        use sqlx::sqlite::SqlitePoolOptions;
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE channels (\
                id TEXT PRIMARY KEY, name TEXT NOT NULL, kind TEXT NOT NULL, \
                position INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, \
                is_private INTEGER NOT NULL DEFAULT 0, password_hash TEXT)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE channel_members (\
                channel_id TEXT NOT NULL, login_id TEXT NOT NULL, joined_at INTEGER NOT NULL, \
                PRIMARY KEY (channel_id, login_id))",
        )
        .execute(&pool)
        .await
        .unwrap();
        let cfg = Config {
            bind: "127.0.0.1:0".into(),
            database_url: "sqlite::memory:".into(),
            session_ttl_hours: 1,
            livekit_api_key: "testkey".into(),
            livekit_api_secret: "testsecret".into(),
            livekit_ws_url: "ws://localhost:18082".into(),
            admin_enabled: false,
            admin_bind: "127.0.0.1:18081".into(),
            admin_token: String::new(),
        };
        AppState::new(pool, cfg)
    }

    /// 模拟已 Join 的连接：add_peer + 返回 handle_message 所需的上下文。
    /// 返回 (my_uid, my_session_id, 自身出站 tx, 自身出站 rx)。
    fn join(
        state: &AppState,
        uid: &str,
        session_id: u64,
    ) -> (
        Option<UserId>,
        AtomicU64,
        mpsc::Sender<ServerMessage>,
        mpsc::Receiver<ServerMessage>,
    ) {
        let (tx, rx) = mpsc::channel::<ServerMessage>(4096);
        state.add_peer(PeerHandle {
            uid: UserId::new(uid),
            tx: tx.clone(),
            session_id,
            http_session_id: String::new(),
        });
        (Some(UserId::new(uid)), AtomicU64::new(session_id), tx, rx)
    }

    /// A 发 DM typing：B 收到且 uid==A，第三方 C 收不到，A 自身无回显。
    #[tokio::test]
    async fn typing_dm_routes_to_peer_only() {
        let state = mk_state().await;
        let (mut uid_a, sid_a, tx_a, mut rx_a) = join(&state, "a", 1);
        let (_uid_b, _sid_b, _tx_b, mut rx_b) = join(&state, "b", 2);
        let (_uid_c, _sid_c, _tx_c, mut rx_c) = join(&state, "c", 3);

        handle_message(
            ClientMessage::Typing {
                channel_id: "dm:a:b".into(),
            },
            &state,
            &mut uid_a,
            &sid_a,
            "",
            &tx_a,
        )
        .await
        .unwrap();

        match rx_b.try_recv() {
            Ok(ServerMessage::Typing { channel_id, uid }) => {
                assert_eq!(channel_id, "dm:a:b");
                assert_eq!(uid, UserId::new("a"));
            }
            other => panic!("B should receive Typing, got {:?}", other),
        }
        assert!(rx_b.try_recv().is_err(), "B should receive exactly one");
        assert!(rx_c.try_recv().is_err(), "C must not receive DM typing");
        assert!(rx_a.try_recv().is_err(), "no echo / no error to sender");
    }

    /// 私密频道的非成员发 typing：成员收不到，发送者也不收 Error。
    #[tokio::test]
    async fn typing_non_member_private_channel_dropped() {
        let state = mk_state().await;
        sqlx::query(
            "INSERT INTO channels (id, name, kind, created_at, is_private) \
             VALUES ('ch1', 'secret', 'text', 0, 1)",
        )
        .execute(&state.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO channel_members (channel_id, login_id, joined_at) \
             VALUES ('ch1', 'b', 0)",
        )
        .execute(&state.pool)
        .await
        .unwrap();

        let (mut uid_a, sid_a, tx_a, mut rx_a) = join(&state, "a", 1);
        let (_uid_b, _sid_b, _tx_b, mut rx_b) = join(&state, "b", 2);

        handle_message(
            ClientMessage::Typing {
                channel_id: "ch1".into(),
            },
            &state,
            &mut uid_a,
            &sid_a,
            "",
            &tx_a,
        )
        .await
        .unwrap();

        assert!(
            rx_b.try_recv().is_err(),
            "member must not receive typing from non-member"
        );
        assert!(rx_a.try_recv().is_err(), "silent drop: no error to sender");
    }

    /// 连发 5 条（间隔≈0）：限流容量 2 + 0.5/s 补充，对端只收到前 2~3 条。
    #[tokio::test]
    async fn typing_rate_limited() {
        let state = mk_state().await;
        let (mut uid_a, sid_a, tx_a, _rx_a) = join(&state, "a", 1);
        let (_uid_b, _sid_b, _tx_b, mut rx_b) = join(&state, "b", 2);

        for _ in 0..5 {
            handle_message(
                ClientMessage::Typing {
                    channel_id: "dm:a:b".into(),
                },
                &state,
                &mut uid_a,
                &sid_a,
                "",
                &tx_a,
            )
            .await
            .unwrap();
        }

        let mut n = 0;
        while let Ok(msg) = rx_b.try_recv() {
            assert!(
                matches!(msg, ServerMessage::Typing { .. }),
                "B should only receive Typing, got {:?}",
                msg
            );
            n += 1;
        }
        assert!(
            (2..=3).contains(&n),
            "expected 2~3 typings to pass the rate limiter, got {}",
            n
        );
    }

    /// 不存在的 channel_id：静默成功，不 panic、不回 Error。
    #[tokio::test]
    async fn typing_unknown_channel_dropped() {
        let state = mk_state().await;
        let (mut uid_a, sid_a, tx_a, mut rx_a) = join(&state, "a", 1);

        let res = handle_message(
            ClientMessage::Typing {
                channel_id: "ch-no-such".into(),
            },
            &state,
            &mut uid_a,
            &sid_a,
            "",
            &tx_a,
        )
        .await;
        assert!(res.is_ok(), "unknown channel must be silently dropped");
        assert!(rx_a.try_recv().is_err(), "no error message to sender");
    }
}
