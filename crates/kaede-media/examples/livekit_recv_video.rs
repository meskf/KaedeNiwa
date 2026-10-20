//! LiveKit 视频订阅 + 显示（I420 传输）。
//!
//! 流程：远端视频 track → LiveKit SDK 解码 → BoxVideoFrame
//!       → to_i420() → 拼接 Y+U+V → LiveKitVideoSink → GStreamer
//!
//! 运行：
//!   1. 另一个窗口：cargo run --release -p kaede-media --example livekit_push
//!   2. 本窗口：   KAEDE_LISTEN_SECS=30 cargo run --release -p kaede-media --example livekit_recv_video

#![allow(clippy::field_reassign_with_default, clippy::manual_is_multiple_of)]

use std::time::Duration;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use futures_util::StreamExt;
use livekit::prelude::*;
use livekit::webrtc::video_stream::native::NativeVideoStream;

use kaede_media::LiveKitVideoSink;

const ROOM: &str = "kaede-test";
const IDENTITY: &str = "bob-video-listener";
const API_KEY: &str = "devkey";
const API_SECRET: &str = "secret";
const LIVEKIT_WS: &str = "ws://localhost:7880";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    println!("=== LiveKit 视频订阅端（I420 显示）===\n");

    let token = make_dev_token(API_KEY, API_SECRET, ROOM, IDENTITY);
    let channel_id = std::env::var("KAEDE_CHANNEL_ID").unwrap_or_else(|_| ROOM.into());
    let api_secret = std::env::var("LIVEKIT_API_SECRET").unwrap_or_else(|_| API_SECRET.into());
    let e2ee_passphrase = derive_e2ee_passphrase(&api_secret, &channel_id);
    println!("[1] token/passphrase 已生成 (channel={})", channel_id);

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

    let (_room, mut events) = Room::connect(LIVEKIT_WS, &token, room_options).await?;
    println!("[2] 已连接 room={}", ROOM);

    let listen_secs: u64 = std::env::var("KAEDE_LISTEN_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(listen_secs);
    println!("[3] 监听 {} 秒，等远端 video track...", listen_secs);

    loop {
        let timeout = deadline.saturating_duration_since(tokio::time::Instant::now());
        if timeout.is_zero() {
            break;
        }
        match tokio::time::timeout(timeout, events.recv()).await {
            Ok(Some(RoomEvent::TrackSubscribed {
                track, participant, ..
            })) => {
                if let RemoteTrack::Video(video) = track {
                    println!("[4] 收到视频 track from={}", participant.identity());

                    let sink = match LiveKitVideoSink::new() {
                        Ok(s) => s,
                        Err(e) => {
                            eprintln!("!! 创建显示 sink 失败: {}", e);
                            return Ok(());
                        }
                    };
                    println!("    GStreamer 显示 sink 已启动（I420）");

                    let mut stream = NativeVideoStream::new(video.rtc_track());
                    let mut count = 0u64;

                    loop {
                        let timeout =
                            deadline.saturating_duration_since(tokio::time::Instant::now());
                        if timeout.is_zero() {
                            break;
                        }
                        match tokio::time::timeout(timeout, stream.next()).await {
                            Ok(Some(frame)) => {
                                count += 1;

                                let buf = frame.buffer.as_ref();
                                let w = buf.width();
                                let h = buf.height();
                                if w == 0 || h == 0 {
                                    continue;
                                }

                                // I420：无字节序问题
                                let i420 = buf.to_i420();
                                let (y, u, v) = i420.data();
                                let mut packed = Vec::with_capacity(y.len() + u.len() + v.len());
                                packed.extend_from_slice(y);
                                packed.extend_from_slice(u);
                                packed.extend_from_slice(v);

                                let pts_ms = frame.timestamp_us / 1000;
                                match sink.push_i420(&packed, w, h, pts_ms) {
                                    Ok(true) => {}
                                    Ok(false) => {}
                                    Err(e) => eprintln!("  push_i420: {}", e),
                                }

                                if count % 60 == 0 {
                                    println!("    frame #{} ({}x{}, {} ms)", count, w, h, pts_ms);
                                }
                            }
                            Ok(None) => break,
                            Err(_) => break,
                        }
                    }

                    sink.stop();
                    println!(
                        "[5] 视频流结束，共 {} 帧，显示 {} 帧，丢 {} 帧",
                        count,
                        sink.frames_pushed(),
                        sink.frames_dropped()
                    );
                }
            }
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(_) => break,
        }
    }

    println!("[6] 结束");
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
