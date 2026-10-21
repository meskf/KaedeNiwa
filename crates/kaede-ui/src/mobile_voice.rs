//! 移动端语音 Sheet（native 空实现 / wasm 完整实现）。
//!
//! - wasm：从底部滑出，操作按钮走 crate::voice_web
//! - native：返回空 rsx，不影响编译（MobileShell 只在 wasm 下被构造）

use dioxus::prelude::*;

#[cfg(target_arch = "wasm32")]
#[component]
pub fn MobileVoiceSheet() -> Element {
    use crate::state::{UiState, VoiceControls, VoiceState};

    let mut ui = use_context::<UiState>();
    let voice = use_context::<VoiceState>();
    let mut controls = use_context::<VoiceControls>();

    if !*ui.voice_sheet_open.read() {
        return rsx! {};
    }
    let Some(ch) = voice.active_channel.read().clone() else {
        return rsx! {};
    };

    let name = ch.name.clone();
    let count = ch.members.len();
    let mic_on = *controls.mic_on.read();
    let spk_on = *controls.speaker_on.read();
    let scr_on = *controls.screen_on.read();

    let close = move |_| ui.voice_sheet_open.set(false);

    rsx! {
        div {
            onclick: close,
            style: "position: fixed; inset: 0; background: rgba(0,0,0,0.35); z-index: 100;",
        }
        div {
            onclick: move |e| e.stop_propagation(),
            style: "
                position: fixed; left: 0; right: 0; bottom: 0;
                background: var(--inner-bg);
                border-top-left-radius: 20px; border-top-right-radius: 20px;
                padding: 16px 16px calc(16px + env(safe-area-inset-bottom));
                z-index: 101;
                box-shadow: 0 -4px 24px rgba(0,0,0,0.25);
                display: flex; flex-direction: column; gap: 16px;
            ",
            div {
                style: "width: 40px; height: 4px; border-radius: 2px; background: var(--fg-faint); margin: 0 auto; opacity: 0.4;"
            }
            div {
                style: "display: flex; align-items: center; gap: 8px;",
                span { style: "font-size: 18px;", "🔊" }
                span { style: "font-weight: 600; font-size: 16px; flex: 1;", "{name}" }
                span { style: "font-size: 12px; color: var(--fg-muted);", "{count} 人" }
            }
            div {
                style: "display: flex; gap: 12px; justify-content: space-around;",
                VoiceAction {
                    active: mic_on,
                    label: if mic_on { "开麦" } else { "关麦" },
                    icon: "🎤",
                    on_click: move |_| {
                        let new_val = !mic_on;
                        let mut c2 = controls;
                        let mut u2 = ui;
                        spawn(async move {
                            if let Err(e) = crate::voice_web::set_microphone(new_val).await {
                                u2.toast.set(Some((format!("麦克风: {}", e), true)));
                                return;
                            }
                            c2.mic_on.set(new_val);
                        });
                    },
                }
                VoiceAction {
                    active: spk_on,
                    label: if spk_on { "开扬声" } else { "关扬声" },
                    icon: "🔊",
                    on_click: move |_| {
                        let v = !*controls.speaker_on.read();
                        controls.speaker_on.set(v);
                    },
                }
                VoiceAction {
                    active: scr_on,
                    label: "屏幕",
                    icon: "🖥",
                    on_click: move |_| {
                        let new_val = !scr_on;
                        let mut c2 = controls;
                        let mut u2 = ui;
                        spawn(async move {
                            if let Err(e) = crate::voice_web::set_screen_share(new_val).await {
                                u2.toast.set(Some((format!("屏幕共享: {}", e), true)));
                                return;
                            }
                            c2.screen_on.set(new_val);
                        });
                    },
                }
                VoiceAction {
                    active: false,
                    label: "离开",
                    icon: "📴",
                    on_click: move |_| {
                        let mut u2 = ui;
                        let mut v2 = voice;
                        spawn(async move {
                            crate::voice_web::leave_voice().await;
                            u2.voice_sheet_open.set(false);
                            v2.active_channel.set(None);
                        });
                    },
                }
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
#[component]
fn VoiceAction(
    active: bool,
    label: String,
    icon: String,
    on_click: EventHandler<MouseEvent>,
) -> Element {
    let bg = if active { "var(--accent-bg)" } else { "var(--active-bg)" };
    let fg = if active { "var(--accent-fg)" } else { "var(--fg)" };
    rsx! {
        button {
            onclick: move |e| on_click.call(e),
            style: "
                flex: 1; height: 64px;
                display: flex; flex-direction: column; align-items: center; justify-content: center;
                gap: 4px; border-radius: 14px;
                background: {bg}; color: {fg};
                font-size: 12px; border: none;
            ",
            span { style: "font-size: 22px;", "{icon}" }
            span { "{label}" }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[component]
pub fn MobileVoiceSheet() -> Element {
    rsx! {}
}
