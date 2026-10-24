//! 内容区：频道头 + 消息列表 + 操作栏 + 输入框。

#![allow(clippy::collapsible_if)]

use crate::i18n::use_i18n;
use dioxus::prelude::*;

use crate::icons::{IconHash, IconVolume2};
use crate::state::{
    AppState, MsgMenuTarget, ReplyTarget, UiMessage, UiState, trim_cached_messages,
};

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
    let mut ui = use_context::<UiState>();
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

    // 记录上一次渲染的频道，用于区分“切换频道”和“当前频道来新消息”。
    let last_channel = use_hook(|| std::sync::Arc::new(std::sync::Mutex::new(String::new())));
    use_effect({
        let last_channel = last_channel.clone();
        move || {
            let _ = rows.read().len();
            let ch = ui.selected_channel.read().clone().unwrap_or_default();
            let switched = {
                let mut prev = last_channel.lock().unwrap_or_else(|e| e.into_inner());
                if *prev == ch {
                    false
                } else {
                    *prev = ch;
                    true
                }
            };
            // 切换频道：强制滚到底部。新消息：只有用户本来就在底部时才跟随，
            // 加载更早历史时不要把人弹回底部。
            let js = if switched {
                "setTimeout(function(){var el=document.getElementById('msg-list');\
                 if(el)el.scrollTop=el.scrollHeight;},0);"
            } else {
                "setTimeout(function(){var el=document.getElementById('msg-list');\
                 if(el && el.scrollHeight-el.scrollTop-el.clientHeight<120)el.scrollTop=el.scrollHeight;},0);"
            };
            let _ = dioxus::document::eval(js);
        }
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
                            reactions: h.reactions,
                            mentions: h.mentions,
                            edited: h.edited,
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
    //
    // 这里按频道记录“已上报的最大 ts”。messages 是整张 map 的信号，
    // 任何频道来消息都会让 effect 重跑；没有这个去重会向服务端重复 POST
    // mark-read（直到限流）。
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
            {
                let mut seen = reported_ts.lock().unwrap_or_else(|e| e.into_inner());
                if seen.get(&ch).is_some_and(|prev| *prev >= ts) {
                    return;
                }
                seen.insert(ch.clone(), ts);
            }
            app.clear_unread(&sid, &ch);
            if let Some(room) = app.room.read().clone() {
                let ch = ch.clone();
                spawn(async move {
                    if let Err(e) = room.mark_channel_read(&ch, ts).await {
                        tracing::debug!("mark_channel_read: {}", e);
                    }
                });
            }
        }
    });

    // 切换频道时清掉回复目标与编辑态，避免把 A 频道的操作带到 B 频道。
    use_effect(move || {
        let _ = ui.selected_channel.read();
        ui.reply_target.set(None);
        ui.editing_message.set(None);
    });

    // 1.2 滚到顶自动加载：监听 #msg-list 滚动，靠近顶部时自动触发"加载更早"按钮。
    //     按钮保留作为 fallback；用 data-loading + 元素属性 + 重试做三重保护。
    use_effect(move || {
        let _ = dioxus::document::eval(
            r#"
            (function(){
                function tryBind(attemptsLeft){
                    const list = document.getElementById('msg-list');
                    if (!list) {
                        if (attemptsLeft > 0) setTimeout(function(){ tryBind(attemptsLeft - 1); }, 100);
                        return;
                    }
                    if (list.__kaedeScrollBound) return;
                    list.__kaedeScrollBound = true;
                    let cooling = false;
                    list.addEventListener('scroll', function(){
                        if (cooling) return;
                        if (list.scrollTop > 120) return;
                        const btn = list.querySelector('.msg-load-earlier');
                        if (!btn) return;
                        if (btn.getAttribute('data-loading') === '1') return;
                        btn.setAttribute('data-loading', '1');
                        btn.click();
                        cooling = true;
                        setTimeout(function(){
                            cooling = false;
                            btn.removeAttribute('data-loading');
                        }, 800);
                    });
                }
                tryBind(10);
            })();
        "#,
        );
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
                                    reactions: h.reactions,
                                    mentions: h.mentions,
                                    edited: h.edited,
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
                        // 用稳定 id 作为 key：下标 key 在前插历史/裁剪窗口时会
                        // 让所有节点重建，丢失 hover/选中状态。
                        MessageGroup { key: "{msg.id}", msg: msg.clone(), merged: *merged }
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
    let i18n = use_i18n();
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

    // 内联编辑态：全局 signal 存 (msg_id, 草稿)，同一时刻只编辑一条。
    let editing = ui
        .editing_message
        .read()
        .as_ref()
        .is_some_and(|(id, _)| id == &mid);
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
        target_id: msg.channel_id.clone(),
        msg_id: mid.clone(),
        author: msg.author.clone(),
        ts: msg.ts,
        text: msg.text.clone(),
        is_dm: false,
    };

    let mid_edit = mid.clone();
    let submit_ch = msg.channel_id.clone();
    let submit_author = msg.author.clone();

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
                let coords = e.client_coordinates();
                ui.msg_menu.set(Some((
                    coords.x as i32,
                    coords.y as i32,
                    menu_target.clone(),
                )));
            },
            if !merged {
                div {
                    class: "msg-head",
                    div { class: "msg-avatar", "{msg.author_initial}" }
                    span { class: "msg-author", "{msg.author}" }
                    span { class: "msg-time", "{msg.time}" }
                    if msg.edited {
                        span {
                            style: "font-size: 11px; color: var(--fg-faint);",
                            "{i18n.t(\"channel.edited\")}"
                        }
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
                div { class: "msg-body",
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
                            // 输入法候选词确认的 Enter 不提交。
                            if e.is_composing() {
                                return;
                            }
                            if e.key() == Key::Escape {
                                ui.editing_message.set(None);
                                return;
                            }
                            if e.key() == Key::Enter && !e.modifiers().contains(Modifiers::SHIFT) {
                                e.prevent_default();
                                submit_edit(ui, app, i18n, submit_ch.clone(), target_ts, submit_author.clone(), false);
                            }
                        },
                    }
                    div {
                        style: "font-size: 11px; color: var(--fg-faint); margin-top: 2px;",
                        "{i18n.t(\"channel.editHint\")}"
                    }
                }
            } else {
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

    // 切换频道时清空草稿：草稿是全局 Signal，不清会串到下一个频道发送。
    let mut prev_channel = use_signal(|| Option::<String>::None);
    use_effect(move || {
        let cur = ui.selected_channel.read().clone();
        if *prev_channel.peek() != cur {
            prev_channel.set(cur);
            ui.draft.set(String::new());
        }
    });

    // typing 指示的过期重渲染：每 2s 推一次本地 tick（只写不读，避免
    // 响应式自触发循环——app.rs ToastContainer 的教训）。记录本身由
    // ClientEvent::Typing 写入，渲染时按 6 秒窗口过滤。
    let mut typing_tick = use_signal(|| 0u64);
    use_future(move || async move {
        loop {
            tick_sleep(2000).await;
            *typing_tick.write() += 1;
        }
    });

    let typing_text = use_memo(move || {
        let _ = *typing_tick.read();
        let sid = app.current_server.read().clone()?;
        let ch = ui.selected_channel.read().clone()?;
        let key = AppState::channel_key(&sid, &ch);
        let now = crate::observer::now_ms();
        let map = app.typing.read();
        let list = map.get(&key)?;
        let names: Vec<String> = list
            .iter()
            .filter(|(_, ts)| now - *ts < 6000)
            .map(|(uid, _)| display_name(&app, &sid, uid))
            .collect();
        if names.is_empty() {
            return None;
        }
        Some(i18n.tf("channel.typing", &[("names", &names.join("、"))]))
    });

    // D1: 拖拽 / 粘贴桥接（wasm-only，幂等）
    #[cfg(target_arch = "wasm32")]
    use_effect(move || {
        crate::dom_bridge::bind_drop_and_paste(ui, app);
        crate::dom_bridge::bind_paste(ui, app);
    });

    rsx! {
        div {
            id: "composer-wrap",
            class: "composer-wrap",
            if let Some(text) = typing_text() {
                div {
                    style: "font-size: 12px; color: var(--fg-faint); padding: 0 4px 4px;",
                    "{text}"
                }
            }
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
                    id: "composer-input",
                    class: "composer-input",
                    placeholder: "{i18n.t(\"channel.inputPlaceholder\")}",
                    value: "{draft}",
                    oninput: move |e| {
                        draft.set(e.value());
                        // 客户端内置 4 秒节流，UI 直接调即可。
                        if let Some(room) = app.room.read().clone()
                            && let Some(ch) = ui.selected_channel.read().clone()
                        {
                            spawn(async move {
                                let _ = room.send_typing(&ch).await;
                            });
                        }
                    },
                    onkeydown: move |e: KeyboardEvent| {
                        // 中文/日文输入法用 Enter 确认候选词，此时不能发送消息。
                        if e.is_composing() {
                            return;
                        }
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
            edited: false,
        };
        {
            let mut map = app.messages.write();
            let list = map.entry(AppState::channel_key(&sid, &ch)).or_default();
            list.push(msg);
            trim_cached_messages(list);
        }
    });
}

