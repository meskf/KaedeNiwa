//! kaede-livekit-web — 自己封装的 LiveKit JS SDK (UMD) 胶水层。
//!
//! 让 kaede-wasm / kaede-ui 在浏览器里通过 WebRTC 实现屏幕共享 + 音视频 AES-256 E2EE。
//!
//! ## 与原生路径的关系
//! - 原生（kaede-media-session + kaede-media）走 LiveKit Rust SDK + GStreamer + NVENC。
//! - wasm（本 crate）走浏览器原生 WebRTC，最高 4K60，覆盖 AMD/Intel/无独显。
//! 两条路径共享 begin_media_session() 的 token 和 e2ee seed，派生出的密钥一致，
//! 因此原生/Web 客户端可以互相看到对方的屏幕共享。
//!
//! ## 宿主页面要求
//! index.html 需加载 LiveKit UMD，版本与 LIVEKIT_UMD_VERSION 一致：
//!
//! ```html
//! <script src="https://cdn.jsdelivr.net/npm/livekit-client@2.14.0/dist/livekit-client.umd.min.js"></script>
//! ```
//!
//! UMD 暴露 window.LivekitClient。本 crate 通过 js_sys::Reflect 动态访问。

pub mod error;
pub use error::LiveKitWebError;

pub const LIVEKIT_UMD_VERSION: &str = "2.14.0";

#[cfg(target_arch = "wasm32")]
mod wasm_impl;
#[cfg(target_arch = "wasm32")]
pub use wasm_impl::LiveKitWebSession;

#[cfg(not(target_arch = "wasm32"))]
mod native_stub;
#[cfg(not(target_arch = "wasm32"))]
pub use native_stub::LiveKitWebSession;
