//! SFrame key 派生的 purpose 维度。
//!
//! # 背景
//!
//! 同一个 MLS group 里多个 sender（Alice / Bob）用同一个 exporter
//! 派生同一个 SFrame key。如果每个 sender 都从 counter=0 开始加密，
//! 会撞 AES-GCM nonce。
//!
//! 同一个 sender 同时开音频 + 视频 + 屏幕共享，也用同一个 channel key，
//! 同样从 counter=0 开始，也撞 nonce。
//!
//! # 解决
//!
//! exporter context = `channel_id || 0x00 || purpose`
//!
//! 每个 purpose 派生独立 key，nonce 空间自然分离。
//! `0x00` 是分隔符，保证 (channel_id, purpose) -> context 是单射。

/// SFrame key 的用途。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SframePurpose {
    /// 向后兼容旧调用。不要在新代码里用。
    Default,
    /// 文本（未来可能，现在文本走 MLS App Message）。
    Text,
    /// 语音 — 麦克风。
    VoiceAudio,
    /// 语音 — 摄像头。
    VoiceVideo,
    /// 屏幕共享 — 画面。
    ScreenVideo,
    /// 屏幕共享 — 系统声音。
    ScreenAudio,
    /// 文件传输（未来）。
    File,
    /// 实时媒体 E2EE（LiveKit 内建 E2EE）。与 SFrame 用途隔离。
    LivekitE2ee,
    /// DM（1v1 私聊）—— B3 方案：per-conv 对称密钥。
    ///
    /// exporter context = `conv_id || 0x00 || "dm"`。
    /// **隔离级别**：DM 与群频道、与其他 DM、与媒体 key 均独立。
    /// 但任何 MLS group 成员都能由 conv_id 推导出 key——防"密钥复用"，
    /// 不防"恶意群成员主动推导"。若需密码学隔离，升级为每对好友独立 2 人 MLS group。
    Dm,
}

impl SframePurpose {
    /// 常量字节表示。写进 exporter context，永不改。
    pub const fn as_bytes(self) -> &'static [u8] {
        match self {
            Self::Default => b"default",
            Self::Text => b"text",
            Self::VoiceAudio => b"voice-audio",
            Self::VoiceVideo => b"voice-video",
            Self::ScreenVideo => b"screen-video",
            Self::ScreenAudio => b"screen-audio",
            Self::File => b"file",
            Self::LivekitE2ee => b"livekit-e2ee",
            Self::Dm => b"dm",
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Text => "text",
            Self::VoiceAudio => "voice-audio",
            Self::VoiceVideo => "voice-video",
            Self::ScreenVideo => "screen-video",
            Self::ScreenAudio => "screen-audio",
            Self::File => "file",
            Self::LivekitE2ee => "livekit-e2ee",
            Self::Dm => "dm",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_purposes_distinct() {
        let all = [
            SframePurpose::Default,
            SframePurpose::Text,
            SframePurpose::VoiceAudio,
            SframePurpose::VoiceVideo,
            SframePurpose::ScreenVideo,
            SframePurpose::ScreenAudio,
            SframePurpose::File,
            SframePurpose::LivekitE2ee,
            SframePurpose::Dm,
        ];
        for i in 0..all.len() {
            for j in (i + 1)..all.len() {
                assert_ne!(
                    all[i].as_bytes(),
                    all[j].as_bytes(),
                    "purpose {} and {} collide",
                    all[i].as_str(),
                    all[j].as_str()
                );
            }
        }
    }

    #[test]
    fn no_purpose_contains_nul() {
        let all = [
            SframePurpose::Default,
            SframePurpose::Text,
            SframePurpose::VoiceAudio,
            SframePurpose::VoiceVideo,
            SframePurpose::ScreenVideo,
            SframePurpose::ScreenAudio,
            SframePurpose::File,
            SframePurpose::LivekitE2ee,
            SframePurpose::Dm,
        ];
        for p in all {
            assert!(!p.as_bytes().contains(&0u8), "{:?} contains NUL", p);
        }
    }
}
