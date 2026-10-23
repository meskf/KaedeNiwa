//! KaedeNiwa 原生移动端入口（Dioxus mobile / Android 优先）。
//!
//! 两种构建模式：
//!
//! - **默认（native-ui）**：Dioxus 原生 UI，复用 `kaede_ui::App`。包含文字频道、
//!   DM、附件、私密频道；不编译 GStreamer/LiveKit 原生媒体栈，因此**不含语音**。
//! - **`web-shell`**：内嵌网页版客户端的壳。网页版已支持语音 + 屏幕共享观看，
//!   适合内测阶段需要完整媒体能力的场景。
//!
//! web-shell 的网页地址在**编译期**通过 `KAEDE_MOBILE_WEB_URL` 指定：
//!
//! ```powershell
//! $env:KAEDE_MOBILE_WEB_URL = "https://kaede.example.com"
//! dx build --package kaede-mobile --platform android --release --features web-shell
//! ```
//!
//! Android 模拟器访问宿主机开发服务器用 `http://10.0.2.2:18080`。
//! 生产请使用 HTTPS；Android 9+ 默认禁止明文 HTTP。

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,kaede_ui=info,kaede_client=info".into()),
        )
        .init();

    launch();
}

#[cfg(feature = "web-shell")]
fn launch() {
    let url = option_env!("KAEDE_MOBILE_WEB_URL").unwrap_or("http://10.0.2.2:18080");
    let index = web_shell_index(url);
    let cfg = dioxus_desktop::Config::new().with_custom_index(index);
    dioxus::LaunchBuilder::new()
        .with_cfg(cfg)
        .launch(kaede_ui::App);
}

#[cfg(not(feature = "web-shell"))]
fn launch() {
    dioxus::LaunchBuilder::new().launch(kaede_ui::App);
}

/// 生成只包含一个全屏 iframe 的 index.html。
///
/// Dioxus 会向 `</body>` 前注入自己的 loader，并挂载到 `#main`；
/// 这里保留一个隐藏的 `#main`，避免壳启动失败，同时让网页版占满整个 WebView。
#[cfg(feature = "web-shell")]
fn web_shell_index(url: &str) -> String {
    let url = html_escape(url);
    format!(
        r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover" />
<title>KaedeNiwa</title>
<style>
  html, body {{ margin: 0; padding: 0; width: 100%; height: 100%; overflow: hidden; background: #111; }}
  #kaede-web {{ position: fixed; inset: 0; width: 100%; height: 100%; border: 0; }}
  #main {{ display: none; }}
</style>
</head>
<body>
<iframe id="kaede-web" src="{url}"
  allow="camera; microphone; display-capture; clipboard-read; clipboard-write; fullscreen; autoplay; encrypted-media"
  allowfullscreen></iframe>
<div id="main"></div>
</body>
</html>"#
    )
}

#[cfg(feature = "web-shell")]
fn html_escape(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(all(test, feature = "web-shell"))]
mod tests {
    use super::*;

    #[test]
    fn web_shell_escapes_url() {
        let html = web_shell_index(r#"https://x.test/?a=1&b="2""#);
        assert!(html.contains("a=1&amp;b=&quot;2&quot;"));
        assert!(!html.contains(r#"src="https://x.test/?a=1&b="2"#));
    }
}
