//! 麦克风输入 → SFrame 加密 → 落盘。
//!
//! 跑：
//! ```sh
//! cargo run --release -p kaede-media --example mic_record
//! ```
//! 输出：`_trash/encrypted_mic.bin`
//!
//! 采集 5 秒，对麦克风说话即可看到帧数。

use std::time::Duration;

use async_trait::async_trait;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use kaede_crypto::sframe::SframeKey;
use kaede_media::{
    AudioCaptureConfig, AudioSession, AudioSourceKind, EncryptedFrame, GstAudioSource, MediaError,
    MediaEvent, MediaSink,
};

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

    println!("[1] 启动 GstAudioSource（麦克风 48kHz 1ch 20ms）…");
    let source = GstAudioSource::start(AudioCaptureConfig {
        sample_rate: 48000,
        channels: 1,
        kind: AudioSourceKind::Microphone,
        ..Default::default()
    })?;

    let key = SframeKey::from_bytes([0xEE; 32]);
    let sender_id = kaede_crypto::sframe::sender_id_from_uid("alice");

    std::fs::create_dir_all("_trash").ok();
    let out = "_trash/encrypted_mic.bin";
    let file = File::create(out).await?;
    let sink = FileSink {
        file: Mutex::new(file),
    };

    println!("[2] AudioSession::start…");
    let session = AudioSession::start(
        source,
        sink,
        key,
        kaede_media::ephemeral_key_id(),
        sender_id,
    )
    .await?;

    let mut events_rx = session.events();
    let ev_task = tokio::spawn(async move {
        let mut errs = 0u32;
        loop {
            match events_rx.recv().await {
                Ok(MediaEvent::Error { message }) => {
                    errs += 1;
                    eprintln!("[event] error: {}", message);
                }
                Ok(MediaEvent::Stopped { .. }) => break,
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("[event] lagged {} events", n);
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
        errs
    });

    println!("[3] 采集 5 秒（对着麦克风说话）…");
    tokio::time::sleep(Duration::from_secs(5)).await;

    let s = session.stats();
    println!("[4] 统计：");
    println!(
        "    encrypted: {} 帧, {} 字节",
        s.frames_encrypted, s.bytes_encrypted
    );
    println!("    sent:      {} 帧, {} 字节", s.frames_sent, s.bytes_sent);
    println!("    dropped:   {} 帧", s.frames_dropped);

    session.stop().await;
    let errs = ev_task.await?;
    println!("[5] errors={}", errs);

    let bytes = tokio::fs::metadata(out).await?.len();
    println!("[6] dump: {} ({} 字节)", out, bytes);

    if s.frames_encrypted == 0 {
        eprintln!("!! 未采集到任何音频帧（麦克风可能被占用或无输入）");
        std::process::exit(1);
    }
    println!("PASS mic_record");
    Ok(())
}
