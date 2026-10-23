//! 媒体会话：一条 LiveKit 连接，多 track。

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use livekit::options::{TrackPublishOptions, VideoCodec, VideoEncoderBackend};
use livekit::prelude::*;
use livekit::webrtc::audio_frame::AudioFrame as LkAudioFrame;
use livekit::webrtc::audio_source::native::NativeAudioSource;
use livekit::webrtc::audio_source::{AudioSourceOptions, RtcAudioSource};
use livekit::webrtc::audio_stream::native::NativeAudioStream;
use livekit::webrtc::video_frame::{EncodedFrameType, EncodedVideoCodec, EncodedVideoFrame};
use livekit::webrtc::video_source::{RtcVideoSource, VideoResolution, native::NativeVideoSource};
use livekit::webrtc::video_stream::native::NativeVideoStream;
use tokio::sync::{broadcast, mpsc, oneshot};

use kaede_capture::{EncodedVideoStream, VideoConfig};
use kaede_media::{
    AudioCaptureConfig, AudioFrame, AudioPlayer, AudioSourceKind, GstAudioSource, LiveKitVideoSink,
    MediaError,
};

use crate::config::{ConnectConfig, PublishSource, TrackKind};
use crate::event::{MediaEvent, RemoteKind, ScreenStats, TrackId};

const EVENT_CAPACITY: usize = 128;

fn hex_key(key: &[u8]) -> String {
    let mut s = String::with_capacity(key.len() * 2);
    for b in key {
        use std::fmt::Write as _;
        let _ = write!(&mut s, "{b:02x}");
    }
    s
}

struct PublishedEntry {
    /// 真实 LiveKit publication sid，用于 unpublish_track。
    #[allow(dead_code)]
    sid: TrackSid,
    stop_tx: Option<oneshot::Sender<()>>,
    handle: tokio::task::JoinHandle<()>,
}

/// 「正在发布」占位。发布流程包含多次 `await`（采集器启动、publish_track），
/// 只用 `published.contains_key` 检查会让并发双击同时通过，后插入者覆盖先前的
/// `PublishedEntry` 并泄漏 track。这个 RAII 守卫在**同一临界区**内完成
/// 检查 + 占位，函数返回（成功或失败）时自动释放占位。
struct PublishClaim<'a> {
    set: &'a Mutex<HashSet<TrackKind>>,
    kind: TrackKind,
}

impl Drop for PublishClaim<'_> {
    fn drop(&mut self) {
        self.set
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.kind);
    }
}

pub struct ConsumerEntry {
    stop_tx: Option<oneshot::Sender<()>>,
    handle: tokio::task::JoinHandle<()>,
}

/// attach 返回的句柄。drop 时自动停止消费。
pub struct ConsumerHandle {
    inner: Option<ConsumerEntry>,
}

impl ConsumerHandle {
    /// 主动停止消费。
    pub async fn stop(mut self) {
        if let Some(mut e) = self.inner.take() {
            if let Some(tx) = e.stop_tx.take() {
                let _ = tx.send(());
            }
            let _ = e.handle.await;
        }
    }
}

impl Drop for ConsumerHandle {
    fn drop(&mut self) {
        if let Some(mut e) = self.inner.take() {
            if let Some(tx) = e.stop_tx.take() {
                let _ = tx.send(());
            }
            e.handle.abort();
        }
    }
}

