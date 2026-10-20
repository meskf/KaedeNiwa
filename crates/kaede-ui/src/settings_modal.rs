//! 设置弹窗（含语言切换）。

use crate::i18n::use_i18n;
use dioxus::prelude::*;

use crate::icons::IconX;
use crate::state::{AppState, UiState};

#[component]
pub fn SettingsModal() -> Element {
    let mut ui = use_context::<UiState>();
    let app = use_context::<AppState>();
    let mut i18n = use_i18n();

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
