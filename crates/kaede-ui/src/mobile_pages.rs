//! 移动端 4 个列表页骨架。
//!
//! M1：仅占位。M2 探查 AppState 的 dm_conversations / friends / channels
//! 的真实类型后再接入数据。

use dioxus::prelude::*;

#[component]
pub fn MobileGroupsPage() -> Element {
    rsx! {
        div {
            style: "padding: 32px 16px; text-align: center; color: var(--fg-faint); font-size: 14px;",
            "频道列表（M2 填充）"
        }
    }
}

#[component]
pub fn MobileDmPage() -> Element {
    rsx! {
        div {
            style: "padding: 32px 16px; text-align: center; color: var(--fg-faint); font-size: 14px;",
            "私聊列表（M2 填充）"
        }
    }
}

#[component]
pub fn MobileFriendsPage() -> Element {
    rsx! {
        div {
            style: "padding: 32px 16px; text-align: center; color: var(--fg-faint); font-size: 14px;",
            "好友列表（M2 填充）"
        }
    }
}
