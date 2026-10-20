//! 冒烟测试 v3 —— 直接用 GStreamer（绕过 Rivulet 的 DXGI/wrapper）。
//!
//! 目标：验证 GStreamer 层面的屏幕 + 音频捕获能否工作。

#![allow(clippy::manual_is_multiple_of, clippy::collapsible_if)]

use anyhow::Result;
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app::AppSink;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

fn main() -> Result<()> {
    gst::init()?;
    println!("GStreamer {} 已初始化\n", gst::version_string());

    // ================================================================
    // [1] 屏幕采集 —— 用 d3d11screencapturesrc
    // ================================================================
    println!("=== [1] 屏幕采集（GStreamer d3d11screencapturesrc）===");

    let counter = Arc::new(AtomicU64::new(0));
    let counter_clone = counter.clone();

    // pipeline: d3d11 屏幕采集 → 转成 CPU 可读 → appsink
    let pipeline = gst::parse::launch(
        "d3d11screencapturesrc show-cursor=true ! \
         videorate ! \
         video/x-raw,framerate=30/1 ! \
         d3d11download ! \
         videoconvert ! \
         video/x-raw,format=RGBA ! \
         appsink name=sink sync=false max-buffers=2 drop=true",
    )?;
    let pipeline = pipeline.downcast::<gst::Pipeline>().unwrap();

    let appsink = pipeline
        .by_name("sink")
        .expect("找不到 appsink")
        .downcast::<AppSink>()
        .unwrap();

    appsink.set_callbacks(
        gstreamer_app::AppSinkCallbacks::builder()
            .new_sample(move |_sink| {
                let n = counter_clone.fetch_add(1, Ordering::Relaxed);
                if n % 10 == 0 {
                    println!("  已采集 {} 帧", n + 1);
                }
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );

    let t0 = Instant::now();
    pipeline.set_state(gst::State::Playing)?;
    std::thread::sleep(Duration::from_secs(3));
    pipeline.set_state(gst::State::Null)?;

    let frames = counter.load(Ordering::Relaxed);
    let elapsed = t0.elapsed().as_secs_f64();
    println!(
        "\n  3 秒共采集 {} 帧 ({:.1} fps)\n",
        frames,
        frames as f64 / elapsed
    );

    // ================================================================
    // [2] 音频采集 —— 用 wasapi2src loopback
    // ================================================================
    println!("=== [2] 系统音频采集（GStreamer wasapi2src loopback）===");
    println!("  ⚠️  确保酷狗在播放音乐！");
    std::thread::sleep(Duration::from_secs(2));

    let audio_counter = Arc::new(AtomicU64::new(0));
    let audio_counter_clone = audio_counter.clone();
    let audio_samples = Arc::new(AtomicU64::new(0));
    let audio_samples_clone = audio_samples.clone();

    // pipeline: WASAPI loopback → 转 f32 → appsink
    let audio_pipeline = gst::parse::launch(
        "wasapi2src loopback=true low-latency=true ! \
         audioconvert ! \
         audio/x-raw,format=F32LE,channels=2,rate=48000 ! \
         appsink name=asink sync=false max-buffers=4 drop=true",
    )?;
    let audio_pipeline = audio_pipeline.downcast::<gst::Pipeline>().unwrap();

    let asink = audio_pipeline
        .by_name("asink")
        .expect("找不到 appsink")
        .downcast::<AppSink>()
        .unwrap();

    asink.set_callbacks(
        gstreamer_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                if let Ok(sample) = sink.pull_sample() {
                    if let Some(buffer) = sample.buffer() {
                        let size = buffer.size();
                        let n = audio_counter_clone.fetch_add(1, Ordering::Relaxed);
                        audio_samples_clone.fetch_add(size as u64, Ordering::Relaxed);
                        if n % 20 == 0 {
                            println!("  已捕获 {} 个音频 buffer", n + 1);
                        }
                    }
                }
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );

    audio_pipeline.set_state(gst::State::Playing)?;
    std::thread::sleep(Duration::from_secs(3));
    audio_pipeline.set_state(gst::State::Null)?;

    let buffers = audio_counter.load(Ordering::Relaxed);
    let bytes = audio_samples.load(Ordering::Relaxed);
    let samples = bytes / 4; // f32
    let secs = samples as f64 / (48000.0 * 2.0);
    println!("\n  3 秒共捕获:");
    println!("    音频 buffer 数: {}", buffers);
    println!("    f32 采样数: {}", samples);
    println!("    估算时长: {:.2}s", secs);

    println!("\n=== 完成 ===");
    Ok(())
}
