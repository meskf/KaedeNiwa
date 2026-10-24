//! DM 视图（主区 + 侧栏）。

use dioxus::prelude::*;
use kaede_client::http::DmConversation;

use crate::i18n::use_i18n;
use crate::state::{AppState, MsgMenuTarget, UiMessage, UiState};

pub(crate) async fn load_messages(mut app: AppState, conv_id: String) {
    let room_opt = app.room.read().clone();
    let Some(room) = room_opt else { return };
    match room.dm_load_history(&conv_id, None, 100).await {
        Ok(history) => {
            let ui_msgs: Vec<UiMessage> = history
                .into_iter()
                .map(|h| UiMessage {
                    id: format!("dm-{}-{}", conv_id, h.ts),
                    channel_id: conv_id.clone(),
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
                    reactions: h.reactions,
                    mentions: h.mentions,
                    edited: h.edited,
                })
                .collect();
            // 合并而不是替换：避免丢掉加载期间收到的实时 DM。
            let mut map = app.dm_messages.write();
            let list = map.entry(conv_id).or_default();
            for h in ui_msgs {
                if !list.iter().any(|m| m.ts == h.ts && m.author == h.author) {
                    list.push(h);
                }
            }
            list.sort_by_key(|m| m.ts);
        }
        Err(e) => tracing::warn!("dm_load_history: {}", e),
    }
}

/// 加载更早的 DM 消息（按 ts 游标向前翻页）。
async fn load_older_messages(mut app: AppState, conv_id: String) {
    let before_ts = app
        .dm_messages
        .read()
        .get(&conv_id)
        .and_then(|list| list.first().map(|m| m.ts))
        .unwrap_or(i64::MAX);
    let room_opt = app.room.read().clone();
    let Some(room) = room_opt else { return };
    match room.dm_load_history(&conv_id, Some(before_ts), 100).await {
        Ok(history) => {
            let older: Vec<UiMessage> = history
                .into_iter()
                .map(|h| UiMessage {
                    id: format!("dm-{}-{}", conv_id, h.ts),
                    channel_id: conv_id.clone(),
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
                    reactions: h.reactions,
                    mentions: h.mentions,
                    edited: h.edited,
                })
                .collect();
            let mut map = app.dm_messages.write();
            let list = map.entry(conv_id).or_default();
            let mut combined = older;
            combined.append(list);
            combined.sort_by_key(|m| m.ts);
            combined.dedup_by(|a, b| a.ts == b.ts && a.author == b.author && a.text == b.text);
            *list = combined;
        }
        Err(e) => tracing::warn!("dm_load_history(before): {}", e),
    }
}

pub(crate) async fn load_conversations(mut app: AppState) {
    let room_opt = app.room.read().clone();
    let Some(room) = room_opt else { return };
    let Some(sid) = app.current_server.read().clone() else {
        return;
    };
    match room.list_dm_conversations().await {
        Ok(convs) => {
            app.dm_conversations.write().insert(sid, convs);
        }
        Err(e) => tracing::warn!("list_dm_conversations: {}", e),
    }
}

