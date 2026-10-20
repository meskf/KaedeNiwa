//! 系统音频 loopback → WAV。
//!
//! 跑：
//! ```sh
//! cargo run --release -p kaede-media --example audio_record
//! ```
//! 输出：`_trash/audio_capture.wav`

use std::time::Duration;

use gstreamer as gst;
use gstreamer::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    gst::init()?;
    std::fs::create_dir_all("_trash").ok();
    let out = "_trash/audio_capture.wav";

    let pipeline_str = format!(
        "wasapi2src loopback=true low-latency=true ! \
         audioconvert ! audioresample ! \
         audio/x-raw,format=S16LE,channels=2,rate=48000 ! \
         wavenc ! filesink location={}",
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
    println!("[4] 输出：{} ({} 字节)", out, size);
    if size < 1024 {
        eprintln!("!! WAV 太小（< 1 KiB），可能没采到音频");
        std::process::exit(1);
    }
    println!("PASS audio_record");
    Ok(())
}
