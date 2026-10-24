//! 编码后的码流（H.264 / HEVC / AV1，不落盘，直接给上层）。
//!
//! # 编码器选择策略（按分辨率和帧率自动）
//!
//! | 分辨率 | 帧率 | 编码器 |
//! |---|---|---|
//! | 8K 任意 | AV1 → HEVC → H.264（8K H264 必败） |
//! | 4K@120+ | AV1 → HEVC |
//! | 4K@60/30 | HEVC → H.264 |
//! | 1440p/1080p@240 | HEVC → H.264 |
//! | 其他 | H.264 |
//!
//! 原因：
//! - H.264 NVENC Level 上限约 5.2（4K@60），8K/4K 高刷物理不支持
//! - HEVC 支持 4K@120/240
//! - AV1 支持 8K（40/50 系 NVENC 才有）

use crate::CaptureError;
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app::AppSink;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

/// 一帧编码后的视频。
#[derive(Debug, Clone)]
pub struct EncodedFrame {
    pub data: Vec<u8>,
    pub is_keyframe: bool,
    pub pts_ms: i64,
}

/// 输出码流的编码格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodec {
    H264,
    HEVC,
    AV1,
}

/// 主显示器分辨率（None 配置时用于编码器档位选择/SDP 通告）。
///
/// Windows 上读取系统指标；其它平台回退 1920×1080。
pub fn primary_display_size() -> (u32, u32) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN,
        };
        // SAFETY: 两个调用都是无副作用的只读系统指标查询。
        let (w, h) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
        if w > 0 && h > 0 {
            return (w as u32, h as u32);
        }
        (1920, 1080)
    }
    #[cfg(not(windows))]
    {
        (1920, 1080)
    }
}

impl VideoCodec {
    /// 根据分辨率和帧率推荐编码格式。
    ///
    /// 用 `max(width, height)` 做判断，横竖屏都能正确识别档位。
    pub fn recommend(width: u32, height: u32, fps: u32) -> Self {
        let longest = width.max(height);
        if longest >= 7680 {
            // 8K：只有 AV1 能实时
            Self::AV1
        } else if longest >= 3840 {
            // 4K：高刷走 AV1，普通走 HEVC
            if fps >= 120 { Self::AV1 } else { Self::HEVC }
        } else if longest >= 1920 && fps >= 240 {
            // 1080p/1440p 超高刷：H.264 支持有限，走 HEVC
            Self::HEVC
        } else {
            Self::H264
        }
    }

    fn parser_element(self) -> &'static str {
        match self {
            Self::H264 => "h264parse config-interval=1",
            Self::HEVC => "h265parse config-interval=1",
            Self::AV1 => "av1parse",
        }
    }

    /// GStreamer caps 字符串。**必须带单引号**，否则 parse_launch 会把
    /// `video/x-raw` 的 `video` 当成元素名，报 `no element "video"`。
    /// 生成 capsfilter 元素字符串。
    ///
    /// **必须用 capsfilter** —— 裸 caps 会被 gst::parse::launch 当成元素名，
    /// 报 "no element video"。capsfilter 的 caps= 属性值不会切分。
    fn output_capsfilter(self) -> &'static str {
        match self {
            Self::H264 => "capsfilter caps=video/x-h264,stream-format=byte-stream,alignment=au",
            Self::HEVC => "capsfilter caps=video/x-h265,stream-format=byte-stream,alignment=au",
            Self::AV1 => "capsfilter caps=video/x-av1,stream-format=obu-stream",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::H264 => "H.264",
            Self::HEVC => "HEVC",
            Self::AV1 => "AV1",
        }
    }
}

/// NVENC 编码器可用性检测 + 降级链。
/// 探测可用的 HEVC 编码器（按优先级）。
///
/// NVENC 的 HEVC 编码器在 GStreamer 里叫 `nvh265enc`（不是 nvhevcenc！）。
fn pick_hevc() -> Option<(&'static str, VideoCodec)> {
    let has = |name: &str| gst::ElementFactory::find(name).is_some();
    if has("nvh265enc") {
        return Some(("nvh265enc", VideoCodec::HEVC));
    }
    if has("nvd3d11h265enc") {
        return Some(("nvd3d11h265enc", VideoCodec::HEVC));
    }
    if has("mfh265enc") {
        return Some(("mfh265enc", VideoCodec::HEVC));
    }
    if has("x265enc") {
        return Some(("x265enc", VideoCodec::HEVC));
    }
    None
}

