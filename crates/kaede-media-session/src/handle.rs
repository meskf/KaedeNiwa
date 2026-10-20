//! UI 层媒体控制器 trait（平台无关）。
//!
//! # 设计
//!
//! - **控制面**：`connect` / `publish` / `unpublish` / `disconnect` / `events`
//! - **不做渲染**：视频/音频渲染天然分平台（Web `<video>` vs Native GStreamer），
//!   trait 不管，UI 层 `#[cfg]` 处理
//! - **不做订阅**：`attach_video` / `attach_audio` 是 Native 专有（Web 用 DOM），
//!   留在具体 impl 里
//!
//! # 使用
//!
//! ```ignore
//! let token = room.get_livekit_token(ch).await?;
//! let key   = room.get_livekit_e2ee_key(ch).await?;
//! let handle = create_handle(ConnectConfig { ws_url: token.ws_url, .. }).await?;
//! handle.publish(PublishSource::Screen { config })?;
//! ```

use async_trait::async_trait;
use tokio::sync::broadcast;

use crate::{ConnectConfig, MediaEvent, PublishSource, TrackKind};
use kaede_media::MediaError;

/// UI 持有的媒体控制器。
#[async_trait]
pub trait MediaSessionHandle: Send + Sync + 'static {
    /// 建连。
    async fn connect(cfg: ConnectConfig) -> Result<std::sync::Arc<Self>, MediaError>
    where
        Self: Sized;

    /// 控制面事件流（fanout）。
    fn events(&self) -> broadcast::Receiver<MediaEvent>;

    /// 当前本地 uid。
    fn local_uid(&self) -> String;

    /// Room 名。
    fn room_name(&self) -> String;

    /// 发布本地 track。
    async fn publish(&self, source: PublishSource) -> Result<(), MediaError>;

    /// 取消发布。
    async fn unpublish(&self, kind: TrackKind) -> Result<(), MediaError>;

    /// 断开。幂等。
    async fn disconnect(&self);
}

/// 平台分派：构造当前平台 handle。
///
/// Native → `NativeHandle`（走 Rust LiveKit SDK）
/// Web    → `WebHandle`（走 LiveKit JS SDK，暂 `Unsupported`）
#[cfg(not(target_arch = "wasm32"))]
pub async fn create_handle(
    cfg: ConnectConfig,
) -> Result<std::sync::Arc<dyn MediaSessionHandle>, MediaError> {
    use crate::native_handle::NativeHandle;
    let h = NativeHandle::connect(cfg).await?;
    Ok(h as std::sync::Arc<dyn MediaSessionHandle>)
}

#[cfg(target_arch = "wasm32")]
pub async fn create_handle(
    cfg: ConnectConfig,
) -> Result<std::sync::Arc<dyn MediaSessionHandle>, MediaError> {
    let _ = cfg;
    Err(MediaError::Sink(
        "Web MediaSessionHandle not yet implemented (use LiveKit JS SDK)".into(),
    ))
}
