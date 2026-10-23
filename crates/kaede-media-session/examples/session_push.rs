//! 用 MediaSession 推屏幕（新 API 验证）。
//!
//! 运行：
//!   KAEDE_PUSH_SECS=30 cargo run --release -p kaede-media-session --example session_push

use std::time::Duration;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use kaede_capture::{EncoderKind, VideoConfig};
use kaede_media_session::{ConnectConfig, MediaSession, PublishSource};

const ROOM: &str = "kaede-test";
const IDENTITY: &str = "alice";
const API_KEY: &str = "devkey";
const API_SECRET: &str = "secret";
const LIVEKIT_WS: &str = "ws://localhost:18082";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    println!("=== MediaSession 屏幕推流验证 ===\n");

    let channel_id = std::env::var("KAEDE_CHANNEL_ID").unwrap_or_else(|_| ROOM.into());
    let api_secret = std::env::var("LIVEKIT_API_SECRET").unwrap_or_else(|_| API_SECRET.into());

    let token = make_dev_token(API_KEY, &api_secret, ROOM, IDENTITY);
    let e2ee_key = derive_e2ee_key(&api_secret, &channel_id);

    let cfg = ConnectConfig {
        ws_url: LIVEKIT_WS.into(),
        token,
        e2ee_key: Some(zeroize::Zeroizing::new(e2ee_key)),
        room_name: format!("kaede-{}", channel_id),
    };

    println!("[1] connect...");
    let session = MediaSession::connect(cfg).await?;
    println!(
        "    uid={} room={}",
        session.local_uid(),
        session.room_name()
    );

    let mut events = session.events();
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(ev) => tracing::info!("[event] {:?}", ev),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("[event] lagged {}", n);
                }
                Err(_) => break,
            }
        }
    });

    println!("[2] publish screen...");
    let video_cfg = VideoConfig {
        encoder: EncoderKind::Auto,
        fps: 60,
        bitrate_kbps: 8000,
        show_cursor: true,
        display_index: 0,
        width: None, // C-5: Some(W) 时启用 videoscale
        height: None,
    };
    session
        .publish(PublishSource::Screen { config: video_cfg })
        .await?;
    println!("    已发布");

    let secs: u64 = std::env::var("KAEDE_PUSH_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    println!("\n[3] 推流 {} 秒...", secs);
    tokio::time::sleep(Duration::from_secs(secs)).await;

    println!("[4] disconnect");
    session.disconnect().await;
    println!("\nPASS session_push");
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

/// dev-only：内联模拟 server 的 `derive_e2ee_key`（examples 不依赖 kaede-client）。
///
/// **生产代码不要用这个**。正确做法：`RoomSession::export_livekit_e2ee_key(ch)`
/// （MLS exporter 派生，server 零参与）。
fn derive_e2ee_key(api_secret: &str, channel_id: &str) -> [u8; 32] {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(api_secret.as_bytes()).expect("hmac init");
    mac.update(b"kaede-e2ee-v1:");
    mac.update(channel_id.as_bytes());
    let out = mac.finalize().into_bytes();
    let mut k = [0u8; 32];
    k.copy_from_slice(&out);
    k
}
