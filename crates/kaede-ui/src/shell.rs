//! 应用外壳：全局 TitleBar（含 tab）+ 主体区域。

use crate::i18n::use_i18n;
use dioxus::prelude::*;

use crate::content::ContentArea;
use crate::icons::{
    IconHelpCircle, IconMail, IconMoon, IconServer, IconSettings, IconSun, IconUserList, IconUsers,
    IconWinClose, IconWinMax, IconWinMin,
};
use crate::logo::AppLogo;
use crate::members::MemberList;
use crate::sidebar::Sidebar;
use crate::state::{AppState, MainTab, UiState, View};
use crate::theme::{Mode, radius, space};

// ============================================================================
// 全局标题栏
// ============================================================================

#[component]
pub fn TitleBar() -> Element {
    let mut ui = use_context::<UiState>();
    let app = use_context::<AppState>();
    let i18n = use_i18n();
    let view = *ui.view.read();
    let current_tab = *ui.main_tab.read();

    #[cfg(not(target_arch = "wasm32"))]
    let on_drag = move |_: Event<MouseData>| {
        dioxus::desktop::window().drag();
    };
    #[cfg(target_arch = "wasm32")]
    let on_drag = move |_: Event<MouseData>| {};

    let theme_icon = if *ui.theme_mode.read() == Mode::Dark {
        rsx! { IconSun { size: 18 } }
    } else {
        rsx! { IconMoon { size: 18 } }
    };

    let server_label = use_memo(move || {
        let sid = app.current_server.read().clone();
        sid.and_then(|s| app.servers.read().get(&s).map(|srv| srv.name.clone()))
            .unwrap_or_else(|| i18n.t("nav.defaultServer"))
    });

    rsx! {
        div {
            onmousedown: on_drag,
            style: "
                height: 56px; flex-shrink: 0;
                display: flex; align-items: center;
                padding: 0 {space::LG};
            ",
            div {
                style: "
                    width: 236px; flex-shrink: 0;
                    display: flex; align-items: center;
                    gap: 10px;
                    padding-left: {space::SM};
                    user-select: none;
                ",
                AppLogo { size: 26 }
                span { style: "font-size: 20px; letter-spacing: 0.5px;", "KaedeNiwa" }
            }
            div {
                style: "display: flex; align-items: center; gap: 6px;",
                if view == View::Main {
                    button {
                        class: if current_tab == MainTab::Server { "top-tab active" } else { "top-tab" },
                        onmousedown: move |e| e.stop_propagation(),
                        onclick: move |_| ui.main_tab.set(MainTab::Server),
                        IconServer { size: 15 }
                        span { class: "label", "{server_label}" }
                    }
                    button {
                        class: if current_tab == MainTab::Dm { "top-tab active" } else { "top-tab" },
                        onmousedown: move |e| e.stop_propagation(),
                        onclick: move |_| ui.main_tab.set(MainTab::Dm),
                        IconMail { size: 15 }
                        span { "{i18n.t(\"nav.dm\")}" }
                    }
                    button {
                        class: if current_tab == MainTab::Friends { "top-tab active" } else { "top-tab" },
                        onmousedown: move |e| e.stop_propagation(),
                        onclick: move |_| ui.main_tab.set(MainTab::Friends),
                        IconUsers { size: 15 }
                        span { "{i18n.t(\"nav.friends\")}" }
                    }
                }
            }
            div { style: "flex: 1;" }
            div {
                style: "display: flex; align-items: center; gap: {space::XS};",
                if view == View::Main {
                    IconButton {
                        title: "{i18n.t(\"settings.title\")}",
                        onclick: move |_| {
                            let srvs = app.servers.read();
                            let cur = ui.server_modal_selected.peek().clone();
                            let valid = cur.as_ref().is_some_and(|c| srvs.contains_key(c));
                            if !valid {
                                let mut ids: Vec<_> = srvs.keys().cloned().collect();
                                ids.sort();
                                ui.server_modal_selected.set(ids.into_iter().next());
                            }
                            drop(srvs);
                            ui.server_modal_open.set(true);
                        },
                        IconServer { size: 18 }
                    }
                }
                IconButton {
                    title: "Theme",
                    onclick: move |_| ui.toggle_theme(),
                    {theme_icon}
                }
                IconButton {
                    title: "{i18n.t(\"settings.title\")}",
                    onclick: move |_| ui.settings_open.set(true),
                    IconSettings { size: 18 }
                }
                IconButton {
                    title: "Help",
                    IconHelpCircle { size: 18 }
                }
                div { style: "width: 8px;" }
                WindowControls {}
            }
        }
    }
}

