//! 服务器管理弹窗。

use dioxus::prelude::*;

use crate::icons::{IconCheck, IconPlus, IconTrash, IconX};
use crate::state::{AccountInfo, AccountMode, AppState, UiServer, UiState};

#[component]
pub fn ServerModal() -> Element {
    let mut ui = use_context::<UiState>();
    let mut app = use_context::<AppState>();

    if !*ui.server_modal_open.read() {
        return rsx! {};
    }

    let mut selected = ui.server_modal_selected;
    let servers = use_memo(move || {
        let s = app.servers.read();
        let mut v: Vec<_> = s.values().cloned().collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    });

    rsx! {
        div {
            class: "server-modal-backdrop",
            onclick: move |_| ui.server_modal_open.set(false),
            div {
                class: "server-modal",
                onclick: move |e| e.stop_propagation(),

                button {
                    class: "server-modal-close",
                    onclick: move |_| ui.server_modal_open.set(false),
                    IconX { size: 18 }
                }

                div {
                    class: "server-modal-nav",
                    for srv in servers.read().iter() {
                        {
                            let sid = srv.id.clone();
                            let is_active = selected.read().as_deref() == Some(sid.as_str());
                            let label = if srv.name.is_empty() {
                                srv.id.clone()
                            } else {
                                srv.name.clone()
                            };
                            let sid_click = sid.clone();
                            rsx! {
                                button {
                                    key: "{sid}",
                                    class: if is_active { "active" } else { "" },
                                    onclick: move |_| selected.set(Some(sid_click.clone())),
                                    "{label}"
                                }
                            }
                        }
                    }
                    button {
                        class: "server-modal-add",
                        onclick: move |_| {
                            let n = app.servers.read().len() + 1;
                            let id = format!("srv-{}", n);
                            let new = UiServer {
                                id: id.clone(),
                                name: format!("服务器 {}", n),
                                http_url: "http://localhost:8080".into(),
                                ws_url: "ws://localhost:8080".into(),
                                ..Default::default()
                            };
                            app.servers.write().insert(id.clone(), new);
                            selected.set(Some(id));
                        },
                        IconPlus { size: 12 }
                        span { "添加服务器" }
                    }
                }

                div {
                    class: "server-modal-content",
                    match selected.read().clone() {
                        Some(id) if servers.read().iter().any(|s| s.id == id) => {
                            rsx! { ServerForm { server_id: id } }
                        }
                        _ => rsx! { EmptyForm {} },
                    }
                }
            }
        }
    }
}

#[component]
fn EmptyForm() -> Element {
    rsx! {
        div {
            div { class: "server-modal-title", "服务器设置" }
            div { class: "form-hint", "还没有服务器。点左下角\u{201C}添加服务器\u{201D}新建一个。" }
        }
    }
}

#[component]
fn ServerForm(server_id: String) -> Element {
    let mut ui = use_context::<UiState>();
    let mut app = use_context::<AppState>();

    let mut name = use_signal(String::new);
    let mut http = use_signal(String::new);
    let mut ws = use_signal(String::new);
    let mut mode = use_signal(|| AccountMode::Common);
    let mut login = use_signal(String::new);
    let mut password = use_signal(String::new);

    use_effect({
        let server_id = server_id.clone();
        move || {
            let _s = ui.server_modal_selected.read();
            let srvs = app.servers.peek();
            if let Some(srv) = srvs.get(&server_id) {
                name.set(srv.name.clone());
                http.set(srv.http_url.clone());
                ws.set(srv.ws_url.clone());
                mode.set(srv.account_mode);
                login.set(srv.per_server_account.login_id.clone());
                password.set(srv.per_server_account.password.clone());
            }
        }
    });

    let sid_save = server_id.clone();
    let save = move |_| {
        {
            let mut srvs = app.servers.write();
            if let Some(srv) = srvs.get_mut(&sid_save) {
                srv.name = name.read().clone();
                srv.http_url = http.read().clone();
                srv.ws_url = ws.read().clone();
                srv.account_mode = *mode.read();
                srv.per_server_account = AccountInfo {
                    login_id: login.read().clone(),
                    password: password.read().clone(),
                };
            }
        }
        ui.server_modal_open.set(false);
    };

    let sid_del = server_id.clone();
    let remove = move |_| {
        app.servers.write().remove(&sid_del);
        let srvs = app.servers.read();
        let mut ids: Vec<_> = srvs.keys().cloned().collect();
        ids.sort();
        ui.server_modal_selected.set(ids.into_iter().next());
    };

    let current_mode = *mode.read();
    let common_login = ui.common_account.read().login_id.clone();

    rsx! {
        div {
            div { class: "server-modal-title", "服务器设置" }

            div { class: "form-field",
                label { class: "form-label", "显示名" }
                input {
                    class: "form-input",
                    value: "{name}",
                    oninput: move |e| name.set(e.value()),
                }
            }
            div { class: "form-field",
                label { class: "form-label", "HTTP URL" }
                input {
                    class: "form-input",
                    placeholder: "http://localhost:8080",
                    value: "{http}",
                    oninput: move |e| http.set(e.value()),
                }
            }
            div { class: "form-field",
                label { class: "form-label", "WebSocket URL" }
                input {
                    class: "form-input",
                    placeholder: "ws://localhost:8080",
                    value: "{ws}",
                    oninput: move |e| ws.set(e.value()),
                }
            }
            div { class: "form-field",
                label { class: "form-label", "账户" }
                div { class: "form-radio-row",
                    div {
                        class: if current_mode == AccountMode::Common { "form-radio active" } else { "form-radio" },
                        onclick: move |_| mode.set(AccountMode::Common),
                        if current_mode == AccountMode::Common { IconCheck { size: 14 } }
                        span { "使用通用账户" }
                    }
                    div {
                        class: if current_mode == AccountMode::PerServer { "form-radio active" } else { "form-radio" },
                        onclick: move |_| mode.set(AccountMode::PerServer),
                        if current_mode == AccountMode::PerServer { IconCheck { size: 14 } }
                        span { "单独账户" }
                    }
                }
            }
            if current_mode == AccountMode::Common {
                div { class: "form-hint", "当前使用通用账户：{common_login}" }
            } else {
                div { class: "form-field",
                    label { class: "form-label", "登录名" }
                    input {
                        class: "form-input",
                        value: "{login}",
                        oninput: move |e| login.set(e.value()),
                    }
                }
                div { class: "form-field",
                    label { class: "form-label", "密码" }
                    input {
                        class: "form-input",
                        r#type: "password",
                        value: "{password}",
                        oninput: move |e| password.set(e.value()),
                    }
                }
            }

            div { class: "form-actions",
                button {
                    class: "btn-danger",
                    onclick: remove,
                    IconTrash { size: 14 }
                    span { "删除" }
                }
                div { style: "flex:1;" }
                button {
                    class: "btn-secondary",
                    onclick: move |_| ui.server_modal_open.set(false),
                    "取消"
                }
                button {
                    class: "btn-primary",
                    onclick: save,
                    "保存"
                }
            }
        }
    }
}
