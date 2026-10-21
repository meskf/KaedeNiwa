//! LiveKit 推流。
//!
//! # 架构
//!
//! ```text
//! EncodedSource ──> LiveKitSession (采集线程, 明文帧)
//!                     │
//!                     └─ async_channel(8) ──> tokio task
//!                                               │
//!                                               v
//!                                         LiveKitPusher::push_encoded
//!                                               │
//!                                               v
//!                                         LiveKit RTP (E2EE)
//! ```
//!
//! 加密由 LiveKit 内建 E2EE（AES-GCM shared key）负责。
//! 需要应用层 SFrame 的场景（文件落盘 / 自建 SFU）继续用 `MediaSession`。

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_channel::TrySendError;
use tokio::sync::broadcast;

use crate::error::MediaError;
use crate::event::{MediaEvent, StopReason};
use crate::source::{EncodedFrame, EncodedSource};
use crate::stats::{MediaState, MediaStats};

const QUEUE_CAPACITY: usize = 8;
const EVENTS_CHANNEL_CAPACITY: usize = 64;

// ============================================================
// 配置
// ============================================================

/// LiveKit 连接配置。
#[derive(Clone)]
pub struct LiveKitConfig {
    /// LiveKit 信令 WS URL，例如 `ws://localhost:7880`。
    pub ws_url: String,
    /// 服务端签发的 JWT。
    pub token: String,
    /// 分辨率（width, height）。
    pub resolution: (u32, u32),
    /// E2EE 共享密钥（16 或 32 字节）。`None` 则只走 DTLS-SRTP。
    pub e2ee_key: Option<Vec<u8>>,
}

impl std::fmt::Debug for LiveKitConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveKitConfig")
            .field("ws_url", &self.ws_url)
            .field("token", &"***")
            .field("resolution", &self.resolution)
            .field("has_e2ee_key", &self.e2ee_key.is_some())
            .finish()
    }
}

impl LiveKitConfig {
    pub fn new(ws_url: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            ws_url: ws_url.into(),
            token: token.into(),
            resolution: (1920, 1080),
            e2ee_key: None,
        }
    }

    pub fn with_resolution(mut self, w: u32, h: u32) -> Self {
        self.resolution = (w, h);
        self
    }

    /// 启用 E2EE。参数是 32 字节原始 key。推荐来源：`RoomSession::export_livekit_e2ee_key()`（MLS exporter，server 零参与）。老 `get_livekit_e2ee_key()` 走 server HTTP，已 deprecated。
    pub fn with_e2ee_key(mut self, key: Vec<u8>) -> Self {
        self.e2ee_key = Some(key);
        self
    }

}

// ============================================================
// Pusher
// ============================================================

#[derive(Default)]
struct PusherStats {
    frames_injected: AtomicU64,
    frames_dropped: AtomicU64,
    bytes_injected: AtomicU64,
}

/// LiveKit 推流器。
///
/// 连接、发布轨道、同步 push 明文帧。**不管理线程或 channel** ——
/// 上层 `LiveKitSession` 决定调度策略。
pub struct LiveKitPusher {
    source: livekit::webrtc::video_source::native::NativeVideoSource,
    resolution: livekit::webrtc::video_source::VideoResolution,
    /// H23: drop 会断开 LiveKit 连接。不可删除此字段（保活资源）。
    #[allow(dead_code)]
    room: livekit::Room,
    stats: PusherStats,
    closed: AtomicBool,
}

impl LiveKitPusher {
    pub async fn connect(config: LiveKitConfig) -> Result<Self, MediaError> {
        use livekit::options::{TrackPublishOptions, VideoCodec, VideoEncoderBackend};
        use livekit::prelude::*;
        use livekit::webrtc::video_source::{
            RtcVideoSource, VideoResolution, native::NativeVideoSource,
        };

        tracing::info!("[livekit] connecting to {}", config.ws_url);

        let room_options = build_room_options(&config);
        let (room, _events) = Room::connect(&config.ws_url, &config.token, room_options)
            .await
            .map_err(|e| MediaError::Sink(format!("Room::connect: {:?}", e)))?;

        tracing::info!("[livekit] connected, room={}", room.name());

        let resolution = VideoResolution {
            width: config.resolution.0,
            height: config.resolution.1,
        };
        let source = NativeVideoSource::new_encoded(resolution.clone());

        let track =
            LocalVideoTrack::create_video_track("screen", RtcVideoSource::Native(source.clone()));

        // E2EE 依赖 packet_trailer 承载加密元数据。
        let mut fm = livekit::options::FrameMetadataFeatures::default();
        fm.user_timestamp = true;
        fm.frame_id = true;
        fm.user_data = true;

        let options = TrackPublishOptions {
            video_codec: VideoCodec::H264,
            video_encoder: VideoEncoderBackend::PreEncoded,
            simulcast: false,
            source: TrackSource::Screenshare,
            frame_metadata_features: fm,
            ..Default::default()
        };

        room.local_participant()
            .publish_track(LocalTrack::Video(track), options)
            .await
            .map_err(|e| MediaError::Sink(format!("publish_track: {:?}", e)))?;

        tracing::info!("[livekit] video track published");

        Ok(Self {
            source,
            resolution,
            room,
            stats: PusherStats::default(),
            closed: AtomicBool::new(false),
        })
    }

