//! 媒体会话状态与统计。

/// 会话状态机。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaState {
    Idle,
    Starting,
    Active,
    Stopping,
    Stopped,
    Failed,
}

impl MediaState {
    pub fn is_active(self) -> bool {
        matches!(self, MediaState::Active)
    }
    pub fn is_terminal(self) -> bool {
        matches!(self, MediaState::Stopped | MediaState::Failed)
    }

    pub(crate) fn as_u8(self) -> u8 {
        match self {
            MediaState::Idle => 0,
            MediaState::Starting => 1,
            MediaState::Active => 2,
            MediaState::Stopping => 3,
            MediaState::Stopped => 4,
            MediaState::Failed => 5,
        }
    }

    pub(crate) fn from_u8(v: u8) -> Self {
        match v {
            0 => MediaState::Idle,
            1 => MediaState::Starting,
            2 => MediaState::Active,
            3 => MediaState::Stopping,
            4 => MediaState::Stopped,
            _ => MediaState::Failed,
        }
    }
}

/// 会话运行统计（快照）。
#[derive(Debug, Clone, Default)]
pub struct MediaStats {
    pub state: MediaState,
    pub frames_encrypted: u64,
    pub frames_sent: u64,
    pub frames_dropped: u64,
    pub bytes_encrypted: u64,
    pub bytes_sent: u64,
    pub last_keyframe_pts_ms: Option<i64>,
}

#[allow(clippy::derivable_impls)]
impl Default for MediaState {
    fn default() -> Self {
        MediaState::Idle
    }
}