#[component]
pub fn DmView() -> Element {
    let app = use_context::<AppState>();
    let mut ui = use_context::<UiState>();
    let i18n = use_i18n();
    let mut input = use_signal(String::new);

    let selected = ui.selected_dm_conv.read().clone();

    // 切换 DM 会话时清掉回复目标，防止把 A 会话的引用带进 B 会话。
    let mut prev_conv = use_signal(|| Option::<String>::None);
    use_effect(move || {
        let cur = ui.selected_dm_conv.read().clone();
        if *prev_conv.peek() != cur {
            prev_conv.set(cur.clone());
            ui.reply_target.set(None);
        }
    });

    let messages: Vec<UiMessage> = selected
        .as_ref()
        .and_then(|cid| app.dm_messages.read().get(cid).cloned())
        .unwrap_or_default();

    // typing 指示的过期重渲染：2s 本地 tick（只写不读，避免自触发循环）。
    let mut typing_tick = use_signal(|| 0u64);
    use_future(move || async move {
        loop {
            crate::content::tick_sleep(2000).await;
            *typing_tick.write() += 1;
        }
    });

    // DM 的 typing 键与 observer 写入处一致：channel_key(server, conv_id)。
    // 不显示自己。
    let typing_text = use_memo(move || {
        let _ = *typing_tick.read();
        let sid = app.current_server.read().clone()?;
        let conv = ui.selected_dm_conv.read().clone()?;
        let key = AppState::channel_key(&sid, &conv);
        let now = crate::observer::now_ms();
        let me = crate::content::current_uid(&ui, &app);
        let map = app.typing.read();
        let list = map.get(&key)?;
        let names: Vec<String> = list
            .iter()
            .filter(|(_, ts)| now - *ts < 6000)
            .filter(|(uid, _)| uid != &me)
            .map(|(uid, _)| crate::content::display_name(&app, &sid, uid))
            .collect();
        if names.is_empty() {
            return None;
        }
        Some(i18n.tf("channel.typing", &[("names", &names.join("、"))]))
    });

    // 已读：查看中的 DM 收到/加载新消息时上报 last_read 并清本地未读。
    // 按会话记录已上报的最大 ts，避免任何 DM 来消息都重复 POST。
    let reported_ts = use_hook(|| {
        std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::<
            String,
            i64,
        >::new()))
    });
    use_effect({
        let reported_ts = reported_ts.clone();
        move || {
            let sid = app.current_server.read().clone();
            let conv = ui.selected_dm_conv.read().clone();
            let (Some(sid), Some(conv)) = (sid, conv) else {
                return;
            };
            let max_ts = app
                .dm_messages
                .read()
                .get(&conv)
                .and_then(|list| list.iter().map(|m| m.ts).max());
            let Some(ts) = max_ts else {
                return;
            };
            {
                let mut seen = reported_ts.lock().unwrap_or_else(|e| e.into_inner());
                if seen.get(&conv).is_some_and(|prev| *prev >= ts) {
                    return;
                }
                seen.insert(conv.clone(), ts);
            }
            app.clear_unread(&sid, &conv);
            if let Some(room) = app.room.read().clone() {
                let conv = conv.clone();
                spawn(async move {
                    if let Err(e) = room.mark_dm_read(&conv, ts).await {
                        tracing::debug!("mark_dm_read: {}", e);
                    }
                });
            }
        }
    });

    let header: String = selected
        .as_ref()
        .and_then(|cid| {
            let sid = app.current_server.read().clone()?;
            let map = app.dm_conversations.read();
            let list = map.get(&sid)?;
            list.iter()
                .find(|c| &c.conv_id == cid)
                .map(|c| c.peer_nickname.clone())
        })
        .unwrap_or_default();

    let Some(conv_id) = selected else {
        return rsx! {
            div {
                style: "flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 16px; color: var(--fg-faint); font-size: 14px;",
                "选择或新建一个会话"
                button {
                    class: "btn-primary",
                    onclick: move |_| {
                        let mut ui2 = ui;
                        ui2.member_picker.set(Some(crate::state::MemberPickerMode::OpenDm));
                    },
                    "新建私信"
                }
            }
        };
    };

    rsx! {
        div {
            style: "flex: 1; min-width: 0; display: flex; flex-direction: column; overflow: hidden;",
            div {
                style: "padding: 16px 24px; border-bottom: 1px solid rgba(128,128,128,0.15); font-size: 15px; font-weight: 600; display: flex; align-items: center;",
                span { style: "flex: 1;", "{header}" }
                button {
                    class: "btn-secondary",
                    style: "padding: 4px 12px; font-size: 12px;",
                    onclick: move |_| {
                        let mut ui2 = ui;
                        ui2.member_picker.set(Some(crate::state::MemberPickerMode::OpenDm));
                    },
                    "+ 新建"
                }
            }
            div {
                style: "flex: 1; overflow-y: auto; padding: 16px 24px; display: flex; flex-direction: column; gap: 10px;",
                button {
                    class: "msg-load-earlier",
                    onclick: {
                        let cid = conv_id.clone();
                        move |_| {
                            let cid = cid.clone();
                            spawn(async move {
                                load_older_messages(app, cid).await;
                            });
                        }
                    },
                    "加载更早"
                }
                for m in messages {
                    DmMessageRow { key: "{m.id}", msg: m.clone(), conv_id: conv_id.clone() }
                }
            }
            div {
                style: "padding: 12px 24px; border-top: 1px solid rgba(128,128,128,0.15);",
                if let Some(text) = typing_text() {
                    div {
                        style: "font-size: 12px; color: var(--fg-faint); padding: 0 4px 4px;",
                        "{text}"
                    }
                }
                if let Some(r) = ui.reply_target.read().clone() {
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
                    style: "display: flex; gap: 8px;",
                    input {
                        r#type: "text",
                        placeholder: "输入消息...",
                        value: "{input}",
                        oninput: {
                            let cid = conv_id.clone();
                            move |e| {
                                input.set(e.value());
                                // 客户端内置 4 秒节流，UI 直接调即可。
                                if let Some(room) = app.room.read().clone() {
                                    let cid = cid.clone();
                                    spawn(async move {
                                        let _ = room.send_typing(&cid).await;
                                    });
                                }
                            }
                        },
                        style: "flex: 1; padding: 10px 14px; border-radius: 8px; border: 1px solid rgba(128,128,128,0.35); background: rgba(0,0,0,0.05); color: var(--fg); font-size: 14px; outline: none;",
                    }
                    button {
                        class: "btn-secondary",
                        title: "上传附件",
                        style: "padding: 8px 12px;",
                        onclick: move |_| {
                            let _ = dioxus::document::eval(
                                "document.getElementById('kaede-dm-file-input')?.click();",
                            );
                        },
                        "📎"
                    }
                    input {
                        id: "kaede-dm-file-input",
                        r#type: "file",
                        multiple: true,
                        style: "display:none;",
                        onchange: {
                            let cid = conv_id.clone();
                            move |e: dioxus::html::FormEvent| {
                                let files = e.files();
                                if files.is_empty() {
                                    return;
                                }
                                let _ = dioxus::document::eval(
                                    "document.getElementById('kaede-dm-file-input').value='';",
                                );
                                send_dm_attachments(ui, app, cid.clone(), files);
                            }
                        },
                    }
                    button {
                        class: "btn-primary",
                        onclick: {
                            let cid = conv_id.clone();
                            move |_| {
                                let text = input.read().clone();
                                if text.is_empty() { return; }
                                let mentions = app
                                    .current_server
                                    .read()
                                    .clone()
                                    .map(|sid| {
                                        crate::content::extract_mentions(
                                            &text,
                                            &crate::content::known_mentions(&app, &sid),
                                        )
                                    })
                                    .unwrap_or_default();
                                let cid2 = cid.clone();
                                let mut ui_for_err = ui;
                                let reply = ui.reply_target.read().clone();
                                spawn(async move {
                                    let room_opt = app.room.read().clone();
                                    let Some(room) = room_opt else { return };
                                    let result = match &reply {
                                        Some(r) => {
                                            room.send_dm_with_reply(
                                                &cid2,
                                                &text,
                                                &mentions,
                                                kaede_protocol::ReplyRef {
                                                    ts: r.ts,
                                                    author: r.author.clone(),
                                                    preview: r.preview.clone(),
                                                },
                                            )
                                            .await
                                        }
                                        None => {
                                            room.send_dm_with_attachments_mentions(
                                                &cid2, &text, &[], &mentions,
                                            )
                                            .await
                                        }
                                    };
                                    if let Err(e) = &result {
                                        tracing::warn!("send_dm: {}", e);
                                        ui_for_err
                                            .toast
                                            .set(Some((format!("发送失败: {}", e), true)));
                                    } else {
                                        ui_for_err.reply_target.set(None);
                                    }
                                    input.set(String::new());
                                });
                            }
                        },
                        "发送"
                    }
                }
            }
        }
    }
}