/// 媒体会话。
pub struct MediaSession {
    room: Arc<livekit::Room>,
    local_uid: String,
    room_name: String,
    events_tx: broadcast::Sender<MediaEvent>,
    published: Arc<Mutex<HashMap<TrackKind, PublishedEntry>>>,
    /// 正在发布中的 kind（并发发布占位，见 [`PublishClaim`]）。
    publishing: Mutex<HashSet<TrackKind>>,
    remote_tracks: Arc<Mutex<HashMap<TrackId, Arc<RemoteTrack>>>>,
    event_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl MediaSession {
    /// 连接 LiveKit。**必须 tokio runtime 上下文。**
    pub async fn connect(cfg: ConnectConfig) -> Result<Arc<Self>, MediaError> {
        use livekit::e2ee::key_provider::{
            KeyDerivationAlgorithm, KeyProvider, KeyProviderOptions,
        };
        use livekit::e2ee::{E2eeOptions, EncryptionType};

        tracing::info!(
            "[media-session] connecting to {} room={}",
            cfg.ws_url,
            cfg.room_name
        );

        let mut opts = livekit::RoomOptions::default();
        if let Some(key) = cfg.e2ee_key {
            let kp_opts = KeyProviderOptions {
                key_derivation_algorithm: KeyDerivationAlgorithm::PBKDF2,
                ratchet_salt: b"LKFrameEncryptionKey".to_vec(),
                ..Default::default()
            };
            // 与 Web 端保持一致：PBKDF2 的 password 是 32 字节 key 的
            // 小写 hex 文本（ASCII），而不是原始二进制。JS 侧
            // setKey(string) 会对 UTF-8 文本做 PBKDF2(100000, SHA-256)。
            let password = hex_key(key.as_slice());
            let kp = KeyProvider::with_shared_key(kp_opts, password.into_bytes());
            opts.encryption = Some(E2eeOptions {
                encryption_type: EncryptionType::Gcm,
                key_provider: kp,
            });
            tracing::info!("[media-session] E2EE enabled");
        } else {
            // 不再允许静默明文媒体：调用方必须先派生 MLS exporter key。
            return Err(MediaError::Sink(
                "E2EE key missing; refusing to connect without media encryption".into(),
            ));
        }

        let (room, room_events) = livekit::Room::connect(&cfg.ws_url, &cfg.token, opts)
            .await
            .map_err(|e| MediaError::Sink(format!("Room::connect: {:?}", e)))?;

        let local_uid = room.local_participant().identity().to_string();
        let room_name = room.name();
        tracing::info!(
            "[media-session] connected uid={} room={}",
            local_uid,
            room_name
        );

        let (events_tx, _) = broadcast::channel::<MediaEvent>(EVENT_CAPACITY);
        let remote_tracks: Arc<Mutex<HashMap<TrackId, Arc<RemoteTrack>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let published: Arc<Mutex<HashMap<TrackKind, PublishedEntry>>> =
            Arc::new(Mutex::new(HashMap::new()));

        // 事件转发 task
        let event_task = {
            let events_tx = events_tx.clone();
            let remote_tracks = remote_tracks.clone();
            let room_name = room_name.clone();
            tokio::spawn(forward_room_events(
                room_name,
                room_events,
                events_tx,
                remote_tracks,
            ))
        };

        let _ = events_tx.send(MediaEvent::Connected {
            room_name: room_name.clone(),
        });

        Ok(Arc::new(Self {
            room: Arc::new(room),
            local_uid,
            room_name,
            events_tx,
            published,
            publishing: Mutex::new(HashSet::new()),
            remote_tracks,
            event_task: Mutex::new(Some(event_task)),
        }))
    }

    pub fn local_uid(&self) -> &str {
        &self.local_uid
    }

    pub fn room_name(&self) -> &str {
        &self.room_name
    }

    pub fn events(&self) -> broadcast::Receiver<MediaEvent> {
        self.events_tx.subscribe()
    }

    /// 原子地检查并占位一个 `TrackKind`，防止并发 publish 覆盖/泄漏。
    fn claim_publish(&self, kind: TrackKind) -> Result<PublishClaim<'_>, MediaError> {
        let published = self.published.lock().unwrap_or_else(|e| e.into_inner());
        let mut publishing = self.publishing.lock().unwrap_or_else(|e| e.into_inner());
        if published.contains_key(&kind) || publishing.contains(&kind) {
            return Err(MediaError::InvalidState(format!(
                "{:?} already published",
                kind
            )));
        }
        publishing.insert(kind);
        Ok(PublishClaim {
            set: &self.publishing,
            kind,
        })
    }

