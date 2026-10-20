//! 帧源抽象。
//!
//! 生产实现由 `kaede_capture::EncodedVideoStream` 提供，
//! 测试可以用 Mock。

use crate::MediaError;

pub use kaede_capture::EncodedFrame;

/// 同步阻塞式帧源。
///
/// # 契约
///
/// - 返回 `Ok(Some(frame))`：正常一帧
/// - 返回 `Ok(None)`：帧源自然结束（对应 `EncodedVideoStream` 的 channel 关闭）
/// - 返回 `Err(_)`：帧源错误
///
/// # 为什么是同步的
///
/// `EncodedVideoStream::next_frame()` 走的是 `std::sync::mpsc::Receiver::recv()`，
/// 是同步阻塞的。强制 `async` 只会引入无谓的 `spawn_blocking`。
/// 让 `MediaSession` 用专用 OS 线程驱动它，async 侧从 channel 拿加密帧。
pub trait EncodedSource: Send + 'static {
    fn next_frame_blocking(&mut self) -> Result<Option<EncodedFrame>, MediaError>;

    /// 带超时的下一帧。
    ///
    /// - `Ok(Some(f))` — 正常一帧
    /// - `Ok(None)`   — 源自然结束
    /// - `Err(Timeout)` — 超时（无帧）；上层应静默重试
    /// - `Err(_)`     — 源错误
    ///
    /// 默认实现退化成 [`Self::next_frame_blocking`]（无超时）。实现者应覆盖，
    /// 否则 `MediaSession::stop()` 在源卡死时无法及时生效。
    fn next_frame_timeout(
        &mut self,
        _timeout: std::time::Duration,
    ) -> Result<Option<EncodedFrame>, MediaError> {
        self.next_frame_blocking()
    }

    fn stop(&mut self);
}

impl EncodedSource for kaede_capture::EncodedVideoStream {
    fn next_frame_blocking(&mut self) -> Result<Option<EncodedFrame>, MediaError> {
        match self.next_frame() {
            Ok(f) => Ok(Some(f)),
            Err(kaede_capture::CaptureError::NotStarted) => Ok(None),
            Err(e) => Err(MediaError::Capture(e.to_string())),
        }
    }

    fn next_frame_timeout(
        &mut self,
        timeout: std::time::Duration,
    ) -> Result<Option<EncodedFrame>, MediaError> {
        match kaede_capture::EncodedVideoStream::next_frame_timeout(self, timeout) {
            Ok(Some(f)) => Ok(Some(f)),
            // 无帧超时：给上层做 stop_flag 检查
            Ok(None) => Err(MediaError::Timeout),
            // channel 关闭 = 源自然结束，不是错误
            Err(kaede_capture::CaptureError::NotStarted) => Ok(None),
            Err(e) => Err(MediaError::Capture(e.to_string())),
        }
    }

    fn stop(&mut self) {
        kaede_capture::EncodedVideoStream::stop(self);
    }
}

// ============================================================================
// 音频帧源
// ============================================================================

/// 音频帧。
///
/// - `data`：PCM S16LE 交错格式（与 GStreamer `audio/x-raw,format=S16LE` 一致）
/// - `pts_ms`：显示时间戳
#[derive(Debug, Clone)]
pub struct AudioFrame {
    pub data: Vec<u8>,
    pub pts_ms: i64,
    pub sample_rate: u32,
    pub channels: u16,
}

/// 音频采集源类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AudioSourceKind {
    /// 系统输出（loopback）。用来共享你电脑正在播放的声音。
    #[default]
    SystemLoopback,
    /// 麦克风输入。用来语音聊天。
    Microphone,
}

/// 音频采集配置。
#[derive(Debug, Clone)]
pub struct AudioCaptureConfig {
    pub sample_rate: u32,
    pub channels: u16,
    /// 采集源类型。默认 [`AudioSourceKind::SystemLoopback`]。
    pub kind: AudioSourceKind,
    /// `Some(pid)` = 指定进程；`None` = 系统 loopback（全系统声音）。
    ///
    /// 注意：GStreamer `wasapi2src` **不支持按进程过滤**。传 `Some(_)`
    /// 且 `kind == SystemLoopback` 会返回 `MediaError::Capture`。
    pub pid: Option<u32>,
    /// 每帧时长（毫秒）。GStreamer `audiobuffersplit` 用它切块。
    pub frame_duration_ms: u32,
}

impl Default for AudioCaptureConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48000,
            channels: 2,
            kind: AudioSourceKind::SystemLoopback,
            pid: None,
            frame_duration_ms: 20,
        }
    }
}

/// 同步阻塞式音频帧源（同 [`EncodedSource`] 的契约）。
pub trait AudioSource: Send + 'static {
    fn next_audio_blocking(&mut self) -> Result<Option<AudioFrame>, MediaError>;

    /// 带超时的下一帧音频块。语义同
    /// [`EncodedSource::next_frame_timeout`]。
    fn next_audio_timeout(
        &mut self,
        _timeout: std::time::Duration,
    ) -> Result<Option<AudioFrame>, MediaError> {
        self.next_audio_blocking()
    }

    fn stop(&mut self);
}