#[component]
fn TopTab(active: bool, onclick: EventHandler<MouseEvent>, children: Element) -> Element {
    let bg = if active {
        "var(--inner-bg)"
    } else {
        "transparent"
    };
    let fg = if active {
        "var(--fg)"
    } else {
        "var(--fg-muted)"
    };
    let weight = if active { "600" } else { "500" };

    rsx! {
        button {
            onmousedown: move |e| e.stop_propagation(),
            onclick: move |e| onclick.call(e),
            style: "
                height: 36px;
                padding: 0 14px;
                display: flex; align-items: center; gap: 7px;
                border-radius: 10px;
                background: {bg} !important;
                color: {fg};
                font-size: 14px;
                font-weight: {weight};
                transition: background 120ms, color 120ms;
                flex-shrink: 0;
                max-width: 220px; outline: none; appearance: none; -webkit-appearance: none;
            ",
            {children}
        }
    }
}

// ============================================================================
// 主体
// ============================================================================

#[component]
pub fn MainBody() -> Element {
    let mut ui = use_context::<UiState>();
    let i18n = use_i18n();
    let tab = *ui.main_tab.read();
    let members_open = *ui.members_open.read();

    rsx! {
        div {
            style: "
                flex: 1; min-height: 0;
                display: flex; gap: {space::LG};
                padding: 0 {space::LG} {space::LG} {space::LG};
            ",
            Sidebar {}
            div {
                style: "
                    flex: 1; min-width: 0; min-height: 0;
                    background: var(--inner-bg);
                    border-radius: {radius::INNER};
                    overflow: hidden;
                    display: flex; flex-direction: column;
                    box-shadow: 0 1px 3px rgba(0,0,0,0.04);
                    position: relative;
                ",
                if tab == MainTab::Server {
                    button {
                        title: "{i18n.t(\"member.toggle\")}",
                        onmousedown: move |e| e.stop_propagation(),
                        onclick: move |_| ui.members_open.set(!members_open),
                        style: "
                            position: absolute;
                            top: 10px; right: 12px;
                            width: 36px; height: 36px;
                            display: flex; align-items: center; justify-content: center;
                            border-radius: 10px;
                            color: var(--fg-muted);
                            z-index: 10;
                            transition: background 120ms, color 120ms;
                        ",
                        IconUserList { size: 18 }
                    }
                }
                match tab {
                    MainTab::Server => rsx! {
                        div {
                            class: "content-split",
                            ContentArea {}
                            if members_open {
                                MemberList {}
                            }
                        }
                    },
                    MainTab::Dm => rsx! { crate::dm_view::DmView {} },
                    MainTab::Friends => rsx! { crate::friends_view::FriendsView {} },
                }
            }
        }
    }
}

#[component]
fn EmptyView(text: String) -> Element {
    rsx! {
        div {
            style: "
                flex: 1; min-height: 0;
                display: flex; align-items: center; justify-content: center;
                color: var(--fg-faint); font-size: 14px;
            ",
            "{text}"
        }
    }
}

// ============================================================================
// 通用小组件
// ============================================================================

#[component]
fn IconButton(
    #[props(default = String::new())] title: String,
    #[props(default)] onclick: EventHandler<MouseEvent>,
    children: Element,
) -> Element {
    rsx! {
        button {
            title: "{title}",
            onmousedown: move |e| e.stop_propagation(),
            onclick: move |e| onclick.call(e),
            style: "
                width: 36px; height: 36px;
                display: flex; align-items: center; justify-content: center;
                border-radius: {radius::ITEM};
                color: var(--fg-muted);
                transition: background 120ms;
                flex-shrink: 0;
            ",
            {children}
        }
    }
}

#[component]
fn WindowControls() -> Element {
    #[cfg(target_arch = "wasm32")]
    {
        rsx! {}
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let w = dioxus::desktop::window();
        let w_min = w.clone();
        let w_max = w.clone();
        let w_close = w;

        rsx! {
            div {
                style: "display: flex; gap: 0;",
                WinBtn {
                    onclick: move |_| { w_min.window.set_minimized(true); },
                    IconWinMin { size: 12 }
                }
                WinBtn {
                    onclick: move |_| { w_max.toggle_maximized(); },
                    IconWinMax { size: 12 }
                }
                WinBtn {
                    onclick: move |_| { w_close.close(); },
                    IconWinClose { size: 12 }
                }
            }
        }
    }
}

#[component]
fn WinBtn(#[props(default)] onclick: EventHandler<MouseEvent>, children: Element) -> Element {
    rsx! {
        button {
            class: "win-btn",
            onmousedown: move |e| e.stop_propagation(),
            onclick: move |e| onclick.call(e),
            style: "
                width: 40px; height: 32px;
                display: flex; align-items: center; justify-content: center;
                border-radius: 6px;
                color: var(--fg-muted);
                transition: background 120ms, color 120ms;
            ",
            {children}
        }
    }
}
