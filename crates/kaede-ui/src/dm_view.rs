//! DM 视图（主区 + 侧栏）。

use dioxus::prelude::*;
use kaede_client::http::DmConversation;

use crate::state::{AppState, UiMessage, UiState};

async fn load_messages(mut app: AppState, conv_id: String) {
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
                    reactions: vec![],
                })
                .collect();
            app.dm_messages.write().insert(conv_id, ui_msgs);
        }
        Err(e) => tracing::warn!("dm_load_history: {}", e),
    }
}

async fn load_conversations(mut app: AppState) {
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
    let ui = use_context::<UiState>();
    let mut input = use_signal(String::new);

    let selected = ui.selected_dm_conv.read().clone();
    let messages: Vec<UiMessage> = selected
        .as_ref()
        .and_then(|cid| app.dm_messages.read().get(cid).cloned())
        .unwrap_or_default();

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
                for m in messages {
                    div {
                        style: "display: flex; gap: 10px; align-items: flex-start;",
                        div {
                            style: "width: 32px; height: 32px; border-radius: 50%; background: var(--accent); color: white; display: flex; align-items: center; justify-content: center; font-size: 13px; font-weight: 600; flex-shrink: 0;",
                            "{m.author_initial}"
                        }
                        div {
                            style: "flex: 1; min-width: 0;",
                            div {
                                style: "display: flex; gap: 8px; align-items: baseline;",
                                span {
                                    style: "font-size: 13px; font-weight: 600; color: var(--fg);",
                                    "{m.author}"
                                }
                                span {
                                    style: "font-size: 11px; color: var(--fg-faint);",
                                    "{m.time}"
                                }
                            }
                            div {
                                style: "font-size: 14px; color: var(--fg); margin-top: 2px; white-space: pre-wrap; word-wrap: break-word;",
                                "{m.text}"
                            }
                        }
                    }
                }
            }
            div {
                style: "padding: 12px 24px; border-top: 1px solid rgba(128,128,128,0.15);",
                div {
                    style: "display: flex; gap: 8px;",
                    input {
                        r#type: "text",
                        placeholder: "输入消息...",
                        value: "{input}",
                        oninput: move |e| input.set(e.value()),
                        style: "flex: 1; padding: 10px 14px; border-radius: 8px; border: 1px solid rgba(128,128,128,0.35); background: rgba(0,0,0,0.05); color: var(--fg); font-size: 14px; outline: none;",
                    }
                    button {
                        class: "btn-primary",
                        onclick: {
                            let cid = conv_id.clone();
                            move |_| {
                                let text = input.read().clone();
                                if text.is_empty() { return; }
                                let cid2 = cid.clone();
                                spawn(async move {
                                    let room_opt = app.room.read().clone();
                                    let Some(room) = room_opt else { return };
                                    if let Err(e) = room.send_dm(&cid2, &text).await {
                                        tracing::warn!("send_dm: {}", e);
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

#[component]
fn ConvItem(conv: DmConversation, selected: bool) -> Element {
    let app = use_context::<AppState>();
    let mut ui = use_context::<UiState>();
    let cnick = conv.peer_nickname.clone();
    let cpeer = conv.peer_login_id.clone();
    let cid = conv.conv_id.clone();

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
                style: "font-size: 13px; font-weight: 500; color: var(--fg);",
                "{cnick}"
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

    use_future(move || {
        let app = app;
        async move {
            load_conversations(app).await;
        }
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
