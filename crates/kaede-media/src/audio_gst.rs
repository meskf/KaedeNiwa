//! GStreamer 音频采集源。
//!
//! # pipeline
//!
//! ```text
//! wasapi2src loopback=true low-latency=true !
//! audioconvert ! audioresample !
//! audio/x-raw,format=S16LE,channels=N,rate=R !
//! audiobuffersplit output-buffer-duration=D/1000 !
//! appsink
//! ```
//!
//! # PID 过滤
//!
//! Windows 原生 AudioSession API 能按 PID 采集（旧 Rivulet 的
//! `AppAudioCapture` 就是这个），但 GStreamer 的 `wasapi2src` 目前
//! **只支持设备级 loopback**，不支持按进程过滤。
//!
//! 传 `pid: Some(_)` 会返回 [`MediaError::Capture`]，UI 应回退到
//! 「共享全系统声音」模式。

use crossbeam_channel::{Receiver, TrySendError, bounded};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app::AppSink;

use crate::error::MediaError;
use crate::source::{AudioCaptureConfig, AudioFrame, AudioSource, AudioSourceKind};

/// GStreamer 音频源。
pub struct GstAudioSource {
    pipeline: gst::Pipeline,
    rx: Receiver<AudioFrame>,
    stopped: Arc<AtomicBool>,
    sample_rate: u32,
    channels: u16,
}

impl GstAudioSource {
    pub fn start(config: AudioCaptureConfig) -> Result<Self, MediaError> {
        gst::init().map_err(|e| MediaError::Capture(format!("gst init: {}", e)))?;

        if config.pid.is_some() {
            return Err(MediaError::Capture(
                "PID-based audio capture not supported by wasapi2src; use system loopback".into(),
            ));
        }

        // 源元素：系统 loopback vs 麦克风输入。
        let src = match config.kind {
            AudioSourceKind::SystemLoopback => "wasapi2src loopback=true low-latency=true",
            AudioSourceKind::Microphone => "wasapi2src low-latency=true",
        };

        let pipeline_str = format!(
            "{src} ! \
             audioconvert ! audioresample ! \
             audio/x-raw,format=S16LE,channels={ch},rate={sr} ! \
             audiobuffersplit output-buffer-duration={dur}/1000 ! \
             appsink name=audiosink sync=false max-buffers=8 drop=true emit-signals=false",
            src = src,
            ch = config.channels,
            sr = config.sample_rate,
            dur = config.frame_duration_ms,
        );

        let pipeline = gst::parse::launch(&pipeline_str)
            .map_err(|e| MediaError::Capture(format!("parse: {}", e)))?
            .downcast::<gst::Pipeline>()
            .map_err(|_| MediaError::Capture("not a pipeline".into()))?;

        let appsink = pipeline
            .by_name("audiosink")
            .ok_or_else(|| MediaError::Capture("no audiosink".into()))?
            .downcast::<AppSink>()
            .map_err(|_| MediaError::Capture("not an appsink".into()))?;

        // H20: bounded + try_send 而非 unbounded + blocking send。
        // 原 std::sync::mpsc::Sender::send 在下游阻塞时会让 GStreamer
        // streaming thread 卡死 → 整个 pipeline 停摆。
        // 队列容量 8 ≈ 160ms @20ms/frame。
        let (tx, rx) = bounded::<AudioFrame>(8);
        let stopped = Arc::new(AtomicBool::new(false));
        let sample_rate = config.sample_rate;
        let channels = config.channels;

        let stopped_cb = stopped.clone();
        appsink.set_callbacks(
            gstreamer_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    if stopped_cb.load(Ordering::Relaxed) {
                        return Err(gst::FlowError::Eos);
                    }
                    let Ok(sample) = sink.pull_sample() else {
                        return Ok(gst::FlowSuccess::Ok);
                    };
                    let Some(buffer) = sample.buffer() else {
                        return Ok(gst::FlowSuccess::Ok);
                    };
                    let Ok(map) = buffer.map_readable() else {
                        return Ok(gst::FlowSuccess::Ok);
                    };
                    let data = map.as_slice().to_vec();
                    let pts_ms = buffer.pts().map(|p| p.mseconds() as i64).unwrap_or(0);
                    let frame = AudioFrame {
                        data,
                        pts_ms,
                        sample_rate,
                        channels,
                    };
                    // H20: try_send —— 队列满时静默丢帧，绝不阻塞 streaming thread
                    match tx.try_send(frame) {
                        Ok(()) => Ok(gst::FlowSuccess::Ok),
                        Err(TrySendError::Full(_)) => Ok(gst::FlowSuccess::Ok),
                        Err(TrySendError::Disconnected(_)) => Err(gst::FlowError::Eos),
                    }
                })
                .build(),
        );

        pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| MediaError::Capture(format!("set_state: {}", e)))?;

        Ok(Self {
            pipeline,
            rx,
            stopped,
            sample_rate,
            channels,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }
}

impl AudioSource for GstAudioSource {
    fn next_audio_blocking(&mut self) -> Result<Option<AudioFrame>, MediaError> {
        match self.rx.recv() {
            Ok(f) => Ok(Some(f)),
            Err(_) => Ok(None),
        }
    }

    fn next_audio_timeout(
        &mut self,
        timeout: std::time::Duration,
    ) -> Result<Option<AudioFrame>, MediaError> {
        match self.rx.recv_timeout(timeout) {
            Ok(f) => Ok(Some(f)),
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => Err(MediaError::Timeout),
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => Ok(None),
        }
    }

    fn stop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        let _ = self.pipeline.send_event(gst::event::Eos::new());
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

impl Drop for GstAudioSource {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}
