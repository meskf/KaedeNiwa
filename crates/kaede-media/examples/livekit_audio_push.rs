//! LiveKit 麦克风推流验证（AES-GCM E2EE）。
//!
//! 流程：麦克风 → GstAudioSource → LiveKitAudioSession → LiveKit RTP（E2EE）
//!
//! 运行：
//!   cargo run --release -p kaede-media --example livekit_audio_push

use std::time::Duration;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use kaede_media::{
    AudioCaptureConfig, AudioSourceKind, GstAudioSource, LiveKitAudioSession, LiveKitConfig,
};

const ROOM: &str = "kaede-test-audio";
const IDENTITY: &str = "alice-audio";
const API_KEY: &str = "devkey";
const API_SECRET: &str = "secret";
const LIVEKIT_WS: &str = "ws://localhost:18082";
const SAMPLE_RATE: u32 = 48000;
const CHANNELS: u32 = 2;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    println!("=== LiveKit 麦克风推流（AES-GCM E2EE）===\n");

    println!("[1] 生成 dev token");
    let token = make_dev_token(API_KEY, API_SECRET, ROOM, IDENTITY);
    let viewer_token = make_dev_token(API_KEY, API_SECRET, ROOM, "bob-viewer");
    println!("    room={} identity={}", ROOM, IDENTITY);

    let channel_id = std::env::var("KAEDE_CHANNEL_ID").unwrap_or_else(|_| ROOM.into());
    let api_secret = std::env::var("LIVEKIT_API_SECRET").unwrap_or_else(|_| API_SECRET.into());
    let e2ee_passphrase = derive_e2ee_passphrase(&api_secret, &channel_id);
    println!("    E2EE passphrase (浏览器填此串): {}", e2ee_passphrase);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async move {
        println!("\n[2] 启动麦克风采集");
        let source = GstAudioSource::start(AudioCaptureConfig {
            sample_rate: SAMPLE_RATE,
            channels: CHANNELS as u16,
            kind: AudioSourceKind::Microphone,
            ..Default::default()
        })?;
        println!("    麦克风已启动");

        println!("\n[3] LiveKitAudioSession::start");
        let lk_cfg = LiveKitConfig::new(LIVEKIT_WS, token.clone())
            .with_e2ee_key(e2ee_passphrase.as_bytes().to_vec());

        let session = LiveKitAudioSession::start(source, lk_cfg, SAMPLE_RATE, CHANNELS).await?;
        println!("    LiveKit 连接已建立");

        let mut events = session.events();
        let ev_task = tokio::spawn(async move {
            let mut errs = 0u32;
            loop {
                match events.recv().await {
                    Ok(kaede_media::MediaEvent::Error { message }) => {
                        errs += 1;
                        eprintln!("[event] error: {}", message);
                    }
                    Ok(kaede_media::MediaEvent::Stopped { .. }) => break,
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(_) => break,
                }
            }
            errs
        });

        let push_secs: u64 = std::env::var("KAEDE_PUSH_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(15);
        println!("\n[4] 推流 {} 秒...", push_secs);
        tokio::time::sleep(Duration::from_secs(push_secs)).await;

        let s = session.stats();
        println!("\n[5] LiveKitAudioSession 统计:");
        println!("    captured: {} 块", s.frames_encrypted);
        println!("    sent:     {} 块 / {} 字节", s.frames_sent, s.bytes_sent);
        println!("    dropped:  {} 块", s.frames_dropped);
        let p = session.pusher();
        println!(
            "    pusher:   injected={} dropped={} bytes={}",
            p.frames_injected(),
            p.frames_dropped(),
            p.bytes_injected()
        );

        session.stop().await;
        let errs = ev_task.await?;
        println!("    事件: errors={}", errs);

        if s.frames_encrypted == 0 {
            eprintln!("!! 未采集到任何音频块");
            std::process::exit(1);
        }

        println!("\nPASS livekit_audio_push");
        Ok::<_, Box<dyn std::error::Error>>(())
    })?;

    println!("\n订阅验证：");
    println!("  浏览器 https://meet.livekit.io");
    println!("  URL: {}", LIVEKIT_WS);
    println!("  Token: {}", viewer_token);
    println!("  勾选 E2EE，填 passphrase（见 [1] 打印）");
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

fn derive_e2ee_passphrase(api_secret: &str, channel_id: &str) -> String {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(api_secret.as_bytes()).expect("hmac init");
    mac.update(b"kaede-e2ee-v1:");
    mac.update(channel_id.as_bytes());
    let bytes = mac.finalize().into_bytes();
    URL_SAFE_NO_PAD.encode(bytes)
}