fn pick_nvenc(requested: VideoCodec) -> (&'static str, VideoCodec) {
    let has = |name: &str| gst::ElementFactory::find(name).is_some();

    match requested {
        VideoCodec::AV1 => {
            if has("nvav1enc") {
                ("nvav1enc", VideoCodec::AV1)
            } else if let Some((elem, codec)) = pick_hevc() {
                tracing::warn!("nvav1enc 不可用；降级到 {}（HEVC）", elem);
                (elem, codec)
            } else {
                tracing::warn!("AV1/HEVC 均不可用；降级到 nvh264enc（8K 会失败）");
                ("nvh264enc", VideoCodec::H264)
            }
        }
        VideoCodec::HEVC => {
            if let Some((elem, codec)) = pick_hevc() {
                (elem, codec)
            } else {
                tracing::warn!("HEVC 均不可用；降级到 nvh264enc");
                ("nvh264enc", VideoCodec::H264)
            }
        }
        VideoCodec::H264 => ("nvh264enc", VideoCodec::H264),
    }
}

/// 构造 NVENC 编码器参数字符串。
fn nvenc_params(element: &str, codec: VideoCodec, bitrate_kbps: u32, fps: u32) -> String {
    let gop = fps * 2;
    match element {
        // NVENC HEVC / H.264：相同的属性集
        "nvh265enc" | "nvd3d11h265enc" | "nvh264enc" | "nvd3d11h264enc" => match codec {
            VideoCodec::H264 | VideoCodec::HEVC => format!(
                "{element} bitrate={bitrate_kbps} rc-mode=cbr zerolatency=true gop-size={gop}"
            ),
            VideoCodec::AV1 => {
                format!("{element} bitrate={bitrate_kbps} rc-mode=cbr gop-size={gop}")
            }
        },
        // Media Foundation HEVC：属性名不同
        "mfh265enc" => format!("{element} bitrate={bitrate_kbps}"),
        // 软件 x265：低延迟预设
        "x265enc" => format!(
            "{element} bitrate={bitrate_kbps} speed-preset=ultrafast tune=zerolatency key-int-max={gop}"
        ),
        // 兜底
        _ => format!("{element} bitrate={bitrate_kbps}"),
    }
}

/// 编码器选择。默认自动选最佳硬件编码器。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncoderKind {
    /// 自动选择（优先 NVENC > QSV > AMF > Software）
    Auto,
    /// NVIDIA NVENC
    Nvenc,
    /// Intel QuickSync
    QuickSync,
    /// AMD AMF
    Amf,
    /// 软件 x264（兜底）
    Software,
}

impl EncoderKind {
    /// 生成 `(encoder_element_with_params, actual_codec)`。
    fn resolve(self, width: u32, height: u32, fps: u32, bitrate_kbps: u32) -> (String, VideoCodec) {
        match self {
            Self::Nvenc | Self::Auto => {
                if gst::ElementFactory::find("nvh264enc").is_some() {
                    let requested = VideoCodec::recommend(width, height, fps);
                    let (elem, actual) = pick_nvenc(requested);
                    let params = nvenc_params(elem, actual, bitrate_kbps, fps);
                    return (params, actual);
                }
                Self::resolve_legacy(fps, bitrate_kbps)
            }
            _ => Self::resolve_legacy(fps, bitrate_kbps),
        }
    }

    fn resolve_legacy(fps: u32, bitrate_kbps: u32) -> (String, VideoCodec) {
        if gst::ElementFactory::find("qsvh264enc").is_some() {
            (
                format!("qsvh264enc bitrate={bitrate_kbps} rate-control=cbr"),
                VideoCodec::H264,
            )
        } else if gst::ElementFactory::find("amfh264enc").is_some() {
            (
                format!("amfh264enc bitrate={bitrate_kbps} rate-control=cbr"),
                VideoCodec::H264,
            )
        } else {
            (
                format!(
                    "x264enc bitrate={bitrate_kbps} speed-preset=ultrafast tune=zerolatency key-int-max={}",
                    fps * 2
                ),
                VideoCodec::H264,
            )
        }
    }
}

