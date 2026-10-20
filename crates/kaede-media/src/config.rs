//! 媒体会话配置。

/// 屏幕共享配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenShareConfig {
    /// 目标分辨率。`None` = 原生分辨率。
    pub resolution: Option<(u32, u32)>,
    /// 目标帧率。
    pub fps: u32,
    /// 码率（kbps）。
    pub bitrate_kbps: u32,
    /// 是否采集光标。
    pub show_cursor: bool,
    /// 显示器索引（0 = 主显示器）。
    pub display_index: u32,
}

impl Default for ScreenShareConfig {
    fn default() -> Self {
        Self {
            resolution: Some((1920, 1080)),
            fps: 60,
            bitrate_kbps: 8000,
            show_cursor: true,
            display_index: 0,
        }
    }
}

/// 音频共享配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioShareConfig {
    pub sample_rate: u32,
    pub channels: u16,
    /// `Some(pid)` = 捕获指定进程（一起听歌）；
    /// `None` = 捕获整个系统的 loopback。
    pub pid: Option<u32>,
}

impl Default for AudioShareConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48000,
            channels: 2,
            pid: None,
        }
    }
}
