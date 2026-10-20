//! Desktop (Wry WebView) 入口。

use dioxus_desktop::tao::dpi::LogicalSize;
use dioxus_desktop::{Config, WindowBuilder};

fn main() {
    tracing_subscriber::fmt::init();

    // Windows: 禁用 Chromium / WebView2 的 Fluent overlay 滚动条，
    // 让 ::-webkit-scrollbar 自定义样式生效。
    // WebView2 会在进程创建时读取 WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS。
    #[cfg(target_os = "windows")]
    {
        let args = "--disable-features=FluentScrollbar,FluentOverlayScrollbar,OverlayScrollbar,Windows11Scrollbar";
        unsafe {
            std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", args);
        }
    }

    let window = WindowBuilder::new()
        .with_title("KaedeNiwa")
        .with_inner_size(LogicalSize::new(1280.0, 800.0))
        .with_min_inner_size(LogicalSize::new(800.0, 500.0))
        .with_decorations(false)
        .with_always_on_top(false);

    let cfg = Config::new().with_window(window);

    dioxus::LaunchBuilder::new()
        .with_cfg(cfg)
        .launch(kaede_ui::App);
}
