//! 内联 SVG 图标（Lucide 风格）。
//!
//! 所有图标共用 24x24 viewBox，stroke="currentColor"，随字体颜色。

#![allow(dead_code)]

use dioxus::prelude::*;

macro_rules! icon {
    ($name:ident, $($child:tt)*) => {
        #[component]
        pub fn $name(#[props(default = 16)] size: u32) -> Element {
            rsx! {
                svg {
                    width: "{size}",
                    height: "{size}",
                    view_box: "0 0 24 24",
                    fill: "none",
                    stroke: "currentColor",
                    stroke_width: "2",
                    stroke_linecap: "round",
                    stroke_linejoin: "round",
                    $($child)*
                }
            }
        }
    };
}

icon!(IconHash,
    line { x1: "4", x2: "20", y1: "9",  y2: "9" }
    line { x1: "4", x2: "20", y1: "15", y2: "15" }
    line { x1: "10", x2: "8", y1: "3",  y2: "21" }
    line { x1: "16", x2: "14", y1: "3", y2: "21" }
);

icon!(IconVolume2,
    polygon { points: "11 5 6 9 2 9 2 15 6 15 11 19 11 5" }
    path { d: "M15.54 8.46a5 5 0 0 1 0 7.07" }
    path { d: "M19.07 4.93a10 10 0 0 1 0 14.14" }
);

icon!(IconMic,
    path { d: "M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3Z" }
    path { d: "M19 10v2a7 7 0 0 1-14 0v-2" }
    line { x1: "12", x2: "12", y1: "19", y2: "22" }
);

icon!(IconMonitor,
    rect { width: "20", height: "14", x: "2", y: "3", rx: "2" }
    line { x1: "8",  x2: "16", y1: "21", y2: "21" }
    line { x1: "12", x2: "12", y1: "17", y2: "21" }
);

icon!(IconX,
    path { d: "M18 6 6 18" }
    path { d: "m6 6 12 12" }
);

icon!(IconMinus, path { d: "M5 12h14" });

icon!(IconLock,
    rect { width: "18", height: "11", x: "3", y: "11", rx: "2" }
    path { d: "M7 11V7a5 5 0 0 1 10 0v4" }
);

icon!(
    IconSquare,
    rect {
        width: "14",
        height: "14",
        x: "5",
        y: "5",
        rx: "1"
    }
);

icon!(IconSun,
    circle { cx: "12", cy: "12", r: "4" }
    path { d: "M12 2v2" }
    path { d: "M12 20v2" }
    path { d: "m4.93 4.93 1.41 1.41" }
    path { d: "m17.66 17.66 1.41 1.41" }
    path { d: "M2 12h2" }
    path { d: "M20 12h2" }
    path { d: "m6.34 17.66-1.41 1.41" }
    path { d: "m19.07 4.93-1.41 1.41" }
);

icon!(
    IconMoon,
    path {
        d: "M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"
    }
);

icon!(IconSettings,
    path { d: "M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z" }
    circle { cx: "12", cy: "12", r: "3" }
);

icon!(IconHelpCircle,
    circle { cx: "12", cy: "12", r: "10" }
    path { d: "M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3" }
    path { d: "M12 17h.01" }
);

icon!(IconSend,
    path { d: "m22 2-7 20-4-9-9-4Z" }
    path { d: "M22 2 11 13" }
);

icon!(IconPlus,
    path { d: "M5 12h14" }
    path { d: "M12 5v14" }
);
icon!(IconServer,
    rect { width: "20", height: "8", x: "2", y: "2", rx: "2", ry: "2" }
    rect { width: "20", height: "8", x: "2", y: "14", rx: "2", ry: "2" }
    line { x1: "6", x2: "6.01", y1: "6", y2: "6" }
    line { x1: "6", x2: "6.01", y1: "18", y2: "18" }
);

icon!(
    IconCheck,
    polyline {
        points: "20 6 9 17 4 12"
    }
);

icon!(IconTrash,
    path { d: "M3 6h18" }
    path { d: "M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6" }
    path { d: "M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" }
);
icon!(IconMail,
    rect { width: "20", height: "16", x: "2", y: "4", rx: "2" }
    path { d: "m22 7-8.97 5.7a1.94 1.94 0 0 1-2.06 0L2 7" }
);

icon!(IconUsers,
    path { d: "M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2" }
    circle { cx: "9", cy: "7", r: "4" }
    path { d: "M22 21v-2a4 4 0 0 0-3-3.87" }
    path { d: "M16 3.13a4 4 0 0 1 0 7.75" }
);
// ----------------------------------------------------------------------------
// 精细图标（stroke-width 1.25）—— 窗口控件和列表按钮专用
// ----------------------------------------------------------------------------

#[component]
pub fn IconWinMin(#[props(default = 12)] size: u32) -> Element {
    rsx! {
        svg {
            width: "{size}", height: "{size}",
            view_box: "0 0 12 12",
            fill: "none", stroke: "currentColor",
            stroke_width: "1.25", stroke_linecap: "round",
            line { x1: "2.5", x2: "9.5", y1: "6", y2: "6" }
        }
    }
}

#[component]
pub fn IconWinMax(#[props(default = 12)] size: u32) -> Element {
    rsx! {
        svg {
            width: "{size}", height: "{size}",
            view_box: "0 0 12 12",
            fill: "none", stroke: "currentColor",
            stroke_width: "1.25", stroke_linecap: "round",
            rect { x: "2.5", y: "2.5", width: "7", height: "7", rx: "0.5" }
        }
    }
}

#[component]
pub fn IconWinClose(#[props(default = 12)] size: u32) -> Element {
    rsx! {
        svg {
            width: "{size}", height: "{size}",
            view_box: "0 0 12 12",
            fill: "none", stroke: "currentColor",
            stroke_width: "1.25", stroke_linecap: "round",
            line { x1: "2.5", x2: "9.5", y1: "2.5", y2: "9.5" }
            line { x1: "9.5", x2: "2.5", y1: "2.5", y2: "9.5" }
        }
    }
}

#[component]
pub fn IconUserList(#[props(default = 16)] size: u32) -> Element {
    rsx! {
        svg {
            width: "{size}", height: "{size}",
            view_box: "0 0 24 24",
            fill: "none", stroke: "currentColor",
            stroke_width: "1.5", stroke_linecap: "round", stroke_linejoin: "round",
            circle { cx: "9", cy: "7", r: "3.5" }
            path { d: "M2.5 20a6.5 6.5 0 0 1 13 0" }
            line { x1: "19", x2: "19", y1: "9",  y2: "15" }
            line { x1: "16", x2: "22", y1: "12", y2: "12" }
        }
    }
}
