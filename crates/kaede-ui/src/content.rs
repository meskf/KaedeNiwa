//! 内容区：频道头 + 消息列表 + 操作栏 + 输入框。

#![allow(clippy::collapsible_if)]

use crate::i18n::use_i18n;
use dioxus::prelude::*;

use crate::icons::{IconHash, IconVolume2};
use crate::state::{AppState, ReplyTarget, UiMessage, UiState};

#[component]
pub fn ContentArea() -> Element {
    rsx! {
        div {
            class: "content",
            ChannelHeader {}
            MessageList {}
            Composer {}
        }
    }
}

#[component]
fn ChannelHeader() -> Element {
    let ui = use_context::<UiState>();
    let app = use_context::<AppState>();
    let i18n = use_i18n();

    let info = use_memo(move || {
        let ch_id = ui.selected_channel.read().clone()?;
        let sid = app.current_server.read().clone()?;
        let chans = app.channels.read();
        let list = chans.get(&sid)?;
        let ch = list.iter().find(|c| c.id == ch_id)?;
        Some((ch.name.clone(), ch.kind.clone()))
    });

    let (name, kind) = match info() {
        Some((n, k)) => (n, k),
        None => (i18n.t("channel.selectChannel"), "text".into()),
    };

    rsx! {
        div {
            class: "channel-header",
            span {
                class: "channel-header-icon",
                if kind == "voice" { IconVolume2 { size: 18 } } else { IconHash { size: 18 } }
            }
            span { class: "channel-header-name", "{name}" }
        }
    }
}

#[derive(Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
enum Row {
    Day(String),
    Msg { msg: UiMessage, merged: bool },
}

fn build_rows(messages: &[UiMessage]) -> Vec<Row> {
    let mut out: Vec<Row> = Vec::new();
    let mut last_day: Option<String> = None;
    let mut last_author: Option<String> = None;

    for m in messages {
        if last_day.as_deref() != Some(m.day.as_str()) {
            out.push(Row::Day(m.day.clone()));
            last_day = Some(m.day.clone());
            last_author = None;
        }
        let merged = last_author.as_deref() == Some(m.author.as_str());
        out.push(Row::Msg {
            msg: m.clone(),
            merged,
        });
        last_author = Some(m.author.clone());
    }
    out
}

