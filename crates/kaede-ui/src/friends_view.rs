//! 好友视图（主区 + 侧栏）。
//!
//! 数据流：`use_future` 挂载时拉 `room.list_friends()` → 写 `app.friends`。
//! 操作（添加/接受/删除）后重新拉，靠 Signal 触发重渲染，不做乐观更新。

use dioxus::prelude::*;
use kaede_client::http::Friend;

use crate::state::AppState;

pub(crate) async fn load_friends_into(mut app: AppState) {
    let room_opt = app.room.read().clone();
    let Some(room) = room_opt else { return };
    let Some(sid) = app.current_server.read().clone() else {
        return;
    };
    match room.list_friends().await {
        Ok(friends) => {
            app.friends.write().insert(sid, friends);
        }
        Err(e) => tracing::warn!("list_friends: {}", e),
    }
}

#[component]
pub fn FriendsView() -> Element {
    let app = use_context::<AppState>();
    let mut ui = use_context::<crate::state::UiState>();

    // M58: use_effect + current_server.read() 订阅，切服务器时重拉；
    // 原 use_future 闭包内没 read 任何 signal → 只跑一次。
    use_effect(move || {
        let _sid = app.current_server.read();
        let app = app;
        spawn(async move {
            load_friends_into(app).await;
        });
    });

    let server_id = app.current_server.read().clone();
    let friends: Vec<Friend> = server_id
        .as_ref()
        .and_then(|sid| app.friends.read().get(sid).cloned())
        .unwrap_or_default();

    let incoming: Vec<Friend> = friends
        .iter()
        .filter(|f| f.direction == "incoming")
        .cloned()
        .collect();
    let accepted: Vec<Friend> = friends
        .iter()
        .filter(|f| f.direction == "friend")
        .cloned()
        .collect();
    let outgoing: Vec<Friend> = friends
        .iter()
        .filter(|f| f.direction == "outgoing")
        .cloned()
        .collect();

    rsx! {
        div {
            style: "flex: 1; min-width: 0; display: flex; flex-direction: column; padding: 32px 40px; overflow-y: auto;",
            h2 {
                style: "margin: 0 0 20px 0; font-size: 22px; font-weight: 600;",
                "好友"
            }
            div {
                style: "display: flex; gap: 8px; margin-bottom: 28px;",
                button {
                    class: "btn-primary",
                    onclick: move |_| {
                        ui.member_picker.set(Some(crate::state::MemberPickerMode::FriendRequest));
                    },
                    "从成员中选择"
                }
            }
            if !incoming.is_empty() {
                Section { title: "待接受" }
                for f in incoming {
                    FriendRow { friend: f, kind: String::from("incoming") }
                }
            }
            Section { title: "好友" }
            if accepted.is_empty() {
                p {
                    style: "color: var(--fg-faint); font-size: 14px; margin: 8px 0 24px 0;",
                    "暂无好友"
                }
            } else {
                for f in accepted {
                    FriendRow { friend: f, kind: String::from("friend") }
                }
            }
            if !outgoing.is_empty() {
                Section { title: "已发送请求" }
                for f in outgoing {
                    FriendRow { friend: f, kind: String::from("outgoing") }
                }
            }
        }
    }
}

#[component]
fn Section(title: String) -> Element {
    rsx! {
        h3 {
            style: "margin: 20px 0 8px 0; font-size: 13px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.05em; color: var(--fg-faint);",
            "{title}"
        }
    }
}

