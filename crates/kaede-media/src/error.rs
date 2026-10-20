//! 媒体层错误。

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("capture: {0}")]
    Capture(String),

    #[error("sframe: {0}")]
    SFrame(String),

    #[error("sink: {0}")]
    Sink(String),

    #[error("invalid state: {0}")]
    InvalidState(String),

    #[error("channel closed")]
    ChannelClosed,

    /// 采集源在超时时间内没有新帧。
    /// `MediaSession` 主循环用它做「定期检查 stop_flag」的节拍，
    /// 不是错误——上层应该静默重试。
    #[error("timeout")]
    Timeout,
}
