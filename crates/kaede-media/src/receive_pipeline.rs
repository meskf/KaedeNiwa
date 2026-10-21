//! 接收端管道：加密帧 → 解密 → 解码 / 播放。
//!
//! 与 `MediaSession` 对称。`MediaSession` 管发送（采集 → 加密 → sink）；
//! `ReceivePipeline` 管接收（source → 解密 → 显示/播放）。
//!
//! # 模式
//!
//! - `Video`：解密后送 H.264 解码器 → `autovideosink`
//! - `Audio { sample_rate, channels }`：解密后送 PCM 播放器 → `autoaudiosink`
//! - `DecryptOnly`：只解密，不做解码/播放（用于测试 / 手动处理）

use std::sync::atomic::{AtomicU64, Ordering};

use kaede_crypto::sframe::SframeKey;

use crate::error::MediaError;
use crate::receiver::{DecryptedFrame, ReceiveError, ReceiveStats, ReceiverSession};
use crate::sink::EncryptedFrame;

#[cfg(not(target_arch = "wasm32"))]
use std::sync::Mutex;

#[cfg(not(target_arch = "wasm32"))]
use crate::{decoder::VideoDecoder, player::AudioPlayer};

/// 接收模式。
pub enum ReceiveMode {
    /// H.264 视频解码 + 自动显示。
    Video,
    /// PCM S16LE 音频播放。
    Audio { sample_rate: u32, channels: u16 },
    /// 只解密，不转发。
    DecryptOnly,
}

/// 接收端管道。
pub struct ReceivePipeline {
    receiver: ReceiverSession,
    #[allow(dead_code)]
    mode: ReceiveMode,
    #[cfg(not(target_arch = "wasm32"))]
    video: Mutex<Option<VideoDecoder>>,
    #[cfg(not(target_arch = "wasm32"))]
    audio: Mutex<Option<AudioPlayer>>,
    forwarded: AtomicU64,
}

impl ReceivePipeline {
    pub fn start(key: SframeKey, key_id: u32, mode: ReceiveMode) -> Result<Self, MediaError> {
        let receiver = ReceiverSession::new(key, key_id);

        #[cfg(target_arch = "wasm32")]
        {
            let _ = mode;
            return Err(MediaError::Capture(
                "decoder/player not supported on wasm (use DecryptOnly)".into(),
            ));
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let (video, audio) = match &mode {
                ReceiveMode::Video => (Some(VideoDecoder::new()?), None),
                ReceiveMode::Audio {
                    sample_rate,
                    channels,
                } => (None, Some(AudioPlayer::new(*sample_rate, *channels)?)),
                ReceiveMode::DecryptOnly => (None, None),
            };
            Ok(Self {
                receiver,
                mode,
                video: Mutex::new(video),
                audio: Mutex::new(audio),
                forwarded: AtomicU64::new(0),
            })
        }
    }

    /// 解密一帧并转发给解码器 / 播放器。
    pub fn decrypt_and_forward(
        &self,
        frame: &EncryptedFrame,
    ) -> Result<DecryptedFrame, ReceiveError> {
        let dec = self.receiver.decrypt(frame)?;

        #[cfg(not(target_arch = "wasm32"))]
        match &self.mode {
            ReceiveMode::Video => {
                if let Some(v) = self
                    .video
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_ref()
                {
                    v.push(&dec.data, dec.pts_ms)
                        .map_err(|e| ReceiveError::Decode(e.to_string()))?;
                }
            }
            ReceiveMode::Audio { .. } => {
                if let Some(a) = self
                    .audio
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_ref()
                {
                    a.push(&dec.data, dec.pts_ms)
                        .map_err(|e| ReceiveError::Decode(e.to_string()))?;
                }
            }
            ReceiveMode::DecryptOnly => {}
        }

        self.forwarded.fetch_add(1, Ordering::Relaxed);
        Ok(dec)
    }

    pub fn stats(&self) -> ReceiveStats {
        self.receiver.stats()
    }

    pub fn forwarded(&self) -> u64 {
        self.forwarded.load(Ordering::Relaxed)
    }

    pub fn stop(&self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(v) = self
                .video
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
            {
                v.stop();
            }
            if let Some(a) = self
                .audio
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
            {
                a.stop();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kaede_crypto::sframe::{Counter, KeyId, encrypt, make_counter_start, sender_id_from_uid};

    const KEY: [u8; 32] = [0xAB; 32];
    const KEY_ID: u32 = 7;

    fn make_frame(uid: &str, seq: u32, payload: &[u8]) -> EncryptedFrame {
        let key = SframeKey::from_bytes(KEY);
        let sid = sender_id_from_uid(uid);
        let counter = make_counter_start(sid) + seq as u64;
        let ct = encrypt(&key, KeyId(KEY_ID), Counter(counter), payload).unwrap();
        EncryptedFrame {
            data: ct,
            is_keyframe: seq == 0,
            pts_ms: seq as i64 * 16,
        }
    }

    #[test]
    fn decrypt_only_forwards_frames() {
        let p =
            ReceivePipeline::start(SframeKey::from_bytes(KEY), KEY_ID, ReceiveMode::DecryptOnly)
                .unwrap();
        for i in 0..5 {
            let f = make_frame("alice", i, format!("f{}", i).as_bytes());
            let d = p.decrypt_and_forward(&f).unwrap();
            assert_eq!(d.seq, i);
            assert_eq!(d.sender_id, sender_id_from_uid("alice"));
        }
        assert_eq!(p.forwarded(), 5);
        assert_eq!(p.stats().decrypted, 5);
        assert_eq!(p.stats().rejected, 0);
    }

    #[test]
    fn replay_rejected() {
        let p =
            ReceivePipeline::start(SframeKey::from_bytes(KEY), KEY_ID, ReceiveMode::DecryptOnly)
                .unwrap();
        let f = make_frame("alice", 0, b"x");
        p.decrypt_and_forward(&f).unwrap();
        let err = p.decrypt_and_forward(&f).unwrap_err();
        assert!(matches!(err, ReceiveError::Replay { .. }));
    }
}