    /// 同步推一帧明文 H.264 Annex B。返回 false 表示被丢弃。
    pub fn push_encoded(&self, data: &[u8], is_keyframe: bool, pts_ms: i64) -> bool {
        use livekit::webrtc::video_frame::{
            EncodedFrameType, EncodedVideoCodec, EncodedVideoFrame,
        };

        if self.closed.load(Ordering::Relaxed) {
            return false;
        }

        let frame = EncodedVideoFrame {
            codec: EncodedVideoCodec::H264,
            payload: data,
            timestamp_us: pts_ms.saturating_mul(1000),
            frame_type: if is_keyframe {
                EncodedFrameType::Key
            } else {
                EncodedFrameType::Delta
            },
            resolution: self.resolution.clone(),
            frame_metadata: None,
        };

        if self.source.capture_encoded_frame(&frame) {
            self.stats.frames_injected.fetch_add(1, Ordering::Relaxed);
            self.stats
                .bytes_injected
                .fetch_add(data.len() as u64, Ordering::Relaxed);
            true
        } else {
            self.stats.frames_dropped.fetch_add(1, Ordering::Relaxed);
            false
        }
    }

    pub fn frames_injected(&self) -> u64 {
        self.stats.frames_injected.load(Ordering::Relaxed)
    }

    pub fn frames_dropped(&self) -> u64 {
        self.stats.frames_dropped.load(Ordering::Relaxed)
    }

    pub fn bytes_injected(&self) -> u64 {
        self.stats.bytes_injected.load(Ordering::Relaxed)
    }

    /// 标记关闭。后续 `push_encoded` 直接返回 false。
    pub fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
    }
}

fn hex_key(key: &[u8]) -> String {
    let mut s = String::with_capacity(key.len() * 2);
    for b in key {
        use std::fmt::Write as _;
        let _ = write!(&mut s, "{b:02x}");
    }
    s
}

fn build_room_options(config: &LiveKitConfig) -> livekit::RoomOptions {
    use livekit::RoomOptions;
    use livekit::e2ee::key_provider::{KeyDerivationAlgorithm, KeyProvider, KeyProviderOptions};
    use livekit::e2ee::{E2eeOptions, EncryptionType};

    let mut opts = RoomOptions::default();

    if let Some(ref key) = config.e2ee_key {
        let kp_opts = KeyProviderOptions {
            key_derivation_algorithm: KeyDerivationAlgorithm::PBKDF2,
            ratchet_salt: b"LKFrameEncryptionKey".to_vec(),
            ..Default::default()
        };
        // 与 Web 端 PBKDF2 输入保持一致：hex 文本而不是原始二进制。
        let password = hex_key(key);
        let key_provider = KeyProvider::with_shared_key(kp_opts, password.into_bytes());

        opts.encryption = Some(E2eeOptions {
            encryption_type: EncryptionType::Gcm,
            key_provider,
        });
        tracing::info!("[livekit] E2EE enabled (AES-GCM, shared key)");
    }

    opts
}

// ============================================================
// Session
// ============================================================

struct SessionStats {
    frames_captured: AtomicU64,
    frames_sent: AtomicU64,
    frames_dropped: AtomicU64,
    bytes_sent: AtomicU64,
    last_keyframe_pts_ms: AtomicI64,
}

impl SessionStats {
    fn new() -> Self {
        Self {
            frames_captured: AtomicU64::new(0),
            frames_sent: AtomicU64::new(0),
            frames_dropped: AtomicU64::new(0),
            bytes_sent: AtomicU64::new(0),
            last_keyframe_pts_ms: AtomicI64::new(i64::MIN),
        }
    }
}

