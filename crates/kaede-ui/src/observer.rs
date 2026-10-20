//! 客户端事件 → UI 状态的桥接。

use async_channel::{Receiver, Sender, unbounded};
#[allow(unused_imports)]
use dioxus::prelude::*;
use kaede_client::{ClientEvent, ClientObserver, DecodedPayload};
use kaede_protocol::UserId;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

use crate::state::{AppState, UiMessage, UiState};

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

pub fn apply_event(mut app: AppState, mut ui: UiState, event: ClientEvent) {
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
            if let Some(uid) = self_uid
                && from.as_str() == uid
            {
                return;
            }
            match kaede_client::decode(&plaintext) {
                DecodedPayload::Message(p) => {
                    if let Some(msg) = decode_message_payload(&server_id, &from, p) {
                        let mut s = app.messages.write();
                        let key = AppState::channel_key(&server_id, &msg.channel_id);
                        s.entry(key).or_default().push(msg);
                    }
                }
                DecodedPayload::Reaction(r) => {
                    apply_reaction(&mut app, &server_id, &r);
                }
                _ => {}
            }
        }

        ClientEvent::DmReceived {
            server_id: _,
            conv_id,
            from: _,
            ts,
            plaintext,
        } => {
            if let Some(p) = kaede_client::MessagePayload::parse(&plaintext) {
                let msg = UiMessage {
                    id: format!("dm-{}-{}", conv_id, ts),
                    channel_id: conv_id.clone(),
                    author: p.u.clone(),
                    author_initial: p
                        .u
                        .chars()
                        .next()
                        .map(|c| c.to_string())
                        .unwrap_or_default(),
                    day: crate::observer::day_label(ts),
                    time: crate::observer::format_hhmm(ts),
                    ts,
                    text: p.t,
                    attachments: p.a,
                    reply_to: p.r,
                    reactions: vec![],
                };
                let mut map = app.dm_messages.write();
                let list = map.entry(conv_id).or_default();
                if !list.iter().any(|m| m.ts == ts) {
                    list.push(msg);
                    list.sort_by_key(|m| m.ts);
                }
            }
        }
        ClientEvent::Disconnected { server_id, .. } => {
            let mut s = app.servers.write();
            if let Some(srv) = s.get_mut(&server_id) {
                srv.state = kaede_client::ClientState::Disconnected;
            }
        }

        ClientEvent::PeerJoined { server_id: _, uid } => {
            // P0-1 完整版：host 收到 PeerJoined 自动 Add 该 peer
            // R3b 强化：
            //   - self-check：host 自己的 PeerJoined 忽略（become_host 已主动进组）
            //   - pending 去重：同一 peer 不并发 add（防止 epoch 前进竞争 / 重复消费 KP）
            //   - 清理：无论成功失败都 remove；唯一 leak 场景 = task panic（host_add_peer
            //     内部仅 await + 简单错误路径，panic 概率极低；若发生则重启 UI 自愈）
            let is_host = app.is_host.read().unwrap_or(false);
            if !is_host {
                return;
            }

            let self_uid = app
                .room
                .read()
                .as_ref()
                .map(|r| r.uid().as_str().to_string());
            if self_uid.as_deref() == Some(uid.as_str()) {
                tracing::debug!("[host] ignore own PeerJoined {}", uid);
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

            let Some(room) = app.room.read().clone() else {
                pending.write().remove(&uid_key);
                return;
            };
            let uid_owned = uid.clone();
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

fn decode_message_payload(
    server_id: &str,
    from: &UserId,
    p: kaede_client::MessagePayload,
) -> Option<UiMessage> {
    let author = if p.u.is_empty() {
        from.as_str().to_string()
    } else {
        p.u.clone()
    };
    let initial = author
        .chars()
        .next()
        .map(|c| c.to_string())
        .unwrap_or_default();
    let ts = now_ms();
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
    })
}

fn apply_reaction(app: &mut AppState, server_id: &str, r: &kaede_client::ReactionPayload) {
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
            if !msg.reactions[i].users.iter().any(|u| u == &r.u) {
                msg.reactions[i].users.push(r.u.clone());
            }
        } else {
            msg.reactions.push(kaede_protocol::Reaction {
                emoji: r.emoji.clone(),
                users: vec![r.u.clone()],
            });
        }
    } else if let Some(i) = idx {
        msg.reactions[i].users.retain(|u| u != &r.u);
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
    let now = now_ms();
    let days = (now - ms).max(0) / 86_400_000;
    match days {
        0 => "今天".into(),
        1 => "昨天".into(),
        n if n < 7 => format!("{} 天前", n),
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
    let secs = (ms / 1000) % 86400;
    let h = (secs / 3600) % 24;
    let m = (secs / 60) % 60;
    format!("{:02}:{:02}", h, m)
}
