//! 用 MediaSession 订阅 + 显示远端视频 + 播放远端音频。
//!
//! 运行：
//!   窗口 B: KAEDE_PUSH_SECS=30 cargo run --release -p kaede-media-session --example session_push
//!   窗口 C: KAEDE_LISTEN_SECS=25 cargo run --release -p kaede-media-session --example session_recv

#![allow(clippy::for_kv_map)]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use kaede_media::{AudioPlayer, LiveKitVideoSink};
use kaede_media_session::{
    ConnectConfig, ConsumerHandle, MediaEvent, MediaSession, RemoteKind, TrackId,
};

const ROOM: &str = "kaede-test";
const IDENTITY: &str = "bob-listener";
const API_KEY: &str = "devkey";
const API_SECRET: &str = "secret";
const LIVEKIT_WS: &str = "ws://localhost:7880";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    println!("=== MediaSession 订阅端验证 ===\n");

    let channel_id = std::env::var("KAEDE_CHANNEL_ID").unwrap_or_else(|_| ROOM.into());
    let api_secret = std::env::var("LIVEKIT_API_SECRET").unwrap_or_else(|_| API_SECRET.into());

    let token = make_dev_token(API_KEY, &api_secret, ROOM, IDENTITY);
    let e2ee_key = derive_e2ee_key(&api_secret, &channel_id);

    let cfg = ConnectConfig {
        ws_url: LIVEKIT_WS.into(),
        token,
        e2ee_key,
        room_name: format!("kaede-{}", channel_id),
    };

    println!("[1] connect...");
    let session = MediaSession::connect(cfg).await?;
    println!(
        "    uid={} room={}",
        session.local_uid(),
        session.room_name()
    );

    let listen_secs: u64 = std::env::var("KAEDE_LISTEN_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    println!("[2] 监听 {} 秒...\n", listen_secs);

    let mut events = session.events();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(listen_secs);

    // 持有的 sink / player / consumer，生命周期跟 task 走
    let mut videos: HashMap<TrackId, (Arc<LiveKitVideoSink>, ConsumerHandle)> = HashMap::new();
    let mut audios: HashMap<TrackId, (Arc<AudioPlayer>, ConsumerHandle)> = HashMap::new();

    loop {
        let timeout = deadline.saturating_duration_since(tokio::time::Instant::now());
        if timeout.is_zero() {
            break;
        }
        match tokio::time::timeout(timeout, events.recv()).await {
            Ok(Ok(ev)) => match ev {
                MediaEvent::TrackAvailable { track, kind } => {
                    println!("[+] TrackAvailable {:?} {}", kind, track.sid);
                    match kind {
                        RemoteKind::Video => {
                            let sink = Arc::new(LiveKitVideoSink::new()?);
                            match session.attach_video(track.clone(), sink.clone()) {
                                Ok(h) => {
                                    println!("    attached video -> GStreamer");
                                    videos.insert(track, (sink, h));
                                }
                                Err(e) => eprintln!("    attach_video failed: {}", e),
                            }
                        }
                        RemoteKind::Audio => {
                            let player = Arc::new(AudioPlayer::new(48000, 2)?);
                            match session.attach_audio(track.clone(), player.clone()) {
                                Ok(h) => {
                                    println!("    attached audio -> GStreamer");
                                    audios.insert(track, (player, h));
                                }
                                Err(e) => eprintln!("    attach_audio failed: {}", e),
                            }
                        }
                    }
                }
                MediaEvent::TrackGone { track, kind } => {
                    println!("[-] TrackGone {:?} {}", kind, track.sid);
                    match kind {
                        RemoteKind::Video => {
                            videos.remove(&track);
                        }
                        RemoteKind::Audio => {
                            audios.remove(&track);
                        }
                    }
                }
                MediaEvent::PeerJoined { uid } => println!("[+] PeerJoined {}", uid),
                MediaEvent::PeerLeft { uid } => println!("[-] PeerLeft {}", uid),
                MediaEvent::Connected { room_name } => {
                    println!("[*] Connected room={}", room_name)
                }
                MediaEvent::Disconnected { reason } => {
                    println!("[*] Disconnected: {}", reason)
                }
                MediaEvent::LocalPublished { kind, sid } => {
                    println!("[*] LocalPublished {:?} {}", kind, sid)
                }
                MediaEvent::LocalUnpublished { kind } => {
                    println!("[*] LocalUnpublished {:?}", kind)
                }
                MediaEvent::Error { message } => {
                    eprintln!("[!] Error: {}", message)
                }

                _ => {}
            },
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(n))) => {
                tracing::warn!("events lagged {}", n);
            }
            Ok(Err(_)) => break,
            Err(_) => break, // timeout
        }
    }

    println!("\n[3] 收尾");
    for (_, (sink, _h)) in &videos {
        sink.stop();
    }
    for (_, (player, _h)) in &audios {
        player.stop();
    }
    videos.clear();
    audios.clear();

    session.disconnect().await;
    println!("PASS session_recv");
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