/// LiveKit 直连会话：`EncodedSource` → LiveKit E2EE。
///
/// 与 [`crate::MediaSession`] 的区别：不做 SFrame 加密，
/// 加密由 LiveKit E2EE 负责。
pub struct LiveKitSession {
    state: Arc<AtomicU8>,
    stop_flag: Arc<AtomicBool>,
    stats: Arc<SessionStats>,
    events_tx: broadcast::Sender<MediaEvent>,
    collector: Mutex<Option<std::thread::JoinHandle<()>>>,
    pusher_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    pusher: Arc<LiveKitPusher>,
}

impl LiveKitSession {
    /// 建立 LiveKit 连接 + 启动采集线程 + 推流 task。
    ///
    /// **必须在 tokio runtime 上下文调用。**
    pub async fn start<S>(mut source: S, config: LiveKitConfig) -> Result<Arc<Self>, MediaError>
    where
        S: EncodedSource,
    {
        let state = Arc::new(AtomicU8::new(MediaState::Starting.as_u8()));
        let stop_flag = Arc::new(AtomicBool::new(false));
        let stats = Arc::new(SessionStats::new());
        let (events_tx, _) = broadcast::channel::<MediaEvent>(EVENTS_CHANNEL_CAPACITY);
        let (frame_tx, frame_rx) = async_channel::bounded::<EncodedFrame>(QUEUE_CAPACITY);

        // 1. 连接 LiveKit
        let pusher = Arc::new(LiveKitPusher::connect(config).await?);

        // 2. 采集线程：直接读帧，不加密
        let collector = {
            let stop = stop_flag.clone();
            let stats = stats.clone();
            let events = events_tx.clone();
            let state = state.clone();

            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match source.next_frame_timeout(Duration::from_millis(200)) {
                        Ok(Some(f)) => {
                            let is_kf = f.is_keyframe;
                            let pts = f.pts_ms;
                            let len = f.data.len();

                            stats.frames_captured.fetch_add(1, Ordering::Relaxed);
                            if is_kf {
                                stats.last_keyframe_pts_ms.store(pts, Ordering::Relaxed);
                            }

                            let _ = events.send(MediaEvent::FrameEncrypted {
                                pts_ms: pts,
                                bytes: len,
                                is_keyframe: is_kf,
                            });

                            match frame_tx.try_send(f) {
                                Ok(()) => {}
                                Err(TrySendError::Full(_)) => {
                                    stats.frames_dropped.fetch_add(1, Ordering::Relaxed);
                                    let _ = events.send(MediaEvent::FrameDropped { pts_ms: pts });
                                }
                                Err(TrySendError::Closed(_)) => break,
                            }
                        }
                        Ok(None) => {
                            let _ = events.send(MediaEvent::Stopped {
                                reason: StopReason::SourceEnded,
                            });
                            break;
                        }
                        Err(MediaError::Timeout) => continue,
                        Err(e) => {
                            let _ = events.send(MediaEvent::Error {
                                message: e.to_string(),
                            });
                            state.store(MediaState::Failed.as_u8(), Ordering::Relaxed);
                            break;
                        }
                    }
                }
                source.stop();
            })
        };

        // 3. 推流 task
        let pusher_task = {
            let stats = stats.clone();
            let events = events_tx.clone();
            let state = state.clone();
            let pusher = pusher.clone();

            tokio::spawn(async move {
                while let Ok(f) = frame_rx.recv().await {
                    let len = f.data.len() as u64;
                    if pusher.push_encoded(&f.data, f.is_keyframe, f.pts_ms) {
                        stats.frames_sent.fetch_add(1, Ordering::Relaxed);
                        stats.bytes_sent.fetch_add(len, Ordering::Relaxed);
                    } else {
                        stats.frames_dropped.fetch_add(1, Ordering::Relaxed);
                        let _ = events.send(MediaEvent::FrameDropped { pts_ms: f.pts_ms });
                    }
                }
                // sink_task 结束时不一定 Failed —— channel closed 是正常退出
                let _ = &state; // 保留绑定，避免未用告警
            })
        };

        state.store(MediaState::Active.as_u8(), Ordering::Relaxed);
        let _ = events_tx.send(MediaEvent::Started);

        Ok(Arc::new(Self {
            state,
            stop_flag,
            stats,
            events_tx,
            collector: Mutex::new(Some(collector)),
            pusher_task: Mutex::new(Some(pusher_task)),
            pusher,
        }))
    }

    pub fn state(&self) -> MediaState {
        MediaState::from_u8(self.state.load(Ordering::Relaxed))
    }

    /// 统计快照。注意：`frames_encrypted` 字段在 LiveKit 路径下代表
    /// "采集到的明文帧数"，`bytes_encrypted` 恒为 0（不适用）。
    pub fn stats(&self) -> MediaStats {
        let kf = self.stats.last_keyframe_pts_ms.load(Ordering::Relaxed);
        MediaStats {
            state: self.state(),
            frames_encrypted: self.stats.frames_captured.load(Ordering::Relaxed),
            frames_sent: self.stats.frames_sent.load(Ordering::Relaxed),
            frames_dropped: self.stats.frames_dropped.load(Ordering::Relaxed),
            bytes_encrypted: 0,
            bytes_sent: self.stats.bytes_sent.load(Ordering::Relaxed),
            last_keyframe_pts_ms: if kf == i64::MIN { None } else { Some(kf) },
        }
    }

    /// 底层 pusher 引用（可选，用于额外查询注入统计）。
    pub fn pusher(&self) -> &Arc<LiveKitPusher> {
        &self.pusher
    }

    pub fn events(&self) -> broadcast::Receiver<MediaEvent> {
        self.events_tx.subscribe()
    }

    /// 停止会话。幂等。
    pub async fn stop(&self) {
        if self.state().is_terminal() {
            return;
        }
        self.stop_flag.store(true, Ordering::Relaxed);
        self.state
            .store(MediaState::Stopping.as_u8(), Ordering::Relaxed);

        // H21: detached std::thread join（同 MediaSession::stop）
        let collector_task = self.collector.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(h) = collector_task {
            std::thread::spawn(move || {
                let _ = h.join();
            });
        }
        // H22: abort 而非 timeout
        let pusher_task = self.pusher_task.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(h) = pusher_task {
            h.abort();
            let _ = h.await;
        }

        self.pusher.close();
        self.state
            .store(MediaState::Stopped.as_u8(), Ordering::Relaxed);
        let _ = self.events_tx.send(MediaEvent::Stopped {
            reason: StopReason::UserRequest,
        });
    }
}

