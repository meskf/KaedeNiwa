//! 端到端录屏验证 v2 —— 修复 MP4 moov atom 问题。

use anyhow::Result;
use gstreamer as gst;
use gstreamer::prelude::*;
use std::time::Duration;

/// 等待 pipeline 处理完所有消息（包括 EOS）。
/// 这是正确关闭 MP4 的关键 —— 必须等 muxer 写完 moov atom。
fn wait_eos(pipeline: &gst::Pipeline, timeout_secs: u64) -> Result<()> {
    let bus = pipeline.bus().ok_or_else(|| anyhow::anyhow!("no bus"))?;
    let timeout = gst::ClockTime::from_seconds(timeout_secs);

    for msg in bus.iter_timed(timeout) {
        use gst::MessageView;
        match msg.view() {
            MessageView::Eos(..) => {
                println!("  收到 EOS");
                return Ok(());
            }
            MessageView::Error(err) => {
                return Err(anyhow::anyhow!(
                    "Pipeline error: {} ({:?})",
                    err.error(),
                    err.debug()
                ));
            }
            _ => {}
        }
    }
    Err(anyhow::anyhow!("等 EOS 超时"))
}

fn main() -> Result<()> {
    gst::init()?;
    println!("GStreamer {}\n", gst::version_string());

    // 删除旧文件
    let _ = std::fs::remove_file("test_screen.mp4");
    let _ = std::fs::remove_file("test_audio.wav");

    // ============================================================
    // [1] 屏幕 → NVENC → MP4
    // ============================================================
    println!("=== [1] 录制屏幕 10 秒 → test_screen.mp4 ===");

    let screen_pipeline = gst::parse::launch(
        "d3d11screencapturesrc show-cursor=true ! \
         d3d11download ! \
         videoconvert ! \
         video/x-raw,format=NV12 ! \
         nvh264enc bitrate=8000 rc-mode=vbr zerolatency=false gop-size=60 ! \
         h264parse ! \
         mp4mux faststart=true ! \
         filesink location=test_screen.mp4",
    )?;
    let screen_pipeline = screen_pipeline.downcast::<gst::Pipeline>().unwrap();

    screen_pipeline.set_state(gst::State::Playing)?;
    println!("  录制中...");

    std::thread::sleep(Duration::from_secs(10));

    // ---- 关键：发 EOS ----
    println!("  发送 EOS...");
    screen_pipeline.send_event(gst::event::Eos::new());

    // ---- 等 EOS 处理完 ----
    wait_eos(&screen_pipeline, 10)?;

    // ---- 现在可以安全关闭 ----
    screen_pipeline.set_state(gst::State::Null)?;
    std::thread::sleep(Duration::from_millis(500)); // 给 OS 时间 flush

    if std::path::Path::new("test_screen.mp4").exists() {
        let size = std::fs::metadata("test_screen.mp4")?.len();
        println!("  ✅ test_screen.mp4 ({:.2} MB)", size as f64 / 1_048_576.0);
    } else {
        println!("  ❌ test_screen.mp4 未生成");
    }

    // ============================================================
    // [2] 音频 → WAV
    // ============================================================
    println!("\n=== [2] 录制音频 10 秒 → test_audio.wav ===");
    println!("  ⚠️  确保酷狗在播放音乐！");
    std::thread::sleep(Duration::from_secs(1));

    let audio_pipeline = gst::parse::launch(
        "wasapi2src loopback=true low-latency=true ! \
         audioconvert ! \
         audioresample ! \
         audio/x-raw,format=S16LE,channels=2,rate=48000 ! \
         wavenc ! \
         filesink location=test_audio.wav",
    )?;
    let audio_pipeline = audio_pipeline.downcast::<gst::Pipeline>().unwrap();

    audio_pipeline.set_state(gst::State::Playing)?;
    println!("  录制中...");

    std::thread::sleep(Duration::from_secs(10));

    println!("  发送 EOS...");
    audio_pipeline.send_event(gst::event::Eos::new());
    wait_eos(&audio_pipeline, 10)?;
    audio_pipeline.set_state(gst::State::Null)?;
    std::thread::sleep(Duration::from_millis(500));

    if std::path::Path::new("test_audio.wav").exists() {
        let size = std::fs::metadata("test_audio.wav")?.len();
        println!("  ✅ test_audio.wav ({:.2} MB)", size as f64 / 1_048_576.0);
    }

    println!("\n=== 完成 ===");
    println!(
        "  test_screen.mp4: {:?}",
        std::fs::canonicalize("test_screen.mp4").unwrap_or_default()
    );
    println!(
        "  test_audio.wav:  {:?}",
        std::fs::canonicalize("test_audio.wav").unwrap_or_default()
    );

    Ok(())
}
