//! 媒体会话事件。

/// 停止原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    /// 用户主动停止。
    UserRequest,
    /// 帧源自然结束（采集器退出）。
    SourceEnded,
    /// 下游发送失败。
    SinkFailed(String),
    /// 会话被 Drop。
    SessionDropped,
}

/// 媒体事件流。UI 通过 `MediaSession::events()` 订阅。
#[derive(Debug, Clone)]
pub enum MediaEvent {
    /// 会话启动。
    Started,
    /// 一帧完成加密。
    FrameEncrypted {
        pts_ms: i64,
        bytes: usize,
        is_keyframe: bool,
    },
    /// 队列满导致丢帧。
    FrameDropped { pts_ms: i64 },
    /// 会话停止。
    Stopped { reason: StopReason },
    /// 错误。
    Error { message: String },
}
