//! 登录页。

use crate::app::ObserverRef;
use crate::i18n::use_i18n;
use crate::observer::history_to_ui;
use crate::state::{AccountInfo, AccountMode, AppState, UiMessage, UiServer, UiState, View};
use dioxus::prelude::*;
use kaede_client::RoomSession;
use std::sync::Arc;

#[derive(Clone)]
enum LoginMsg {
    Connect {
        http: String,
        ws: String,
        user: String,
        pass: String,
    },
}

#[component]
pub fn LoginView() -> Element {
    let mut ui = use_context::<UiState>();
    let mut app = use_context::<AppState>();
    let obs = use_context::<ObserverRef>();
    let i18n = use_i18n();

    let mut username = use_signal(String::new);
    let mut password = use_signal(String::new);
    let mut server_url = use_signal(|| "http://localhost:8080".to_string());
    let mut error = use_signal(|| Option::<String>::None);
    let mut connecting = use_signal(|| false);

    let (tx, rx) = use_hook(async_channel::unbounded::<LoginMsg>);

    use_future(move || {
        let rx = rx.clone();
        let observer: Arc<dyn kaede_client::ClientObserver> = obs.0.clone();
        async move {
            while let Ok(msg) = rx.recv().await {
                match msg {
                    LoginMsg::Connect {
                        http,
                        ws,
                        user,
                        pass,
                    } => {
                        connecting.set(true);
                        error.set(None);
                        match connect_and_load(&http, &ws, &user, &pass, observer.clone()).await {
                            Ok(loaded) => {
                                let LoadedData {
                                    server_id,
                                    room,
                                    channels,
                                    members,
                                    messages,
                                    is_host,
                                } = loaded;
                                app.is_host.set(Some(is_host));
                                let epoch = room.epoch();
                                let srv = UiServer {
                                    id: server_id.clone(),
                                    name: format!("{} 的服务器", user),
                                    http_url: http.clone(),
                                    ws_url: ws.clone(),
                                    state: kaede_client::ClientState::MlsReady,
                                    epoch,
                                    account_mode: AccountMode::Common,
                                    per_server_account: AccountInfo::default(),
                                };
                                app.servers.write().insert(server_id.clone(), srv);
                                app.current_server.set(Some(server_id.clone()));
                                app.channels.write().insert(server_id.clone(), channels);
                                app.members.write().insert(server_id.clone(), members);
                                for (ch, msgs) in messages {
                                    let key = AppState::channel_key(&server_id, &ch);
                                    app.messages.write().insert(key, msgs);
                                }
                                app.room.set(Some(room));
                                ui.common_account.set(AccountInfo {
                                    login_id: user.clone(),
                                    password: pass.clone(),
                                });
                                let first = {
                                    let chans = app.channels.read();
                                    chans.get(&server_id).and_then(|list| {
                                        list.iter()
                                            .find(|c| c.kind != "voice")
                                            .map(|c| c.id.clone())
                                    })
                                };
                                ui.selected_channel.set(first);
                                ui.view.set(View::Main);
                            }
                            Err(e) => {
                                error.set(Some(e));
                            }
                        }
                        connecting.set(false);
                    }
                }
            }
        }
    });

    let submit = move |_| {
        if *connecting.read() {
            return;
        }
        let u = username.read().trim().to_string();
        let p = password.read().clone();
        if u.is_empty() || p.is_empty() {
            error.set(Some(i18n.t("login.error")));
            return;
        }
        let server_input = server_url.read().clone();
        let endpoint = match kaede_client::parse_server_url(&server_input) {
            Ok(e) => e,
            Err(e) => {
                error.set(Some(format!("{}: {}", i18n.t("login.serverError"), e)));
                return;
            }
        };
        let _ = tx.try_send(LoginMsg::Connect {
            http: endpoint.http_url,
            ws: endpoint.ws_url,
            user: u,
            pass: p,
        });
    };

    let is_connecting = *connecting.read();
    let btn_label = if is_connecting {
        format!("{}...", i18n.t("login.submit"))
    } else {
        i18n.t("login.submit")
    };

    rsx! {
        div {
            class: "login-wrap",
            div {
                class: "login-card",
                div { class: "login-title", "KaedeNiwa" }
                input {
                    class: "login-input",
                    placeholder: "{i18n.t(\"login.serverHint\")}",
                    value: "{server_url}",
                    disabled: is_connecting,
                    oninput: move |e| server_url.set(e.value()),
                }
                input {
                    class: "login-input",
                    placeholder: "{i18n.t(\"login.username\")}",
                    value: "{username}",
                    disabled: is_connecting,
                    oninput: move |e| username.set(e.value()),
                }
                input {
                    class: "login-input",
                    r#type: "password",
                    placeholder: "{i18n.t(\"login.password\")}",
                    value: "{password}",
                    disabled: is_connecting,
                    oninput: move |e| password.set(e.value()),
                }
                if let Some(e) = error() {
                    div { class: "login-error", "{e}" }
                }
                button {
                    class: "login-btn",
                    disabled: is_connecting,
                    onclick: submit,
                    "{btn_label}"
                }
            }
        }
    }
}