/// 视频编码配置。
#[derive(Debug, Clone)]
pub struct VideoConfig {
    pub encoder: EncoderKind,
    pub fps: u32,
    pub bitrate_kbps: u32,
    pub show_cursor: bool,
    pub display_index: u32,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

impl Default for VideoConfig {
    fn default() -> Self {
        Self {
            encoder: EncoderKind::Auto,
            fps: 60,
            bitrate_kbps: 8000,
            show_cursor: true,
            display_index: 0,
            width: None,
            height: None,
        }
    }
}

/// 编码视频流。
pub struct EncodedVideoStream {
    pipeline: gst::Pipeline,
    rx: Receiver<EncodedFrame>,
    stopped: Arc<AtomicBool>,
    frames_sent: Arc<AtomicU64>,
    actual_codec: VideoCodec,
}

/// 尝试构建 GPU 直通管道（全程在 D3D11 内存上，无 CPU 拷贝）。
///
/// 返回 `None` 表示当前编码器没有对应的 D3D11 版本，无法走 GPU 直通。
fn build_gpu_pipeline(config: &VideoConfig, actual_codec: VideoCodec) -> Option<String> {
    let has = |name: &str| gst::ElementFactory::find(name).is_some();

    // GPU 直通要求编码器支持 D3D11Memory 输入
    let enc_elem = match actual_codec {
        VideoCodec::H264 => {
            if !has("nvd3d11h264enc") {
                return None;
            }
            "nvd3d11h264enc"
        }
        VideoCodec::HEVC => {
            if !has("nvd3d11h265enc") {
                return None;
            }
            "nvd3d11h265enc"
        }
        VideoCodec::AV1 => {
            // 当前 GStreamer 无 nvd3d11av1enc；AV1 走 CPU 路径
            return None;
        }
    };

    let fps = config.fps;

    let enc_params = nvenc_params(enc_elem, actual_codec, config.bitrate_kbps, fps);
    let parser = actual_codec.parser_element();
    let out_caps = actual_codec.output_capsfilter();

    // 关键：caps= 后必须写 memory:D3D11Memory，告诉 d3d11convert 输出 GPU 纹理
    // 括号不是空格，gst::parse::launch 不会把整段切成元素名。
    // 未显式配置分辨率时**不要**写 width/height，让采集保持原生分辨率。
    let gpu_caps = match (config.width, config.height) {
        (Some(w), Some(h)) if w > 0 && h > 0 => format!(
            "capsfilter caps=video/x-raw(memory:D3D11Memory),format=NV12,width={w},height={h},framerate={fps}/1"
        ),
        _ => {
            format!("capsfilter caps=video/x-raw(memory:D3D11Memory),format=NV12,framerate={fps}/1")
        }
    };

    Some(format!(
        "d3d11screencapturesrc monitor-index={monitor} show-cursor={cursor} ! \
         d3d11convert ! \
         {gpu_caps} ! \
         {enc_params} ! \
         {parser} ! \
         {out_caps} ! \
         appsink name=sink sync=false max-buffers=4 drop=true emit-signals=false",
        monitor = config.display_index,
        cursor = config.show_cursor,
        gpu_caps = gpu_caps,
        enc_params = enc_params,
        parser = parser,
        out_caps = out_caps,
    ))
}

/// 构建 CPU 路径管道（现有逻辑，作为降级方案）。
fn build_cpu_pipeline(config: &VideoConfig, actual_codec: VideoCodec) -> String {
    let (dw, dh) = primary_display_size();
    let w = config.width.unwrap_or(dw);
    let h = config.height.unwrap_or(dh);
    let fps = config.fps;

    let (enc_params, _) = config.encoder.resolve(w, h, fps, config.bitrate_kbps);
    let parser = actual_codec.parser_element();
    let out_caps = actual_codec.output_capsfilter();

    let scale_segment = match (config.width, config.height) {
        (Some(sw), Some(sh)) if sw > 0 && sh > 0 => {
            format!("videoscale ! capsfilter caps=video/x-raw,width={sw},height={sh} ! ")
        }
        _ => String::new(),
    };

    format!(
        "d3d11screencapturesrc monitor-index={monitor} show-cursor={cursor} ! \
         d3d11download ! \
         videoconvert ! \
         {scale_segment}\
         capsfilter caps=video/x-raw,framerate={fps}/1 ! \
         {enc_params} ! \
         {parser} ! \
         {out_caps} ! \
         appsink name=sink sync=false max-buffers=4 drop=true emit-signals=false",
        cursor = config.show_cursor,
        monitor = config.display_index,
        scale_segment = scale_segment,
        enc_params = enc_params,
        parser = parser,
        out_caps = out_caps,
    )
}

impl EncodedVideoStream {
    pub fn start(config: VideoConfig) -> Result<Self, CaptureError> {
        gst::init().map_err(|e| CaptureError::GStreamer(e.to_string()))?;

        let (dw, dh) = primary_display_size();
        let w = config.width.unwrap_or(dw);
        let h = config.height.unwrap_or(dh);
        let (_, actual_codec) = config
            .encoder
            .resolve(w, h, config.fps, config.bitrate_kbps);

        // 1. 优先尝试 GPU 直通
        let mut used_gpu = false;
        let pipeline = if let Some(gpu_str) = build_gpu_pipeline(&config, actual_codec) {
            tracing::info!(
                "capture pipeline (GPU direct): {}x{}@{}fps {}kbps codec={}",
                w,
                h,
                config.fps,
                config.bitrate_kbps,
                actual_codec.as_str()
            );
            tracing::debug!("GPU pipeline: {}", gpu_str);
            match gst::parse::launch(&gpu_str).and_then(|e| {
                e.downcast::<gst::Pipeline>().map_err(|_| {
                    gst::glib::Error::new(gst::glib::FileError::Failed, "not a pipeline")
                })
            }) {
                Ok(p) => {
                    used_gpu = true;
                    tracing::info!("capture: GPU direct pipeline OK");
                    p
                }
                Err(e) => {
                    tracing::warn!(
                        "GPU direct pipeline build failed: {e}; falling back to CPU (4K+ 会明显掉帧)"
                    );
                    let cpu_str = build_cpu_pipeline(&config, actual_codec);
                    tracing::debug!("CPU pipeline: {}", cpu_str);
                    gst::parse::launch(&cpu_str)
                        .map_err(|e| CaptureError::GStreamer(format!("cpu pipeline build: {e}")))?
                        .downcast::<gst::Pipeline>()
                        .map_err(|_| CaptureError::GStreamer("not a pipeline".into()))?
                }
            }
        } else {
            tracing::info!(
                "capture: GPU direct unavailable (nvd3d11h265enc/nvd3d11h264enc missing); using CPU pipeline"
            );
            let cpu_str = build_cpu_pipeline(&config, actual_codec);
            tracing::info!(
                "capture pipeline (CPU): {}x{}@{}fps {}kbps codec={}",
                w,
                h,
                config.fps,
                config.bitrate_kbps,
                actual_codec.as_str()
            );
            tracing::debug!("CPU pipeline: {}", cpu_str);
            gst::parse::launch(&cpu_str)
                .map_err(|e| CaptureError::GStreamer(format!("cpu pipeline build: {e}")))?
                .downcast::<gst::Pipeline>()
                .map_err(|_| CaptureError::GStreamer("not a pipeline".into()))?
        };

        tracing::info!(
            "capture mode: {}",
            if used_gpu {
                "GPU_DIRECT"
            } else {
                "CPU_FALLBACK"
            }
        );

        let appsink = pipeline
            .by_name("sink")
            .ok_or_else(|| CaptureError::GStreamer("找不到 appsink".into()))?
            .downcast::<AppSink>()
            .map_err(|_| CaptureError::GStreamer("not an appsink".into()))?;

        let (tx, rx) = std::sync::mpsc::channel::<EncodedFrame>();
        let stopped = Arc::new(AtomicBool::new(false));
        let frames_sent = Arc::new(AtomicU64::new(0));

        Self::install_callbacks(&appsink, tx, stopped.clone(), frames_sent.clone());

        pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| CaptureError::GStreamer(format!("set_state(Playing): {e}")))?;

