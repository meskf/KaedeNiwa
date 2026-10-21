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
        // 只有用户本来就在底部时才跟随新消息；加载更早历史时不要把人弹回底部。
        let _ = dioxus::document::eval(
            "setTimeout(function(){var el=document.getElementById('msg-list');\
             if(el && el.scrollHeight-el.scrollTop-el.clientHeight<120)el.scrollTop=el.scrollHeight;},0);",
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
        if app.loaded_channels.read().contains(&key) {
            return;
        }
        let is_empty = app
            .messages
            .read()
            .get(&key)
            .map(|v| v.is_empty())
            .unwrap_or(true);
        // 本次会话内只尝试一次：解密失败/为空也不再重复请求。
        app.loaded_channels.write().insert(key.clone());
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
                            mentions: h.mentions,
                        })
                        .collect();
                    // 合并而不是替换：加载期间到达的实时消息不能被历史覆盖。
                    let mut map = app.messages.write();
                    let list = map.entry(key).or_default();
                    for h in ui_msgs {
                        if !list.iter().any(|m| m.ts == h.ts && m.author == h.author) {
                            list.push(h);
                        }
                    }
                    list.sort_by_key(|m| m.ts);
                }
                Err(e) => tracing::warn!("load_history: {}", e),
            }
        });
    });

    // 已读：查看中的频道收到/加载新消息时上报 last_read 并清本地未读。
    use_effect(move || {
        let sid = app.current_server.read().clone();
        let ch = ui.selected_channel.read().clone();
        let (Some(sid), Some(ch)) = (sid, ch) else {
            return;
        };
        let max_ts = app
            .messages
            .read()
            .get(&AppState::channel_key(&sid, &ch))
            .and_then(|list| list.iter().map(|m| m.ts).max());
        let Some(ts) = max_ts else {
            return;
        };
        app.clear_unread(&sid, &ch);
        if let Some(room) = app.room.read().clone() {
            let ch = ch.clone();
            spawn(async move {
                if let Err(e) = room.mark_channel_read(&ch, ts).await {
                    tracing::debug!("mark_channel_read: {}", e);
                }
            });
        }
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
                                    mentions: h.mentions,
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
    let app = use_context::<AppState>();
    let mut hovered = use_signal(|| false);

    let is_private = {
        let sid = app.current_server.read().clone();
        sid.and_then(|sid| {
            app.channels
                .read()
                .get(&sid)
                .and_then(|list| list.iter().find(|c| c.id == msg.channel_id))
                .map(|c| c.is_private)
        })
        .unwrap_or(false)
    };

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
            }
            if let Some(r) = &msg.reply_to {
                div {
                    class: "msg-reply",
                    span { class: "msg-reply-author", "{r.author}" }
                    span { class: "msg-reply-preview", " {r.preview}" }
                }
            }
            div { class: "msg-body",
                for seg in split_mentions(&msg.text, &msg.mentions) {
                    if seg.1 {
                        span {
                            style: "padding: 0 2px; border-radius: 3px; background: rgba(88,101,242,0.20); font-weight: 600;",
                            "{seg.0}"
                        }
                    } else {
                        span { "{seg.0}" }
                    }
                }
            }
            if !msg.attachments.is_empty() {
                div {
                    class: "msg-attachments",
                    style: "margin-top: 8px; display: flex; flex-direction: column; gap: 6px;",
                    for att in msg.attachments.iter() {
                        // 认证下载 + 解密后渲染（直接 URL 会 401 / 显示密文）。
                        crate::attachment::AttachmentView {
                            att: att.clone(),
                            target_id: msg.channel_id.clone(),
                        }
                    }
                }
            }

            if !is_private && !msg.reactions.is_empty() {
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
                    if !is_private {
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
                    let mut ui_for_err = ui;
                    spawn(async move {
                        if let Err(e) = room.send_reaction(&ch, target_ts, &em, &act).await {
                            tracing::warn!("send_reaction: {}", e);
                            ui_for_err
                                .toast
                                .set(Some((format!("表情发送失败: {}", e), true)));
                        }
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
                    title: "上传附件",
                    onclick: move |_| {
                        let _ = dioxus::document::eval(
                            "document.getElementById('kaede-file-input')?.click();",
                        );
                    },
                    "📎"
                }
                input {
                    id: "kaede-file-input",
                    r#type: "file",
                    multiple: true,
                    style: "display:none;",
                    onchange: move |e: dioxus::html::FormEvent| {
                        let files = e.files();
                        if files.is_empty() {
                            return;
                        }
                        let Some(ch) = ui.selected_channel.read().clone() else {
                            return;
                        };
                        let Some(sid) = app.current_server.read().clone() else {
                            return;
                        };
                        // 允许连续两次选择同一个文件。
                        let _ = dioxus::document::eval(
                            "document.getElementById('kaede-file-input').value='';",
                        );
                        send_attachments(ui, app, ch, sid, files);
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

/// 上传一批文件 → 发送一条带附件元数据的消息 → 本地回显。
fn send_attachments(
    mut ui: UiState,
    mut app: AppState,
    ch: String,
    sid: String,
    files: Vec<dioxus::html::FileData>,
) {
    let room = app.room.read().clone();
    spawn(async move {
        let Some(room) = room else { return };
        let mut atts = Vec::new();
        for f in files {
            match crate::attachment::upload_file(room.clone(), &ch, f).await {
                Ok(a) => atts.push(a),
                Err(e) => {
                    ui.toast.set(Some((format!("上传失败: {}", e), true)));
                }
            }
        }
        ui.upload_progress.set(None);
        if atts.is_empty() {
            return;
        }
        // 本地乐观消息与 payload 必须使用同一个 ts，否则 reaction 的
        // target_ts 永远匹配不上。
        let ts = crate::observer::now_ms();
        if let Err(e) = room.send_text_with_attachments_at(&ch, "", &atts, ts).await {
            ui.toast.set(Some((format!("发送附件失败: {}", e), true)));
            return;
        }
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
        let msg = UiMessage {
            id: format!("local-file-{}-{}", ts, atts.len()),
            channel_id: ch.clone(),
            author,
            author_initial: initial,
            day: crate::observer::day_label(ts),
            time: crate::observer::format_hhmm(ts),
            ts,
            text: String::new(),
            attachments: atts,
            reply_to: None,
            reactions: Vec::new(),
            mentions: Vec::new(),
        };
        app.messages
            .write()
            .entry(AppState::channel_key(&sid, &ch))
            .or_default()
            .push(msg);
    });
}

fn send_message(mut ui: UiState, mut app: AppState, text: &str) {
    let Some(ch) = ui.selected_channel.read().clone() else {
        return;
    };
    let Some(sid) = app.current_server.read().clone() else {
        return;
    };
    let mentions = extract_mentions(text, &known_mentions(&app, &sid));
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
        mentions: mentions.clone(),
    };
    let key = AppState::channel_key(&sid, &ch);
    app.messages.write().entry(key).or_default().push(msg);

    ui.reply_target.set(None);

    if let Some(room) = app.room.read().clone() {
        let ch = ch.clone();
        let t = text.to_string();
        let mentions_for_send = mentions;
        let mut ui_for_err = ui;
        spawn(async move {
            let result = match reply_ref {
                Some(r) => {
                    room.send_text_with_reply_mentions_at(&ch, &t, r, &mentions_for_send, ts)
                        .await
                }
                None => {
                    room.send_text_with_mentions_at(&ch, &t, &mentions_for_send, ts)
                        .await
                }
            };
            if let Err(e) = result {
                tracing::warn!("send failed: {}", e);
                // 本地乐观消息仍在，必须让用户知道没有真正发出。
                ui_for_err
                    .toast
                    .set(Some((format!("发送失败: {}", e), true)));
            }
        });
    }
}

// ============================================================
// EmojiPicker（全局弹层）
// ============================================================

/// 当前 server 中可被 @ 的 login_id：成员 + 已接受好友 + 自己。
pub(crate) fn known_mentions(app: &AppState, server_id: &str) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    if let Some(list) = app.members.read().get(server_id) {
        out.extend(list.iter().map(|m| m.login_id.clone()));
    }
    if let Some(list) = app.friends.read().get(server_id) {
        out.extend(
            list.iter()
                .filter(|f| f.direction == "friend")
                .map(|f| f.login_id.clone()),
        );
    }
    if let Some(room) = app.room.read().as_ref() {
        out.insert(room.uid().as_str().to_string());
    }
    out
}

/// 从文本中提取 `@login_id`，只保留已知用户。
pub(crate) fn extract_mentions(
    text: &str,
    known: &std::collections::HashSet<String>,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cursor = 0;
    while let Some(pos) = text[cursor..].find('@') {
        let start = cursor + pos + 1;
        let mut end = start;
        for (idx, ch) in text[start..].char_indices() {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                end = start + idx + ch.len_utf8();
            } else {
                break;
            }
        }
        if end > start {
            let token = &text[start..end];
            if known.contains(token) && !out.iter().any(|m| m == token) {
                out.push(token.to_string());
            }
        }
        cursor = end.max(start);
        if cursor >= text.len() {
            break;
        }
    }
    out
}

/// 把文本切成普通片段与 @ 片段，供消息正文高亮。
pub(crate) fn split_mentions(text: &str, mentions: &[String]) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut cursor = 0;
    let mut plain = String::new();
    while cursor < text.len() {
        let Some(pos) = text[cursor..].find('@') else {
            plain.push_str(&text[cursor..]);
            break;
        };
        let at = cursor + pos;
        plain.push_str(&text[cursor..at]);
        let start = at + 1;
        let mut end = start;
        for (idx, ch) in text[start..].char_indices() {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                end = start + idx + ch.len_utf8();
            } else {
                break;
            }
        }
        if end > start {
            let token = &text[start..end];
            if mentions.iter().any(|m| m == token) {
                if !plain.is_empty() {
                    out.push((std::mem::take(&mut plain), false));
                }
                out.push((format!("@{token}"), true));
                cursor = end;
                continue;
            }
        }
        plain.push('@');
        cursor = at + 1;
    }
    if !plain.is_empty() {
        out.push((plain, false));
    }
    out
}

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
                                        let mut ui_for_err = ui;
                                        spawn(async move {
                                            if let Err(e) = room
                                                .send_reaction(&ch_s, target_ts, &em_s, "add")
                                                .await
                                            {
                                                tracing::warn!("send_reaction: {}", e);
                                                ui_for_err.toast.set(Some((
                                                    format!("表情发送失败: {}", e),
                                                    true,
                                                )));
                                            }
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
