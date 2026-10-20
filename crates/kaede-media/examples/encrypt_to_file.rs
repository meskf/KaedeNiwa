//! 端到端：真实屏幕 → NVENC → SFrame 加密 → 写文件。
//!
//! 跑：
//! ```sh
//! cargo run --release -p kaede-media --example encrypt_to_file
//! ```
//! 输出：`_trash/encrypted_dump.bin`
//!
//! 每帧格式：`[pts_ms: i64 BE][len: u32 BE][data]`

use std::time::Duration;

use async_trait::async_trait;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use kaede_capture::{EncodedVideoStream, EncoderKind, VideoConfig};
use kaede_crypto::sframe::SframeKey;
use kaede_media::{EncryptedFrame, MediaError, MediaEvent, MediaSession, MediaSink};

struct FileSink {
    file: Mutex<File>,
}

#[async_trait]
impl MediaSink for FileSink {
    async fn send(&self, frame: EncryptedFrame) -> Result<(), MediaError> {
        let mut f = self.file.lock().await;
        f.write_all(&frame.pts_ms.to_be_bytes())
            .await
            .map_err(|e| MediaError::Sink(e.to_string()))?;
        f.write_all(&(frame.data.len() as u32).to_be_bytes())
            .await
            .map_err(|e| MediaError::Sink(e.to_string()))?;
        f.write_all(&frame.data)
            .await
            .map_err(|e| MediaError::Sink(e.to_string()))?;
        Ok(())
    }
    async fn close(&self) -> Result<(), MediaError> {
        self.file
            .lock()
            .await
            .flush()
            .await
            .map_err(|e| MediaError::Sink(e.to_string()))?;
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    kaede_capture::init()?;

    let config = VideoConfig {
        encoder: EncoderKind::Auto,
        fps: 60,
        bitrate_kbps: 8000,
        show_cursor: true,
        display_index: 0,
        width: None, // C-5: Some(W) 时启用 videoscale
        height: None,
    };

    println!("[1] 启动采集…");
    let source = EncodedVideoStream::start(config)?;

    let key = SframeKey::from_bytes([0xAB; 32]);

    let dump_path = "_trash/encrypted_dump.bin";
    tokio::fs::create_dir_all("_trash").await.ok();
    let file = File::create(dump_path).await?;
    let sink = FileSink {
        file: Mutex::new(file),
    };

    println!("[2] MediaSession::start…");
    let sender_id = kaede_crypto::sframe::sender_id_from_uid("alice");
    let session = MediaSession::start(
        source,
        sink,
        key,
        kaede_media::ephemeral_key_id(),
        sender_id,
    )
    .await?;

    let mut events_rx = session.events();
    let ev_task = tokio::spawn(async move {
        let mut kf = 0u32;
        let mut errs = 0u32;
        while let Ok(ev) = events_rx.recv().await {
            match ev {
                MediaEvent::FrameEncrypted { is_keyframe, .. } if is_keyframe => kf += 1,
                MediaEvent::Error { message } => {
                    errs += 1;
                    eprintln!("[event] error: {}", message);
                }
                MediaEvent::Stopped { .. } => break,
                _ => {}
            }
        }
        (kf, errs)
    });

    println!("[3] 采集 5 秒…");
    tokio::time::sleep(Duration::from_secs(5)).await;

    let s = session.stats();
    println!("[4] 统计：");
    println!(
        "    encrypted: {} 帧, {} 字节",
        s.frames_encrypted, s.bytes_encrypted
    );
    println!("    sent:      {} 帧, {} 字节", s.frames_sent, s.bytes_sent);
    println!("    dropped:   {} 帧", s.frames_dropped);
    println!("    keyframe:  {:?}", s.last_keyframe_pts_ms);

    session.stop().await;

    let (kf, errs) = ev_task.await?;
    println!("[5] 事件：keyframe={} errors={}", kf, errs);

    let bytes = tokio::fs::metadata(dump_path).await?.len();
    println!("[6] dump: {} ({} 字节)", dump_path, bytes);

    if s.frames_encrypted == 0 {
        eprintln!("!! 未采集到任何帧");
        std::process::exit(1);
    }
    println!("PASS encrypt_to_file");
    Ok(())
}
