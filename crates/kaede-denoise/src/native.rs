//! RNNoise 降噪装饰器（48kHz mono，480 样本/块，约 10ms 延迟）。
//!
//! # 用法
//!
//! `DenoisedSource::new(source)` 包装任意 `AudioSource`，对外仍是 `AudioSource`。
//! 内部把交错 PCM 下混到 mono、逐 480 样本块调 RNNoise、再上混回原声道数。
//!
//! # 限制
//!
//! 只处理 48kHz 输入（RNNoise 只内置 48kHz 模型）。非 48kHz 直接透传。
//!
//! # 延迟
//!
//! 每 480 样本块引入恰好 480 样本（10ms）算法延迟。语音场景可接受。

use nnnoiseless::DenoiseState;

use crate::error::MediaError;
use crate::source::{AudioFrame, AudioSource};

/// RNNoise 每帧样本数（48kHz 下 10ms）。
const RNNOISE_FRAME: usize = 480;

/// RNNoise 支持的采样率。
const RNNOISE_SAMPLE_RATE: u32 = 48_000;

/// RNNoise 降噪装饰器。
pub struct DenoisedSource<S: AudioSource> {
    inner: S,
    denoiser: Box<DenoiseState<'static>>,
    /// 尚未凑满 480 样本块的 mono 缓冲（f32，范围 ±1.0）。
    pending_mono: Vec<f32>,
}

impl<S: AudioSource> DenoisedSource<S> {
    /// 用给定的源创建降噪装饰器。
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            denoiser: DenoiseState::new(),
            pending_mono: Vec::with_capacity(RNNOISE_FRAME * 2),
        }
    }

    fn process_one(&mut self, frame: AudioFrame) -> Result<AudioFrame, MediaError> {
        // RNNoise 只支持 48kHz；其它采样率直接透传
        if frame.sample_rate != RNNOISE_SAMPLE_RATE {
            return Ok(frame);
        }
        let channels = frame.channels.max(1) as usize;
        if frame.data.len() < 2 * channels {
            return Ok(frame);
        }

        // S16LE 交错 → mono f32（多声道平均下混）
        let samples = frame.data.len() / 2;
        let mono_len = samples / channels;
        let mut mono = Vec::with_capacity(mono_len);
        for i in 0..mono_len {
            let mut sum = 0.0f32;
            for c in 0..channels {
                let idx = (i * channels + c) * 2;
                let s = i16::from_le_bytes([frame.data[idx], frame.data[idx + 1]]);
                sum += s as f32 / 32768.0;
            }
            mono.push(sum / channels as f32);
        }

        self.pending_mono.extend_from_slice(&mono);

        let mut processed: Vec<f32> = Vec::with_capacity(self.pending_mono.len());
        let mut in_buf = [0.0f32; RNNOISE_FRAME];
        let mut out_buf = [0.0f32; RNNOISE_FRAME];
        while self.pending_mono.len() >= RNNOISE_FRAME {
            in_buf.copy_from_slice(&self.pending_mono[..RNNOISE_FRAME]);
            self.denoiser.process_frame(&mut out_buf, &in_buf);
            processed.extend_from_slice(&out_buf);
            self.pending_mono.drain(..RNNOISE_FRAME);
        }

        // mono f32 → 原声道数 S16LE
        let out_frames = processed.len();
        let mut out_data = Vec::with_capacity(out_frames * channels * 2);
        for sample in processed {
            let s = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
            let bytes = s.to_le_bytes();
            for _ in 0..channels {
                out_data.extend_from_slice(&bytes);
            }
        }

        Ok(AudioFrame {
            data: out_data,
            pts_ms: frame.pts_ms,
            sample_rate: frame.sample_rate,
            channels: frame.channels,
        })
    }
}

impl<S: AudioSource> AudioSource for DenoisedSource<S> {
    fn next_audio_blocking(&mut self) -> Result<Option<AudioFrame>, MediaError> {
        match self.inner.next_audio_blocking()? {
            Some(f) => self.process_one(f).map(Some),
            None => Ok(None),
        }
    }

    fn next_audio_timeout(
        &mut self,
        timeout: std::time::Duration,
    ) -> Result<Option<AudioFrame>, MediaError> {
        match self.inner.next_audio_timeout(timeout)? {
            Some(f) => self.process_one(f).map(Some),
            None => Ok(None),
        }
    }

    fn stop(&mut self) {
        self.inner.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    struct MockSource {
        frames: Vec<AudioFrame>,
        idx: usize,
    }

    impl AudioSource for MockSource {
        fn next_audio_blocking(&mut self) -> Result<Option<AudioFrame>, MediaError> {
            if self.idx >= self.frames.len() {
                return Ok(None);
            }
            let f = self.frames[self.idx].clone();
            self.idx += 1;
            Ok(Some(f))
        }
        fn next_audio_timeout(
            &mut self,
            _timeout: Duration,
        ) -> Result<Option<AudioFrame>, MediaError> {
            self.next_audio_blocking()
        }
        fn stop(&mut self) {}
    }

    fn frame_20ms_48k_stereo(fill: u8) -> AudioFrame {
        AudioFrame {
            data: vec![fill; 3840],
            pts_ms: 0,
            sample_rate: 48_000,
            channels: 2,
        }
    }

    #[test]
    fn processes_48khz_frame_and_preserves_length() {
        let src = MockSource {
            frames: vec![frame_20ms_48k_stereo(0)],
            idx: 0,
        };
        let mut den = DenoisedSource::new(src);
        let out = den.next_audio_blocking().unwrap().unwrap();
        assert_eq!(out.data.len(), 3840);
        assert_eq!(out.channels, 2);
        assert_eq!(out.sample_rate, 48_000);
    }

    #[test]
    fn non_48khz_passes_through() {
        let mut f = frame_20ms_48k_stereo(0);
        f.sample_rate = 16_000;
        let src = MockSource {
            frames: vec![f],
            idx: 0,
        };
        let mut den = DenoisedSource::new(src);
        let out = den.next_audio_blocking().unwrap().unwrap();
        assert_eq!(out.sample_rate, 16_000);
        assert_eq!(out.data.len(), 3840);
    }

    #[test]
    fn silence_in_produces_near_silence_out() {
        let src = MockSource {
            frames: vec![frame_20ms_48k_stereo(0)],
            idx: 0,
        };
        let mut den = DenoisedSource::new(src);
        let out = den.next_audio_blocking().unwrap().unwrap();
        let max_abs = out
            .data
            .chunks(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]).unsigned_abs())
            .max()
            .unwrap_or(0);
        assert!(
            max_abs < 500,
            "expected near-silence, got max_abs={}",
            max_abs
        );
    }
}
