//! 客户端事件 → UI 状态的桥接。

use async_channel::{Receiver, Sender, unbounded};
#[allow(unused_imports)]
use dioxus::prelude::*;
use kaede_client::{ClientEvent, ClientObserver, DecodedPayload};
use kaede_protocol::UserId;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

use crate::state::{AppState, MainTab, UiMessage, UiState, View, VoiceState};

pub struct UiObserver {
    tx: Sender<ClientEvent>,
}

impl UiObserver {
    pub fn new() -> (Self, Receiver<ClientEvent>) {
        let (tx, rx) = unbounded();
        (Self { tx }, rx)
    }
}

impl ClientObserver for UiObserver {
    fn on_event(&self, event: ClientEvent) {
        let _ = self.tx.try_send(event);
    }
}

pub fn apply_event(
    mut app: AppState,
    mut ui: UiState,
    mut voice: VoiceState,
    observer: std::sync::Arc<dyn kaede_client::ClientObserver>,
    event: ClientEvent,
) {
    match event {
        ClientEvent::StateChanged { server_id, to, .. } => {
            let mut s = app.servers.write();
            s.entry(server_id).or_default().state = to;
        }

        ClientEvent::HelloAck {
            server_id,
            server_name,
            ..
        } => {
            let mut s = app.servers.write();
            let srv = s.entry(server_id.clone()).or_default();
            srv.id = server_id;
            srv.name = server_name;
        }

        ClientEvent::ChannelCreated { server_id, channel } => {
            let mut s = app.channels.write();
            let list = s.entry(server_id).or_default();
            if list.iter().any(|c| c.id == channel.id) {
                return;
            }
            list.push(channel);
            list.sort_by_key(|c| c.position);
        }

        ClientEvent::ChannelUpdated { server_id, channel } => {
            let mut s = app.channels.write();
            if let Some(list) = s.get_mut(&server_id)
                && let Some(slot) = list.iter_mut().find(|c| c.id == channel.id)
            {
                *slot = channel;
            }
        }

        ClientEvent::ChannelDeleted {
            server_id,
            channel_id,
        } => {
            let mut s = app.channels.write();
            if let Some(list) = s.get_mut(&server_id) {
                list.retain(|c| c.id != channel_id);
            }
        }

        ClientEvent::ChannelJoined {
            server_id,
            channel_id,
        } => {
            ui.toast
                .set(Some((format!("已加入频道 {}", channel_id), false)));
            // 迟到的 Welcome 可能发生在 10s 超时之后：刷新频道列表让 UI 出现该频道。
            let mut app2 = app;
            spawn(async move {
                if let Some(room) = app2.room.read().clone()
                    && let Ok(list) = room.list_channels().await
                {
                    app2.channels.write().insert(server_id, list);
                }
            });
        }

        ClientEvent::PresenceUpdated { presence, .. } => {
            let mut s = app.presence.write();
            for p in presence {
                s.insert(p.login_id, p.status);
            }
        }

        ClientEvent::MemberUpdated { server_id, member } => {
            let mut s = app.members.write();
            let list = s.entry(server_id).or_default();
            if let Some(slot) = list.iter_mut().find(|m| m.login_id == member.login_id) {
                *slot = member;
            } else {
                list.push(member);
            }
        }

        ClientEvent::MemberRemoved {
            server_id,
            login_id,
        } => {
            let mut s = app.members.write();
            if let Some(list) = s.get_mut(&server_id) {
                list.retain(|m| m.login_id != login_id);
            }
        }

        ClientEvent::FriendsChanged { server_id } => {
            // 只刷新当前 server 的缓存；切服时 use_effect 会重新拉取。
            let is_current = app.current_server.read().as_deref() == Some(server_id.as_str());
            if is_current {
                spawn(async move {
                    crate::friends_view::load_friends_into(app).await;
                });
            }
        }

        ClientEvent::AttachmentProgress {
            server_id: _,
            name,
            sent,
            total,
        } => {
            let pct = if total == 0 {
                0
            } else {
                ((sent.saturating_mul(100) / total).min(100)) as u32
            };
            ui.upload_progress.set(Some((name, pct)));
        }

        ClientEvent::AppMessage {
            server_id,
            from,
            plaintext,
        } => {
            // 自己发的消息本地已乐观插入，跳过服务器 echo
            let self_uid = app
                .room
                .read()
                .as_ref()
                .map(|r| r.uid().as_str().to_string());
            if let Some(uid) = self_uid.as_deref()
                && from.as_str() == uid
            {
                return;
            }
            match kaede_client::decode(&plaintext) {
                DecodedPayload::Message(p) => {
                    if let Some(msg) = decode_message_payload(&server_id, &from, p) {
                        // 只接受公开频道的主群广播：私密频道必须走 ChannelMessage。
                        let accepted = app
                            .channels
                            .read()
                            .get(&server_id)
                            .and_then(|list| list.iter().find(|c| c.id == msg.channel_id))
                            .map(|c| !c.is_private)
                            .unwrap_or(false);
                        if !accepted {
                            tracing::warn!(
                                "drop message for unknown/private channel {} (main-group broadcast)",
                                msg.channel_id
                            );
                            return;
                        }
                        let viewing = *ui.view.read() == View::Main
                            && *ui.main_tab.read() == MainTab::Server
                            && ui.selected_channel.read().as_deref()
                                == Some(msg.channel_id.as_str());
                        if !viewing {
                            let mention_me = self_uid
                                .as_deref()
                                .is_some_and(|uid| msg.mentions.iter().any(|m| m == uid));
                            app.bump_unread(&server_id, &msg.channel_id, mention_me);
                        }
                        let mut s = app.messages.write();
                        let key = AppState::channel_key(&server_id, &msg.channel_id);
                        s.entry(key).or_default().push(msg);
                    }
                }
                DecodedPayload::Reaction(r) => {
                    let accepted = app
                        .channels
                        .read()
                        .get(&server_id)
                        .and_then(|list| list.iter().find(|c| c.id == r.c))
                        .map(|c| !c.is_private)
                        .unwrap_or(false);
                    if accepted {
                        // 用 MLS 注入的 from 覆盖 payload 声明的 u，防伪造反应。
                        apply_reaction(&mut app, &server_id, &r, from.as_str());
                    }
                }
                _ => {}
            }
        }

        ClientEvent::ChannelMessage {
            server_id,
            channel_id,
            from,
            ts,
            plaintext,
        } => {
            // 私密频道实时消息：频道 id 以服务器事件为准，时间用服务器 ts。
            if let Some(p) = kaede_client::MessagePayload::parse(&plaintext) {
                let author = from.as_str().to_string();
                let initial = author
                    .chars()
                    .next()
                    .map(|c| c.to_string())
                    .unwrap_or_default();
                let msg = UiMessage {
                    id: format!("{}::{}::{}::{}", server_id, author, ts, next_seq()),
                    channel_id: channel_id.clone(),
                    author,
                    author_initial: initial,
                    day: day_label(ts),
                    time: format_hhmm(ts),
                    ts,
                    text: p.t,
                    attachments: p.a,
                    reply_to: p.r,
                    reactions: Vec::new(),
                    mentions: p.m,
                };
                let viewing = *ui.view.read() == View::Main
                    && *ui.main_tab.read() == MainTab::Server
                    && ui.selected_channel.read().as_deref() == Some(channel_id.as_str());
                if !viewing {
                    let me = app
                        .room
                        .read()
                        .as_ref()
                        .map(|r| r.uid().as_str().to_string());
                    let mention_me = me.is_some_and(|uid| msg.mentions.iter().any(|m| m == &uid));
                    app.bump_unread(&server_id, &channel_id, mention_me);
                }
                let mut s = app.messages.write();
                let key = AppState::channel_key(&server_id, &channel_id);
                let list = s.entry(key).or_default();
                if !list.iter().any(|m| m.ts == ts && m.author == msg.author) {
                    list.push(msg);
                    list.sort_by_key(|m| m.ts);
                }
            }
        }

        ClientEvent::DmReceived {
            server_id,
            conv_id,
            from,
            ts,
            plaintext,
        } => {
            if let Some(p) = kaede_client::MessagePayload::parse(&plaintext) {
                // C19 (DM 侧): 强制用 server 注入的 from 覆盖 payload 声明的 u。
                // 与主群 AppMessage 的处理保持一致（见 decode_message_payload）。
                if !p.u.is_empty() && p.u.as_str() != from.as_str() {
                    tracing::warn!(
                        "dm sender mismatch: payload.u={} but server from={}; using from",
                        p.u,
                        from
                    );
                }
                // 统一使用服务器 ts：排序、去重、翻页游标必须同源。
                // 仅在旧服务器没有 ts 时回退到 payload ts。
                let actual_ts = if ts > 0 { ts } else { p.ts };
                let msg = UiMessage {
                    id: format!("dm-{}-{}", conv_id, actual_ts),
                    channel_id: conv_id.clone(),
                    author: from.as_str().to_string(),
                    author_initial: p
                        .u
                        .chars()
                        .next()
                        .map(|c| c.to_string())
                        .unwrap_or_default(),
                    day: crate::observer::day_label(ts),
                    time: crate::observer::format_hhmm(ts),
                    ts: actual_ts,
                    text: p.t,
                    attachments: p.a,
                    reply_to: p.r,
                    reactions: vec![],
                    mentions: p.m,
                };
                let viewing = *ui.view.read() == View::Main
                    && *ui.main_tab.read() == MainTab::Dm
                    && ui.selected_dm_conv.read().as_deref() == Some(conv_id.as_str());
                if !viewing {
                    let me = app
                        .room
                        .read()
                        .as_ref()
                        .map(|r| r.uid().as_str().to_string());
                    let is_me = me.as_deref() == Some(from.as_str());
                    let mention_me = me.is_some_and(|uid| msg.mentions.iter().any(|m| m == &uid));
                    if !is_me {
                        app.bump_unread(&server_id, &conv_id, mention_me);
                    }
                }
                let mut map = app.dm_messages.write();
                let list = map.entry(conv_id).or_default();
                if !list.iter().any(|m| m.ts == actual_ts) {
                    list.push(msg);
                    list.sort_by_key(|m| m.ts);
                }
            }
        }
        ClientEvent::Disconnected { server_id, reason } => {
            {
                let mut s = app.servers.write();
                if let Some(srv) = s.get_mut(&server_id) {
                    srv.state = kaede_client::ClientState::Disconnected;
                }
            }
            // M55: 清理连接级状态，防 stale host / in-flight add
            app.is_host.set(None);
            app.pending_adds.write().clear();

            // M55 扩展：清理 VoiceState（如果断线时在语音里）
            // 同时通知服务器结束媒体会话，否则 seed 会一直保留到 1 小时清理。
            if let Some(ch) = voice.active_channel.read().as_ref().map(|c| c.id.clone()) {
                if let Some(room) = app.room.read().clone() {
                    spawn(async move {
                        let _ = room.leave_media_room(&ch).await;
                    });
                }
            }
            voice.active_channel.set(None);

            #[cfg(not(target_arch = "wasm32"))]
            {
                if let Some(h) = voice.handle.write().take() {
                    spawn(async move {
                        h.inner().disconnect().await;
                    });
                }
                // 清理所有挂起的 ConsumerHandle
                let consumers = std::mem::take(&mut *voice.media_consumers.write());
                spawn(async move {
                    for (_, h) in consumers {
                        h.stop().await;
                    }
                });
            }

            // 自动重连（Kicked / 版本不兼容等不可恢复错误除外）。
            maybe_auto_reconnect(app, ui, observer, server_id, &reason);
        }

        ClientEvent::MlsEpochChanged { epoch, .. } => {
            // 语音中主群 epoch 变化 → key 已变，必须重建媒体会话，
            // 否则成员变更后双方互相解不开。
            if let Some(ch) = voice.active_channel.read().as_ref().map(|c| c.id.clone()) {
                tracing::info!(
                    "[voice] MLS epoch changed to {}; requesting media reconnect",
                    epoch
                );
                voice.reconnect_request.set(Some(ch));
                ui.toast
                    .set(Some(("语音密钥已轮换，正在自动重连…".to_string(), false)));
            }
        }

        ClientEvent::ChannelEpochChanged {
            channel_id, epoch, ..
        } => {
            // 块 B：私密频道 epoch 变化。只有正在该频道语音时才重连。
            let active = voice.active_channel.read().as_ref().map(|c| c.id.clone());
            if active.as_deref() == Some(channel_id.as_str()) {
                tracing::info!(
                    "[voice] private channel {} epoch changed to {}; reconnect",
                    channel_id,
                    epoch
                );
                voice.reconnect_request.set(Some(channel_id));
                ui.toast.set(Some((
                    "私密频道密钥已轮换，正在自动重连…".to_string(),
                    false,
                )));
            }
        }

        ClientEvent::Joined { peers, .. } => {
            // 房主在**自己登录时**也要补加已经在房间里的成员。
            // 服务端 Join 只回 peers 列表，不会为这些老成员重发 PeerJoined；
            // 不处理会导致新装/丢快照的 guest 永远进不了主群。
            let Some(room) = app.room.read().clone() else {
                return;
            };
            if !room.connection().is_host() {
                return;
            }
            for uid in peers {
                spawn_host_add(app, room.clone(), uid);
            }
        }

        ClientEvent::PeerJoined { server_id: _, uid } => {
            // host 收到 PeerJoined 自动 Add 该 peer。
            let Some(room) = app.room.read().clone() else {
                return;
            };
            if !room.connection().is_host() {
                return;
            }
            spawn_host_add(app, room, uid);
        }
        ClientEvent::MlsSyncError {
            server_id,
            user_message,
            recoverable,
            detail,
        } => {
            tracing::warn!(
                "[mls-sync-error] server={} recoverable={} {} ({})",
                server_id,
                recoverable,
                user_message,
                detail
            );
            ui.toast.set(Some((user_message, true)));
        }

        _ => {}
    }
}

