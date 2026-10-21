//! 移动端外壳：底部 4 Tab + 顶栏 + 内容区。
//!
//! M1：仅布局骨架，列表页由 mobile_pages.rs 渲染占位。

use dioxus::prelude::*;

use crate::icons::{IconMail, IconSettings, IconUserList, IconUsers};
use crate::mobile_pages::{MobileDmPage, MobileFriendsPage, MobileGroupsPage};
use crate::state::{MainTab, MobileDetailKind, UiState, VoiceState};

#[component]
pub fn MobileShell() -> Element {
    let mut ui = use_context::<UiState>();
    let tab = *ui.main_tab.read();
    let detail = ui.mobile_detail.read().clone();

    let on_back = move |_| {
        ui.mobile_detail.set(None);
        ui.mobile_detail_kind.set(None);
    };

    rsx! {
        div {
            style: "height: 100%; display: flex; flex-direction: column; background: var(--outer-bg);",
            MobileTopBar { detail: detail.clone(), on_back }
            div {
                style: "flex: 1; min-height: 0; overflow-y: auto; padding: 8px 12px;",
                if detail.is_some() {
                    match *ui.mobile_detail_kind.read() {
                        Some(MobileDetailKind::Channel) => rsx! {
                            div {
                                style: "height: 100%; display: flex; flex-direction: column;",
                                crate::content::ContentArea {}
                            }
                        },
                        Some(MobileDetailKind::Dm) => rsx! {
                            div {
                                style: "padding: 32px 16px; text-align: center; color: var(--fg-faint); font-size: 14px;",
                                "私聊消息（M3 实现）"
                            }
                        },
                        None => rsx! {},
                    }
                } else {
                    match tab {
                        MainTab::Server => rsx! { MobileGroupsPage {} },
                        MainTab::Dm => rsx! { MobileDmPage {} },
                        MainTab::Friends => rsx! { MobileFriendsPage {} },
                    }
                }
            }
            MobileVoiceBar {}
            MobileTabBar {}
        }
    }
}

#[component]
fn MobileVoiceBar() -> Element {
    let voice = use_context::<VoiceState>();
    let ch = voice.active_channel.read().clone();
    let Some(ch) = ch else {
        return rsx! {};
    };
    let count = ch.members.len();
    let name = ch.name.clone();
    rsx! {
        div {
            style: "padding: 10px 14px; background: var(--positive); color: #fff; font-size: 13px; font-weight: 500; display: flex; align-items: center; gap: 8px; flex-shrink: 0;",
            span { "🔊" }
            span { "{name}" }
            span { style: "opacity: 0.8; font-size: 11px;", "· {count} 人" }
        }
    }
}

#[component]
fn MobileTopBar(detail: Option<String>, on_back: EventHandler<MouseEvent>) -> Element {
    let title = if detail.is_some() { "会话".to_string() } else { "KaedeNiwa".to_string() };

    rsx! {
        div {
            style: "
                height: 56px; flex-shrink: 0;
                display: flex; align-items: center; gap: 12px;
                padding: 0 16px;
                background: var(--inner-bg);
                border-bottom: 1px solid var(--border);
            ",
            if detail.is_some() {
                button {
                    onclick: move |e| on_back.call(e),
                    style: "width: 44px; height: 44px; border-radius: 10px; background: transparent; color: var(--fg); font-size: 20px;",
                    "←"
                }
            } else {
                span { style: "display: inline-block; width: 44px; height: 44px;", "" }
            }
            span { style: "font-weight: 600; font-size: 17px;", "{title}" }
        }
    }
}

#[component]
fn MobileTabBar() -> Element {
    let tab = *use_context::<UiState>().main_tab.read();

    rsx! {
        div {
            style: "
                height: 56px; flex-shrink: 0;
                display: flex; align-items: center; justify-content: space-around;
                background: var(--inner-bg);
                border-top: 1px solid var(--border);
                padding-bottom: env(safe-area-inset-bottom);
            ",
            TabButton { target: MainTab::Server,  current: tab, label: "群组", icon: "users" }
            TabButton { target: MainTab::Dm,      current: tab, label: "私聊", icon: "mail" }
            TabButton { target: MainTab::Friends, current: tab, label: "好友", icon: "userlist" }
            TabButton { target: MainTab::Friends, current: tab, label: "个人", icon: "settings" }
        }
    }
}

#[component]
fn TabButton(target: MainTab, current: MainTab, label: String, icon: String) -> Element {
    let mut ui = use_context::<UiState>();
    let active = target == current;
    let color = if active { "var(--accent-bg)" } else { "var(--fg-muted)" };

    rsx! {
        button {
            onclick: move |_| {
                ui.mobile_detail.set(None);
                ui.mobile_detail_kind.set(None);
                ui.main_tab.set(target);
            },
            style: "flex: 1; height: 44px; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 2px; background: transparent; border: none; color: {color}; font-size: 10px;",
            match icon.as_str() {
                "users" => rsx! { IconUsers { size: 18 } },
                "mail" => rsx! { IconMail { size: 18 } },
                "userlist" => rsx! { IconUserList { size: 18 } },
                _ => rsx! { IconSettings { size: 18 } },
            }
            span { "{label}" }
        }
    }
}