#[component]
fn MessageList() -> Element {
    let ui = use_context::<UiState>();
    let mut app = use_context::<AppState>();
    let i18n = use_i18n();

    let rows = use_memo(move || {
        let sid = app.current_server.read().clone();
        let ch = ui.selected_channel.read().clone();
        let (Some(sid), Some(ch)) = (sid, ch) else {
            return Vec::<Row>::new();
        };
        let key = AppState::channel_key(&sid, &ch);
        let msgs = app.messages.read();
        let list = msgs.get(&key).cloned().unwrap_or_default();
        build_rows(&list)
    });

    use_effect(move || {
        let _ = rows.read().len();
        let _ = dioxus::document::eval(
            "setTimeout(function(){var el=document.getElementById('msg-list');if(el)el.scrollTop=el.scrollHeight;},0);",
        );
    });

    // 7d-a: 进频道时 load_history（消息为空时）
    use_effect(move || {
        let sid = app.current_server.read().clone();
        let ch = ui.selected_channel.read().clone();
        let (Some(sid), Some(ch)) = (sid, ch) else {
            return;
        };
        let key = AppState::channel_key(&sid, &ch);
        let is_empty = app
            .messages
            .read()
            .get(&key)
            .map(|v| v.is_empty())
            .unwrap_or(true);
        if !is_empty {
            return;
        }
        spawn(async move {
            let room_opt = app.room.read().clone();
            let Some(room) = room_opt else { return };
            match room.load_history(&ch, 50).await {
                Ok(history) => {
                    let ui_msgs: Vec<UiMessage> = history
                        .into_iter()
                        .map(|h| UiMessage {
                            id: format!("{}-{}-{}", ch, h.ts, h.sender),
                            channel_id: ch.clone(),
                            author: h.sender.clone(),
                            author_initial: h
                                .sender
                                .chars()
                                .next()
                                .map(|c| c.to_string())
                                .unwrap_or_default(),
                            day: crate::observer::day_label(h.ts),
                            time: crate::observer::format_hhmm(h.ts),
                            ts: h.ts,
                            text: h.plaintext,
                            attachments: h.attachments,
                            reply_to: h.reply_to,
                            reactions: vec![],
                        })
                        .collect();
                    app.messages.write().insert(key, ui_msgs);
                }
                Err(e) => tracing::warn!("load_history: {}", e),
            }
        });
    });

    rsx! {
        div {
            id: "msg-list",
            class: "msg-list",
            button {
                class: "msg-load-earlier",
                onclick: move |_| {
                    let sid = app.current_server.read().clone();
                    let ch = ui.selected_channel.read().clone();
                    let (Some(sid), Some(ch)) = (sid, ch) else { return };
                    spawn(async move {
                        let key = AppState::channel_key(&sid, &ch);
                        let before_ts = {
                            let msgs = app.messages.read();
                            msgs.get(&key).and_then(|l| l.first()).map(|m| m.ts).unwrap_or(i64::MAX)
                        };
                        let room_opt = app.room.read().clone();
                        let Some(room) = room_opt else { return };
                        match room.load_history_before(&ch, before_ts, 50).await {
                            Ok(history) => {
                                let ui_msgs: Vec<UiMessage> = history.into_iter().map(|h| UiMessage {
                                    id: format!("{}-{}-{}", ch, h.ts, h.sender),
                                    channel_id: ch.clone(),
                                    author: h.sender.clone(),
                                    author_initial: h.sender.chars().next().map(|c| c.to_string()).unwrap_or_default(),
                                    day: crate::observer::day_label(h.ts),
                                    time: crate::observer::format_hhmm(h.ts),
                                    ts: h.ts,
                                    text: h.plaintext,
                                    attachments: h.attachments,
                                    reply_to: h.reply_to,
                                    reactions: vec![],
                                }).collect();
                                let mut map = app.messages.write();
                                let list = map.entry(key).or_default();
                                let mut combined: Vec<UiMessage> = ui_msgs;
                                combined.append(list);
                                // P2-6: 按 (ts, author, text) 排序，再按三元组去重。
                                // 原 `dedup_by_key(|m| m.ts)` 会误删同毫秒不同用户的消息。
                                combined.sort_by(|a, b| {
                                    a.ts.cmp(&b.ts)
                                        .then_with(|| a.author.cmp(&b.author))
                                        .then_with(|| a.text.cmp(&b.text))
                                });
                                combined.dedup_by(|a, b| {
                                    a.ts == b.ts && a.author == b.author && a.text == b.text
                                });
                                *list = combined;
                            }
                            Err(e) => tracing::warn!("load_history_before: {}", e),
                        }
                    });
                },
                "{i18n.t(\"channel.loadEarlier\")}"
            }
            for (i, row) in rows.read().iter().enumerate() {
                match row {
                    Row::Day(d) => rsx! {
                        div { key: "day-{i}", class: "date-divider", "{d}" }
                    },
                    Row::Msg { msg, merged } => rsx! {
                        MessageGroup { key: "msg-{i}", msg: msg.clone(), merged: *merged }
                    },
                }
            }
        }
    }
}

