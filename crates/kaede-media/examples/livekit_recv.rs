//! LiveKit 订阅端：接收远端音频 → 播放。
//!
//! 运行：
//!   1. 另一个窗口：cargo run --release -p kaede-media --example livekit_audio_push
//!   2. 本窗口：   cargo run --release -p kaede-media --example livekit_recv

#![allow(clippy::field_reassign_with_default)]

use std::time::Duration;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use futures_util::StreamExt;
use kaede_media::AudioPlayer;
use livekit::prelude::*;
use livekit::webrtc::audio_stream::native::NativeAudioStream;

const ROOM: &str = "kaede-test-audio";
const IDENTITY: &str = "bob-listener";
const API_KEY: &str = "devkey";
const API_SECRET: &str = "secret";
const LIVEKIT_WS: &str = "ws://localhost:7880";
// E2EE passphrase 由 server 同算法派生

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    println!("=== LiveKit 订阅端（audio）===\n");

    let token = make_dev_token(API_KEY, API_SECRET, ROOM, IDENTITY);
    let channel_id = std::env::var("KAEDE_CHANNEL_ID").unwrap_or_else(|_| ROOM.into());
    let api_secret = std::env::var("LIVEKIT_API_SECRET").unwrap_or_else(|_| API_SECRET.into());
    let e2ee_passphrase = derive_e2ee_passphrase(&api_secret, &channel_id);
    println!("[1] token 已生成; E2EE passphrase={}", e2ee_passphrase);

    let mut room_options = RoomOptions::default();
    {
        use livekit::e2ee::key_provider::{
            KeyDerivationAlgorithm, KeyProvider, KeyProviderOptions,
        };
        use livekit::e2ee::{E2eeOptions, EncryptionType};
        let mut kp_opts = KeyProviderOptions::default();
        kp_opts.key_derivation_algorithm = KeyDerivationAlgorithm::PBKDF2;
        kp_opts.ratchet_salt = b"LKFrameEncryptionKey".to_vec();
        let kp = KeyProvider::with_shared_key(kp_opts, e2ee_passphrase.as_bytes().to_vec());
        room_options.encryption = Some(E2eeOptions {
            encryption_type: EncryptionType::Gcm,
            key_provider: kp,
        });
    }
    println!("[2] E2EE 已配置");

    let (room, mut events) = Room::connect(LIVEKIT_WS, &token, room_options).await?;
    println!("[3] 已连接 room={}", room.name());

    let listen_secs: u64 = std::env::var("KAEDE_LISTEN_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);

    println!("[4] 监听 {} 秒，等待远端 audio track...", listen_secs);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(listen_secs);
    loop {
        let timeout = deadline.saturating_duration_since(tokio::time::Instant::now());
        if timeout.is_zero() {
            break;
        }
        match tokio::time::timeout(timeout, events.recv()).await {
            Ok(Some(RoomEvent::TrackSubscribed {
                track, participant, ..
            })) => {
                match track {
                    RemoteTrack::Audio(audio) => {
                        println!(
                            "[5] 收到音频 track sid={} from={}",
                            audio.sid(),
                            participant.identity()
                        );
                        let rtc_track = audio.rtc_track();
                        let player = AudioPlayer::new(48000, 2)?;
                        let player = std::sync::Arc::new(player);

                        let mut stream = NativeAudioStream::new(rtc_track, 48000, 2);
                        let player_for_task = player.clone();
                        let task = tokio::spawn(async move {
                            while let Some(frame) = stream.next().await {
                                // frame.data: Cow<[i16]>
                                let mut bytes = Vec::with_capacity(frame.data.len() * 2);
                                for s in frame.data.iter() {
                                    bytes.extend_from_slice(&s.to_le_bytes());
                                }
                                let pts = 0i64;
                                if let Err(e) = player_for_task.push(&bytes, pts) {
                                    eprintln!("playback: {}", e);
                                    break;
                                }
                            }
                        });

                        tokio::select! {
                            _ = task => {}
                            _ = tokio::time::sleep_until(deadline) => {
                                break;
                            }
                        }
                    }
                    RemoteTrack::Video(_) => {
                        println!("[!] 收到视频 track（忽略）");
                    }
                }
            }
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(_) => break,
        }
    }

    println!("[6] 结束");
    drop(room);
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

/// 与 kaede-server 相同的派生算法：
/// HMAC-SHA256(api_secret, "kaede-e2ee-v1:" || channel_id) → hex 字符串
fn derive_e2ee_passphrase(api_secret: &str, channel_id: &str) -> String {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(api_secret.as_bytes()).expect("hmac init");
    mac.update(b"kaede-e2ee-v1:");
    mac.update(channel_id.as_bytes());
    let bytes = mac.finalize().into_bytes();
    URL_SAFE_NO_PAD.encode(bytes)
}
