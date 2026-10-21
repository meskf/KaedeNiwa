//! 应用根组件。

use dioxus::prelude::*;
use std::sync::Arc;

use crate::content::EmojiPicker;
use crate::css;
use crate::i18n::I18n;
use crate::login::LoginView;
use crate::member_picker::MemberPickerModal;
use crate::members::MemberContextMenu;
use crate::mobile_shell::MobileShell;
use crate::observer::{UiObserver, apply_event};
use crate::search::SearchModal;
use crate::server_modal::ServerModal;
use crate::settings_modal::SettingsModal;
use crate::shell::{MainBody, TitleBar};
use crate::sidebar::{
    ChannelContextMenu, ChannelMembersModal, CreateChannelModal, JoinChannelModal,
};
use crate::state::{AppState, LayoutMode, UiState, View, VoiceControls, VoiceState};
use crate::video_picker::VideoProfilePicker;

#[derive(Clone)]
pub struct ObserverRef(pub Arc<UiObserver>);

#[component]
pub fn App() -> Element {
    #[allow(unused_mut)] // wasm 下 use_effect 内 set 需要 mut，native 不需要
    let mut ui = use_context_provider(UiState::new);
    let app_state = use_context_provider(AppState::new);
    let voice_state = use_context_provider(VoiceState::new);
    let _voice_controls = use_context_provider(VoiceControls::new);
    let _i18n = use_context_provider(I18n::new);

    let (observer, rx) = use_hook(|| {
        let (obs, rx) = UiObserver::new();
        (Arc::new(obs), rx)
    });
    use_context_provider(|| ObserverRef(observer.clone()));

    let observer_for_events = observer.clone();
    use_future(move || {
        let rx = rx.clone();
        let observer = observer_for_events.clone();
        async move {
            while let Ok(event) = rx.recv().await {
                // M55 扩展：把 VoiceState 也传进去，以便处理 Disconnected 清理
                apply_event(app_state, ui, voice_state, observer.clone(), event);
            }
        }
    });

    let mode = *ui.theme_mode.read();
    let view = *ui.view.read();
    let layout = *ui.layout_mode.read();

    // wasm 下根据窗口宽度自动切换 LayoutMode（M1 简化：只在挂载时检测一次）
    #[cfg(target_arch = "wasm32")]
    {
        use_effect(move || {
            if let Some(w) = web_sys::window()
                && let Ok(inner) = w.inner_width()
                && let Some(w) = inner.as_f64()
            {
                let new_mode = if w < 768.0 {
                    LayoutMode::Mobile
                } else if w < 1200.0 {
                    LayoutMode::Tablet
                } else {
                    LayoutMode::Desktop
                };
                if new_mode != layout {
                    ui.layout_mode.set(new_mode);
                }
            }
        });
    }
    let css_text = use_memo(css::global);

    rsx! {
        style { dangerous_inner_html: "{css_text}" }
        div {
            "data-theme": mode.as_attr(),
            style: "
                height: 100%; width: 100%;
                display: flex; flex-direction: column;
                overflow: hidden;
                background: var(--outer-bg);
                color: var(--fg);
            ",
            TitleBar {}
            div {
                style: if view == View::Login { "flex: 1; min-height: 0; display: flex; flex-direction: column;" } else { "display: none;" },
                LoginView {}
            }
            if view == View::Main {
                match layout {
                    LayoutMode::Mobile => rsx! { MobileShell {} },
                    LayoutMode::Tablet => rsx! { MainBody { compact: true } },
                    LayoutMode::Desktop => rsx! { MainBody { compact: false } },
                }
            }
            ServerModal {}
            CreateChannelModal {}
            JoinChannelModal {}
            ChannelContextMenu {}
            ChannelMembersModal {}
            SearchModal {}
            EmojiPicker {}
            UploadProgressBar {}
            ToastContainer {}
            SettingsModal {}
            VideoProfilePicker {}
            MemberPickerModal {}
            MemberContextMenu {}
        }
    }
}

#[component]
fn UploadProgressBar() -> Element {
    let ui = use_context::<UiState>();
    let Some((name, pct)) = ui.upload_progress.read().clone() else {
        return rsx! {};
    };

    rsx! {
        div {
            style: "position: fixed; left: 16px; bottom: 16px; z-index: 120; min-width: 260px; max-width: 360px; padding: 10px 12px; border-radius: 8px; background: var(--inner-bg); box-shadow: 0 8px 24px rgba(0,0,0,0.25);",
            div {
                style: "display: flex; gap: 8px; align-items: center; font-size: 12px; color: var(--fg);",
                span {
                    style: "flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;",
                    "上传附件：{name}"
                }
                span { style: "color: var(--fg-muted); font-variant-numeric: tabular-nums;", "{pct}%" }
            }
            div {
                style: "margin-top: 6px; height: 4px; border-radius: 2px; background: rgba(128,128,128,0.2); overflow: hidden;",
                div {
                    style: "height: 100%; width: {pct}%; background: var(--active-bg); transition: width 0.15s linear;",
                }
            }
        }
    }
}

#[component]
fn ToastContainer() -> Element {
    let mut ui = use_context::<UiState>();
    // 每条 toast 一个自增 token；旧定时器不能清掉新提示。
    let mut toast_seq = use_signal(|| 0u64);

    use_effect(move || {
        if ui.toast.read().is_some() {
            let my_seq = *toast_seq.read() + 1;
            toast_seq.set(my_seq);
            spawn(async move {
                // M52: wasm 上 document::eval 是同步返回，必须 await 返回的 Promise
                // 才能真等 3s。之前注入 setTimeout 但立即返回，toast 一闪而过。
                #[cfg(target_arch = "wasm32")]
                {
                    let _ = dioxus::document::eval("return new Promise(r => setTimeout(r, 3000));")
                        .await;
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                }
                if *toast_seq.read() == my_seq {
                    ui.toast.set(None);
                }
            });
        }
    });

    let Some((msg, is_err)) = ui.toast.read().clone() else {
        return rsx! {};
    };

    let bg = if is_err {
        "var(--negative)"
    } else {
        "var(--positive)"
    };

    rsx! {
        div {
            style: "position: fixed; bottom: 24px; right: 24px; z-index: 9999; background: {bg}; color: #fff; padding: 12px 20px; border-radius: 8px; box-shadow: 0 4px 12px rgba(0,0,0,0.15); font-size: 14px;",
            "{msg}"
        }
    }
}