    /// 发布一个源。同 kind 已发布时返回错误。
    pub async fn publish(self: &Arc<Self>, source: PublishSource) -> Result<(), MediaError> {
        match source {
            PublishSource::Screen { config } => self.publish_screen(config).await,
            PublishSource::Mic {
                sample_rate,
                channels,
                denoise,
            } => {
                self.publish_audio(
                    sample_rate,
                    channels,
                    AudioSourceKind::Microphone,
                    TrackKind::VoiceAudio,
                    denoise,
                )
                .await
            }
            PublishSource::System {
                sample_rate,
                channels,
            } => {
                self.publish_audio(
                    sample_rate,
                    channels,
                    AudioSourceKind::SystemLoopback,
                    TrackKind::ScreenAudio,
                    false,
                )
                .await
            }
        }
    }

    async fn publish_screen(self: &Arc<Self>, config: VideoConfig) -> Result<(), MediaError> {
        let kind = TrackKind::ScreenVideo;
        // 检查 + 占位在同一个锁内完成；并发双击只有第一个能进入发布流程。
        let _claim = self.claim_publish(kind)?;

        // C-5: 未显式指定时使用主显示器实际分辨率，而不是硬编码 1080p。
        let (dw, dh) = kaede_capture::primary_display_size();
        let width = config.width.unwrap_or(dw);
        let height = config.height.unwrap_or(dh);
        let resolution = VideoResolution { width, height };

        let target_bitrate_bps = (config.bitrate_kbps as u64) * 1000;
        let target_fps = config.fps as f64;
        let source =
            EncodedVideoStream::start(config).map_err(|e| MediaError::Capture(e.to_string()))?;
        let actual_codec = source.actual_codec();
        let lk_codec = match actual_codec {
            kaede_capture::VideoCodec::H264 => VideoCodec::H264,
            kaede_capture::VideoCodec::HEVC => VideoCodec::H265,
            kaede_capture::VideoCodec::AV1 => VideoCodec::AV1,
        };
        let frame_codec = match actual_codec {
            kaede_capture::VideoCodec::H264 => EncodedVideoCodec::H264,
            kaede_capture::VideoCodec::HEVC => EncodedVideoCodec::H265,
            kaede_capture::VideoCodec::AV1 => EncodedVideoCodec::AV1,
        };
        tracing::info!(
            "publish_screen: actual_codec={:?}, target={}kbps@{}fps",
            actual_codec,
            target_bitrate_bps / 1000,
            target_fps as u32
        );

        let lk_source = NativeVideoSource::new_encoded(resolution.clone());
        let track = LocalVideoTrack::create_video_track(
            "screen",
            RtcVideoSource::Native(lk_source.clone()),
        );

        let mut fm = livekit::options::FrameMetadataFeatures::default();
        fm.user_timestamp = true;
        fm.frame_id = true;
        fm.user_data = true;

        let opts = TrackPublishOptions {
            video_codec: lk_codec,
            video_encoding: Some(livekit::options::VideoEncoding {
                max_bitrate: target_bitrate_bps,
                max_framerate: target_fps,
            }),
            video_encoder: VideoEncoderBackend::PreEncoded,
            simulcast: false,
            source: TrackSource::Screenshare,
            frame_metadata_features: fm,
            ..Default::default()
        };

        let publication = self
            .room
            .local_participant()
            .publish_track(LocalTrack::Video(track), opts)
            .await
            .map_err(|e| MediaError::Sink(format!("publish_track(screen): {:?}", e)))?;
        // 必须保存真实 publication sid，否则 unpublish 无法撤销 track。
        let sid = publication.sid();
        let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
        let events = self.events_tx.clone();
        let events_for_outer = events.clone();
        let sid_for_ev = sid.to_string();
        let res_for_ev = resolution.clone();
        let published_map = self.published.clone();
        let room_handle = self.room.clone();
        let sid_for_cleanup = sid.clone();
        let events_for_cleanup = events.clone();

        let handle = tokio::spawn(async move {
            let (frame_tx, frame_rx) = async_channel::bounded::<kaede_capture::EncodedFrame>(8);
            let stop_flag = Arc::new(AtomicBool::new(false));
            let stop_flag_thread = stop_flag.clone();
            let mut last_report = std::time::Instant::now();
            let mut frames_in_window: u32 = 0;
            let mut first_frame_seen = false;
            let mut bytes_in_window: u64 = 0;
            let events_for_stats = events.clone();

            let collector = std::thread::spawn(move || {
                while !stop_flag_thread.load(Ordering::Relaxed) {
                    match source.next_frame_timeout(Duration::from_millis(200)) {
                        Ok(Some(f)) => {
                            let _ = frame_tx.try_send(f);
                        }
                        Ok(None) => break,
                        Err(_) => continue,
                    }
                }
            });

            let first_frame_deadline = tokio::time::sleep(Duration::from_secs(3));
            tokio::pin!(first_frame_deadline);
            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    _ = &mut first_frame_deadline, if !first_frame_seen => {
                        tracing::error!("screen share: no frames within 3s; hardware may not support this profile");
                        let _ = events.send(MediaEvent::Error {
                            message: format!(
                                "3 秒未收到帧，当前硬件可能不支持该分辨率/帧率（{}x{}@{}fps）",
                                res_for_ev.width, res_for_ev.height, target_fps as u32
                            ),
                        });
                        break;
                    }
                    recv = frame_rx.recv() => {
                        match recv {
                            Ok(f) => {
                                first_frame_seen = true;
                                let lk_frame = EncodedVideoFrame {
                                    codec: frame_codec,
                                    payload: &f.data,
                                    timestamp_us: f.pts_ms.saturating_mul(1000),
                                    frame_type: if f.is_keyframe {
                                        EncodedFrameType::Key
                                    } else {
                                        EncodedFrameType::Delta
                                    },
                                    resolution: res_for_ev.clone(),
                                    frame_metadata: None,
                                };
                                let _ = lk_source.capture_encoded_frame(&lk_frame);
                                bytes_in_window += f.data.len() as u64;
                                    frames_in_window += 1;
                                    if last_report.elapsed() >= Duration::from_secs(1) {
                                        let elapsed = last_report.elapsed().as_secs_f64();
                                        let actual = frames_in_window as f64 / elapsed;
                                        let actual_kbps = (bytes_in_window as f64 * 8.0 / 1000.0 / elapsed) as u32;
                                        tracing::info!(
                                            "screen share actual fps: {:.1} (target {:.0}), bitrate: target={}kbps actual={}kbps, window={} frames / {} bytes",
                                            actual, target_fps,
                                            target_bitrate_bps / 1000,
                                            actual_kbps,
                                            frames_in_window, bytes_in_window
                                        );
                                        let _ = events_for_stats.send(MediaEvent::ScreenStats(ScreenStats {
                                            width: res_for_ev.width,
                                            height: res_for_ev.height,
                                            fps_actual: actual,
                                            fps_target: target_fps,
                                            bitrate_kbps: (target_bitrate_bps / 1000) as u32,
                                            bitrate_actual_kbps: actual_kbps,
                                        }));
                                        last_report = std::time::Instant::now();
                                        frames_in_window = 0;
                                        bytes_in_window = 0;
                                    }
                            }
                            Err(_) => break,
                        }
                    }
                }
            }
            stop_flag.store(true, Ordering::Relaxed);
            let _ = tokio::task::spawn_blocking(move || {
                let _ = collector.join();
            })
            .await;

            // 任务无论因 stop 还是异常（3s 无帧）退出，都要撤销 track 并清理表项。
            published_map
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&kind);
            let _ = room_handle
                .local_participant()
                .unpublish_track(&sid_for_cleanup)
                .await;
            let _ = events_for_cleanup.send(MediaEvent::LocalUnpublished { kind });
        });

        self.published
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                kind,
                PublishedEntry {
                    sid: sid.clone(),
                    stop_tx: Some(stop_tx),
                    handle,
                },
            );
        let _ = events_for_outer.send(MediaEvent::LocalPublished {
            kind,
            sid: sid_for_ev,
        });
        Ok(())
    }

    async fn publish_audio(
        self: &Arc<Self>,
        sample_rate: u32,
        channels: u16,
        kind_source: AudioSourceKind,
        track_kind: TrackKind,
        denoise: bool,
    ) -> Result<(), MediaError> {
        let _claim = self.claim_publish(track_kind)?;

        let cfg = AudioCaptureConfig {
            sample_rate,
            channels,
            kind: kind_source,
            pid: None,
            frame_duration_ms: 20,
        };
        let mut source: Box<dyn kaede_media::AudioSource> = {
            let s = GstAudioSource::start(cfg)?;
            if denoise {
                Box::new(kaede_media::DenoisedSource::new(s))
            } else {
                Box::new(s)
            }
        };

        let lk_source = NativeAudioSource::new(
            AudioSourceOptions::default(),
            sample_rate,
            channels as u32,
            100,
        );
        let track =
            LocalAudioTrack::create_audio_track("audio", RtcAudioSource::Native(lk_source.clone()));

        let opts = TrackPublishOptions {
            source: match kind_source {
                AudioSourceKind::Microphone => TrackSource::Microphone,
                AudioSourceKind::SystemLoopback => TrackSource::ScreenshareAudio,
            },
            ..Default::default()
        };

        let publication = self
            .room
            .local_participant()
            .publish_track(LocalTrack::Audio(track), opts)
            .await
            .map_err(|e| MediaError::Sink(format!("publish_track(audio): {:?}", e)))?;

        let sid = publication.sid();
        let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
        let events = self.events_tx.clone();
        let sid_for_ev = sid.to_string();
        let bytes_per_frame = 2 * channels as u32;

        let published_map = self.published.clone();
        let room_handle = self.room.clone();
        let sid_for_cleanup = sid.clone();
        let events_for_cleanup = events.clone();

        let events_for_collector = events.clone();
        let handle = tokio::spawn(async move {
            let (frame_tx, frame_rx) = async_channel::bounded::<AudioFrame>(8);
            let stop_flag = Arc::new(AtomicBool::new(false));
            let stop_flag_thread = stop_flag.clone();

            let collector = std::thread::spawn(move || {
                let mut consecutive_timeouts: u32 = 0;
                while !stop_flag_thread.load(Ordering::Relaxed) {
                    match source.next_audio_timeout(Duration::from_millis(200)) {
                        Ok(Some(f)) => {
                            consecutive_timeouts = 0;
                            let _ = frame_tx.try_send(f);
                        }
                        Ok(None) => break,
                        Err(MediaError::Timeout) => {
                            // 50 × 200ms = 10s 无帧：设备切换 / 驱动异常 / 独占模式冲突。
                            // 主动 break + emit error，让上层有机会提示用户。
                            consecutive_timeouts += 1;
                            if consecutive_timeouts >= 50 {
                                let _ = events_for_collector.send(MediaEvent::Error {
                                    message: "audio source produced no frames for 10s (device switched or driver failure); stopping track".into(),
                                });
                                break;
                            }
                        }
                        Err(_) => continue,
                    }
                }
            });

            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    recv = frame_rx.recv() => {
                        match recv {
                            Ok(f) => {
                                let mut samples: Vec<i16> =
                                    Vec::with_capacity(f.data.len() / 2);
                                let (pairs, _rem) = f.data.as_chunks::<2>();
                                for &[lo, hi] in pairs {
                                    samples.push(i16::from_le_bytes([lo, hi]));
                                }
                                let samples_per_channel = (f.data.len() as u32)
                                    .checked_div(bytes_per_frame)
                                    .unwrap_or(0);
                                let lk_frame = LkAudioFrame {
                                    data: samples.into(),
                                    sample_rate,
                                    num_channels: channels as u32,
                                    samples_per_channel,
                                };
                                if let Err(e) = lk_source.capture_frame(&lk_frame).await {
                                    tracing::warn!(
                                        "[media-session] capture_frame: {:?}",
                                        e
                                    );
                                }
                            }
                            Err(_) => break,
                        }
                    }
                }
            }
            stop_flag.store(true, Ordering::Relaxed);
            let _ = tokio::task::spawn_blocking(move || {
                let _ = collector.join();
            })
            .await;

            published_map
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&track_kind);
            let _ = room_handle
                .local_participant()
                .unpublish_track(&sid_for_cleanup)
                .await;
            let _ = events_for_cleanup.send(MediaEvent::LocalUnpublished { kind: track_kind });
        });

        self.published
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                track_kind,
                PublishedEntry {
                    sid: sid.clone(),
                    stop_tx: Some(stop_tx),
                    handle,
                },
            );
        let _ = events.send(MediaEvent::LocalPublished {
            kind: track_kind,
            sid: sid_for_ev,
        });
        Ok(())
    }

    /// 取消发布某 track。
    pub async fn unpublish(&self, kind: TrackKind) -> Result<(), MediaError> {
        let entry = self
            .published
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&kind);
        let Some(mut e) = entry else {
            return Err(MediaError::InvalidState(format!(
                "{:?} not published",
                kind
            )));
        };
        if let Some(tx) = e.stop_tx.take() {
            let _ = tx.send(());
        }
        // 由发布任务在退出时统一 unpublish_track + 发送 LocalUnpublished，
        // 这里只负责停止喂帧并等待其完成。
        let _ = e.handle.await;
        Ok(())
    }

    /// 把远端视频 track 喂给 sink。返回的句柄 drop 时自动停。
    pub fn attach_video(
        &self,
        track: TrackId,
        sink: Arc<LiveKitVideoSink>,
    ) -> Result<ConsumerHandle, MediaError> {
        let remote = {
            let map = self.remote_tracks.lock().unwrap_or_else(|e| e.into_inner());
            map.get(&track).cloned()
        };
        let Some(remote) = remote else {
            return Err(MediaError::Sink(format!("no remote track {:?}", track)));
        };

        let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
        let handle = tokio::spawn(async move {
            let RemoteTrack::Video(video) = &*remote else {
                tracing::warn!("[media-session] attach_video: not a video track");
                return;
            };
            let mut stream = NativeVideoStream::new(video.rtc_track());
            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    frame = stream.next() => {
                        match frame {
                            Some(frame) => {
                                let buf = frame.buffer.as_ref();
                                let w = buf.width();
                                let h = buf.height();
                                if w == 0 || h == 0 { continue; }
                                let i420 = buf.to_i420();
                                let (y, u, v) = i420.data();
                                let mut packed =
                                    Vec::with_capacity(y.len() + u.len() + v.len());
                                packed.extend_from_slice(y);
                                packed.extend_from_slice(u);
                                packed.extend_from_slice(v);
                                let pts_ms = frame.timestamp_us / 1000;
                                let _ = sink.push_i420(&packed, w, h, pts_ms);
                            }
                            None => break,
                        }
                    }
                }
            }
        });

        Ok(ConsumerHandle {
            inner: Some(ConsumerEntry {
                stop_tx: Some(stop_tx),
                handle,
            }),
        })
    }

    /// 把远端音频 track 喂给 player。
    pub fn attach_audio(
        &self,
        track: TrackId,
        player: Arc<AudioPlayer>,
    ) -> Result<ConsumerHandle, MediaError> {
        let remote = {
            let map = self.remote_tracks.lock().unwrap_or_else(|e| e.into_inner());
            map.get(&track).cloned()
        };
        let Some(remote) = remote else {
            return Err(MediaError::Sink(format!("no remote track {:?}", track)));
        };

        let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
        let handle = tokio::spawn(async move {
            let RemoteTrack::Audio(audio) = &*remote else {
                tracing::warn!("[media-session] attach_audio: not an audio track");
                return;
            };
            let mut stream = NativeAudioStream::new(audio.rtc_track(), 48000, 2);
            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    frame = stream.next() => {
                        match frame {
                            Some(f) => {
                                let mut bytes = Vec::with_capacity(f.data.len() * 2);
                                for s in f.data.iter() {
                                    bytes.extend_from_slice(&s.to_le_bytes());
                                }
                                let _ = player.push(&bytes, 0);
                            }
                            None => break,
                        }
                    }
                }
            }
        });

        Ok(ConsumerHandle {
            inner: Some(ConsumerEntry {
                stop_tx: Some(stop_tx),
                handle,
            }),
        })
    }

    /// 已订阅的远端 track 列表（快照）。
    pub fn remote_tracks(&self) -> Vec<(TrackId, RemoteKind)> {
        self.remote_tracks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(id, t)| {
                let kind = match &**t {
                    RemoteTrack::Video(_) => RemoteKind::Video,
                    RemoteTrack::Audio(_) => RemoteKind::Audio,
                };
                (id.clone(), kind)
            })
            .collect()
    }

    /// 断开连接。幂等。
    pub async fn disconnect(&self) {
        // 停所有 published
        let published: Vec<_> = self
            .published
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain()
            .collect();
        for (_, mut e) in published {
            if let Some(tx) = e.stop_tx.take() {
                let _ = tx.send(());
            }
            let _ = e.handle.await;
        }

        // 停 event task
        if let Some(h) = self
            .event_task
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            h.abort();
        }

        let _ = self.events_tx.send(MediaEvent::Disconnected {
            reason: "user requested".into(),
        });
        // room 随 Arc 最后一个引用 drop 而断开
    }
}