        Ok(Self {
            pipeline,
            rx,
            stopped,
            frames_sent,
            actual_codec,
        })
    }

    pub fn actual_codec(&self) -> VideoCodec {
        self.actual_codec
    }

    fn install_callbacks(
        appsink: &AppSink,
        tx: Sender<EncodedFrame>,
        stopped: Arc<AtomicBool>,
        frames_sent: Arc<AtomicU64>,
    ) {
        appsink.set_callbacks(
            gstreamer_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    if stopped.load(Ordering::Relaxed) {
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
                    let is_keyframe = !buffer.flags().contains(gst::BufferFlags::DELTA_UNIT);
                    let pts_ms = buffer.pts().map(|p| p.mseconds() as i64).unwrap_or(0);

                    let frame = EncodedFrame {
                        data,
                        is_keyframe,
                        pts_ms,
                    };
                    if tx.send(frame).is_err() {
                        return Err(gst::FlowError::Eos);
                    }
                    frames_sent.fetch_add(1, Ordering::Relaxed);
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );
    }

    pub fn next_frame(&self) -> Result<EncodedFrame, CaptureError> {
        self.rx.recv().map_err(|_| CaptureError::NotStarted)
    }

    pub fn next_frame_timeout(
        &self,
        timeout: std::time::Duration,
    ) -> Result<Option<EncodedFrame>, CaptureError> {
        match self.rx.recv_timeout(timeout) {
            Ok(f) => Ok(Some(f)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(_) => Err(CaptureError::NotStarted),
        }
    }

    pub fn frames_sent(&self) -> u64 {
        self.frames_sent.load(Ordering::Relaxed)
    }

    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        let _ = self.pipeline.send_event(gst::event::Eos::new());
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codec_recommendation_matches_hardware_limits() {
        assert_eq!(VideoCodec::recommend(7680, 4320, 60), VideoCodec::AV1);
        assert_eq!(VideoCodec::recommend(7680, 4320, 120), VideoCodec::AV1);
        assert_eq!(VideoCodec::recommend(3840, 2160, 120), VideoCodec::AV1);
        assert_eq!(VideoCodec::recommend(3840, 2160, 240), VideoCodec::AV1);
        assert_eq!(VideoCodec::recommend(3840, 2160, 60), VideoCodec::HEVC);
        assert_eq!(VideoCodec::recommend(2560, 1440, 240), VideoCodec::HEVC);
        assert_eq!(VideoCodec::recommend(2560, 1440, 120), VideoCodec::H264);
        assert_eq!(VideoCodec::recommend(1920, 1080, 240), VideoCodec::HEVC);
        assert_eq!(VideoCodec::recommend(1920, 1080, 120), VideoCodec::H264);
        assert_eq!(VideoCodec::recommend(1280, 720, 60), VideoCodec::H264);
    }

    #[test]
    fn vertical_resolution_detected_as_longest() {
        assert_eq!(VideoCodec::recommend(2160, 3840, 60), VideoCodec::HEVC);
        assert_eq!(VideoCodec::recommend(4320, 7680, 60), VideoCodec::AV1);
    }

    /// GPU 直通用的 `nvd3d11h264enc` 曾经漏配参数，导致 CBR/zerolatency/GOP
    /// 全部丢失（关键帧间隔变成编码器默认 75 帧）。所有 NVENC 元素名都必须命中。
    #[test]
    fn nvenc_params_apply_low_latency_for_every_nvenc_element() {
        for elem in ["nvh264enc", "nvd3d11h264enc", "nvh265enc", "nvd3d11h265enc"] {
            let p = nvenc_params(elem, VideoCodec::H264, 4000, 30);
            assert!(p.starts_with(elem), "{p}");
            assert!(p.contains("rc-mode=cbr"), "{p}");
            assert!(p.contains("zerolatency=true"), "{p}");
            assert!(p.contains("gop-size=60"), "{p}");
        }
    }
}
