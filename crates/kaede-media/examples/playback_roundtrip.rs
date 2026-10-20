//! 本地回环播放：读 `_trash/encrypted_dump.bin` → 解密 → 解码/播放。
//!
//! 依赖 `encrypt_to_file` 先跑过一次（生成 dump 文件）。

use std::time::Duration;

use kaede_crypto::sframe::SframeKey;
use kaede_media::{EncryptedFrame, ReceiveMode, ReceivePipeline};

const KEY_BYTES: [u8; 32] = [0xAB; 32];
const KEY_ID: u32 = 42;
const DUMP_PATH: &str = "_trash/encrypted_dump.bin";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let bytes = match tokio::fs::read(DUMP_PATH).await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("读文件失败: {} ({})", DUMP_PATH, e);
            eprintln!("请先跑: cargo run --release -p kaede-media --example encrypt_to_file");
            std::process::exit(1);
        }
    };
    println!("[1] {} 字节 from {}", bytes.len(), DUMP_PATH);

    // 解析：每帧 [pts_ms: i64 BE][len: u32 BE][data]
    let mut frames: Vec<EncryptedFrame> = Vec::new();
    let mut pos = 0usize;
    while pos + 12 <= bytes.len() {
        let pts = i64::from_be_bytes(bytes[pos..pos + 8].try_into()?);
        let len = u32::from_be_bytes(bytes[pos + 8..pos + 12].try_into()?) as usize;
        pos += 12;
        if pos + len > bytes.len() {
            break;
        }
        let data = bytes[pos..pos + len].to_vec();
        pos += len;
        frames.push(EncryptedFrame {
            data,
            is_keyframe: false,
            pts_ms: pts,
        });
    }
    println!("[2] 解析出 {} 帧", frames.len());
    if frames.is_empty() {
        eprintln!("没有帧可播放");
        std::process::exit(1);
    }

    println!("[3] 启动 ReceivePipeline (Video) ...");
    let pipeline =
        ReceivePipeline::start(SframeKey::from_bytes(KEY_BYTES), KEY_ID, ReceiveMode::Video)?;

    // 按 pts_ms 节流回放
    let mut prev_pts: Option<i64> = None;
    for (i, f) in frames.iter().enumerate() {
        if let Some(p) = prev_pts {
            let dt = (f.pts_ms - p).max(0) as u64;
            if dt > 0 && dt < 200 {
                tokio::time::sleep(Duration::from_millis(dt)).await;
            }
        }
        prev_pts = Some(f.pts_ms);

        match pipeline.decrypt_and_forward(f) {
            Ok(_) => {
                if i % 30 == 0 {
                    println!("  [{}] pts={}", i, f.pts_ms);
                }
            }
            Err(e) => {
                eprintln!("  [{}] err: {}", i, e);
            }
        }
    }

    println!("[4] 完成，等待 2 秒显示末尾...");
    tokio::time::sleep(Duration::from_secs(2)).await;

    let s = pipeline.stats();
    println!(
        "[5] stats: decrypted={} rejected={} forwarded={}",
        s.decrypted,
        s.rejected,
        pipeline.forwarded()
    );

    pipeline.stop();
    println!("PASS playback_roundtrip");
    Ok(())
}
