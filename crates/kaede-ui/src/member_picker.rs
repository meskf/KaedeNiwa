//! 从当前服务器成员中挑选（好友请求 / 新建 DM）。

#![allow(clippy::collapsible_if)]
use dioxus::prelude::*;
use kaede_protocol::Member;

use crate::state::{AppState, MemberPickerMode, UiState};

#[component]
pub fn MemberPickerModal() -> Element {
    let mut ui = use_context::<UiState>();
    let mut app = use_context::<AppState>();
    let mut search = use_signal(String::new);

    let Some(mode) = *ui.member_picker.read() else {
        return rsx! {};
    };

    let me = ui.common_account.read().login_id.clone();
    let sid = app.current_server.read().clone();
    let members: Vec<Member> = sid
        .as_ref()
        .and_then(|s| app.members.read().get(s).cloned())
        .unwrap_or_default();
    let filter = search.read().to_lowercase();

    let candidates: Vec<Member> = members
        .into_iter()
        .filter(|m| m.login_id != me)
        .filter(|m| {
            filter.is_empty()
                || m.login_id.to_lowercase().contains(&filter)
                || m.nickname.to_lowercase().contains(&filter)
        })
        .collect();

    let title = match mode {
        MemberPickerMode::FriendRequest => "选择好友",
        MemberPickerMode::OpenDm => "新建私信",
    };

    rsx! {
        div {
            class: "server-modal-backdrop",
            onclick: move |_| ui.member_picker.set(None),
            div {
                class: "server-modal",
                style: "width: 480px; height: auto; max-height: 70vh; padding: 24px; flex-direction: column;",
                onclick: move |e| e.stop_propagation(),
                div { class: "server-modal-title", "{title}" }
                div { class: "form-field",
                    input {
                        class: "form-input",
                        placeholder: "搜索 login_id 或昵称",
                        value: "{search}",
                        oninput: move |e| search.set(e.value()),
                    }
                }
                div {
                    style: "flex: 1; overflow-y: auto; display: flex; flex-direction: column; gap: 4px; min-height: 200px;",
                    if candidates.is_empty() {
                        div {
                            style: "padding: 16px; text-align: center; color: var(--fg-faint); font-size: 13px;",
                            "没有可选的成员"
                        }
                    } else {
                        for m in candidates {
                            {
                                let login_id = m.login_id.clone();
                                let nickname = m.nickname.clone();
                                let initial = m
                                    .login_id
                                    .chars()
                                    .next()
                                    .map(|c| c.to_string())
                                    .unwrap_or_default();
                                rsx! {
                                    button {
                                        key: "{login_id}",
                                        class: "btn-secondary",
                                        style: "width: 100%; justify-content: flex-start; padding: 10px 14px; gap: 12px; text-align: left;",
                                        onclick: move |_| {
                                            let id = login_id.clone();
                                            let mode_now = mode;
                                            spawn(async move {
                                                let room_opt = app.room.read().clone();
                                                let Some(room) = room_opt else { return; };
                                                match mode_now {
                                                    MemberPickerMode::FriendRequest => {
                                                        match room.send_friend_request(&id).await {
                                                            Ok(()) => {
                                                                ui.toast.set(Some((format!("已发送请求给 {}", id), false)));
                                                                // 刷新好友列表
                                                                if let Ok(list) = room.list_friends().await {
                                                                    if let Some(sid) = app.current_server.read().clone() {
                                                                        app.friends.write().insert(sid, list);
                                                                    }
                                                                }
                                                            }
                                                            Err(e) => {
                                                                ui.toast.set(Some((format!("请求失败: {}", e), true)));
                                                            }
                                                        }
                                                    }
                                                    MemberPickerMode::OpenDm => {
                                                        match room.open_dm(&id).await {
                                                            Ok(conv) => {
                                                                if let Some(sid) = app.current_server.read().clone() {
                                                                    let mut map = app.dm_conversations.write();
                                                                    let list = map.entry(sid).or_default();
                                                                    if !list.iter().any(|c| c.conv_id == conv.conv_id) {
                                                                        list.push(conv.clone());
                                                                    }
                                                                }
                                                                let cid = conv.conv_id.clone();
                                                                ui.selected_dm_conv.set(Some(cid.clone()));
                                                                ui.main_tab.set(crate::state::MainTab::Dm);
                                                                // 与 DM 列表点击一致：选中后立即拉历史，
                                                                // 否则通过成员选择器打开已有会话会显示空白。
                                                                crate::dm_view::load_messages(app, cid).await;
                                                            }
                                                            Err(e) => {
                                                                ui.toast.set(Some((format!("打开失败: {}", e), true)));
                                                            }
                                                        }
                                                    }
                                                }
                                                ui.member_picker.set(None);
                                            });
                                        },
                                        div {
                                            style: "width: 28px; height: 28px; border-radius: 50%; background: var(--accent); color: white; display: flex; align-items: center; justify-content: center; font-size: 12px; font-weight: 600; flex-shrink: 0;",
                                            "{initial}"
                                        }
                                        div {
                                            style: "flex: 1; min-width: 0;",
                                            div { style: "font-weight: 600;", "{nickname}" }
                                            div { style: "font-size: 11px; color: var(--fg-faint);", "{login_id}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                div { class: "form-actions", style: "margin-top: 12px;",
                    div { style: "flex: 1;" }
                    button {
                        class: "btn-secondary",
                        onclick: move |_| ui.member_picker.set(None),
                        "取消"
                    }
                }
            }
        }
    }
}