impl Drop for MediaSession {
    fn drop(&mut self) {
        if let Some(e) = self
            .event_task
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            e.abort();
        }
        // published 里的 task 会在 Arc drop 后由 oneshot 失效自然终止
    }
}

/// 后台转发 LiveKit RoomEvent → MediaEvent。
async fn forward_room_events(
    _room_name: String,
    mut events: mpsc::UnboundedReceiver<RoomEvent>,
    events_tx: broadcast::Sender<MediaEvent>,
    remote_tracks: Arc<Mutex<HashMap<TrackId, Arc<RemoteTrack>>>>,
) {
    while let Some(ev) = events.recv().await {
        match ev {
            RoomEvent::TrackSubscribed {
                track,
                participant,
                publication,
            } => {
                let uid = participant.identity().to_string();
                let sid = publication.sid().to_string();
                let kind = match &track {
                    RemoteTrack::Video(_) => RemoteKind::Video,
                    RemoteTrack::Audio(_) => RemoteKind::Audio,
                };
                let id = TrackId::new(uid, sid);
                remote_tracks
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(id.clone(), Arc::new(track));
                let _ = events_tx.send(MediaEvent::TrackAvailable { track: id, kind });
            }
            RoomEvent::TrackUnsubscribed {
                track,
                participant,
                publication,
            } => {
                let uid = participant.identity().to_string();
                let sid = publication.sid().to_string();
                let kind = match &track {
                    RemoteTrack::Video(_) => RemoteKind::Video,
                    RemoteTrack::Audio(_) => RemoteKind::Audio,
                };
                let id = TrackId::new(uid, sid);
                remote_tracks
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&id);
                let _ = events_tx.send(MediaEvent::TrackGone { track: id, kind });
            }
            RoomEvent::ParticipantConnected(p) => {
                let _ = events_tx.send(MediaEvent::PeerJoined {
                    uid: p.identity().to_string(),
                });
            }
            RoomEvent::ParticipantDisconnected(p) => {
                let uid = p.identity().to_string();
                remote_tracks
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .retain(|id, _| id.uid != uid);
                let _ = events_tx.send(MediaEvent::PeerLeft { uid });
            }
            RoomEvent::Disconnected { .. } => {
                let _ = events_tx.send(MediaEvent::Disconnected {
                    reason: "server".into(),
                });
                break;
            }
            _ => {}
        }
    }
}
