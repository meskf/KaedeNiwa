//! LiveKit 预编码帧 API 验证 spike。
//!
//! 目标：确认 `NativeVideoSource::new_encoded` + `VideoEncoderBackend::PreEncoded`
//! 在当前 livekit crate 版本可用。
//!
//! 不真正推流，只构造对象，验证编译期 API 存在。

use livekit::options::{TrackPublishOptions, VideoCodec, VideoEncoderBackend};
use livekit::prelude::*;
use livekit::webrtc::video_source::{RtcVideoSource, VideoResolution, native::NativeVideoSource};

/// 验证预编码帧 API 是否可用。
pub fn spike_pre_encoded_api() -> Result<(), Box<dyn std::error::Error>> {
    let resolution = VideoResolution {
        width: 1920,
        height: 1080,
    };

    // 1. 创建预编码源
    let _source = NativeVideoSource::new_encoded(resolution);

    // 2. 创建视频轨道
    let _track =
        LocalVideoTrack::create_video_track("screen", RtcVideoSource::Native(_source.clone()));

    // 3. 发布选项
    let _options = TrackPublishOptions {
        video_codec: VideoCodec::H264,
        video_encoder: VideoEncoderBackend::PreEncoded,
        simulcast: false,
        source: TrackSource::Screenshare,
        ..Default::default()
    };

    tracing::info!("pre-encoded API available");
    Ok(())
}

/// 版本信息。
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    // 显式 import 被测函数，避免 use super::* 引入无关 warning
    use super::{spike_pre_encoded_api, version};

    #[test]
    fn api_compiles() {
        let r = spike_pre_encoded_api();
        assert!(r.is_ok());
    }

    #[test]
    fn version_is_set() {
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
    }
}

#[cfg(test)]
mod e2ee_tests {
    use livekit::e2ee::key_provider::{KeyProvider, KeyProviderOptions};
    use livekit::e2ee::{E2eeOptions, EncryptionType};

    #[test]
    fn e2ee_256_key_accepted() {
        // 32 字节密钥必须被 KeyProvider 接受（AES-256）
        let key_32: Vec<u8> = (0u8..32).collect();
        let kp = KeyProvider::with_shared_key(KeyProviderOptions::default(), key_32);
        let _opts = E2eeOptions {
            encryption_type: EncryptionType::Gcm,
            key_provider: kp,
        };
        println!("32-byte key accepted");
    }

    #[test]
    fn e2ee_16_key_accepted() {
        // 16 字节密钥也接受（AES-128 fallback），只构造不推流
        let key_16: Vec<u8> = (0u8..16).collect();
        let _kp = KeyProvider::with_shared_key(KeyProviderOptions::default(), key_16);
        println!("16-byte key accepted");
    }
}