impl Drop for LiveKitSession {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        // C-4: detached join 采集线程，避免 OS 线程泄漏
        // （同步 join 会阻塞 Drop；这里用 std::thread 后台 join）
        let collector_task = self.collector.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(h) = collector_task {
            std::thread::spawn(move || {
                let _ = h.join();
            });
        }
    }
}

// ============================================================
// 音频
// ============================================================

/// LiveKit 音频推流器（`NativeAudioSource` → LiveKit）。
pub struct LiveKitAudioPusher {
    source: livekit::webrtc::audio_source::native::NativeAudioSource,
    /// H23: 保活字段，不可删除。
    #[allow(dead_code)]
    room: livekit::Room,
    sample_rate: u32,
    channels: u32,
    stats: PusherStats,
    closed: AtomicBool,
}

impl LiveKitAudioPusher {
    pub async fn connect(
        config: LiveKitConfig,
        sample_rate: u32,
        channels: u32,
    ) -> Result<Self, MediaError> {
        use livekit::options::TrackPublishOptions;
        use livekit::prelude::*;
        use livekit::webrtc::audio_source::RtcAudioSource;
        use livekit::webrtc::audio_source::native::NativeAudioSource;

        tracing::info!("[livekit-audio] connecting to {}", config.ws_url);

        let room_options = build_room_options(&config);
        let (room, _events) = Room::connect(&config.ws_url, &config.token, room_options)
            .await
            .map_err(|e| MediaError::Sink(format!("Room::connect: {:?}", e)))?;

        tracing::info!("[livekit-audio] connected, room={}", room.name());

        // NativeAudioSource::new(options, sample_rate, num_channels, queue_size_ms)
        // 内部缓冲区大小（100ms）控制积压容忍度。
        let source = NativeAudioSource::new(
            livekit::webrtc::audio_source::AudioSourceOptions::default(),
            sample_rate,
            channels,
            100,
        );

        let track =
            LocalAudioTrack::create_audio_track("mic", RtcAudioSource::Native(source.clone()));

        let options = TrackPublishOptions {
            source: TrackSource::Microphone,
            ..Default::default()
        };

        room.local_participant()
            .publish_track(LocalTrack::Audio(track), options)
            .await
            .map_err(|e| MediaError::Sink(format!("publish_track(audio): {:?}", e)))?;

        tracing::info!("[livekit-audio] audio track published");

        Ok(Self {
            source,
            room,
            sample_rate,
            channels,
            stats: PusherStats::default(),
            closed: AtomicBool::new(false),
        })
    }