// ============================================================================
// 自动重连
// ============================================================================

#[cfg(not(target_arch = "wasm32"))]
async fn retry_sleep(ms: u64) {
    tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
}

#[cfg(target_arch = "wasm32")]
async fn retry_sleep(ms: u64) {
    gloo_timers::future::TimeoutFuture::new(ms as u32).await;
}

/// 断线后自动重建连接（指数退避，最多 10 次）。
///
/// - 优先用旧 session token；token 失效则回退到保存的密码重新登录
/// - 重连成功后恢复 MLS（从本地快照）、Join、上报 epoch、补加已在线成员
/// - `Kicked` / 版本不兼容 / 认证失败等不可恢复错误不重试
fn maybe_auto_reconnect(
    mut app: AppState,
    mut ui: UiState,
    observer: std::sync::Arc<dyn kaede_client::ClientObserver>,
    server_id: String,
    reason: &str,
) {
    if reason.contains("Kicked")
        || reason.contains("UnsupportedVersion")
        || reason.contains("Unauthenticated")
        || reason.contains("MalformedMessage")
    {
        return;
    }

    let Some(old_room) = app.room.read().clone() else {
        return;
    };
    let Some((http, ws)) = old_room.server_url().split_once("::") else {
        return;
    };
    let http = http.to_string();
    let ws = ws.to_string();
    let uid = old_room.uid().as_str().to_string();
    let token = old_room.session_token();
    let storage = old_room.storage();
    let password = ui.common_account.read().password.clone();

    {
        if *app.reconnecting.read() {
            return;
        }
        app.reconnecting.set(true);
    }
    ui.toast
        .set(Some(("连接断开，正在自动重连…".to_string(), false)));

    spawn(async move {
        let mut delay_ms: u64 = 800;
        let mut last_err = String::new();

        for _ in 0..10 {
            retry_sleep(delay_ms).await;
            delay_ms = (delay_ms * 2).min(10_000);

            let mut room_opt: Option<kaede_client::RoomSession> = None;

            if let Some(tok) = token.clone() {
                match kaede_client::RoomSession::connect_with_token_and_storage(
                    &http,
                    &ws,
                    &uid,
                    &tok,
                    storage.clone(),
                    Some(observer.clone()),
                )
                .await
                {
                    Ok(r) => {
                        // token 可能已被服务端拒绝：用一次认证请求验证。
                        if r.list_members().await.is_ok() {
                            room_opt = Some(r);
                        }
                    }
                    Err(e) => last_err = e.to_string(),
                }
            }

            if room_opt.is_none() {
                match kaede_client::RoomSession::connect_with_storage(
                    &http,
                    &ws,
                    &uid,
                    password.as_str(),
                    storage.clone(),
                    Some(observer.clone()),
                )
                .await
                {
                    Ok(r) => room_opt = Some(r),
                    Err(e) => last_err = e.to_string(),
                }
            }

            let Some(room) = room_opt else {
                continue;
            };

            let members = room.list_members().await.unwrap_or_default();
            let is_host = members
                .iter()
                .find(|m| m.login_id == uid)
                .map(|m| m.op == "op5")
                .unwrap_or(false);
            room.connection().set_is_host(is_host);

            if let Err(e) = room.join_room().await {
                last_err = e.to_string();
                continue;
            }
            let _ = room.set_presence("online").await;

            if let Ok(channels) = room.list_channels().await {
                app.channels.write().insert(server_id.clone(), channels);
            }
            app.members.write().insert(server_id.clone(), members);
            app.is_host.set(Some(is_host));
            // 重连后刷新未读（离线期间的消息计数）。
            app.refresh_unread().await;

            let epoch = room.epoch();
            {
                let mut srvs = app.servers.write();
                if let Some(srv) = srvs.get_mut(&server_id) {
                    srv.state = kaede_client::ClientState::MlsReady;
                    srv.epoch = epoch;
                }
            }

            // 清掉历史加载标记，让当前频道重新拉取离线期间的消息。
            let prefix = format!("{}::", server_id);
            app.loaded_channels
                .write()
                .retain(|k| !k.starts_with(&prefix));
            app.messages.write().retain(|k, _| !k.starts_with(&prefix));

            let room = std::sync::Arc::new(room);
            room.spawn_event_loop();
            app.room.set(Some(room));
            app.reconnecting.set(false);
            ui.toast.set(Some(("已重新连接".to_string(), false)));
            tracing::info!(
                "[reconnect] server {} reconnected (epoch={})",
                server_id,
                epoch
            );
            return;
        }

        app.reconnecting.set(false);
        ui.toast.set(Some((
            format!("自动重连失败，请重新登录（{}）", last_err),
            true,
        )));
    });
}

