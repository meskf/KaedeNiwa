//! H.264 解码器（GStreamer）。
//!
//! # Pipeline
//!
//! ```text
//! appsrc ! h264parse ! avdec_h264 ! videoconvert ! autovideosink
//! ```
//!
//! `avdec_h264` 需要 GStreamer 的 `gst-libav` 插件。
//! 若插件缺失，[`VideoDecoder::new`] 会返回错误。

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app::AppSrc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::MediaError;

/// H.264 解码器。
pub struct VideoDecoder {
    pipeline: gst::Pipeline,
    appsrc: AppSrc,
    frames_pushed: AtomicU64,
}

impl VideoDecoder {
    pub fn new() -> Result<Self, MediaError> {
        gst::init().map_err(|e| MediaError::Capture(format!("gst init: {:?}", e)))?;

        let appsrc = gst::ElementFactory::make("appsrc")
            .name("video-src")
            .property("format", gst::Format::Time)
            .property("is-live", true)
            .build()
            .map_err(|e| MediaError::Capture(format!("appsrc: {:?}", e)))?
            .downcast::<AppSrc>()
            .map_err(|_| MediaError::Capture("not an AppSrc".into()))?;

        let caps = gst::Caps::builder("video/x-h264")
            .field("stream-format", "byte-stream")
            .field("alignment", "au")
            .build();
        appsrc.set_caps(Some(&caps));

        let h264parse = gst::ElementFactory::make("h264parse")
            .build()
            .map_err(|e| MediaError::Capture(format!("h264parse: {:?}", e)))?;
        let decoder = gst::ElementFactory::make("avdec_h264")
            .build()
            .map_err(|e| MediaError::Capture(format!("avdec_h264 (need gst-libav): {:?}", e)))?;
        let convert = gst::ElementFactory::make("videoconvert")
            .build()
            .map_err(|e| MediaError::Capture(format!("videoconvert: {:?}", e)))?;
        let sink = gst::ElementFactory::make("autovideosink")
            .build()
            .map_err(|e| MediaError::Capture(format!("autovideosink: {:?}", e)))?;

        let pipeline = gst::Pipeline::new();
        let elements: [&gst::Element; 5] =
            [appsrc.upcast_ref(), &h264parse, &decoder, &convert, &sink];
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
            frames_pushed: AtomicU64::new(0),
        })
    }

    /// 推入一帧 H.264 Annex B 数据。
    pub fn push(&self, data: &[u8], pts_ms: i64) -> Result<(), MediaError> {
        let mut buf = gst::Buffer::with_size(data.len())
            .map_err(|e| MediaError::Capture(format!("buffer: {:?}", e)))?;
        {
            let buf_ref = buf.get_mut().unwrap();
            buf_ref
                .copy_from_slice(0, data)
                .map_err(|e| MediaError::Capture(format!("copy: {}", e)))?;
            let pts = gst::ClockTime::from_mseconds(pts_ms.max(0) as u64);
            buf_ref.set_pts(pts);
            buf_ref.set_duration(gst::ClockTime::from_mseconds(16));
        }
        self.appsrc
            .push_buffer(buf)
            .map_err(|e| MediaError::Capture(format!("push_buffer: {:?}", e)))?;
        self.frames_pushed.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    #[allow(dead_code)]
    pub fn frames_pushed(&self) -> u64 {
        self.frames_pushed.load(Ordering::Relaxed)
    }

    pub fn stop(&self) {
        let _ = self.appsrc.end_of_stream();
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

impl Drop for VideoDecoder {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}