#[component]
fn FriendRow(friend: Friend, kind: String) -> Element {
    let mut app = use_context::<AppState>();
    let mut ui = use_context::<crate::state::UiState>();

    let nickname = friend.nickname.clone();
    let login_id_display = friend.login_id.clone();

    let lid_accept = friend.login_id.clone();
    let lid_reject = friend.login_id.clone();
    let lid_dm = friend.login_id.clone();
    let lid_remove = friend.login_id.clone();

    rsx! {
        div {
            style: "display: flex; align-items: center; gap: 12px; padding: 10px 12px; border-radius: 6px; background: rgba(0,0,0,0.03); margin-bottom: 6px;",
            div {
                style: "flex: 1; min-width: 0;",
                div {
                    style: "font-size: 14px; font-weight: 500; color: var(--fg);",
                    "{nickname}"
                }
                div {
                    style: "font-size: 12px; color: var(--fg-faint); margin-top: 2px;",
                    "@{login_id_display}"
                }
            }
            if kind == "incoming" {
                button {
                    class: "btn-primary",
                    onclick: move |_| {
                        let id = lid_accept.clone();
                        spawn(async move {
                            let room_opt = app.room.read().clone();
                            let Some(room) = room_opt else { return };
                            if let Err(e) = room.accept_friend(&id).await {
                                tracing::warn!("accept_friend: {}", e);
                                return;
                            }
                            load_friends_into(app).await;
                        });
                    },
                    "接受"
                }
                button {
                    class: "btn-secondary",
                    onclick: move |_| {
                        let id = lid_reject.clone();
                        spawn(async move {
                            let room_opt = app.room.read().clone();
                            let Some(room) = room_opt else { return };
                            if let Err(e) = room.remove_friend(&id).await {
                                tracing::warn!("remove_friend: {}", e);
                                return;
                            }
                            load_friends_into(app).await;
                        });
                    },
                    "拒绝"
                }
            } else if kind == "friend" {
                button {
                    class: "btn-primary",
                    onclick: move |_| {
                        let id = lid_dm.clone();
                        spawn(async move {
                            let room_opt = app.room.read().clone();
                            let Some(room) = room_opt else { return };
                            match room.open_dm(&id).await {
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
                                    ui.main_tab.set(crate::state::MainTab::Dm);
                                    match room.dm_load_history(&cid, None, 100).await {
                                        Ok(history) => {
                                            let ui_msgs: Vec<crate::state::UiMessage> = history
                                                .into_iter()
                                                .map(|h| crate::state::UiMessage {
                                                    id: format!("dm-{}-{}", cid, h.ts),
                                                    channel_id: cid.clone(),
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
                                                })
                                                .collect();
                                            app.dm_messages.write().insert(cid, ui_msgs);
                                        }
                                        Err(e) => tracing::warn!("dm_load_history: {}", e),
                                    }
                                }
                                Err(e) => tracing::warn!("open_dm: {}", e),
                            }
                        });
                    },
                    "私信"
                }
                button {
                    class: "btn-secondary",
                    onclick: move |_| {
                        let id = lid_remove.clone();
                        spawn(async move {
                            let room_opt = app.room.read().clone();
                            let Some(room) = room_opt else { return };
                            if let Err(e) = room.remove_friend(&id).await {
                                tracing::warn!("remove_friend: {}", e);
                                return;
                            }
                            load_friends_into(app).await;
                        });
                    },
                    "删除"
                }
            } else {
                span {
                    style: "font-size: 12px; color: var(--fg-faint);",
                    "等待接受"
                }
            }
        }
    }
}

#[component]
pub fn FriendsSidebar() -> Element {
    let app = use_context::<AppState>();
    let server_id = app.current_server.read().clone();
    let friends: Vec<Friend> = server_id
        .as_ref()
        .and_then(|sid| app.friends.read().get(sid).cloned())
        .unwrap_or_default();
    let accepted: Vec<Friend> = friends
        .iter()
        .filter(|f| f.direction == "friend")
        .cloned()
        .collect();

    if accepted.is_empty() {
        return rsx! {
            div {
                style: "padding: 24px 12px; color: var(--fg-faint); font-size: 13px; text-align: center;",
                "好友列表为空"
            }
        };
    }

    rsx! {
        div {
            style: "padding: 12px 8px; overflow-y: auto;",
            for f in accepted {
                div {
                    style: "padding: 6px 10px; border-radius: 4px; font-size: 13px; color: var(--fg);",
                    "{f.nickname}"
                }
            }
        }
    }
}