/// host 自动 Add 一个 peer（带去重）。
///
/// 调用前提：`room.connection().is_host()`。
pub(crate) fn spawn_host_add(
    app: AppState,
    room: std::sync::Arc<kaede_client::RoomSession>,
    uid: UserId,
) {
    let self_uid = room.uid().as_str().to_string();
    if self_uid == uid.as_str() {
        tracing::debug!("[host] ignore self in add queue {}", uid);
        return;
    }
    let uid_key = uid.as_str().to_string();
    let mut pending = app.pending_adds;
    {
        let mut set = pending.write();
        if !set.insert(uid_key.clone()) {
            tracing::debug!("[host] add for {} already in flight, skip", uid);
            return;
        }
    }
    let uid_owned = uid;
    spawn(async move {
        let mut pending = pending;
        let result = room.host_add_peer(&uid_owned).await;
        pending.write().remove(&uid_key);
        match result {
            Ok(()) => tracing::info!("[host] auto-added peer {}", uid_owned),
            Err(e) => tracing::warn!("[host] auto-add peer {} failed: {}", uid_owned, e),
        }
    });
}

fn decode_message_payload(
    server_id: &str,
    from: &UserId,
    p: kaede_client::MessagePayload,
) -> Option<UiMessage> {
    // C19: 强制用 MLS 层的真实发送者覆盖 payload 声明的 u。
    // 恶意 group 成员可构造 payload.u = "victim" 冒充他人身份。
    if !p.u.is_empty() && p.u.as_str() != from.as_str() {
        tracing::warn!(
            "sender mismatch: payload.u={} but MLS from={}; using from",
            p.u,
            from
        );
    }
    let author = from.as_str().to_string();
    let initial = author
        .chars()
        .next()
        .map(|c| c.to_string())
        .unwrap_or_default();
    // C18: 优先用发送端真实时间，旧版本(ts=0)回退到本地时间
    let ts = if p.ts > 0 { p.ts } else { now_ms() };
    Some(UiMessage {
        id: format!("{}::{}::{}::{}", server_id, author, ts, next_seq()),
        channel_id: p.c,
        author_initial: initial,
        author,
        day: day_label(ts),
        time: format_hhmm(ts),
        ts,
        text: p.t,
        attachments: p.a,
        reply_to: p.r,
        reactions: Vec::new(),
        mentions: p.m,
    })
}

