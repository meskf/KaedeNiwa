//! 屏幕采集 → NVENC → MP4。
//!
//! 跑：
//! ```sh
//! cargo run --release -p kaede-media --example mp4_record
//! ```
//! 输出：`_trash/screen_recording.mp4`

use std::time::Duration;

use gstreamer as gst;
use gstreamer::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    gst::init()?;
    std::fs::create_dir_all("_trash").ok();
    let out = "_trash/screen_recording.mp4";

    let pipeline_str = format!(
        "d3d11screencapturesrc show-cursor=true ! \
         d3d11download ! videoconvert ! \
         video/x-raw,format=NV12,framerate=60/1 ! \
         nvh264enc bitrate=8000 rc-mode=cbr zerolatency=true gop-size=120 ! \
         h264parse ! mp4mux ! filesink location={}",
        out
    );

    println!("[1] pipeline: {}", pipeline_str);
    let pipeline = gst::parse::launch(&pipeline_str)?
        .downcast::<gst::Pipeline>()
        .map_err(|_| "not a pipeline")?;

    pipeline.set_state(gst::State::Playing)?;
    println!("[2] 采集 5 秒…");
    std::thread::sleep(Duration::from_secs(5));

    pipeline.send_event(gst::event::Eos::new());

    let bus = pipeline.bus().ok_or("no bus")?;
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if let Some(msg) = bus.pop() {
            use gst::MessageView;
            match msg.view() {
                MessageView::Eos(_) => {
                    println!("[3] EOS");
                    break;
                }
                MessageView::Error(e) => {
                    eprintln!("!! GStreamer error: {} ({:?})", e.error(), e.debug());
                    pipeline.set_state(gst::State::Null)?;
                    std::process::exit(1);
                }
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    pipeline.set_state(gst::State::Null)?;

    let size = std::fs::metadata(out)?.len();
    println!(
        "[4] 输出：{} ({:.1} MiB)",
        out,
        size as f64 / 1024.0 / 1024.0
    );
    if size == 0 {
        eprintln!("!! 空文件");
        std::process::exit(1);
    }
    println!("PASS mp4_record");
    Ok(())
}