/// DM 消息行：右键菜单（自己的消息可编辑/删除）、内联编辑、（已编辑）标记。
#[component]
fn DmMessageRow(msg: UiMessage, conv_id: String) -> Element {
    let mut ui = use_context::<UiState>();
    let app = use_context::<AppState>();
    let i18n = use_i18n();

    let editing = ui
        .editing_message
        .read()
        .as_ref()
        .is_some_and(|(id, _)| id == &msg.id);
    let edit_draft = if editing {
        ui.editing_message
            .read()
            .as_ref()
            .map(|(_, d)| d.clone())
            .unwrap_or_default()
    } else {
        String::new()
    };

    let menu_target = MsgMenuTarget {
        target_id: conv_id.clone(),
        msg_id: msg.id.clone(),
        author: msg.author.clone(),
        ts: msg.ts,
        text: msg.text.clone(),
        is_dm: true,
    };

    let mid_edit = msg.id.clone();
    let target_ts = msg.ts;
    let submit_author = msg.author.clone();

    rsx! {
        div {
            style: "display: flex; gap: 10px; align-items: flex-start;",
            oncontextmenu: move |e| {
                e.prevent_default();
                // 菜单项在 MsgContextMenu 内过滤：回复所有人可用，
                // 编辑/删除仅自己的消息（mine 判断在菜单组件里）。
                let coords = e.client_coordinates();
                ui.msg_menu
                    .set(Some((coords.x as i32, coords.y as i32, menu_target.clone())));
            },
            div {
                style: "width: 32px; height: 32px; border-radius: 50%; background: var(--accent); color: white; display: flex; align-items: center; justify-content: center; font-size: 13px; font-weight: 600; flex-shrink: 0;",
                "{msg.author_initial}"
            }
            div {
                style: "flex: 1; min-width: 0;",
                div {
                    style: "display: flex; gap: 8px; align-items: baseline;",
                    span {
                        style: "font-size: 13px; font-weight: 600; color: var(--fg);",
                        "{msg.author}"
                    }
                    span {
                        style: "font-size: 11px; color: var(--fg-faint);",
                        "{msg.time}"
                    }
                    if msg.edited {
                        span {
                            style: "font-size: 11px; color: var(--fg-faint);",
                            "{i18n.t(\"channel.edited\")}"
                        }
                    }
                }
                if let Some(r) = &msg.reply_to {
                    div {
                        class: "msg-reply",
                        span { class: "msg-reply-author", "{r.author}" }
                        span { class: "msg-reply-preview", " {r.preview}" }
                    }
                }
                if editing {
                    div {
                        textarea {
                            id: "msg-edit-input",
                            rows: 2,
                            autofocus: true,
                            value: "{edit_draft}",
                            style: "width: 100%; box-sizing: border-box; padding: 6px 10px; border-radius: 6px; border: 1px solid rgba(128,128,128,0.4); background: rgba(0,0,0,0.05); color: var(--fg); font-size: 14px; outline: none; resize: none; font-family: inherit;",
                            oninput: move |e| {
                                ui.editing_message.set(Some((mid_edit.clone(), e.value())));
                            },
                            onkeydown: move |e: KeyboardEvent| {
                                if e.is_composing() {
                                    return;
                                }
                                if e.key() == Key::Escape {
                                    ui.editing_message.set(None);
                                    return;
                                }
                                if e.key() == Key::Enter
                                    && !e.modifiers().contains(Modifiers::SHIFT)
                                {
                                    e.prevent_default();
                                    crate::content::submit_edit(
                                        ui,
                                        app,
                                        i18n,
                                        conv_id.clone(),
                                        target_ts,
                                        submit_author.clone(),
                                        true,
                                    );
                                }
                            },
                        }
                        div {
                            style: "font-size: 11px; color: var(--fg-faint); margin-top: 2px;",
                            "{i18n.t(\"channel.editHint\")}"
                        }
                    }
                } else {
                    div {
                        style: "font-size: 14px; color: var(--fg); margin-top: 2px; white-space: pre-wrap; word-wrap: break-word;",
                        for seg in crate::content::split_mentions(&msg.text, &msg.mentions) {
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
                }
                // M57: DM 附件渲染（复用 content.rs 的 URL 白名单）
                if !msg.attachments.is_empty() {
                    div {
                        style: "margin-top: 6px; display: flex; flex-direction: column; gap: 4px;",
                        for att in msg.attachments.iter() {
                            crate::attachment::AttachmentView {
                                att: att.clone(),
                                target_id: msg.channel_id.clone(),
                            }
                        }
                    }
                }
            }
        }
    }
}

/// 上传 DM 附件并发送（本地回显由 ClientEvent::DmReceived 负责）。
fn send_dm_attachments(
    mut ui: UiState,
    app: AppState,
    conv_id: String,
    files: Vec<dioxus::html::FileData>,
) {
    let room = app.room.read().clone();
    spawn(async move {
        let Some(room) = room else { return };
        // 附件 key 来自 DM group 的 File purpose；先确保 group 存在。
        if let Err(e) = room.ensure_dm_group(&conv_id).await {
            ui.toast.set(Some((format!("初始化 DM 失败: {}", e), true)));
            return;
        }
        let mut atts = Vec::new();
        for f in files {
            match crate::attachment::upload_file(room.clone(), &conv_id, f).await {
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
        if let Err(e) = room.send_dm_with_attachments(&conv_id, "", &atts).await {
            ui.toast.set(Some((format!("发送附件失败: {}", e), true)));
        }
    });
}

#[component]
fn ConvItem(conv: DmConversation, selected: bool) -> Element {
    let app = use_context::<AppState>();
    let mut ui = use_context::<UiState>();
    let cnick = conv.peer_nickname.clone();
    let cpeer = conv.peer_login_id.clone();
    let cid = conv.conv_id.clone();
    let (unread_count, mention_count) = {
        let sid = app.current_server.read().clone();
        sid.and_then(|sid| {
            app.unread
                .read()
                .get(&AppState::channel_key(&sid, &cid))
                .map(|u| (u.count, u.mentions))
        })
        .unwrap_or((0, 0))
    };

    rsx! {
        div {
            style: if selected {
                "padding: 8px 12px; border-radius: 4px; cursor: pointer; background: rgba(128,128,128,0.15);"
            } else {
                "padding: 8px 12px; border-radius: 4px; cursor: pointer;"
            },
            onclick: move |_| {
                let cid = cid.clone();
                ui.selected_dm_conv.set(Some(cid.clone()));
                spawn(async move {
                    load_messages(app, cid).await;
                });
            },
            div {
                style: "display: flex; align-items: center; gap: 6px;",
                div {
                    style: "flex: 1; min-width: 0; font-size: 13px; font-weight: 500; color: var(--fg); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;",
                    "{cnick}"
                }
                if unread_count > 0 {
                    span {
                        title: if mention_count > 0 { "@ 提及" } else { "未读消息" },
                        style: if mention_count > 0 {
                            "flex-shrink: 0; min-width: 18px; text-align: center; padding: 1px 5px; border-radius: 9px; background: var(--negative); color: #fff; font-size: 10px; font-weight: 700;"
                        } else {
                            "flex-shrink: 0; min-width: 18px; text-align: center; padding: 1px 5px; border-radius: 9px; background: var(--active-bg); color: var(--fg); font-size: 10px; font-weight: 600;"
                        },
                        "{unread_count}"
                    }
                }
            }
            div {
                style: "font-size: 11px; color: var(--fg-faint);",
                "@{cpeer}"
            }
        }
    }
}

#[component]
pub fn DmSidebar() -> Element {
    let mut app = use_context::<AppState>();
    let mut ui = use_context::<UiState>();
    let mut new_peer = use_signal(String::new);

    // M58: use_effect + current_server.read() 订阅，切服务器时重拉
    use_effect(move || {
        let _sid = app.current_server.read();
        let app = app;
        spawn(async move {
            load_conversations(app).await;
        });
    });

    let server_id = app.current_server.read().clone();
    let convs: Vec<DmConversation> = server_id
        .as_ref()
        .and_then(|sid| app.dm_conversations.read().get(sid).cloned())
        .unwrap_or_default();
    let selected = ui.selected_dm_conv.read().clone();

    rsx! {
        div {
            style: "padding: 12px 8px; overflow-y: auto; display: flex; flex-direction: column; gap: 4px;",
            div {
                style: "display: flex; gap: 4px; padding: 0 4px 8px 4px;",
                input {
                    r#type: "text",
                    placeholder: "新私信 login_id",
                    value: "{new_peer}",
                    oninput: move |e| new_peer.set(e.value()),
                    style: "flex: 1; padding: 6px 8px; border-radius: 4px; border: 1px solid rgba(128,128,128,0.35); background: rgba(0,0,0,0.05); color: var(--fg); font-size: 12px; outline: none;",
                }
                button {
                    class: "btn-primary",
                    onclick: move |_| {
                        let peer = new_peer.read().clone();
                        if peer.is_empty() { return; }
                        spawn(async move {
                            let room_opt = app.room.read().clone();
                            let Some(room) = room_opt else { return };
                            match room.open_dm(&peer).await {
                                Ok(conv) => {
                                    let Some(sid) = app.current_server.read().clone() else { return };
                                    let cid = conv.conv_id.clone();
                                    {
                                        let mut map = app.dm_conversations.write();
                                        let list = map.entry(sid).or_default();
                                        if !list.iter().any(|c| c.conv_id == cid) {
                                            list.push(conv.clone());
                                        }
                                    }
                                    ui.selected_dm_conv.set(Some(cid.clone()));
                                    new_peer.set(String::new());
                                    load_messages(app, cid).await;
                                }
                                Err(e) => tracing::warn!("open_dm: {}", e),
                            }
                        });
                    },
                    "→"
                }
            }
            for c in convs {
                ConvItem { conv: c.clone(), selected: selected.as_ref() == Some(&c.conv_id) }
            }
        }
    }
}
