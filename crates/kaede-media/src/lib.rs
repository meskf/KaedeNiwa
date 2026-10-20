//! KaedeNiwa 媒体会话层（UI 无关）。
//!
//! # 职责
//!
//! 把「采集 → SFrame 加密 → 下游传输」串成一条链，
//! 对 UI 只暴露 `start / stop / stats / events` 四个动作。
//!
//! # 架构
//!
//! ```text
//! EncodedSource ──> MediaSession ──> MediaSink
//!  (采集)             (加密+转发)       (下游)
//! ```
//!
//! - [`EncodedSource`]：同步阻塞式帧源（专用 OS 线程读取）
//! - [`MediaSink`]：异步发送器（tokio task 调用）
//! - [`MediaSession`]：状态机 + 统计 + 事件流
//!
//! # 线程模型
//!
//! ```text
//! std::thread (采集 + 加密) ─┐
//!                            ├─ async_channel(8) ─> tokio task (sink.send)
//!                            └─ events (unbounded)
//! ```
//!
//! 采集端使用 `try_send`，队列满时丢帧并累加 `frames_dropped`。
//! 实时场景延迟优先，不阻塞采集。

mod audio_gst;
mod config;
#[cfg(not(target_arch = "wasm32"))]
mod decoder;
mod error;
mod event;
mod livekit_sink;
mod livekit_video_sink;
#[cfg(not(target_arch = "wasm32"))]
mod player;
mod receive_pipeline;
mod receiver;
mod session;
mod sink;
mod source;
mod stats;

pub use audio_gst::GstAudioSource;
pub use config::{AudioShareConfig, ScreenShareConfig};
#[cfg(not(target_arch = "wasm32"))]
pub use decoder::VideoDecoder;
pub use error::MediaError;
pub use event::{MediaEvent, StopReason};
pub use livekit_sink::{
    LiveKitAudioPusher, LiveKitAudioSession, LiveKitConfig, LiveKitPusher, LiveKitSession,
};
pub use livekit_video_sink::LiveKitVideoSink;
pub use player::AudioPlayer;
pub use receive_pipeline::{ReceiveMode, ReceivePipeline};
pub use receiver::{DecryptedFrame, ReceiveError, ReceiveStats, ReceiverSession};
pub use session::{AudioSession, MediaSession, OutgoingFrame};
pub use sink::{EncryptedFrame, MediaSink, NullSink};
pub use source::{AudioCaptureConfig, AudioFrame, AudioSource, AudioSourceKind, EncodedSource};
pub use stats::{MediaState, MediaStats};

/// crate 版本。
/// 生成一个"临时 key_id"，用于 examples 等一次性会话。
///
/// 每次调用返回不同值（系统时间纳秒 + 进程内单调计数器混合），
/// 保证同进程内多次调用不重复 → 多个 SFrame 会话 nonce 空间不重叠。
///
/// **不适用于生产**：生产应该用 MLS epoch（epoch 前进自动变化）作为 key_id。
///
/// # 背景（P0-1）
///
/// `MediaSession::start` / `AudioSession::start` 的 SFrame counter 从 0 开始，
/// 不跨调用持久化。两次会话用相同 `(key, key_id, sender_id)` 会导致
/// AES-GCM nonce 复用。examples 应该用本函数生成独立 key_id。
pub fn ephemeral_key_id() -> u32 {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static EPHEMERAL_SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let seq = EPHEMERAL_SEQ.fetch_add(1, Ordering::Relaxed);
    ((nanos.wrapping_add(seq.wrapping_mul(0x9E37_79B1))) & 0xFFFF_FFFF) as u32
}
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