#[component]
fn MessageGroup(msg: UiMessage, merged: bool) -> Element {
    let mut ui = use_context::<UiState>();
    let mut hovered = use_signal(|| false);

    let mid = msg.id.clone();
    let mid_dbl = mid.clone();

    let is_sel = use_memo({
        let mid = mid.clone();
        move || ui.selected_message.read().as_deref() == Some(mid.as_str())
    });

    let cls = match (is_sel(), merged) {
        (true, true) => "msg-group msg-group-merged selected",
        (true, false) => "msg-group selected",
        (false, true) => "msg-group msg-group-merged",
        (false, false) => "msg-group",
    };

    let ch_id = msg.channel_id.clone();
    let target_ts = msg.ts;
    let author = msg.author.clone();
    let preview: String = msg.text.chars().take(80).collect();

    let mid_ctx = mid.clone();

    rsx! {
        div {
            class: "{cls}",
            onmouseenter: move |_| hovered.set(true),
            onmouseleave: move |_| hovered.set(false),
            ondoubleclick: move |_| {
                if ui.selected_message.read().as_deref() == Some(mid_dbl.as_str()) {
                    ui.selected_message.set(None);
                } else {
                    ui.selected_message.set(Some(mid_dbl.clone()));
                }
            },
            oncontextmenu: move |e| {
                e.prevent_default();
                ui.selected_message.set(Some(mid_ctx.clone()));
            },
            if !merged {
                div {
                    class: "msg-head",
                    div { class: "msg-avatar", "{msg.author_initial}" }
                    span { class: "msg-author", "{msg.author}" }
                    span { class: "msg-time", "{msg.time}" }
                }
                if let Some(r) = &msg.reply_to {
                    div {
                        class: "msg-reply",
                        span { class: "msg-reply-author", "{r.author}" }
                        span { class: "msg-reply-preview", " {r.preview}" }
                    }
                }
            }
            div { class: "msg-body", "{msg.text}" }
            if !msg.attachments.is_empty() {
                div {
                    class: "msg-attachments",
                    style: "margin-top: 8px; display: flex; flex-direction: column; gap: 6px;",
                    for att in msg.attachments.iter() {
                        if att.mime.starts_with("image/") {
                            img {
                                src: "{att.url}",
                                alt: "{att.name}",
                                style: "max-width: 400px; max-height: 300px; border-radius: 8px; object-fit: contain;",
                            }
                        } else {
                            a {
                                href: "{att.url}",
                                download: "{att.name}",
                                style: "display: inline-flex; align-items: center; gap: 6px; padding: 6px 10px; border-radius: 6px; background: rgba(0,0,0,0.06); color: var(--fg); font-size: 13px; text-decoration: none; max-width: 320px; overflow: hidden; text-overflow: ellipsis;",
                                "📎 {att.name}"
                            }
                        }
                    }
                }
            }

            if !msg.reactions.is_empty() {
                div {
                    class: "msg-reactions",
                    for r in msg.reactions.iter() {
                        ReactionPill {
                            key: "{msg.ts}-{r.emoji}",
                            channel_id: msg.channel_id.clone(),
                            target_ts: msg.ts,
                            reaction: r.clone(),
                        }
                    }
                }
            }

            if hovered() {
                div {
                    class: "msg-hover-actions",
                    button {
                        class: "msg-hover-btn",
                        title: "回复",
                        onclick: move |_| {
                            ui.reply_target.set(Some(ReplyTarget {
                                author: author.clone(),
                                preview: preview.clone(),
                                ts: target_ts,
                            }));
                        },
                        "↩"
                    }
                    button {
                        class: "msg-hover-btn",
                        title: "添加反应",
                        onclick: move |_| {
                            ui.emoji_picker.set(Some((ch_id.clone(), target_ts)));
                        },
                        "😀"
                    }
                }
            }
        }
    }
}

