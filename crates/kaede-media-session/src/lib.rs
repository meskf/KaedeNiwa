//! KaedeNiwa 媒体会话层（UI 无关，单一 LiveKit 连接，多轨）。
//!
//! # 职责
//!
//! 一个 [`MediaSession`] 拥有**一条** LiveKit 连接，UI 通过它：
//! - 发布本地 track（屏幕 / 麦克风 / 系统音频）
//! - 订阅远端 track 并 attach 到 sink / player
//! - 收到 peer / track 事件
//!
//! # 与 kaede-media 的关系
//!
//! `kaede-media` 的 `LiveKitSession` / `LiveKitAudioSession` 是**一次性**
//! 会话（一条连接一个 track）。`MediaSession` 是**共享连接**模型，
//! 适合真实 UI 场景（一个人同时推屏幕 + 麦克风 + 订阅多人）。
//!
//! # UI 无关
//!
//! 输入是 [`ConnectConfig`]（ws_url / token / e2ee_key），
//! **不依赖 `kaede-client`**。UI 负责向 client 要 token，喂给本 crate。
//!
//! # 跨平台控制面
//!
//! [`MediaSessionHandle`] 是平台无关的控制面 trait（连接 / 发布 / 事件）。
//! Native 走 [`NativeHandle`]（包装 [`MediaSession`]）；
//! Web 待接 LiveKit JS SDK。

mod config;
mod event;
mod handle;
#[cfg(not(target_arch = "wasm32"))]
mod native_handle;
mod session;

pub use config::{ConnectConfig, PublishSource, TrackKind};
pub use event::{MediaEvent, RemoteKind, ScreenStats, TrackId};
pub use handle::{MediaSessionHandle, create_handle};
#[cfg(not(target_arch = "wasm32"))]
pub use native_handle::NativeHandle;
pub use session::{ConsumerHandle, MediaSession};

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
