//! 应用根组件。

use dioxus::prelude::*;
use std::sync::Arc;

use crate::content::EmojiPicker;
use crate::css;
use crate::i18n::I18n;
use crate::login::LoginView;
use crate::member_picker::MemberPickerModal;
use crate::members::MemberContextMenu;
use crate::observer::{UiObserver, apply_event};
use crate::server_modal::ServerModal;
use crate::settings_modal::SettingsModal;
use crate::mobile_shell::MobileShell;
use crate::shell::{MainBody, TitleBar};
use crate::sidebar::{ChannelContextMenu, CreateChannelModal, JoinChannelModal};
use crate::state::{AppState, UiState, View, VoiceControls, VoiceState, LayoutMode};
use crate::video_picker::VideoProfilePicker;

#[derive(Clone)]
pub struct ObserverRef(pub Arc<UiObserver>);

#[component]
pub fn App() -> Element {
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
            if let Some(w) = web_sys::window() {
                if let Ok(inner) = w.inner_width() {
                    if let Some(w) = inner.as_f64() {
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
                if layout == LayoutMode::Mobile {
                    MobileShell {}
                } else {
                    MainBody {}
                }
            }
            ServerModal {}
            CreateChannelModal {}
            JoinChannelModal {}
            ChannelContextMenu {}
            EmojiPicker {}
            ToastContainer {}
            SettingsModal {}
            VideoProfilePicker {}
            MemberPickerModal {}
            MemberContextMenu {}
        }
    }
}

#[component]
fn ToastContainer() -> Element {
    let mut ui = use_context::<UiState>();

    use_effect(move || {
        if ui.toast.read().is_some() {
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
                ui.toast.set(None);
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