    /// 推 PCM S16LE 交错数据。`samples_per_channel` = 帧内每声道样本数。
    pub async fn push_pcm(&self, data: &[u8], samples_per_channel: u32, _pts_ms: i64) -> bool {
        use livekit::webrtc::audio_frame::AudioFrame;

        if self.closed.load(Ordering::Relaxed) {
            return false;
        }

        // 转 i16
        let mut samples: Vec<i16> = Vec::with_capacity(data.len() / 2);
        // M42: PCM S16LE 要求偶数长度。奇数尾字节静默丢弃，但留日志方便诊断
        let (pairs, rem) = data.as_chunks::<2>();
        if !rem.is_empty() {
            tracing::warn!(
                "push_pcm: odd data len {} (dropping trailing {} byte)",
                data.len(),
                rem.len()
            );
        }
        for &[lo, hi] in pairs {
            samples.push(i16::from_le_bytes([lo, hi]));
        }

        let frame = AudioFrame {
            data: samples.into(),
            sample_rate: self.sample_rate,
            num_channels: self.channels,
            samples_per_channel,
        };

        // NativeAudioSource::capture_frame 是同步的
        match self.source.capture_frame(&frame).await {
            Ok(_) => {
                self.stats.frames_injected.fetch_add(1, Ordering::Relaxed);
                self.stats
                    .bytes_injected
                    .fetch_add(data.len() as u64, Ordering::Relaxed);
                true
            }
            Err(e) => {
                tracing::warn!("[livekit-audio] capture_frame: {:?}", e);
                self.stats.frames_dropped.fetch_add(1, Ordering::Relaxed);
                false
            }
        }
    }

    pub fn frames_injected(&self) -> u64 {
        self.stats.frames_injected.load(Ordering::Relaxed)
    }

    pub fn frames_dropped(&self) -> u64 {
        self.stats.frames_dropped.load(Ordering::Relaxed)
    }

    pub fn bytes_injected(&self) -> u64 {
        self.stats.bytes_injected.load(Ordering::Relaxed)
    }

    pub fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
    }
}

/// LiveKit 音频直连会话（`AudioSource` → LiveKit E2EE）。
pub struct LiveKitAudioSession {
    state: Arc<AtomicU8>,
    stop_flag: Arc<AtomicBool>,
    stats: Arc<SessionStats>,
    events_tx: broadcast::Sender<MediaEvent>,
    collector: Mutex<Option<std::thread::JoinHandle<()>>>,
    pusher_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    pusher: Arc<LiveKitAudioPusher>,
}