#[cfg(target_arch = "wasm32")]
/// 从原始字节上传附件（供 dom_bridge 拖拽 / 粘贴复用）。
pub(crate) fn send_attachments_bytes(
    mut ui: UiState,
    mut app: AppState,
    ch: String,
    sid: String,
    files: Vec<crate::state::UploadFile>,
) {
    let room = app.room.read().clone();
    spawn(async move {
        let Some(room) = room else { return };
        let mut atts = Vec::new();
        for f in files {
            match crate::attachment::upload_file_from_bytes(
                room.clone(),
                &ch,
                &f.name,
                Some(f.mime.as_str()),
                f.data,
            )
            .await
            {
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
            edited: false,
        };
        {
            let mut map = app.messages.write();
            let list = map.entry(AppState::channel_key(&sid, &ch)).or_default();
            list.push(msg);
            trim_cached_messages(list);
        }
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
        edited: false,
    };
    let key = AppState::channel_key(&sid, &ch);
    {
        let mut map = app.messages.write();
        let list = map.entry(key).or_default();
        list.push(msg);
        trim_cached_messages(list);
    }

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
// 消息右键菜单 / 编辑 / 删除 / Typing（频道与 DM 共用）
// ============================================================

/// 当前用户 login_id（优先通用账户，回退到 room uid）。
pub(crate) fn current_uid(ui: &UiState, app: &AppState) -> String {
    let acc = ui.common_account.read().login_id.clone();
    if !acc.is_empty() {
        return acc;
    }
    app.room
        .read()
        .as_ref()
        .map(|r| r.uid().as_str().to_string())
        .unwrap_or_default()
}

/// uid → 展示名（成员昵称，昵称为空用 login_id；非成员回退 uid）。
pub(crate) fn display_name(app: &AppState, server_id: &str, uid: &str) -> String {
    app.members
        .read()
        .get(server_id)
        .and_then(|l| l.iter().find(|m| m.login_id == uid).cloned())
        .map(|m| {
            if m.nickname.is_empty() {
                m.login_id
            } else {
                m.nickname
            }
        })
        .unwrap_or_else(|| uid.to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn tick_sleep(ms: u64) {
    tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
}

#[cfg(target_arch = "wasm32")]
pub(crate) async fn tick_sleep(ms: u64) {
    gloo_timers::future::TimeoutFuture::new(ms as u32).await;
}

/// 提交内联编辑：先发 send_edit，成功后乐观本地应用。
///
/// 公开/私密频道服务端不回声自己，必须靠乐观应用；DM 有本地 echo
/// （ClientEvent::DmReceived）会再应用一次，apply_edit 幂等无害。
pub(crate) fn submit_edit(
    mut ui: UiState,
    mut app: AppState,
    i18n: crate::i18n::I18n,
    channel_id: String,
    target_ts: i64,
    author: String,
    is_dm: bool,
) {
    let draft = ui
        .editing_message
        .read()
        .as_ref()
        .map(|(_, d)| d.clone())
        .unwrap_or_default();
    let new_text = draft.trim().to_string();
    ui.editing_message.set(None);
    if new_text.is_empty() {
        return;
    }
    let Some(room) = app.room.read().clone() else {
        return;
    };
    let sid = app.current_server.read().clone();
    spawn(async move {
        if let Err(e) = room.send_edit(&channel_id, target_ts, &new_text).await {
            ui.toast.set(Some((
                i18n.tf("channel.editFailed", &[("err", &e.to_string())]),
                true,
            )));
            return;
        }
        if is_dm {
            crate::observer::apply_dm_edit(&mut app, &channel_id, target_ts, &author, &new_text);
        } else if let Some(sid) = sid {
            crate::observer::apply_edit(&mut app, &sid, &channel_id, target_ts, &author, &new_text);
        }
    });
}

/// 删除消息：无需确认直接 send_delete，成功后乐观从列表移除。
pub(crate) fn delete_message(
    mut ui: UiState,
    mut app: AppState,
    i18n: crate::i18n::I18n,
    target: MsgMenuTarget,
) {
    let Some(room) = app.room.read().clone() else {
        return;
    };
    let sid = app.current_server.read().clone();
    spawn(async move {
        if let Err(e) = room.send_delete(&target.target_id, target.ts).await {
            ui.toast.set(Some((
                i18n.tf("channel.deleteFailed", &[("err", &e.to_string())]),
                true,
            )));
            return;
        }
        if target.is_dm {
            crate::observer::apply_dm_delete(
                &mut app,
                &target.target_id,
                target.ts,
                &target.author,
            );
        } else if let Some(sid) = sid {
            crate::observer::apply_delete(
                &mut app,
                &sid,
                &target.target_id,
                target.ts,
                &target.author,
            );
        }
    });
}

/// 消息右键菜单（全局弹层）：回复、添加反应（仅频道）、编辑 / 删除（仅自己的消息）。
#[component]
pub fn MsgContextMenu() -> Element {
    let mut ui = use_context::<UiState>();
    let app = use_context::<AppState>();
    let i18n = use_i18n();

    let Some((x, y, target)) = ui.msg_menu.read().clone() else {
        return rsx! {};
    };

    let me = current_uid(&ui, &app);
    let mine = target.author == me;

    let btn_style = "width: 100%; text-align: left; padding: 8px 12px; border-radius: 6px; font-size: 14px; background: transparent; border: none; cursor: pointer; color: var(--fg);";

    rsx! {
        div {
            style: "position: fixed; inset: 0; z-index: 1000;",
            onclick: move |_| ui.msg_menu.set(None),
            oncontextmenu: move |e| {
                e.prevent_default();
                ui.msg_menu.set(None);
            },
            div {
                onclick: move |e| e.stop_propagation(),
                style: "position: fixed; left: {x}px; top: {y}px; background: var(--inner-bg); border-radius: 10px; padding: 6px; min-width: 160px; box-shadow: 0 8px 32px rgba(0,0,0,0.25); display: flex; flex-direction: column; gap: 2px;",
                button {
                    style: "{btn_style}",
                    onclick: {
                        let t = target.clone();
                        move |_| {
                            ui.reply_target.set(Some(ReplyTarget {
                                author: t.author.clone(),
                                preview: t.text.chars().take(80).collect(),
                                ts: t.ts,
                            }));
                            ui.msg_menu.set(None);
                        }
                    },
                    "{i18n.t(\"channel.reply\")}"
                }
                if !target.is_dm {
                    button {
                        style: "{btn_style}",
                        onclick: {
                            let t = target.clone();
                            move |_| {
                                ui.emoji_picker.set(Some((t.target_id.clone(), t.ts)));
                                ui.msg_menu.set(None);
                            }
                        },
                        "{i18n.t(\"channel.react\")}"
                    }
                }
                if mine {
                    button {
                        style: "{btn_style}",
                        onclick: {
                            let t = target.clone();
                            move |_| {
                                ui.editing_message.set(Some((t.msg_id.clone(), t.text.clone())));
                                ui.msg_menu.set(None);
                                // autofocus 之外再兜底一次：动态插入的元素
                                // 某些 webview 不触发 autofocus。
                                let _ = dioxus::document::eval(
                                    "setTimeout(function(){var el=document.getElementById('msg-edit-input');if(el){el.focus();el.setSelectionRange(el.value.length,el.value.length);}},0);",
                                );
                            }
                        },
                        "{i18n.t(\"channel.edit\")}"
                    }
                    button {
                        style: "{btn_style}",
                        onclick: {
                            let t = target.clone();
                            move |_| {
                                ui.msg_menu.set(None);
                                delete_message(ui, app, i18n, t.clone());
                            }
                        },
                        "{i18n.t(\"channel.delete\")}"
                    }
                }
            }
        }
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
