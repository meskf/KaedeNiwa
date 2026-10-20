//! 会话配置。

/// 连接到 LiveKit 需要的一次性输入。
///
/// UI 侧流程：
/// ```ignore
/// let token = room.get_livekit_token(channel_id).await?;
/// let key   = room.get_livekit_e2ee_key(channel_id).await?;
/// let cfg = ConnectConfig {
///     ws_url: token.ws_url,
///     token:  token.token,
///     e2ee_key: key,
///     room_name: token.room,
/// };
/// let session = MediaSession::connect(cfg).await?;
/// ```
#[derive(Clone)]
pub struct ConnectConfig {
    /// LiveKit 信令 WS URL，例如 `ws://localhost:7880`。
    pub ws_url: String,
    /// 服务端签发的 JWT。
    pub token: String,
    /// E2EE 共享密钥（32 字节 AES-256）。
    pub e2ee_key: [u8; 32],
    /// LiveKit room 名（一般 `kaede-{channel_id}`）。
    pub room_name: String,
}

impl std::fmt::Debug for ConnectConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectConfig")
            .field("ws_url", &self.ws_url)
            .field("token", &"***")
            .field("e2ee_key", &"***")
            .field("room_name", &self.room_name)
            .finish()
    }
}

/// 要发布的源。
pub enum PublishSource {
    /// 屏幕 + NVENC。
    Screen { config: kaede_capture::VideoConfig },
    /// 麦克风。
    Mic { sample_rate: u32, channels: u16 },
    /// 系统 loopback（一起听歌 / 共享系统声音）。
    System { sample_rate: u32, channels: u16 },
}

/// 本地发布的 track 类型（一个 session 每种最多一个）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrackKind {
    ScreenVideo,
    ScreenAudio,
    VoiceAudio,
}

impl TrackKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ScreenVideo => "screen-video",
            Self::ScreenAudio => "screen-audio",
            Self::VoiceAudio => "voice-audio",
        }
    }
}