fn apply_reaction(
    app: &mut AppState,
    server_id: &str,
    r: &kaede_client::ReactionPayload,
    from: &str,
) {
    let key = AppState::channel_key(server_id, &r.c);
    let mut s = app.messages.write();
    let Some(list) = s.get_mut(&key) else {
        return;
    };
    let Some(msg) = list.iter_mut().find(|m| m.ts == r.target_ts) else {
        return;
    };

    let idx = msg.reactions.iter().position(|re| re.emoji == r.emoji);
    if r.action == "add" {
        if let Some(i) = idx {
            if !msg.reactions[i].users.iter().any(|u| u == from) {
                msg.reactions[i].users.push(from.to_string());
            }
        } else {
            msg.reactions.push(kaede_protocol::Reaction {
                emoji: r.emoji.clone(),
                users: vec![from.to_string()],
            });
        }
    } else if let Some(i) = idx {
        msg.reactions[i].users.retain(|u| u != from);
        if msg.reactions[i].users.is_empty() {
            msg.reactions.remove(i);
        }
    }
}

pub fn history_to_ui(
    h: kaede_client::HistoryMessage,
    channel_id: &str,
    server_id: &str,
) -> UiMessage {
    let author = h.sender;
    let initial = author
        .chars()
        .next()
        .map(|c| c.to_string())
        .unwrap_or_default();
    let ts = h.ts;
    UiMessage {
        id: format!("{}::{}::{}::{}", server_id, author, ts, next_seq()),
        channel_id: channel_id.into(),
        author_initial: initial,
        author,
        day: day_label(ts),
        time: format_hhmm(ts),
        ts,
        text: h.plaintext,
        attachments: h.attachments,
        reply_to: h.reply_to,
        reactions: Vec::new(),
        mentions: h.mentions,
    }
}

