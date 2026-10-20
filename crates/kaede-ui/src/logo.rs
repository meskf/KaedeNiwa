//! 应用 logo（用户提供的楓葉 SVG）。
//!
//! # 处理
//!
//! - 编译期 `include_str!`，不解析（156 KB 路径型 SVG）
//! - 运行期替换 `fill="#000000"` → `fill="currentColor"`（跟随主题）
//! - 注入 `viewBox`（原 SVG 无此属性）以便 CSS 尺寸缩放
//! - `use_memo` 缓存，只算一次

use dioxus::prelude::*;

const RAW: &str = include_str!("../assets/logo.svg");

fn prepare(size: u32) -> String {
    // 替换纯黑填充为主题色
    let s = RAW.replace("fill=\"#000000\"", "fill=\"currentColor\"");

    // 注入 viewBox 到 <svg> 标签。原 SVG：<svg version="1.1" xmlns="..." width="1280" height="1280">
    let needle = r#"<svg version="1.1""#;
    let inject = format!(
        r#"<svg version="1.1" viewBox="0 0 1280 1280" width="{s}" height="{s}""#,
        s = size
    );
    s.replacen(needle, &inject, 1)
}

/// 应用 logo。`size` 为像素边长。
#[component]
pub fn AppLogo(#[props(default = 28)] size: u32) -> Element {
    let html = use_memo(move || prepare(size));

    rsx! {
        div {
            style: "
                width: {size}px;
                height: {size}px;
                flex-shrink: 0;
                display: flex;
                align-items: center;
                justify-content: center;
                color: var(--fg);
            ",
            dangerous_inner_html: "{html}",
        }
    }
}
