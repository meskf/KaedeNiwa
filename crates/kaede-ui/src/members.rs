//! 右栏：成员列表。

use crate::i18n::use_i18n;
use crate::state::{AppState, UiState};
use dioxus::prelude::*;

#[component]
pub fn MemberList() -> Element {
    let app = use_context::<AppState>();
    let i18n = use_i18n();

    let groups = use_memo(move || {
        let sid = app.current_server.read().clone();
        let Some(sid) = sid else {
            return (Vec::new(), Vec::new());
        };
        let members = app.members.read();
        let presence = app.presence.read();
        let list = members.get(&sid).cloned().unwrap_or_default();
        let mut online = Vec::new();
        let mut offline = Vec::new();
        for m in list {
            let status = presence
                .get(&m.login_id)
                .map(|s| s.as_str())
                .unwrap_or("offline");
            if status == "offline" {
                offline.push(m);
            } else {
                online.push(m);
            }
        }
        (online, offline)
    });

    let (online, offline) = groups();

    rsx! {
        div {
            class: "member-list",
            if !online.is_empty() {
                div { class: "member-section",
                    "{i18n.tf(\"member.online\", &[(\"count\", &online.len().to_string())])}"
                }
                for m in online.iter() {
                    MemberRow { key: "{m.login_id}", member: m.clone(), offline: false }
                }
            }
            if !offline.is_empty() {
                div { class: "member-section",
                    "{i18n.tf(\"member.offline\", &[(\"count\", &offline.len().to_string())])}"
                }
                for m in offline.iter() {
                    MemberRow { key: "{m.login_id}", member: m.clone(), offline: true }
                }
            }
        }
    }
}

#[component]
fn MemberRow(member: kaede_protocol::Member, #[props(default)] offline: bool) -> Element {
    let mut ui = use_context::<UiState>();
    let mid_ctx = member.login_id.clone();
    let name = if member.nickname.is_empty() {
        member.login_id.clone()
    } else {
        member.nickname.clone()
    };
    let initial = name
        .chars()
        .next()
        .map(|c| c.to_string())
        .unwrap_or_default();
    let color = avatar_color(&member.login_id);
    let is_op = member.op == "op5" || member.op == "op4";
    let is_mod = member.op == "op3";
    let cls = if offline {
        "member-row member-offline"
    } else {
        "member-row"
    };

    rsx! {
        button {
            class: "{cls}",
            title: "{member.login_id}",
            oncontextmenu: move |e| {
                e.prevent_default();
                ui.member_menu.set(Some(mid_ctx.clone()));
            },
            div {
                style: "position: relative; flex-shrink: 0;",
                div {
                    style: "width: 28px; height: 28px; border-radius: 50%; background: {color}; color: #fff; display: flex; align-items: center; justify-content: center; font-size: 13px; font-weight: 600;",
                    "{initial}"
                }
                if !offline {
                    div {
                        style: "position: absolute; bottom: -1px; right: -1px; width: 10px; height: 10px; border-radius: 50%; background: #22C55E; border: 2px solid var(--inner-bg);",
                    }
                }
            }
            div {
                style: "min-width: 0; flex: 1; display: flex; align-items: center; gap: 6px;",
                span {
                    style: "font-size: 14px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;",
                    "{name}"
                }
                if is_op {
                    span {
                        style: "font-size: 10px; padding: 1px 6px; border-radius: 4px; background: var(--accent-bg); color: var(--accent-fg); flex-shrink: 0;",
                        "管理员"
                    }
                }
                if is_mod {
                    span {
                        style: "font-size: 10px; padding: 1px 6px; border-radius: 4px; background: var(--active-bg); color: var(--fg-muted); flex-shrink: 0;",
                        "版主"
                    }
                }
            }
        }
    }
}

