//! 移动端 3 个列表页（频道 / 私聊 / 好友）。
//!
//! 数据源：AppState 三个 HashMap<server_id, Vec<T>>。
//! 点击频道 → mobile_detail=ch.id + kind=Channel，详情页渲染 ContentArea。
//! 点击 DM → mobile_detail=conv_id + kind=Dm，详情页 M3 实现。

use dioxus::prelude::*;
use kaede_client::http::{DmConversation, Friend};

use crate::state::{
    AppState, MobileDetailKind, UiState, VoiceChannel, VoiceControls, VoiceMember, VoiceState,
};

#[component]
pub fn MobileGroupsPage() -> Element {
    let app = use_context::<AppState>();
    let mut ui = use_context::<UiState>();
    let mut voice = use_context::<VoiceState>();
    #[allow(unused_variables)]
    let controls = use_context::<VoiceControls>();

    let channels: Vec<kaede_protocol::Channel> = {
        let sid = app.current_server.read().clone();
        sid.and_then(|s| app.channels.read().get(&s).cloned())
            .unwrap_or_default()
    };
    let selected = ui.selected_channel.read().clone();

    rsx! {
        div {
            style: "display: flex; flex-direction: column; gap: 4px;",
            if channels.is_empty() {
                div {
                    style: "padding: 32px 16px; text-align: center; color: var(--fg-faint); font-size: 14px;",
                    "暂无频道"
                }
            }
            for ch in channels.iter() {
                {
                    let ch_id = ch.id.clone();
                    let ch_id2 = ch.id.clone();
                    let ch_name = ch.name.clone();
                    let ch_name_closure = ch.name.clone();
                    let ch_kind = ch.kind.clone();
                    let is_active = selected.as_deref() == Some(ch.id.as_str());
                    let bg = if is_active { "var(--active-bg)" } else { "transparent" };
                    let prefix = if ch_kind == "voice" { "🔊" } else { "#" };
                    rsx! {
                        div {
                            key: "{ch_id}",
                            onclick: move |_| {
                                if ch_kind == "voice" {
                                    voice.active_channel.set(Some(VoiceChannel {
                                        id: ch_id2.clone(),
                                        name: ch_name_closure.clone(),
                                        members: vec![VoiceMember {
                                            uid: "me".into(),
                                            name: "我".into(),
                                            initial: "我".into(),
                                            speaking: false,
                                        }],
                                    }));
                                    #[cfg(target_arch = "wasm32")]
                                    {
                                        let Some(room) = app.room.read().clone() else {
                                            ui.toast.set(Some(("未连接".into(), true)));
                                            voice.active_channel.set(None);
                                            return;
                                        };
                                        let mut voice_h = voice;
                                        let mut ui_h = ui;
                                        let cid = ch_id2.clone();
                                        spawn(async move {
                                            if let Err(e) = crate::voice_web::connect_voice(
                                                voice_h, controls, room, cid,
                                            )
                                            .await
                                            {
                                                tracing::warn!("[mobile voice] connect: {}", e);
                                                ui_h.toast
                                                    .set(Some((format!("语音连接失败: {}", e), true)));
                                                voice_h.active_channel.set(None);
                                            }
                                        });
                                    }
                                    #[cfg(all(
                                        not(target_arch = "wasm32"),
                                        feature = "desktop-media"
                                    ))]
                                    {
                                        let mut ui_h = ui;
                                        let cid = ch_id2.clone();
                                        spawn(async move {
                                            if let Err(e) = crate::sidebar::connect_voice(
                                                app, voice, controls, cid,
                                            )
                                            .await
                                            {
                                                tracing::warn!("[mobile voice] connect: {}", e);
                                                ui_h.toast
                                                    .set(Some((format!("语音连接失败: {}", e), true)));
                                                voice.active_channel.set(None);
                                            }
                                        });
                                    }
                                    // 移动端原生构建不包含 GStreamer/LiveKit 原生栈：
                                    // 语音与屏幕共享请使用网页版或 web-shell 构建。
                                    #[cfg(all(
                                        not(target_arch = "wasm32"),
                                        not(feature = "desktop-media")
                                    ))]
                                    {
                                        let mut ui_h = ui;
                                        voice.active_channel.set(None);
                                        spawn(async move {
                                            ui_h.toast.set(Some((
                                                "当前移动端构建不含语音/屏幕共享，请使用网页版或 web-shell"
                                                    .into(),
                                                true,
                                            )));
                                        });
                                    }
                                    return;
                                }
                                ui.selected_channel.set(Some(ch_id2.clone()));
                                ui.mobile_detail.set(Some(ch_id2.clone()));
                                ui.mobile_detail_kind.set(Some(MobileDetailKind::Channel));
                            },
                            style: "padding: 14px 12px; border-radius: 10px; display: flex; align-items: center; gap: 10px; cursor: pointer; background: {bg}; color: var(--fg);",
                            span { style: "color: var(--fg-faint); font-size: 14px;", "{prefix}" }
                            span { style: "font-size: 15px;", "{ch_name}" }
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub fn MobileDmPage() -> Element {
    let app = use_context::<AppState>();
    let mut ui = use_context::<UiState>();

    // 移动端没有挂载桌面版 DmSidebar；这里的 effect 必须自己拉会话列表。
    use_effect(move || {
        let sid = app.current_server.read().clone();
        if sid.is_some() {
            let app_for_load = app;
            spawn(async move {
                crate::dm_view::load_conversations(app_for_load).await;
            });
        }
    });

    let convs: Vec<DmConversation> = {
        let sid = app.current_server.read().clone();
        sid.and_then(|s| app.dm_conversations.read().get(&s).cloned())
            .unwrap_or_default()
    };

    rsx! {
        div {
            style: "display: flex; flex-direction: column; gap: 4px;",
            if convs.is_empty() {
                div {
                    style: "padding: 32px 16px; text-align: center; color: var(--fg-faint); font-size: 14px;",
                    "暂无私聊"
                }
            }
            for c in convs.iter() {
                {
                    let cid = c.conv_id.clone();
                    let cid2 = c.conv_id.clone();
                    let cid3 = c.conv_id.clone();
                    let nick = c.peer_nickname.clone();
                    let initial = nick.chars().next().map(|c| c.to_string()).unwrap_or_else(|| "?".to_string());
                    let app_for_load = app;
                    rsx! {
                        div {
                            key: "{cid}",
                            onclick: move |_| {
                                ui.selected_dm_conv.set(Some(cid2.clone()));
                                ui.mobile_detail.set(Some(cid2.clone()));
                                ui.mobile_detail_kind.set(Some(MobileDetailKind::Dm));
                                let cid3 = cid3.clone();
                                spawn(async move {
                                    crate::dm_view::load_messages(app_for_load, cid3).await;
                                });
                            },
                            style: "padding: 14px 12px; border-radius: 10px; display: flex; align-items: center; gap: 10px; cursor: pointer; color: var(--fg);",
                            div {
                                style: "width: 36px; height: 36px; border-radius: 50%; background: var(--active-bg); display: flex; align-items: center; justify-content: center; font-size: 14px; flex-shrink: 0;",
                                "{initial}"
                            }
                            span { style: "font-size: 15px;", "{nick}" }
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub fn MobileFriendsPage() -> Element {
    let app = use_context::<AppState>();

    use_effect(move || {
        let sid = app.current_server.read().clone();
        if sid.is_some() {
            let app_for_load = app;
            spawn(async move {
                crate::friends_view::load_friends_into(app_for_load).await;
            });
        }
    });

    let friends: Vec<Friend> = {
        let sid = app.current_server.read().clone();
        let all = sid
            .and_then(|s| app.friends.read().get(&s).cloned())
            .unwrap_or_default();
        // 只显示已接受的好友（status == "friend" 或 "accepted"）
        all.into_iter()
            .filter(|f| f.direction.to_lowercase() == "friend")
            .collect()
    };

    rsx! {
        div {
            style: "display: flex; flex-direction: column; gap: 4px;",
            if friends.is_empty() {
                div {
                    style: "padding: 32px 16px; text-align: center; color: var(--fg-faint); font-size: 14px;",
                    "暂无好友"
                }
            }
            for f in friends.iter() {
                {
                    let lid = f.login_id.clone();
                    let nick = f.nickname.clone();
                    let initial = nick.chars().next().map(|c| c.to_string()).unwrap_or_else(|| "?".to_string());
                    rsx! {
                        div {
                            key: "{lid}",
                            style: "padding: 14px 12px; border-radius: 10px; display: flex; align-items: center; gap: 10px; color: var(--fg);",
                            div {
                                style: "width: 36px; height: 36px; border-radius: 50%; background: var(--active-bg); display: flex; align-items: center; justify-content: center; font-size: 14px; flex-shrink: 0;",
                                "{initial}"
                            }
                            div {
                                style: "display: flex; flex-direction: column; gap: 2px;",
                                span { style: "font-size: 15px;", "{nick}" }
                                span { style: "font-size: 12px; color: var(--fg-faint);", "{lid}" }
                            }
                        }
                    }
                }
            }
        }
    }
}
