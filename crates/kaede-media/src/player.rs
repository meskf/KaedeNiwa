//! 音频播放器（GStreamer）。
//!
//! # Pipeline
//!
//! ```text
//! appsrc ! audioconvert ! audioresample ! autoaudiosink
//! ```

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app::AppSrc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::MediaError;

pub struct AudioPlayer {
    pipeline: gst::Pipeline,
    appsrc: AppSrc,
    volume: gst::Element,
    #[allow(dead_code)]
    sample_rate: u32,
    #[allow(dead_code)]
    channels: u16,
    frames_pushed: AtomicU64,
}

impl AudioPlayer {
    pub fn new(sample_rate: u32, channels: u16) -> Result<Self, MediaError> {
        gst::init().map_err(|e| MediaError::Capture(format!("gst init: {:?}", e)))?;

        let appsrc = gst::ElementFactory::make("appsrc")
            .name("audio-src")
            .property("format", gst::Format::Time)
            .property("is-live", true)
            .build()
            .map_err(|e| MediaError::Capture(format!("appsrc: {:?}", e)))?
            .downcast::<AppSrc>()
            .map_err(|_| MediaError::Capture("not an AppSrc".into()))?;

        let caps = gst::Caps::builder("audio/x-raw")
            .field("format", "S16LE")
            .field("channels", channels as i32)
            .field("rate", sample_rate as i32)
            .build();
        appsrc.set_caps(Some(&caps));

        let convert = gst::ElementFactory::make("audioconvert")
            .build()
            .map_err(|e| MediaError::Capture(format!("audioconvert: {:?}", e)))?;
        let resample = gst::ElementFactory::make("audioresample")
            .build()
            .map_err(|e| MediaError::Capture(format!("audioresample: {:?}", e)))?;
        let volume = gst::ElementFactory::make("volume")
            .build()
            .map_err(|e| MediaError::Capture(format!("volume: {:?}", e)))?;
        let sink = gst::ElementFactory::make("autoaudiosink")
            .build()
            .map_err(|e| MediaError::Capture(format!("autoaudiosink: {:?}", e)))?;

        let pipeline = gst::Pipeline::new();
        let elements: [&gst::Element; 5] =
            [appsrc.upcast_ref(), &convert, &resample, &volume, &sink];
        pipeline
            .add_many(elements)
            .map_err(|e| MediaError::Capture(format!("add_many: {:?}", e)))?;
        gst::Element::link_many(elements)
            .map_err(|e| MediaError::Capture(format!("link_many: {:?}", e)))?;

        pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| MediaError::Capture(format!("set_state: {:?}", e)))?;

        Ok(Self {
            pipeline,
            appsrc,
            volume,
            sample_rate,
            channels,
            frames_pushed: AtomicU64::new(0),
        })
    }

    pub fn push(&self, pcm: &[u8], pts_ms: i64) -> Result<(), MediaError> {
        let mut buf = gst::Buffer::with_size(pcm.len())
            .map_err(|e| MediaError::Capture(format!("buffer: {:?}", e)))?;
        {
            let buf_ref = buf.get_mut().unwrap();
            buf_ref
                .copy_from_slice(0, pcm)
                .map_err(|e| MediaError::Capture(format!("copy: {}", e)))?;
            buf_ref.set_pts(gst::ClockTime::from_mseconds(pts_ms.max(0) as u64));
        }
        self.appsrc
            .push_buffer(buf)
            .map_err(|e| MediaError::Capture(format!("push_buffer: {:?}", e)))?;
        self.frames_pushed.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    #[allow(dead_code)]
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    #[allow(dead_code)]
    pub fn channels(&self) -> u16 {
        self.channels
    }
    #[allow(dead_code)]
    pub fn frames_pushed(&self) -> u64 {
        self.frames_pushed.load(Ordering::Relaxed)
    }

    /// 静音/恢复远端音频输出（不影响采集与传输）。
    pub fn set_muted(&self, muted: bool) {
        self.volume.set_property("mute", muted);
    }

    pub fn stop(&self) {
        let _ = self.appsrc.end_of_stream();
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}