static SEQ: AtomicU64 = AtomicU64::new(0);

fn next_seq() -> u64 {
    SEQ.fetch_add(1, AtomicOrdering::Relaxed)
}

pub(crate) fn day_label(ms: i64) -> String {
    if ms <= 0 {
        return "更早".into();
    }
    // 用本地时区的“日历日”计算，而不是 UTC 毫秒差。
    use chrono::{Local, TimeZone};
    let Some(msg_dt) = Local.timestamp_millis_opt(ms).single() else {
        return "更早".into();
    };
    let days = (Local::now().date_naive() - msg_dt.date_naive()).num_days();
    match days {
        0 => "今天".into(),
        1 => "昨天".into(),
        n if n > 0 && n < 7 => format!("{} 天前", n),
        _ => "更早".into(),
    }
}

pub(crate) fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub(crate) fn format_hhmm(ms: i64) -> String {
    // 旧实现用 `ms % 86400`（UTC），东八区会整体偏移 8 小时。
    use chrono::{Local, TimeZone};
    match Local.timestamp_millis_opt(ms).single() {
        Some(dt) => dt.format("%H:%M").to_string(),
        None => {
            let secs = (ms / 1000).rem_euclid(86400);
            format!("{:02}:{:02}", secs / 3600, (secs / 60) % 60)
        }
    }
}
