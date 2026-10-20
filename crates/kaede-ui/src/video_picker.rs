//! 视频画质档位选择器（Phase 3a：UI 骨架，未接入 media 层）。

use dioxus::prelude::*;

use crate::state::{UiState, VoiceState};

#[component]
pub fn VideoProfilePicker() -> Element {
    let mut ui = use_context::<UiState>();
    let mut voice = use_context::<VoiceState>();

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
                    "Phase 3a：当前为 UI 预览，暂未接入编码器。实际生效见 Phase 3b。"
                }
                div {
                    style: "display: flex; gap: 6px; margin-bottom: 12px; align-items: center;",
                    span {
                        style: "font-size: 13px; color: var(--fg-muted); margin-right: 4px;",
                        "码率："
                    }
                    for q in kaede_protocol::VideoBitrate::all().iter() {
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
                                        ui.toast.set(Some((format!("码率：{}（Phase 3b 生效）", q.label()), false)));
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
                                p.bitrate_label(kaede_protocol::VideoBitrate::Low),
                                p.bitrate_label(kaede_protocol::VideoBitrate::Standard),
                                p.bitrate_label(kaede_protocol::VideoBitrate::Lossless),
                            );
                            rsx! {
                                button {
                                    key: "{p.width}x{p.height}@{p.fps}",
                                    class: if is_current { "btn-primary" } else { "btn-secondary" },
                                    style: "width: 100%; justify-content: flex-start; padding: 10px 14px; text-align: left; display: flex; flex-direction: column; align-items: flex-start; gap: 2px;",
                                    onclick: move |_| {
                                        voice.current_profile.set(p);
                                        ui.video_profile_picker_open.set(false);
                                        ui.toast.set(Some((format!("画质：{}（Phase 3b 生效）", p.label()), false)));
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