struct LoadedData {
    server_id: String,
    room: Arc<RoomSession>,
    channels: Vec<kaede_protocol::Channel>,
    members: Vec<kaede_protocol::Member>,
    messages: Vec<(String, Vec<UiMessage>)>,
    /// P0-1 完整版：当前用户是否是 host（op5）。
    is_host: bool,
}

async fn connect_and_load(
    http: &str,
    ws: &str,
    user: &str,
    pass: &str,
    observer: Arc<dyn kaede_client::ClientObserver>,
) -> Result<LoadedData, String> {
    #[cfg(target_arch = "wasm32")]
    let _ = kaede_client::keystore_web::init_web_keystore().await;

    let server_id = format!("{}::{}", http, ws);

    // P0-1: 接 storage 以持久化 MLS 快照（避免每次登录丢身份）
    let store = kaede_client::open_default_store("kaedeniwa.db")
        .await
        .map_err(|e| format!("open_default_store: {}", e))?;

    let room = RoomSession::connect_with_storage(http, ws, user, pass, Some(store), Some(observer))
        .await
        .map_err(|e| format!("connect: {}", e))?;
    room.spawn_event_loop();

    // P0-1 完整版：先拉成员表判断当前用户 op（server 第一个注册 = op5 = host）
    let members = room
        .list_members()
        .await
        .map_err(|e| format!("list_members: {}", e))?;
    let my_op = members
        .iter()
        .find(|m| m.login_id == user)
        .map(|m| m.op.clone())
        .unwrap_or_else(|| "op2".into());
    let is_host = my_op == "op5";
    let already_ready = room.is_mls_ready();

    if !already_ready {
        if is_host {
            room.become_host()
                .await
                .map_err(|e| format!("become_host: {}", e))?;
            tracing::info!("[login] created fresh MLS group (host op={})", my_op);
        } else {
            room.become_guest()
                .await
                .map_err(|e| format!("become_guest: {}", e))?;
            tracing::info!(
                "[login] published KP as guest (op={}), waiting for host Add",
                my_op
            );
        }
    } else {
        tracing::info!(
            "[login] restored MLS group from snapshot (is_host={})",
            is_host
        );
    }

    let _ = room.set_presence("online").await;
    room.join_room()
        .await
        .map_err(|e| format!("join_room: {}", e))?;

    let channels = room
        .list_channels()
        .await
        .map_err(|e| format!("list_channels: {}", e))?;
    // members 已在上面拉过

    let mut messages = Vec::new();
    for ch in &channels {
        if ch.kind == "voice" {
            continue;
        }
        if let Ok(history) = room.load_history(&ch.id, 50).await {
            let ui_msgs: Vec<UiMessage> = history
                .into_iter()
                .map(|h| history_to_ui(h, &ch.id, &server_id))
                .collect();
            messages.push((ch.id.clone(), ui_msgs));
        }
    }

    Ok(LoadedData {
        server_id,
        room: Arc::new(room),
        channels,
        members,
        messages,
        is_host,
    })
}
