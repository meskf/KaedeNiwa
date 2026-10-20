//! 验证 appsink 拿 H.264 码流。

#![allow(clippy::single_match)]

use kaede_capture::{EncodedVideoStream, EncoderKind, VideoConfig};
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    println!("=== H.264 码流测试 ===\n");

    let config = VideoConfig {
        encoder: EncoderKind::Auto,
        fps: 60,
        bitrate_kbps: 8000,
        show_cursor: true,
        display_index: 0,
        width: None, // C-5: Some(W) 时启用 videoscale
        height: None,
    };

    let stream = EncodedVideoStream::start(config)?;
    println!("✓ Pipeline 启动\n");

    let t0 = Instant::now();
    let total = Duration::from_secs(5);
    let mut frames = 0u64;
    let mut keyframes = 0u64;
    let mut total_bytes = 0u64;
    let mut last_report = 0u64;

    while t0.elapsed() < total {
        match stream.next_frame_timeout(Duration::from_millis(100))? {
            Some(frame) => {
                frames += 1;
                total_bytes += frame.data.len() as u64;
                if frame.is_keyframe {
                    keyframes += 1;
                }
            }
            None => {} // 没帧就继续等
        }

        let sec = t0.elapsed().as_secs();
        if sec != last_report {
            println!(
                "  [{}s] 帧数={} 关键帧={} 累计={:.2} MB",
                sec,
                frames,
                keyframes,
                total_bytes as f64 / 1_048_576.0
            );
            last_report = sec;
        }
    }

    stream.stop();
    let elapsed = t0.elapsed().as_secs_f64();

    println!("\n=== 结果 ===");
    println!("  总帧数: {}", frames);
    println!("  关键帧: {}", keyframes);
    println!("  总字节: {:.2} MB", total_bytes as f64 / 1_048_576.0);
    println!("  时长: {:.2}s", elapsed);
    println!("  fps: {:.1}", frames as f64 / elapsed);
    println!("  平均帧大小: {} B", total_bytes / frames.max(1));
    println!(
        "  码率: {:.0} kbps",
        total_bytes as f64 * 8.0 / elapsed / 1000.0
    );

    if frames > 0 {
        println!("\n✅ 成功拿到 H.264 码流");
    } else {
        println!("\n❌ 没有拿到帧");
    }
    Ok(())
}
