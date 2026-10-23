//! LiveKit 端到端推流验证（AES-GCM E2EE，无 SFrame）。
//!
//! 流程：屏幕采集 → NVENC → LiveKitSession → LiveKit RTP（E2EE）
//!
//! 运行：
//!   cargo run --release -p kaede-media --example livekit_push
//!
//! E2EE passphrase 由 server 同算法派生：
//!   HMAC-SHA256(api_secret, "kaede-e2ee-v1:" || channel_id) → base64
//! 浏览器订阅时填 example 打印的字符串即可解密。

use std::time::Duration;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use kaede_capture::{EncodedVideoStream, EncoderKind, VideoConfig};
use kaede_media::{LiveKitConfig, LiveKitSession};

const ROOM: &str = "kaede-test";
const IDENTITY: &str = "alice";
const API_KEY: &str = "devkey";
const API_SECRET: &str = "secret";
const LIVEKIT_WS: &str = "ws://localhost:18082";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    println!("=== LiveKit 端到端推流（AES-GCM E2EE）===\n");
    println!("[1] 生成 dev token");
    let token = make_dev_token(API_KEY, API_SECRET, ROOM, IDENTITY);
    println!("    房间: {}", ROOM);
    println!("    身份: {}", IDENTITY);

    let viewer_token = make_dev_token(API_KEY, API_SECRET, ROOM, "bob-viewer");
    println!("\n    订阅端 token（浏览器用）:");
    println!("    {}", viewer_token);

    let channel_id = std::env::var("KAEDE_CHANNEL_ID").unwrap_or_else(|_| ROOM.into());
    let api_secret = std::env::var("LIVEKIT_API_SECRET").unwrap_or_else(|_| API_SECRET.into());
    let e2ee_passphrase = derive_e2ee_passphrase(&api_secret, &channel_id);
    println!("    E2EE: AES-GCM 已启用 (channel={})", channel_id);
    println!("    E2EE passphrase (浏览器填此串): {}", e2ee_passphrase);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async move {
        println!("\n[2] 启动屏幕采集");
        let video_cfg = VideoConfig {
            encoder: EncoderKind::Auto,
            fps: 60,
            bitrate_kbps: 8000,
            show_cursor: true,
            display_index: 0,
            width: None, // C-5: Some(W) 时启用 videoscale
            height: None,
        };
        let source = EncodedVideoStream::start(video_cfg)?;
        println!("    采集已启动");

        println!("\n[3] LiveKitSession::start（采集 → LiveKit E2EE）");
        let lk_cfg = LiveKitConfig::new(LIVEKIT_WS, token.clone())
            .with_resolution(1920, 1080)
            .with_e2ee_key(e2ee_passphrase.as_bytes().to_vec());

        let session = LiveKitSession::start(source, lk_cfg).await?;
        println!("    LiveKit 连接已建立");

        let mut events = session.events();
        let ev_task = tokio::spawn(async move {
            let mut kf = 0u32;
            let mut errs = 0u32;
            loop {
                match events.recv().await {
                    Ok(kaede_media::MediaEvent::FrameEncrypted { is_keyframe, .. })
                        if is_keyframe =>
                    {
                        kf += 1;
                    }
                    Ok(kaede_media::MediaEvent::Error { message }) => {
                        errs += 1;
                        eprintln!("[event] error: {}", message);
                    }
                    Ok(kaede_media::MediaEvent::Stopped { .. }) => break,
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("[event] lagged {}", n);
                    }
                    Err(_) => break,
                }
            }
            (kf, errs)
        });

        let push_secs: u64 = std::env::var("KAEDE_PUSH_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(60);
        println!("\n[4] 推流 {} 秒（Ctrl+C 提前停止）...", push_secs);
        tokio::time::sleep(Duration::from_secs(push_secs)).await;

        let s = session.stats();
        println!("\n[5] LiveKitSession 统计:");
        println!("    captured: {} 帧", s.frames_encrypted);
        println!("    sent:     {} 帧 / {} 字节", s.frames_sent, s.bytes_sent);
        println!("    dropped:  {} 帧", s.frames_dropped);
        println!("    keyframe: {:?}", s.last_keyframe_pts_ms);

        let p = session.pusher();
        println!(
            "    pusher:   injected={} dropped={} bytes={}",
            p.frames_injected(),
            p.frames_dropped(),
            p.bytes_injected()
        );

        session.stop().await;
        let (kf, errs) = ev_task.await?;
        println!("    事件: keyframe={} errors={}", kf, errs);

        if s.frames_encrypted == 0 {
            eprintln!("!! 未采集到任何帧");
            std::process::exit(1);
        }

        println!("\nPASS livekit_push (E2EE, no SFrame)");
        Ok::<_, Box<dyn std::error::Error>>(())
    })?;

    println!("\n订阅验证：浏览器打开 https://meet.livekit.io");
    println!("  1. 选 Custom");
    println!("  2. LiveKit URL: {}", LIVEKIT_WS);
    println!("  3. Token: 粘贴上面的 viewer_token");
    println!("  4. 勾选 'Enable end-to-end encryption'");
    println!("  5. 填入 E2EE passphrase（见 [1] 打印）");
    println!("  6. 点 Connect");
    Ok(())
}

fn make_dev_token(api_key: &str, api_secret: &str, room: &str, identity: &str) -> String {
    type HmacSha256 = Hmac<Sha256>;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let exp = now + 6 * 3600;
    let header = serde_json::json!({ "alg": "HS256", "typ": "JWT" });
    let payload = serde_json::json!({
        "exp": exp, "iss": api_key, "nbf": now - 10,
        "sub": identity, "name": identity,
        "video": { "room": room, "roomJoin": true,
                   "canPublish": true, "canSubscribe": true, "canPublishData": true }
    });
    let h = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).unwrap());
    let p = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap());
    let signing_input = format!("{}.{}", h, p);
    let mut mac = HmacSha256::new_from_slice(api_secret.as_bytes()).unwrap();
    mac.update(signing_input.as_bytes());
    let s = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    format!("{}.{}", signing_input, s)
}

/// 与 kaede-server 完全一致的派生：
/// HMAC-SHA256(api_secret, "kaede-e2ee-v1:" || channel_id) → base64 URL-safe no-pad
fn derive_e2ee_passphrase(api_secret: &str, channel_id: &str) -> String {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(api_secret.as_bytes()).expect("hmac init");
    mac.update(b"kaede-e2ee-v1:");
    mac.update(channel_id.as_bytes());
    let bytes = mac.finalize().into_bytes();
    URL_SAFE_NO_PAD.encode(bytes)
}