#[component]
fn ReactionPill(channel_id: String, target_ts: i64, reaction: kaede_protocol::Reaction) -> Element {
    let ui = use_context::<UiState>();
    let mut app = use_context::<AppState>();
    let me = ui.common_account.read().login_id.clone();
    let mine = reaction.users.iter().any(|u| u == &me);
    let cls = if mine {
        "reaction-pill reaction-mine"
    } else {
        "reaction-pill"
    };
    let tooltip = reaction.users.join(", ");
    let count = reaction.users.len();
    let emoji = reaction.emoji.clone();

    let ch_id = channel_id.clone();
    let emoji_click = emoji.clone();

    rsx! {
        button {
            class: "{cls}",
            title: "{tooltip}",
            onclick: move |e| {
                e.stop_propagation();
                let action = if mine { "remove" } else { "add" };
                let ch = ch_id.clone();
                let em = emoji_click.clone();
                let me_uid = me.clone();
                let is_add = action == "add";

                // 本地立即更新（防止服务器 echo 被 observer 过滤后无反馈）
                {
                    let sid = app.current_server.read().clone();
                    if let Some(sid) = sid {
                        let key = AppState::channel_key(&sid, &ch);
                        if let Some(list) = app.messages.write().get_mut(&key) {
                            if let Some(m) = list.iter_mut().find(|m| m.ts == target_ts) {
                                let idx = m.reactions.iter().position(|r| r.emoji == em);
                                if is_add {
                                    if let Some(i) = idx {
                                        if !m.reactions[i].users.iter().any(|u| u == &me_uid) {
                                            m.reactions[i].users.push(me_uid);
                                        }
                                    } else {
                                        m.reactions.push(kaede_protocol::Reaction {
                                            emoji: em.clone(),
                                            users: vec![me_uid],
                                        });
                                    }
                                } else if let Some(i) = idx {
                                    m.reactions[i].users.retain(|u| u != &me_uid);
                                    if m.reactions[i].users.is_empty() {
                                        m.reactions.remove(i);
                                    }
                                }
                            }
                        }
                    }
                }

                // 后台发送
                if let Some(room) = app.room.read().clone() {
                    let act = action.to_string();
                    spawn(async move {
                        let _ = room.send_reaction(&ch, target_ts, &em, &act).await;
                    });
                }
            },
            span { class: "reaction-emoji", "{emoji}" }
            span { class: "reaction-count", "{count}" }
        }
    }
}

#[component]
fn Composer() -> Element {
    let mut ui = use_context::<UiState>();
    let app = use_context::<AppState>();
    let i18n = use_i18n();

    let mut draft = ui.draft;
    let is_empty = draft.read().trim().is_empty();
    let reply = ui.reply_target.read().clone();

    rsx! {
        div {
            class: "composer-wrap",
            if let Some(r) = reply {
                div {
                    class: "reply-preview",
                    span { class: "reply-preview-icon", "↪" }
                    span { class: "reply-preview-author", "@{r.author}" }
                    span { class: "reply-preview-text", "{r.preview}" }
                    button {
                        class: "reply-preview-cancel",
                        onclick: move |_| ui.reply_target.set(None),
                        "✕"
                    }
                }
            }
            div {
                class: "composer",
                textarea {
                    class: "composer-input",
                    placeholder: "{i18n.t(\"channel.inputPlaceholder\")}",
                    value: "{draft}",
                    oninput: move |e| draft.set(e.value()),
                    onkeydown: move |e: KeyboardEvent| {
                        if e.key() == Key::Enter && !e.modifiers().contains(Modifiers::SHIFT) {
                            e.prevent_default();
                            let t = draft.read().trim().to_string();
                            if !t.is_empty() {
                                send_message(ui, app, &t);
                                draft.set(String::new());
                            }
                        }
                    },
                }
                button {
                    class: "composer-send",
                    disabled: is_empty,
                    onclick: move |_| {
                        let t = draft.read().trim().to_string();
                        if t.is_empty() { return; }
                        send_message(ui, app, &t);
                        draft.set(String::new());
                    },
                    "{i18n.t(\"channel.send\")}"
                }
            }
        }
    }
}

