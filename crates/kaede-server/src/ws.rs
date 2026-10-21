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
    loop {
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

        // P1-M：5 分钟无任何消息（含 ping）则断开，防 5000 空闲连接耗尽 semaphore。
        let msg = match recv_with_idle_timeout(&mut stream, WS_IDLE_TIMEOUT).await {
            RecvOutcome::Timeout => {
                tracing::debug!("ws idle timeout ({}s)", WS_IDLE_TIMEOUT.as_secs());
                break;
            }
            RecvOutcome::Closed => break,
            RecvOutcome::Error(e) => {
                tracing::debug!("ws read error: {}", e);
                break;
            }
            RecvOutcome::Message(m) => m,
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
            Some(uid) => state.check_rate(uid, 30.0, 60.0),
            None => limiter.try_consume(1.0),
        };
        if !allowed {
            let _ = tx.try_send(ServerMessage::Error {
                code: ErrorCode::RateLimited,
                message: "too many messages, slow down".into(),
            });
            continue;
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
    let envelopes = match crate::db::take_envelopes(&state.pool, uid.as_str()).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("take_envelopes for {} failed: {}", uid, e);
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
    for (from, kind, channel_id, payload) in envelopes {
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
                        continue;
                    }
                }
            }
            _ => ServerMessage::ChannelSync {
                channel_id: channel_id.unwrap_or_default(),
                payload,
            },
        };
        if tx.try_send(msg).is_err() {
            // 出站队列满：把这条重新放回队首是做不到的；记录并继续。
            // 正常情况下 1024 容量的队列在 Join 阶段不会满。
            tracing::warn!("pending envelope dropped for {} (outbound queue full)", uid);
        }
    }
}

fn extract_bytes(m: Message) -> anyhow::Result<Vec<u8>> {
    match m {
        Message::Binary(b) => Ok(b.to_vec()),
        Message::Text(_) => anyhow::bail!("text frames not supported; use binary CBOR"),
        _ => anyhow::bail!("expected binary/text"),
    }
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

        ClientMessage::SdpOffer { to, sdp } => {
            let from = ensure_joined(state, my_uid, my_session_id)?;
            if !state.send_to(&to, ServerMessage::SdpOffer { from, sdp }) {
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::TargetNotFound,
                    message: to.0,
                });
            }
        }

        ClientMessage::SdpAnswer { to, sdp } => {
            let from = ensure_joined(state, my_uid, my_session_id)?;
            if !state.send_to(&to, ServerMessage::SdpAnswer { from, sdp }) {
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::TargetNotFound,
                    message: to.0,
                });
            }
        }

        ClientMessage::IceCandidate { to, candidate } => {
            let from = ensure_joined(state, my_uid, my_session_id)?;
            if !state.send_to(&to, ServerMessage::IceCandidate { from, candidate }) {
                let _ = tx.try_send(ServerMessage::Error {
                    code: ErrorCode::TargetNotFound,
                    message: to.0,
                });
            }
        }

        ClientMessage::ChannelSync {
            to,
            channel_id,
            payload,
        } => {
            let from = ensure_joined(state, my_uid, my_session_id)?;
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
                // 只保留最近 256 个 epoch。
                let _ = sqlx::query(
                    "DELETE FROM mls_commits WHERE epoch <= \
                     (SELECT COALESCE(MAX(epoch), 0) - 256 FROM mls_commits)",
                )
                .execute(&state.pool)
                .await;
            }
        }

        ClientMessage::ReportEpoch { epoch } => {
            let _uid = ensure_joined(state, my_uid, my_session_id)?;
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

        ClientMessage::SfuTokenRequest => {
            let _ = tx.try_send(ServerMessage::Error {
                code: ErrorCode::InternalError,
                message: "sfu token not implemented".into(),
            });
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

/// 用 HTTP session token 换 uid。
async fn validate_session(state: &AppState, token: &str) -> anyhow::Result<UserId> {
    let row: Option<(String, i64)> =
        sqlx::query_as("SELECT login_id, expires_at FROM sessions WHERE token = ? OR token = ?")
            .bind(crate::db::hash_token(token))
            .bind(token)
            .fetch_optional(&state.pool)
            .await?;
    let (login_id, expires_at) = row.ok_or_else(|| anyhow::anyhow!("no session"))?;
    if expires_at < crate::db::now_ms() {
        anyhow::bail!("session expired");
    }
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
