//! 设置弹窗（含语言切换）。

use crate::i18n::use_i18n;
use dioxus::prelude::*;

use crate::icons::IconX;
use crate::state::{AppState, UiState};

#[component]
pub fn SettingsModal() -> Element {
    let mut ui = use_context::<UiState>();
    let app = use_context::<AppState>();
    let voice = use_context::<crate::state::VoiceState>();
    let mut i18n = use_i18n();
    let mut sessions = use_signal(Vec::<kaede_client::http::SessionInfo>::new);
    let mut sessions_loading = use_signal(|| false);
    let mut sessions_error = use_signal(|| Option::<String>::None);

    // 打开设置时拉取登录设备（放在 early return 之前，保证 hook 顺序稳定）。
    use_effect(move || {
        if !*ui.settings_open.read() {
            return;
        }
        let room_opt = app.room.read().clone();
        sessions_loading.set(true);
        sessions_error.set(None);
        spawn(async move {
            let Some(room) = room_opt else {
                sessions_loading.set(false);
                return;
            };
            match room.list_sessions().await {
                Ok(list) => sessions.set(list),
                Err(e) => sessions_error.set(Some(e.to_string())),
            }
            sessions_loading.set(false);
        });
    });

    if !*ui.settings_open.read() {
        return rsx! {};
    }

    let current = i18n.locale();
    let is_zh = current == "zh-CN";
    let is_en = current == "en";

    rsx! {
        div {
            class: "server-modal-backdrop",
            onclick: move |_| ui.settings_open.set(false),
            div {
                class: "server-modal settings-modal",
                onclick: move |e| e.stop_propagation(),

                button {
                    class: "server-modal-close",
                    onclick: move |_| ui.settings_open.set(false),
                    IconX { size: 18 }
                }

                div {
                    class: "server-modal-content",
                    div { class: "server-modal-title", "{i18n.t(\"settings.title\")}" }

                    div { class: "form-field",
                        label { class: "form-label", "{i18n.t(\"settings.language\")}" }
                        div { class: "form-radio-row",
                            div {
                                class: if is_zh { "form-radio active" } else { "form-radio" },
                                onclick: move |_| i18n.set_locale("zh-CN"),
                                span { "{i18n.t(\"settings.languageZh\")}" }
                            }
                            div {
                                class: if is_en { "form-radio active" } else { "form-radio" },
                                onclick: move |_| i18n.set_locale("en"),
                                span { "{i18n.t(\"settings.languageEn\")}" }
                            }
                        }
                    }

                    div { class: "form-field",
                        label { class: "form-label", "状态" }
                        {
                            let me = ui.common_account.read().login_id.clone();
                            let current = if me.is_empty() {
                                "online".to_string()
                            } else {
                                app.presence.read().get(&me).cloned().unwrap_or_else(|| "online".to_string())
                            };
                            let is_online = current == "online";
                            let is_idle = current == "idle";
                            let is_dnd = current == "dnd";
                            rsx! {
                                div { class: "form-radio-row",
                                    div {
                                        class: if is_online { "form-radio active" } else { "form-radio" },
                                        onclick: move |_| set_my_presence(ui, app, "online"),
                                        span { "在线" }
                                    }
                                    div {
                                        class: if is_idle { "form-radio active" } else { "form-radio" },
                                        onclick: move |_| set_my_presence(ui, app, "idle"),
                                        span { "空闲" }
                                    }
                                    div {
                                        class: if is_dnd { "form-radio active" } else { "form-radio" },
                                        onclick: move |_| set_my_presence(ui, app, "dnd"),
                                        span { "请勿打扰" }
                                    }
                                }
                            }
                        }
                    }
                    div { class: "form-field",
                        label { class: "form-label", "登录设备" }
                        div { style: "font-size: 12px; color: var(--fg-faint); margin-bottom: 8px;",
                            "当前架构为单设备在线：新登录会顶替旧连接。撤销后该设备的 token 立即失效。"
                        }
                        if *sessions_loading.read() {
                            div { style: "font-size: 13px; color: var(--fg-faint);", "加载中…" }
                        } else if let Some(e) = sessions_error.read().clone() {
                            div { style: "font-size: 13px; color: var(--negative);", "{e}" }
                        } else {
                            div { style: "display: flex; flex-direction: column; gap: 4px;",
                                for s in sessions.read().iter() {
                                    {
                                        let sid = s.id.clone();
                                        let device = if s.device_name.is_empty() {
                                            "未知设备".to_string()
                                        } else {
                                            s.device_name.clone()
                                        };
                                        let last = format!(
                                            "{} {}",
                                            crate::observer::day_label(s.last_seen_at),
                                            crate::observer::format_hhmm(s.last_seen_at)
                                        );
                                        let is_current = s.current;
                                        rsx! {
                                            div {
                                                key: "{s.id}",
                                                style: "display: flex; align-items: center; gap: 8px; padding: 8px 10px; border-radius: 6px; background: rgba(0,0,0,0.03);",
                                                div {
                                                    style: "flex: 1; min-width: 0;",
                                                    div {
                                                        style: "font-size: 13px; color: var(--fg); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;",
                                                        "{device}"
                                                    }
                                                    div {
                                                        style: "font-size: 11px; color: var(--fg-faint); margin-top: 2px;",
                                                        "最近活跃 {last}"
                                                    }
                                                }
                                                if is_current {
                                                    span {
                                                        style: "font-size: 11px; color: var(--fg-muted); padding: 2px 6px; border-radius: 4px; background: var(--active-bg);",
                                                        "当前设备"
                                                    }
                                                } else {
                                                    button {
                                                        class: "btn-secondary",
                                                        style: "padding: 4px 10px; font-size: 12px;",
                                                        onclick: move |_| {
                                                            let sid = sid.clone();
                                                            let mut sessions = sessions;
                                                            let mut ui2 = ui;
                                                            spawn(async move {
                                                                let Some(room) = app.room.read().clone() else { return };
                                                                match room.revoke_session(&sid).await {
                                                                    Ok(()) => {
                                                                        sessions.write().retain(|x| x.id != sid);
                                                                        ui2.toast.set(Some(("已撤销设备".into(), false)));
                                                                    }
                                                                    Err(e) => {
                                                                        ui2.toast.set(Some((format!("撤销失败: {}", e), true)));
                                                                    }
                                                                }
                                                            });
                                                        },
                                                        "撤销"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            button {
                                class: "btn-secondary",
                                style: "margin-top: 8px;",
                                onclick: move |_| {
                                    let mut sessions = sessions;
                                    let mut ui2 = ui;
                                    spawn(async move {
                                        let Some(room) = app.room.read().clone() else { return };
                                        match room.revoke_other_sessions().await {
                                            Ok(()) => {
                                                sessions.write().retain(|x| x.current);
                                                ui2.toast.set(Some(("已撤销其他设备".into(), false)));
                                            }
                                            Err(e) => {
                                                ui2.toast.set(Some((format!("撤销失败: {}", e), true)));
                                            }
                                        }
                                    });
                                },
                                "撤销其他所有设备"
                            }
                        }
                    }
                    div { class: "form-field",
                        label { class: "form-label", "账号" }
                        div {
                            style: "font-size: 12px; color: var(--fg-faint); margin-bottom: 8px;",
                            "退出后会撤销当前会话并断开实时连接；本地加密身份与已解密历史按账号保留。"
                        }
                        button {
                            class: "btn-secondary",
                            style: "color: var(--negative);",
                            onclick: move |_| {
                                crate::observer::logout_and_clear(app, ui, voice);
                            },
                            "退出登录"
                        }
                    }
                    div { class: "form-actions",
                        div { style: "flex:1;" }
                        button {
                            class: "btn-primary",
                            onclick: move |_| ui.settings_open.set(false),
                            "{i18n.t(\"settings.close\")}"
                        }
                    }
                }
            }
        }
    }
}
fn set_my_presence(ui: UiState, mut app: AppState, status: &'static str) {
    let me = ui.common_account.read().login_id.clone();
    if !me.is_empty() {
        app.presence.write().insert(me, status.to_string());
    }
    if let Some(room) = app.room.read().clone() {
        spawn(async move {
            if let Err(e) = room.set_presence(status).await {
                tracing::warn!("set_presence: {}", e);
            }
        });
    }
}