fn avatar_color(id: &str) -> &'static str {
    const COLORS: &[&str] = &[
        "#7C3AED", "#0EA5E9", "#F59E0B", "#10B981", "#EF4444", "#8B5CF6", "#EC4899", "#14B8A6",
    ];
    let hash = id
        .bytes()
        .fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));
    COLORS[(hash as usize) % COLORS.len()]
}
#[component]
pub fn MemberContextMenu() -> Element {
    let mut ui = use_context::<UiState>();
    let mut app = use_context::<AppState>();

    let Some(target_id) = ui.member_menu.read().clone() else {
        return rsx! {};
    };
    let Some(sid) = app.current_server.read().clone() else {
        return rsx! {};
    };

    let (target, my_op, me) = {
        let members = app.members.read();
        let list = members.get(&sid).cloned().unwrap_or_default();
        let target = list.iter().find(|m| m.login_id == target_id).cloned();
        let me = ui.common_account.read().login_id.clone();
        let my_op = list
            .iter()
            .find(|m| m.login_id == me)
            .map(|m| m.op.clone())
            .unwrap_or_else(|| "op1".to_string());
        (target, my_op, me)
    };

    let Some(target) = target else {
        return rsx! {};
    };

    let is_self = me == target_id;
    let is_admin = my_op == "op4" || my_op == "op5";
    let target_name = if target.nickname.is_empty() {
        target.login_id.clone()
    } else {
        target.nickname.clone()
    };
    let target_op = target.op.clone();
    let target_title = target.title.clone();
    let editing = ui.member_editing_title.read().clone();

    rsx! {
        div {
            class: "server-modal-backdrop",
            style: "background: rgba(0,0,0,0.15);",
            onclick: move |_| {
                ui.member_menu.set(None);
                ui.member_editing_title.set(None);
            },
            div {
                onclick: move |e| e.stop_propagation(),
                style: "background: var(--inner-bg); border-radius: 12px; padding: 8px; min-width: 240px; box-shadow: 0 8px 32px rgba(0,0,0,0.15);",
                div {
                    style: "padding: 8px 12px; font-size: 11px; color: var(--fg-faint); text-transform: uppercase; letter-spacing: 0.8px;",
                    "{target_name} · {target_op}"
                }

                if !is_self {
                    button {
                        style: "width: 100%; text-align: left; padding: 8px 12px; border-radius: 6px; font-size: 14px;",
                        onclick: {
                            let target_id_dm = target_id.clone();
                            move |_| {
                                let target_id_dm = target_id_dm.clone();
                                spawn(async move {
                                    let Some(room) = app.room.read().clone() else { return };
                                    match room.open_dm(&target_id_dm).await {
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
                                            ui.member_menu.set(None);
                                            ui.member_editing_title.set(None);
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
                            }
                        },
                        "私信"
                    }
                }

                if is_self || is_admin {
                    if editing.is_some() {
                        div {
                            style: "padding: 8px; display: flex; flex-direction: column; gap: 6px;",
                            input {
                                r#type: "text",
                                value: "{editing.clone().unwrap_or_default()}",
                                placeholder: "称号",
                                oninput: move |e| ui.member_editing_title.set(Some(e.value())),
                                style: "padding: 6px 10px; border-radius: 6px; border: 1px solid rgba(128,128,128,0.35); background: rgba(0,0,0,0.05); color: var(--fg); font-size: 14px; outline: none;",
                            }
                            div {
                                style: "display: flex; gap: 6px;",
                                button {
                                    class: "btn-secondary",
                                    style: "flex: 1;",
                                    onclick: move |_| ui.member_editing_title.set(None),
                                    "取消"
                                }
                                button {
                                    class: "btn-primary",
                                    style: "flex: 1;",
                                    onclick: {
                                        let target_id_edit = target_id.clone();
                                        move |_| {
                                            let new_title = ui.member_editing_title.read().clone().unwrap_or_default();
                                            let target_id_edit = target_id_edit.clone();
                                            spawn(async move {
                                                let Some(room) = app.room.read().clone() else { return };
                                                let patch = kaede_client::http::UpdateMemberReq {
                                                    title: Some(new_title),
                                                    nickname: None,
                                                    bio: None,
                                                    avatar_url: None,
                                                    op: None,
                                                };
                                                match room.update_member(&target_id_edit, patch).await {
                                                    Ok(_) => {
                                                        ui.member_menu.set(None);
                                                        ui.member_editing_title.set(None);
                                                        ui.toast.set(Some(("称号已更新".into(), false)));
                                                    }
                                                    Err(e) => {
                                                        ui.toast.set(Some((format!("更新失败: {}", e), true)));
                                                    }
                                                }
                                            });
                                        }
                                    },
                                    "保存"
                                }
                            }
                        }
                    } else {
                        button {
                            style: "width: 100%; text-align: left; padding: 8px 12px; border-radius: 6px; font-size: 14px;",
                            onclick: move |_| ui.member_editing_title.set(Some(target_title.clone())),
                            "设置称号"
                        }
                    }
                }

                if is_admin && !is_self {
                    div {
                        style: "padding: 6px 12px 2px 12px; font-size: 11px; color: var(--fg-faint); text-transform: uppercase;",
                        "权限"
                    }
                    button {
                        style: if target_op == "op1" { "width: 100%; text-align: left; padding: 6px 12px; border-radius: 6px; font-size: 13px; background: rgba(128,128,128,0.15);" } else { "width: 100%; text-align: left; padding: 6px 12px; border-radius: 6px; font-size: 13px;" },
                        onclick: {
                            let t = target_id.clone();
                            move |_| {
                                let t = t.clone();
                                spawn(async move { set_op(app, ui, &t, "op1").await; });
                            }
                        },
                        "访客"
                    }
                    button {
                        style: if target_op == "op2" { "width: 100%; text-align: left; padding: 6px 12px; border-radius: 6px; font-size: 13px; background: rgba(128,128,128,0.15);" } else { "width: 100%; text-align: left; padding: 6px 12px; border-radius: 6px; font-size: 13px;" },
                        onclick: {
                            let t = target_id.clone();
                            move |_| {
                                let t = t.clone();
                                spawn(async move { set_op(app, ui, &t, "op2").await; });
                            }
                        },
                        "成员"
                    }
                    button {
                        style: if target_op == "op3" { "width: 100%; text-align: left; padding: 6px 12px; border-radius: 6px; font-size: 13px; background: rgba(128,128,128,0.15);" } else { "width: 100%; text-align: left; padding: 6px 12px; border-radius: 6px; font-size: 13px;" },
                        onclick: {
                            let t = target_id.clone();
                            move |_| {
                                let t = t.clone();
                                spawn(async move { set_op(app, ui, &t, "op3").await; });
                            }
                        },
                        "普管"
                    }
                    button {
                        style: if target_op == "op4" { "width: 100%; text-align: left; padding: 6px 12px; border-radius: 6px; font-size: 13px; background: rgba(128,128,128,0.15);" } else { "width: 100%; text-align: left; padding: 6px 12px; border-radius: 6px; font-size: 13px;" },
                        onclick: {
                            let t = target_id.clone();
                            move |_| {
                                let t = t.clone();
                                spawn(async move { set_op(app, ui, &t, "op4").await; });
                            }
                        },
                        "超管"
                    }
                }

                if is_admin && !is_self {
                    button {
                        style: "width: 100%; text-align: left; padding: 8px 12px; border-radius: 6px; color: var(--negative); font-size: 14px;",
                        onclick: {
                            let target_id_kick = target_id.clone();
                            move |_| {
                                let target_id_kick = target_id_kick.clone();
                                spawn(async move {
                                    let Some(room) = app.room.read().clone() else { return };
                                    match room.remove_member(&target_id_kick).await {
                                        Ok(_) => {
                                            ui.member_menu.set(None);
                                            ui.toast.set(Some(("已踢出".into(), false)));
                                        }
                                        Err(e) => {
                                            ui.toast.set(Some((format!("踢出失败: {}", e), true)));
                                        }
                                    }
                                });
                            }
                        },
                        "踢出服务器"
                    }
                }
            }
        }
    }
}

async fn set_op(app: AppState, mut ui: UiState, target_id: &str, op: &'static str) {
    let Some(room) = app.room.read().clone() else {
        return;
    };
    let patch = kaede_client::http::UpdateMemberReq {
        op: Some(op.to_string()),
        nickname: None,
        bio: None,
        title: None,
        avatar_url: None,
    };
    match room.update_member(target_id, patch).await {
        Ok(_) => {
            ui.member_menu.set(None);
            ui.toast.set(Some(("权限已更新".into(), false)));
        }
        Err(e) => {
            ui.toast.set(Some((format!("更新失败: {}", e), true)));
        }
    }
}
