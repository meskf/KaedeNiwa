//! 视频画质档位定义。
//!
//! **不包含硬件探测逻辑**——所有候选档位都预先列出，
//! 由调用方（UI / media 层）根据实际场景自行过滤或完整展示。
//!
//! 设计原则：
//! - 分辨率从高到低：8K → 4K → 1440p → 1080p → 720p → 480p
//! - 每个分辨率都包含最低 30fps 的档位
//! - 像素率最高 8K120（NVENC 双编码器 + AV1 理论上限）
//! - 向下兼容到 480p30（低带宽 / 低端手机兜底）

use serde::{Deserialize, Serialize};

/// 一档视频编码参数。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VideoProfile {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

impl VideoProfile {
    pub const fn new(width: u32, height: u32, fps: u32) -> Self {
        Self { width, height, fps }
    }

    /// 人类可读标签，例："8K 120fps" / "1440p 60fps"。
    pub fn label(&self) -> String {
        let res = match (self.width, self.height) {
            (7680, 4320) => "8K",
            (3840, 2160) => "4K",
            (2560, 1440) => "1440p",
            (1920, 1080) => "1080p",
            (1280, 720) => "720p",
            (854, 480) => "480p",
            _ => return format!("{}x{} {}fps", self.width, self.height, self.fps),
        };
        format!("{} {}fps", res, self.fps)
    }

    /// 像素吞吐率（像素/秒）。用于按性能排序或估算码率。
    pub fn pixel_rate(&self) -> u64 {
        self.width as u64 * self.height as u64 * self.fps as u64
    }
}

/// 全部候选档位（从高到低排列）。
///
/// 每个分辨率都覆盖从最高帧率到 30fps 的完整阶梯。
pub const ALL_VIDEO_PROFILES: &[VideoProfile] = &[
    // ── 8K ──
    VideoProfile::new(7680, 4320, 120),
    VideoProfile::new(7680, 4320, 60),
    VideoProfile::new(7680, 4320, 30),
    // ── 4K ──
    VideoProfile::new(3840, 2160, 240),
    VideoProfile::new(3840, 2160, 120),
    VideoProfile::new(3840, 2160, 60),
    VideoProfile::new(3840, 2160, 30),
    // ── 1440p ──
    VideoProfile::new(2560, 1440, 240),
    VideoProfile::new(2560, 1440, 120),
    VideoProfile::new(2560, 1440, 60),
    VideoProfile::new(2560, 1440, 30),
    // ── 1080p ──
    VideoProfile::new(1920, 1080, 240),
    VideoProfile::new(1920, 1080, 120),
    VideoProfile::new(1920, 1080, 60),
    VideoProfile::new(1920, 1080, 30),
    // ── 720p ──
    VideoProfile::new(1280, 720, 120),
    VideoProfile::new(1280, 720, 60),
    VideoProfile::new(1280, 720, 30),
    // ── 480p ──
    VideoProfile::new(854, 480, 60),
    VideoProfile::new(854, 480, 30),
];

/// 默认档位：1080p60。适合大多数场景，手机/桌面都能稳跑。
pub const DEFAULT_VIDEO_PROFILE: VideoProfile = VideoProfile::new(1920, 1080, 60);

/// 码率档位（每个分辨率档位有三档码率可选）。
///
/// - Low：节流，慢网络 / 低端设备
/// - Standard：默认，平衡画质与带宽
/// - Lossless：视觉无损，适合大屏 / 静态内容 / 局域网
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VideoBitrate {
    Low,
    Standard,
    Lossless,
}

impl VideoBitrate {
    pub fn label(self) -> &'static str {
        match self {
            Self::Low => "低",
            Self::Standard => "标准",
            Self::Lossless => "无损",
        }
    }

    pub fn all() -> &'static [Self] {
        &[Self::Low, Self::Standard, Self::Lossless]
    }
}

/// 默认码率档位。
pub const DEFAULT_VIDEO_BITRATE: VideoBitrate = VideoBitrate::Standard;

