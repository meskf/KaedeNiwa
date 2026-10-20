//! 控制面事件。

/// 远端 track 的唯一标识。`uid` 是 LiveKit participant identity，
/// `sid` 是 track sid。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TrackId {
    pub uid: String,
    pub sid: String,
}

impl TrackId {
    pub fn new(uid: impl Into<String>, sid: impl Into<String>) -> Self {
        Self {
            uid: uid.into(),
            sid: sid.into(),
        }
    }
}

/// 远端 track 类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteKind {
    Video,
    Audio,
}

/// 屏幕共享实时统计。
#[derive(Debug, Clone)]
pub struct ScreenStats {
    pub width: u32,
    pub height: u32,
    pub fps_actual: f64,
    pub fps_target: f64,
    /// 目标码率（来自配置）
    pub bitrate_kbps: u32,
    /// 实际输出码率（编码器每秒产出的字节数 × 8）
    pub bitrate_actual_kbps: u32,
}

/// 控制面事件流（broadcast，fanout）。
#[derive(Debug, Clone)]
pub enum MediaEvent {
    /// 与 LiveKit room 建立连接。
    Connected { room_name: String },
    /// 连接断开。
    Disconnected { reason: String },

    /// 本地某 track 发布成功。
    LocalPublished { kind: crate::TrackKind, sid: String },
    /// 本地某 track 取消发布。
    LocalUnpublished { kind: crate::TrackKind },

    /// 远端 peer 加入。
    PeerJoined { uid: String },
    /// 远端 peer 离开。
    PeerLeft { uid: String },

    /// 远端出现新 track。UI 收到后决定是否 attach。
    TrackAvailable { track: TrackId, kind: RemoteKind },
    /// 远端 track 消失。
    TrackGone { track: TrackId, kind: RemoteKind },

    /// 屏幕共享实时统计。每秒发一次。
    ScreenStats(ScreenStats),

    /// 错误（非致命）。
    Error { message: String },
}
