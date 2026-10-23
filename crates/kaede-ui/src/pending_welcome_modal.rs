//! Round 7：主群 Welcome 确认对话框。
//!
//! 收到 `ClientEvent::PendingWelcome` 时由 `observer.rs` 推入
//! `UiState::pending_welcomes`，本组件从队列头弹出并等待用户操作。

use dioxus::prelude::*;

use crate::state::{AppState, PendingWelcomeUi, UiState};

#[component]
pub fn PendingWelcomeModal() -> Element {
    let ui = use_context::<UiState>();
    let app = use_context::<AppState>();

    let current: Option<PendingWelcomeUi> = ui.pending_welcomes.read().first().cloned();
    let Some(pending) = current else {
        return rsx! {};
    };

    let sender_label = pending.sender_login_id.clone();
    let server_label = app
        .servers
        .read()
        .get(&pending.server_id)
        .map(|s| {
            if s.name.is_empty() {
                s.id.clone()
            } else {
                s.name.clone()
            }
        })
        .unwrap_or_else(|| pending.server_id.clone());
    let is_rebuild = pending.is_rebuild;

    let p_accept = pending.clone();
    let app_accept = app;
    let mut ui_accept = ui;
    let on_accept = move |_| {
        let p = p_accept.clone();
        // 条目必须等客户端真正处理成功、由 `PendingWelcomeResolved` 事件移除。
        // 这里不能先出队，否则断线/切服务器时会静默丢掉用户的“接受”。
        if app_accept.current_server.read().as_deref() != Some(p.server_id.as_str()) {
            ui_accept
                .toast
                .set(Some(("请先连接到该服务器再处理邀请".into(), true)));
            return;
        }
        let Some(room) = app_accept.room.read().as_ref().cloned() else {
            ui_accept
                .toast
                .set(Some(("连接已断开，无法处理邀请；请重新登录".into(), true)));
            return;
        };
        let sid = p.sender_login_id.clone();
        spawn(async move {
            match room.connection().accept_pending_welcome(&sid) {
                Ok(()) => {
                    // 主群 epoch 已前进，必须立刻落盘；否则关闭应用会回滚
                    // 到旧群状态，导致与服务器永久分叉。
                    if let Err(e) = room.save_snapshot().await {
                        tracing::warn!("save_snapshot after accepting welcome failed: {}", e);
                    }
                }
                Err(e) => tracing::warn!("accept_pending_welcome({}) failed: {}", sid, e),
            }
        });
    };

    let p_reject = pending.clone();
    let app_reject = app;
    let mut ui_reject = ui;
    let on_reject = move |_| {
        let p = p_reject.clone();
        let Some(room) = app_reject.room.read().as_ref().cloned() else {
            // 断线时仍允许在本地丢弃，避免弹窗永远卡住。
            ui_reject.pending_welcomes.write().retain(|x| {
                !(x.server_id == p.server_id && x.sender_login_id == p.sender_login_id)
            });
            return;
        };
        room.connection().reject_pending_welcome(&p.sender_login_id);
    };

    rsx! {
        div {
            class: "server-modal-backdrop",
            div {
                class: "server-modal",
                onclick: move |e| e.stop_propagation(),

                div { class: "server-modal-title", "加入服务器邀请" }
                div { style: "padding: 16px 24px;",
                    div {
                        strong { "{sender_label}" }
                        " 想把你加入服务器 "
                        strong { "{server_label}" }
                        "。"
                    }
                    if is_rebuild {
                        div {
                            style: "margin-top: 12px; padding: 8px 12px; \
                                    background: rgba(220, 53, 69, 0.15); \
                                    border-left: 3px solid #dc3545; \
                                    border-radius: 4px; \
                                    color: #dc3545; font-size: 13px;",
                            "⚠️ 接受会重建本地主群，旧群历史消息将不可恢复。"
                        }
                    }
                    div {
                        style: "margin-top: 12px; color: var(--fg-faint); font-size: 12px;",
                        "只接受你认识的人发来的邀请。接受后，对方会成为主群成员，\
                         能解密你在此服务器公开频道的消息。"
                    }
                }
                div { class: "form-actions",
                    button {
                        class: "btn-secondary",
                        onclick: on_reject,
                        "拒绝"
                    }
                    div { style: "flex:1;" }
                    button {
                        class: "btn-primary",
                        onclick: on_accept,
                        "接受"
                    }
                }
            }
        }
    }
}
