//! 视频画质档位选择器。

use dioxus::prelude::*;
use kaede_protocol::VideoBitrate;

use crate::state::{UiState, VoiceControls, VoiceState};

/// 应用当前档位/码率。
///
/// 正在屏幕共享时执行 unpublish → publish（Web 为 stop → start），
/// 让新分辨率立即生效；未共享时只更新信号，下次共享生效。
fn apply_profile(voice: VoiceState, controls: VoiceControls, mut ui: UiState) {
    if !*controls.screen_on.read() {
        ui.toast
            .set(Some(("已更新画质，下次共享生效".into(), false)));
        return;
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let profile = *voice.current_profile.read();
        let bitrate = *voice.current_bitrate.read();
        let handle_opt = voice.handle.read().clone();
        let mut controls = controls;
        spawn(async move {
            let Some(handle) = handle_opt else {
                ui.toast.set(Some(("未连接媒体会话".into(), true)));
                controls.screen_on.set(false);
                return;
            };
            if let Err(e) = handle
                .inner()
                .unpublish(kaede_media_session::TrackKind::ScreenVideo)
                .await
            {
                tracing::warn!("screen republish unpublish: {}", e);
            }
            let bps = profile.bitrate_bps(bitrate);
            let config = kaede_capture::VideoConfig {
                encoder: kaede_capture::EncoderKind::Auto,
                fps: profile.fps,
                bitrate_kbps: (bps / 1000) as u32,
                show_cursor: true,
                display_index: 0,
                width: Some(profile.width),
                height: Some(profile.height),
            };
            match handle
                .inner()
                .publish(kaede_media_session::PublishSource::Screen { config })
                .await
            {
                Ok(()) => {
                    ui.toast
                        .set(Some((format!("已按 {} 重新共享", profile.label()), false)));
                }
                Err(e) => {
                    tracing::warn!("screen republish publish: {}", e);
                    controls.screen_on.set(false);
                    ui.toast.set(Some((format!("重新共享失败: {}", e), true)));
                }
            }
        });
    }

    #[cfg(target_arch = "wasm32")]
    {
        let profile = *voice.current_profile.read();
        let mut controls = controls;
        spawn(async move {
            if let Err(e) = crate::voice_web::set_screen_share(false, profile).await {
                tracing::warn!("[voice-web] stop for republish: {}", e);
            }
            match crate::voice_web::set_screen_share(true, profile).await {
                Ok(()) => {
                    ui.toast
                        .set(Some((format!("已按 {} 重新共享", profile.label()), false)));
                }
                Err(e) => {
                    tracing::warn!("[voice-web] republish: {}", e);
                    controls.screen_on.set(false);
                    ui.toast.set(Some((format!("重新共享失败: {}", e), true)));
                }
            }
        });
    }
}

#[component]
pub fn VideoProfilePicker() -> Element {
    let mut ui = use_context::<UiState>();
    let mut voice = use_context::<VoiceState>();
    let controls = use_context::<VoiceControls>();

    if !*ui.video_profile_picker_open.read() {
        return rsx! {};
    }

    let current = *voice.current_profile.read();
    let current_bitrate = *voice.current_bitrate.read();

    rsx! {
        div {
            class: "server-modal-backdrop",
            onclick: move |_| ui.video_profile_picker_open.set(false),
            div {
                class: "server-modal",
                style: "width: 480px; height: auto; max-height: 80vh; padding: 24px; flex-direction: column;",
                onclick: move |e| e.stop_propagation(),
                div { class: "server-modal-title", "选择视频画质档位" }
                div {
                    style: "font-size: 12px; color: var(--fg-faint); margin-bottom: 12px;",
                    "档位会传入编码器；共享中切换会自动重建屏幕共享。"
                }
                div {
                    style: "display: flex; gap: 6px; margin-bottom: 12px; align-items: center;",
                    span {
                        style: "font-size: 13px; color: var(--fg-muted); margin-right: 4px;",
                        "码率："
                    }
                    for q in VideoBitrate::all().iter() {
                        {
                            let q = *q;
                            let is_cur = q == current_bitrate;
                            let label = q.label();
                            rsx! {
                                button {
                                    key: "{label}",
                                    class: if is_cur { "btn-primary" } else { "btn-secondary" },
                                    style: "padding: 6px 14px; font-size: 13px;",
                                    onclick: move |_| {
                                        voice.current_bitrate.set(q);
                                        apply_profile(voice, controls, ui);
                                    },
                                    "{label}"
                                }
                            }
                        }
                    }
                }
                div {
                    style: "flex: 1; overflow-y: auto; display: flex; flex-direction: column; gap: 4px; min-height: 0;",
                    for p in kaede_protocol::ALL_VIDEO_PROFILES.iter() {
                        {
                            let p = *p;
                            let is_current = p == current;
                            let bitrate_hint = format!(
                                "低 {} / 标准 {} / 无损 {}",
                                p.bitrate_label(VideoBitrate::Low),
                                p.bitrate_label(VideoBitrate::Standard),
                                p.bitrate_label(VideoBitrate::Lossless),
                            );
                            rsx! {
                                button {
                                    key: "{p.width}x{p.height}@{p.fps}",
                                    class: if is_current { "btn-primary" } else { "btn-secondary" },
                                    style: "width: 100%; justify-content: flex-start; padding: 10px 14px; text-align: left; display: flex; flex-direction: column; align-items: flex-start; gap: 2px;",
                                    onclick: move |_| {
                                        voice.current_profile.set(p);
                                        ui.video_profile_picker_open.set(false);
                                        apply_profile(voice, controls, ui);
                                    },
                                    div {
                                        style: "display: flex; align-items: center; width: 100%; gap: 8px;",
                                        span { "{p.label()}" }
                                        // 选中态由 btn-primary 高亮表达
                                    }
                                    div {
                                        style: "font-size: 11px; color: var(--fg-faint); font-weight: normal;",
                                        "{bitrate_hint}"
                                    }
                                }
                            }
                        }
                    }
                }
                div { class: "form-actions", style: "margin-top: 16px;",
                    div { style: "flex: 1;" }
                    button {
                        class: "btn-secondary",
                        onclick: move |_| ui.video_profile_picker_open.set(false),
                        "关闭"
                    }
                }
            }
        }
    }
}
