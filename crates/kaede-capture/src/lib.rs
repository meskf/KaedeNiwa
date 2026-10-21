//! KaedeNiwa 屏幕 + 音频捕获。
//!
//! # 架构
//!
//! 本 crate 只做**采集 + 编码**，输出 H.264 码流给上层：
//! - [`EncodedVideoStream`]：屏幕 → NVENC/QSV/AMF → H.264 Annex B
//!
//! 下游由 [`kaede_media`] 处理（SFrame 加密 → 传输）。
//! 音频采集见后续阶段（`wasapi2src` 直接走 GStreamer）。
//!
//! # 平台支持
//!
//! | 平台 | 屏幕 | 状态 |
//! |---|---|---|
//! | Windows | d3d11screencapturesrc + NVENC | ✅ 已实现 |
//! | Linux   | pipewiresrc + VAAPI | 待实现 |
//! | macOS   | avfvideosrc + VideoToolbox | 待实现 |
//!
//! # 用法
//!
//! ```no_run
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use kaede_capture::{EncodedVideoStream, VideoConfig, EncoderKind};
//!
//! let stream = EncodedVideoStream::start(VideoConfig {
//!     encoder: EncoderKind::Auto,
//!     fps: 60,
//!     bitrate_kbps: 8000,
//!     show_cursor: true,
//!     display_index: 0,
//!         ..Default::default()  // C-5: width/height 默认 None = 原生分辨率
//! })?;
//!
//! while let Ok(frame) = stream.next_frame() {
//!     // frame.data: H.264 Annex B
//!     // frame.is_keyframe: 是否 IDR
//!     // frame.pts_ms: 时间戳
//!     let _ = frame;
//!     break;
//! }
//! # Ok(())
//! # }
//! ```

pub mod encoded;
pub mod error;

pub use encoded::{
    EncodedFrame, EncodedVideoStream, EncoderKind, VideoCodec, VideoConfig, primary_display_size,
};
pub use error::CaptureError;

/// 初始化 GStreamer。
///
/// **必须在任何捕获操作之前调用。** 多次调用是安全的（内部有 OnceLock 保护）。
pub fn init() -> Result<(), CaptureError> {
    gstreamer::init().map_err(|e| CaptureError::GStreamer(e.to_string()))?;
    Ok(())
}

/// crate 版本。
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