impl VideoProfile {
    /// 估算目标码率（bps）。
    ///
    /// 基于像素率 × 每像素比特系数（0.032），再按档位缩放：
    /// - Low:      0.4x
    /// - Standard: 1.0x
    /// - Lossless: 3.0x
    ///
    /// 0.032 bpp 的经验依据：1080p60 ≈ 4 Mbps（屏幕共享 / H.264 中等预设）。
    /// 实际编码器可能偏离 ±30%，此值用于 UI 展示和带宽预估。
    pub fn bitrate_bps(&self, quality: VideoBitrate) -> u64 {
        let base = (self.pixel_rate() as f64 * 0.032) as u64;
        match quality {
            VideoBitrate::Low => (base as f64 * 0.4) as u64,
            VideoBitrate::Standard => base,
            VideoBitrate::Lossless => base * 3,
        }
    }

    /// 人类可读码率标签，例："4.0 Mbps" / "950 kbps"。
    pub fn bitrate_label(&self, quality: VideoBitrate) -> String {
        let bps = self.bitrate_bps(quality);
        if bps >= 1_000_000 {
            format!("{:.1} Mbps", bps as f64 / 1_000_000.0)
        } else {
            format!("{:.0} kbps", bps as f64 / 1_000.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_profiles_are_descending() {
        // 排序规则：分辨率降序优先，同分辨率内帧率降序。
        // 注意：不能用 pixel_rate 比较，因为 8K30 的像素率 < 4K240，
        // 但按分辨率优先，8K30 应排在 4K240 前面。
        for w in ALL_VIDEO_PROFILES.windows(2) {
            let (a, b) = (w[0], w[1]);
            assert!(
                (a.width, a.height, a.fps) > (b.width, b.height, b.fps),
                "profiles not sorted descending: {:?} vs {:?}",
                a,
                b
            );
        }
    }

    #[test]
    fn every_resolution_has_30fps_floor() {
        // 每个分辨率都必须有 30fps 的档位
        let resolutions: &[(u32, u32)] = &[
            (7680, 4320),
            (3840, 2160),
            (2560, 1440),
            (1920, 1080),
            (1280, 720),
            (854, 480),
        ];
        for &(w, h) in resolutions {
            assert!(
                ALL_VIDEO_PROFILES
                    .iter()
                    .any(|p| p.width == w && p.height == h && p.fps == 30),
                "resolution {}x{} missing 30fps profile",
                w,
                h
            );
        }
    }

    #[test]
    fn label_reads_well() {
        assert_eq!(VideoProfile::new(7680, 4320, 120).label(), "8K 120fps");
        assert_eq!(VideoProfile::new(1920, 1080, 60).label(), "1080p 60fps");
        assert_eq!(VideoProfile::new(1234, 567, 30).label(), "1234x567 30fps");
    }

    #[test]
    fn bitrate_labels_are_reasonable() {
        let p = VideoProfile::new(1920, 1080, 60);
        let std = p.bitrate_label(VideoBitrate::Standard);
        assert!(
            std.contains("Mbps"),
            "1080p60 standard should be Mbps, got {}",
            std
        );
        assert!(p.bitrate_bps(VideoBitrate::Lossless) > p.bitrate_bps(VideoBitrate::Standard));
        assert!(p.bitrate_bps(VideoBitrate::Standard) > p.bitrate_bps(VideoBitrate::Low));
    }

    #[test]
    fn bitrate_scales_with_resolution() {
        let hd = VideoProfile::new(1920, 1080, 60);
        let uhd = VideoProfile::new(3840, 2160, 60);
        assert!(uhd.bitrate_bps(VideoBitrate::Standard) > hd.bitrate_bps(VideoBitrate::Standard));
    }

    #[test]
    fn default_profile_is_valid() {
        assert!(ALL_VIDEO_PROFILES.contains(&DEFAULT_VIDEO_PROFILE));
        assert_eq!(DEFAULT_VIDEO_PROFILE.label(), "1080p 60fps");
    }
}
