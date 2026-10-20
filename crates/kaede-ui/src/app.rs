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
use crate::shell::{MainBody, TitleBar};
use crate::sidebar::{ChannelContextMenu, CreateChannelModal, JoinChannelModal};
use crate::state::{AppState, UiState, View, VoiceControls, VoiceState};
use crate::video_picker::VideoProfilePicker;

#[derive(Clone)]
pub struct ObserverRef(pub Arc<UiObserver>);

#[component]
pub fn App() -> Element {
    let ui = use_context_provider(UiState::new);
    let app_state = use_context_provider(AppState::new);
    let _voice_state = use_context_provider(VoiceState::new);
    let _voice_controls = use_context_provider(VoiceControls::new);
    let _i18n = use_context_provider(I18n::new);

    let (observer, rx) = use_hook(|| {
        let (obs, rx) = UiObserver::new();
        (Arc::new(obs), rx)
    });
    use_context_provider(|| ObserverRef(observer.clone()));

    use_future(move || {
        let rx = rx.clone();
        async move {
            while let Ok(event) = rx.recv().await {
                apply_event(app_state, ui, event);
            }
        }
    });

    let mode = *ui.theme_mode.read();
    let view = *ui.view.read();
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
                MainBody {}
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
                #[cfg(target_arch = "wasm32")]
                {
                    let _ = dioxus::document::eval(
                        "setTimeout(() => window.dispatchEvent(new Event('dummy')), 3000);",
                    );
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