impl LiveKitAudioSession {
    pub async fn start<S>(
        mut source: S,
        config: LiveKitConfig,
        sample_rate: u32,
        channels: u32,
    ) -> Result<Arc<Self>, MediaError>
    where
        S: crate::source::AudioSource,
    {
        let state = Arc::new(AtomicU8::new(MediaState::Starting.as_u8()));
        let stop_flag = Arc::new(AtomicBool::new(false));
        let stats = Arc::new(SessionStats::new());
        let (events_tx, _) = broadcast::channel::<MediaEvent>(EVENTS_CHANNEL_CAPACITY);
        let (frame_tx, frame_rx) =
            async_channel::bounded::<crate::source::AudioFrame>(QUEUE_CAPACITY);

        let pusher = Arc::new(LiveKitAudioPusher::connect(config, sample_rate, channels).await?);

        let collector = {
            let stop = stop_flag.clone();
            let stats = stats.clone();
            let events = events_tx.clone();
            let state = state.clone();
            let sr = sample_rate;
            let ch = channels as u16;

            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match source.next_audio_timeout(Duration::from_millis(200)) {
                        Ok(Some(f)) => {
                            let pts = f.pts_ms;
                            let len = f.data.len();
                            stats.frames_captured.fetch_add(1, Ordering::Relaxed);
                            let _ = events.send(MediaEvent::FrameEncrypted {
                                pts_ms: pts,
                                bytes: len,
                                is_keyframe: false,
                            });
                            // 直接改字段，兼容不同采集源的采样率
                            let mut f = f;
                            if f.sample_rate == 0 {
                                f.sample_rate = sr;
                            }
                            if f.channels == 0 {
                                f.channels = ch;
                            }

                            match frame_tx.try_send(f) {
                                Ok(()) => {}
                                Err(TrySendError::Full(_)) => {
                                    stats.frames_dropped.fetch_add(1, Ordering::Relaxed);
                                    let _ = events.send(MediaEvent::FrameDropped { pts_ms: pts });
                                }
                                Err(TrySendError::Closed(_)) => break,
                            }
                        }
                        Ok(None) => {
                            let _ = events.send(MediaEvent::Stopped {
                                reason: StopReason::SourceEnded,
                            });
                            break;
                        }
                        Err(MediaError::Timeout) => continue,
                        Err(e) => {
                            let _ = events.send(MediaEvent::Error {
                                message: e.to_string(),
                            });
                            state.store(MediaState::Failed.as_u8(), Ordering::Relaxed);
                            break;
                        }
                    }
                }
                source.stop();
            })
        };

        let pusher_task = {
            let stats = stats.clone();
            let events = events_tx.clone();
            let state = state.clone();
            let pusher = pusher.clone();
            let bytes_per_sample_frame: u32 = 2 * channels;

            tokio::spawn(async move {
                while let Ok(f) = frame_rx.recv().await {
                    let len = f.data.len() as u64;
                    let samples_per_channel = (f.data.len() as u32)
                        .checked_div(bytes_per_sample_frame)
                        .unwrap_or(0);
                    if pusher
                        .push_pcm(&f.data, samples_per_channel, f.pts_ms)
                        .await
                    {
                        stats.frames_sent.fetch_add(1, Ordering::Relaxed);
                        stats.bytes_sent.fetch_add(len, Ordering::Relaxed);
                    } else {
                        stats.frames_dropped.fetch_add(1, Ordering::Relaxed);
                        let _ = events.send(MediaEvent::FrameDropped { pts_ms: f.pts_ms });
                    }
                }
                let _ = &state;
            })
        };

        state.store(MediaState::Active.as_u8(), Ordering::Relaxed);
        let _ = events_tx.send(MediaEvent::Started);

        Ok(Arc::new(Self {
            state,
            stop_flag,
            stats,
            events_tx,
            collector: Mutex::new(Some(collector)),
            pusher_task: Mutex::new(Some(pusher_task)),
            pusher,
        }))
    }

    pub fn state(&self) -> MediaState {
        MediaState::from_u8(self.state.load(Ordering::Relaxed))
    }

    pub fn stats(&self) -> MediaStats {
        MediaStats {
            state: self.state(),
            frames_encrypted: self.stats.frames_captured.load(Ordering::Relaxed),
            frames_sent: self.stats.frames_sent.load(Ordering::Relaxed),
            frames_dropped: self.stats.frames_dropped.load(Ordering::Relaxed),
            bytes_encrypted: 0,
            bytes_sent: self.stats.bytes_sent.load(Ordering::Relaxed),
            last_keyframe_pts_ms: None,
        }
    }

    pub fn pusher(&self) -> &Arc<LiveKitAudioPusher> {
        &self.pusher
    }

    pub fn events(&self) -> broadcast::Receiver<MediaEvent> {
        self.events_tx.subscribe()
    }

    pub async fn stop(&self) {
        if self.state().is_terminal() {
            return;
        }
        self.stop_flag.store(true, Ordering::Relaxed);
        self.state
            .store(MediaState::Stopping.as_u8(), Ordering::Relaxed);

        // H21: detached std::thread join
        let collector_task = self.collector.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(h) = collector_task {
            std::thread::spawn(move || {
                let _ = h.join();
            });
        }
        // H22: abort 而非 timeout
        let pusher_task = self.pusher_task.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(h) = pusher_task {
            h.abort();
            let _ = h.await;
        }

        self.pusher.close();
        self.state
            .store(MediaState::Stopped.as_u8(), Ordering::Relaxed);
        let _ = self.events_tx.send(MediaEvent::Stopped {
            reason: StopReason::UserRequest,
        });
    }
}

impl Drop for LiveKitAudioSession {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        // C-4: detached join 采集线程，避免 OS 线程泄漏
        // （同步 join 会阻塞 Drop；这里用 std::thread 后台 join）
        let collector_task = self.collector.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(h) = collector_task {
            std::thread::spawn(move || {
                let _ = h.join();
            });
        }
    }
}
