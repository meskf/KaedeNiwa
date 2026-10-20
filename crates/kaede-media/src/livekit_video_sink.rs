//! LiveKit 视频显示 sink（GStreamer）。
//!
//! LiveKit SDK 的 `NativeVideoStream` 已经解码成 `BoxVideoFrame`。
//! 本 sink 用 **I420**（YUV 3 平面）传输，避开 RGB 字节序歧义：
//! libwebrtc `to_argb()` 的通道顺序与 GStreamer caps 命名有多套约定，
//! 而 I420 是标准 YUV，无字节序问题。
//!
//! # pipeline
//!
//! ```text
//! appsrc(I420) ! videoscale ! capsfilter(1280x720) ! videoconvert ! autovideosink
//! ```

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app::AppSrc;

use crate::error::MediaError;

pub struct LiveKitVideoSink {
    pipeline: gst::Pipeline,
    appsrc: AppSrc,
    caps_set: AtomicBool,
    frames_pushed: AtomicU64,
    frames_dropped: AtomicU64,
    last_width: AtomicU64,
    last_height: AtomicU64,
}

impl LiveKitVideoSink {
    pub fn new() -> Result<Self, MediaError> {
        gst::init().map_err(|e| MediaError::Capture(format!("gst init: {:?}", e)))?;

        let appsrc = gst::ElementFactory::make("appsrc")
            .name("lk-video-src")
            .build()
            .map_err(|e| MediaError::Capture(format!("appsrc: {:?}", e)))?
            .downcast::<AppSrc>()
            .map_err(|_| MediaError::Capture("not an AppSrc".into()))?;

        appsrc.set_property("is-live", true);
        appsrc.set_property("format", gst::Format::Time);
        appsrc.set_property("block", false);
        appsrc.set_property("max-bytes", 64u64 * 1024 * 1024);
        appsrc.set_property_from_str("leaky-type", "downstream");

        let videoscale = gst::ElementFactory::make("videoscale")
            .build()
            .map_err(|e| MediaError::Capture(format!("videoscale: {:?}", e)))?;
        let capsfilter = gst::ElementFactory::make("capsfilter")
            .property(
                "caps",
                gst::Caps::builder("video/x-raw")
                    .field("width", 1280i32)
                    .field("height", 720i32)
                    .build(),
            )
            .build()
            .map_err(|e| MediaError::Capture(format!("capsfilter: {:?}", e)))?;
        let convert = gst::ElementFactory::make("videoconvert")
            .build()
            .map_err(|e| MediaError::Capture(format!("videoconvert: {:?}", e)))?;
        let sink = gst::ElementFactory::make("autovideosink")
            .property("sync", false)
            .build()
            .map_err(|e| MediaError::Capture(format!("autovideosink: {:?}", e)))?;

        let pipeline = gst::Pipeline::new();
        let elements: [&gst::Element; 5] = [
            appsrc.upcast_ref(),
            &videoscale,
            &capsfilter,
            &convert,
            &sink,
        ];
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
            caps_set: AtomicBool::new(false),
            frames_pushed: AtomicU64::new(0),
            frames_dropped: AtomicU64::new(0),
            last_width: AtomicU64::new(0),
            last_height: AtomicU64::new(0),
        })
    }

    /// 推一帧 I420。`data` 必须是 `Y || U || V` 紧凑拼接
    /// （Y = w*h, U = V = (w/2)*(h/2)）。
    ///
    /// 返回 `Ok(true)` = 入队，`Ok(false)` = 反压或非法尺寸丢弃。
    pub fn push_i420(
        &self,
        data: &[u8],
        width: u32,
        height: u32,
        pts_ms: i64,
    ) -> Result<bool, MediaError> {
        if width == 0 || height == 0 || !width.is_multiple_of(2) || !height.is_multiple_of(2) {
            return Ok(false);
        }
        let expected = (width * height) as usize + 2 * ((width / 2) * (height / 2)) as usize;
        if data.len() < expected {
            return Err(MediaError::Capture(format!(
                "I420 data too short: {} < {}",
                data.len(),
                expected
            )));
        }

        let prev_w = self.last_width.load(Ordering::Relaxed) as u32;
        let prev_h = self.last_height.load(Ordering::Relaxed) as u32;
        if !self.caps_set.load(Ordering::Relaxed) || prev_w != width || prev_h != height {
            let caps = gst::Caps::builder("video/x-raw")
                .field("format", "I420")
                .field("width", width as i32)
                .field("height", height as i32)
                .build();
            self.appsrc.set_caps(Some(&caps));
            self.last_width.store(width as u64, Ordering::Relaxed);
            self.last_height.store(height as u64, Ordering::Relaxed);
            self.caps_set.store(true, Ordering::Relaxed);
        }

        let mut buffer = gst::Buffer::with_size(expected)
            .map_err(|e| MediaError::Capture(format!("buffer: {:?}", e)))?;
        {
            let buf_ref = buffer.get_mut().unwrap();
            buf_ref
                .copy_from_slice(0, &data[..expected])
                .map_err(|e| MediaError::Capture(format!("copy: {}", e)))?;
            let pts = gst::ClockTime::from_mseconds(pts_ms.max(0) as u64);
            buf_ref.set_pts(pts);
            buf_ref.set_duration(gst::ClockTime::from_mseconds(33));
        }

        match self.appsrc.push_buffer(buffer) {
            Ok(_) => {
                self.frames_pushed.fetch_add(1, Ordering::Relaxed);
                Ok(true)
            }
            Err(_) => {
                self.frames_dropped.fetch_add(1, Ordering::Relaxed);
                Ok(false)
            }
        }
    }

    pub fn frames_pushed(&self) -> u64 {
        self.frames_pushed.load(Ordering::Relaxed)
    }

    pub fn frames_dropped(&self) -> u64 {
        self.frames_dropped.load(Ordering::Relaxed)
    }

    pub fn stop(&self) {
        let _ = self.appsrc.end_of_stream();
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

impl Drop for LiveKitVideoSink {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}