fn send_message(mut ui: UiState, mut app: AppState, text: &str) {
    let Some(ch) = ui.selected_channel.read().clone() else {
        return;
    };
    let Some(sid) = app.current_server.read().clone() else {
        return;
    };
    let ts = crate::observer::now_ms();
    let acc = ui.common_account.read();
    let author = if acc.login_id.is_empty() {
        "me".to_string()
    } else {
        acc.login_id.clone()
    };
    drop(acc);
    let initial = author
        .chars()
        .next()
        .map(|c| c.to_string())
        .unwrap_or_default();

    // 在清空之前先取 reply_target（含 ts）
    let reply = ui.reply_target.read().clone();
    let reply_ref = reply.as_ref().map(|r| kaede_protocol::ReplyRef {
        ts: r.ts,
        author: r.author.clone(),
        preview: r.preview.clone(),
    });

    let msg = UiMessage {
        id: format!("local::{}::{}::{}", sid, author, ts),
        channel_id: ch.clone(),
        author,
        author_initial: initial,
        day: crate::observer::day_label(ts),
        time: crate::observer::format_hhmm(ts),
        ts,
        text: text.to_string(),
        attachments: vec![],
        reply_to: reply_ref.clone(),
        reactions: vec![],
    };
    let key = AppState::channel_key(&sid, &ch);
    app.messages.write().entry(key).or_default().push(msg);

    ui.reply_target.set(None);

    if let Some(room) = app.room.read().clone() {
        let ch = ch.clone();
        let t = text.to_string();
        spawn(async move {
            match reply_ref {
                Some(r) => {
                    if let Err(e) = room.send_text_with_reply(&ch, &t, r).await {
                        tracing::warn!("send_text_with_reply failed: {}", e);
                    }
                }
                None => {
                    if let Err(e) = room.send_text_to(&ch, &t).await {
                        tracing::warn!("send_text_to failed: {}", e);
                    }
                }
            }
        });
    }
}

// ============================================================
// EmojiPicker（全局弹层）
// ============================================================

const EMOJIS: &[&str] = &[
    "👍", "❤️", "😂", "🎉", "😮", "😢", "🔥", "👀", "✅", "❌", "🚀", "💯",
];

#[component]
pub fn EmojiPicker() -> Element {
    let mut ui = use_context::<UiState>();
    let mut app = use_context::<AppState>();

    let Some((ch_id, target_ts)) = ui.emoji_picker.read().clone() else {
        return rsx! {};
    };

    let me = ui.common_account.read().login_id.clone();

    rsx! {
        div {
            class: "server-modal-backdrop",
            style: "background: rgba(0,0,0,0.15);",
            onclick: move |_| ui.emoji_picker.set(None),
            div {
                onclick: move |e| e.stop_propagation(),
                style: "background: var(--inner-bg); border-radius: 12px; padding: 12px; box-shadow: 0 8px 32px rgba(0,0,0,0.15); display: flex; gap: 6px; flex-wrap: wrap; max-width: 320px;",
                for (i, em) in EMOJIS.iter().enumerate() {
                    {
                        let em_str = em.to_string();
                        let em_clone = em_str.clone();
                        let ch = ch_id.clone();
                        let me_uid = me.clone();
                        rsx! {
                            button {
                                key: "{i}-{em}",
                                style: "font-size: 22px; width: 42px; height: 42px; border-radius: 8px; display: flex; align-items: center; justify-content: center; transition: background 120ms;",
                                onclick: move |_| {
                                    // 本地更新
                                    {
                                        let sid = app.current_server.read().clone();
                                        if let Some(sid) = sid {
                                            let key = AppState::channel_key(&sid, &ch);
                                            if let Some(list) = app.messages.write().get_mut(&key) {
                                                if let Some(m) = list.iter_mut().find(|m| m.ts == target_ts) {
                                                    let idx = m.reactions.iter().position(|r| r.emoji == em_clone);
                                                    if let Some(i) = idx {
                                                        if !m.reactions[i].users.iter().any(|u| u == &me_uid) {
                                                            m.reactions[i].users.push(me_uid.clone());
                                                        }
                                                    } else {
                                                        m.reactions.push(kaede_protocol::Reaction {
                                                            emoji: em_clone.clone(),
                                                            users: vec![me_uid.clone()],
                                                        });
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    // 发送
                                    if let Some(room) = app.room.read().clone() {
                                        let ch_s = ch.clone();
                                        let em_s = em_clone.clone();
                                        spawn(async move {
                                            let _ = room.send_reaction(&ch_s, target_ts, &em_s, "add").await;
                                        });
                                    }
                                    ui.emoji_picker.set(None);
                                },
                                "{em_str}"
                            }
                        }
                    }
                }
            }
        }
    }
}
