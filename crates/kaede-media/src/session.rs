//! 媒体会话：采集 → SFrame 加密 → 下游发送。
//!
//! # 线程模型
//!
//! ```text
//! std::thread ──(EncodedFrame)──> encrypt ──(EncryptedFrame)──┐
//!                                                             ├─ async_channel(8)
//!                                                             │      try_send 满则丢帧
//!                                                             └─> tokio task → sink.send
//! ```

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicI64, AtomicU8, AtomicU64, Ordering},
};

use async_channel::TrySendError;
use kaede_crypto::sframe::{Counter, KeyId, SframeKey, encrypt};
use tokio::sync::broadcast;

use crate::error::MediaError;
use crate::event::{MediaEvent, StopReason};
use crate::sink::{EncryptedFrame, MediaSink};
use crate::source::EncodedSource;
use crate::stats::{MediaState, MediaStats};

/// outbound 帧的别名（方便 UI 层 import）。
pub type OutgoingFrame = EncryptedFrame;

/// 有界队列容量：8 帧 ≈ 130ms @60fps。
/// 超过则丢帧（实时场景延迟优先）。
const QUEUE_CAPACITY: usize = 8;

/// 事件广播通道容量。多个订阅者都能收到全部事件（fanout）。
/// 慢的订阅者会丢老事件（`RecvError::Lagged`），但内存有界。
/// UI 只关心最新状态，丢老事件是正确取舍。
const EVENTS_CHANNEL_CAPACITY: usize = 64;

struct StatsInner {
    frames_encrypted: AtomicU64,
    frames_sent: AtomicU64,
    frames_dropped: AtomicU64,
    bytes_encrypted: AtomicU64,
    bytes_sent: AtomicU64,
    /// i64::MIN 表示 None。
    last_keyframe_pts_ms: AtomicI64,
}

impl StatsInner {
    fn new() -> Self {
        Self {
            frames_encrypted: AtomicU64::new(0),
            frames_sent: AtomicU64::new(0),
            frames_dropped: AtomicU64::new(0),
            bytes_encrypted: AtomicU64::new(0),
            bytes_sent: AtomicU64::new(0),
            last_keyframe_pts_ms: AtomicI64::new(i64::MIN),
        }
    }

    fn snapshot(&self) -> MediaStats {
        let kf = self.last_keyframe_pts_ms.load(Ordering::Relaxed);
        MediaStats {
            state: MediaState::Idle, // 由 MediaSession::stats 覆盖
            frames_encrypted: self.frames_encrypted.load(Ordering::Relaxed),
            frames_sent: self.frames_sent.load(Ordering::Relaxed),
            frames_dropped: self.frames_dropped.load(Ordering::Relaxed),
            bytes_encrypted: self.bytes_encrypted.load(Ordering::Relaxed),
            bytes_sent: self.bytes_sent.load(Ordering::Relaxed),
            last_keyframe_pts_ms: if kf == i64::MIN { None } else { Some(kf) },
        }
    }
}

/// 媒体会话。
pub struct MediaSession {
    state: Arc<AtomicU8>,
    stop_flag: Arc<AtomicBool>,
    stats: Arc<StatsInner>,
    events_tx: broadcast::Sender<MediaEvent>,
    collector: Mutex<Option<std::thread::JoinHandle<()>>>,
    sink_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl MediaSession {
    /// 启动会话。
    ///
    /// - `source`：帧源（生产用 `kaede_capture::EncodedVideoStream`）
    /// - `sink`：下游（首批只有 `NullSink`）
    /// - `sframe_key`：从 MLS exporter 派生
    /// - `key_id`：一般等于 MLS epoch
    /// - `sender_id`：本发送者 32 位标识。同一个 key 被多个 sender 共用时，
    ///   counter 高 32 位 = sender_id，保证各自 nonce 空间不重叠。
    ///   用 `kaede_crypto::sframe::sender_id_from_uid(uid)` 派生。
    ///
    /// # ⚠️ 安全警告（P0-1，2026-10-19 审查发现）
    ///
    /// **SFrame counter 从 0 开始，不跨调用持久化。**
    ///
    /// 若用**相同的 `(sframe_key, key_id, sender_id)`** 启动两次会话，
    /// 两次会话产生的 nonce 序列**完全重叠** → AES-GCM nonce 复用
    /// → **明文可 XOR 恢复 + 认证可伪造**，是密码学灾难。
    ///
    /// 调用方 MUST 保证以下**至少一条**成立：
    ///
    /// 1. **两次调用之间 `key_id` 或 `sframe_key` 变化**
    ///    （推荐：用 `kaede_media::ephemeral_key_id()` 生成独立 key_id）
    /// 2. 或者调用方自己持久化 counter 并从上次位置继续
    ///    （当前 API 不支持，需未来加 `counter_seed: Option<u64>` 参数）
    ///
    /// **UI 层语音走 `kaede-media-session` 的 LiveKit 内建 E2EE，不触发此风险。**
    /// 触发场景：离线落盘、自建 SFU、直接调用本 crate 的 examples。
    pub async fn start<S, K>(
        mut source: S,
        sink: K,
        sframe_key: SframeKey,
        key_id: u32,
        sender_id: u32,
    ) -> Result<Arc<Self>, MediaError>
    where
        S: EncodedSource,
        K: MediaSink,
    {
        let state = Arc::new(AtomicU8::new(MediaState::Starting.as_u8()));
        let stop_flag = Arc::new(AtomicBool::new(false));
        let stats = Arc::new(StatsInner::new());
        let (events_tx, _) = broadcast::channel::<MediaEvent>(EVENTS_CHANNEL_CAPACITY);
        let (frame_tx, frame_rx) = async_channel::bounded::<EncryptedFrame>(QUEUE_CAPACITY);

        // ---- 采集线程 ----
        let collector = {
            let stop = stop_flag.clone();
            let stats = stats.clone();
            let events = events_tx.clone();
            let state = state.clone();
            std::thread::spawn(move || {
                // counter 高 32 位 = sender_id，低 32 位 = 帧序号。
                // 多个 sender 共享同一个 key 时，nonce 空间不重叠。
                let mut counter = kaede_crypto::sframe::make_counter_start(sender_id);
                while !stop.load(Ordering::Relaxed) {
                    match source.next_frame_timeout(std::time::Duration::from_millis(200)) {
                        Ok(Some(f)) => {
                            let enc = match encrypt(
                                &sframe_key,
                                KeyId(key_id),
                                Counter(counter),
                                &f.data,
                            ) {
                                Ok(e) => e,
                                Err(e) => {
                                    let _ = events.send(MediaEvent::Error {
                                        message: format!("sframe encrypt: {}", e),
                                    });
                                    state.store(MediaState::Failed.as_u8(), Ordering::Relaxed);
                                    break;
                                }
                            };
                            let next = counter.wrapping_add(1);
                            if kaede_crypto::sframe::sender_id_of(Counter(next))
                                != kaede_crypto::sframe::sender_id_of(Counter(counter))
                            {
                                let _ = events.send(MediaEvent::Error {
                                    message: format!(
                                        "sframe counter overflow: sender={:#010x}, stopping track to preserve nonce safety",
                                        kaede_crypto::sframe::sender_id_of(Counter(counter))
                                    ),
                                });
                                state.store(MediaState::Failed.as_u8(), Ordering::Relaxed);
                                break;
                            }
                            counter = next;
                            let enc_len = enc.len();
                            let is_kf = f.is_keyframe;
                            let pts = f.pts_ms;

                            stats.frames_encrypted.fetch_add(1, Ordering::Relaxed);
                            stats
                                .bytes_encrypted
                                .fetch_add(enc_len as u64, Ordering::Relaxed);
                            if is_kf {
                                stats.last_keyframe_pts_ms.store(pts, Ordering::Relaxed);
                            }

                            let _ = events.send(MediaEvent::FrameEncrypted {
                                pts_ms: pts,
                                bytes: enc_len,
                                is_keyframe: is_kf,
                            });

                            let ef = EncryptedFrame {
                                data: enc,
                                is_keyframe: is_kf,
                                pts_ms: pts,
                            };
                            match frame_tx.try_send(ef) {
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
                        Err(MediaError::Timeout) => {
                            // 200ms 没帧，回循环头再检查一次 stop_flag。
                            // 不报错——采集源正常静默时不产出帧。
                            continue;
                        }
                        Err(e) => {
                            let _ = events.send(MediaEvent::Error {
                                message: e.to_string(),
                            });
                            state.store(MediaState::Failed.as_u8(), Ordering::Relaxed);
                            break;
                        }
                    }
                }
                // 显式停止帧源，释放 GStreamer pipeline
                source.stop();
            })
        };

        // ---- 下游 task ----
        let sink_task = {
            let stats = stats.clone();
            let events = events_tx.clone();
            let state = state.clone();
            tokio::spawn(async move {
                while let Ok(frame) = frame_rx.recv().await {
                    let bytes = frame.data.len() as u64;
                    match sink.send(frame).await {
                        Ok(()) => {
                            stats.frames_sent.fetch_add(1, Ordering::Relaxed);
                            stats.bytes_sent.fetch_add(bytes, Ordering::Relaxed);
                        }
                        Err(e) => {
                            let _ = events.send(MediaEvent::Error {
                                message: e.to_string(),
                            });
                            state.store(MediaState::Failed.as_u8(), Ordering::Relaxed);
                            break;
                        }
                    }
                }
                let _ = sink.close().await;
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
            sink_task: Mutex::new(Some(sink_task)),
        }))
    }

    /// 当前状态。
    pub fn state(&self) -> MediaState {
        MediaState::from_u8(self.state.load(Ordering::Relaxed))
    }

    /// 统计快照。
    pub fn stats(&self) -> MediaStats {
        let mut s = self.stats.snapshot();
        s.state = self.state();
        s
    }

    /// 事件接收器。多次调用返回同一个底层 channel 的多个 clone。
    /// 订阅事件流。**支持多订阅者**（fanout）——每次调用返回独立 receiver，
    /// 每个 receiver 都能拿到全部事件。
    ///
    /// 慢订阅者会看到 `RecvError::Lagged(n)`，表示丢了 n 条事件（通常是
    /// 高频的 `FrameEncrypted` / `FrameDropped`）。慢订阅者应该忽略 Lagged 继续。
    pub fn events(&self) -> broadcast::Receiver<MediaEvent> {
        self.events_tx.subscribe()
    }

    /// 停止会话。幂等——重复调用安全。
    pub async fn stop(&self) {
        if self.state().is_terminal() {
            return;
        }
        self.stop_flag.store(true, Ordering::Relaxed);
        self.state
            .store(MediaState::Stopping.as_u8(), Ordering::Relaxed);

        // H21: detached std::thread join —— collector 是 std::thread::spawn 出来的
        // OS 线程；用 tokio::task::spawn_blocking 会占用 tokio blocking pool，
        // 且 timeout 不取消 blocking task → collector 卡死时永久占一个 blocking slot。
        // 改成 std::thread::spawn（OS 管理，不占 tokio pool），stop() 立即返回。
        let collector_task = self.collector.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(h) = collector_task {
            std::thread::spawn(move || {
                let _ = h.join();
            });
        }
        // H22: abort 而非 timeout —— timeout 返回后 sink_task 仍在后台跑，
        // 若它卡在 channel recv 上会永久占用一个 tokio worker。abort 立即取消。
        let sink_task = self.sink_task.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(h) = sink_task {
            h.abort();
            let _ = h.await;
        }

        self.state
            .store(MediaState::Stopped.as_u8(), Ordering::Relaxed);
        let _ = self.events_tx.send(MediaEvent::Stopped {
            reason: StopReason::UserRequest,
        });
    }
}

impl Drop for MediaSession {
    fn drop(&mut self) {
        // 只设 flag，不 join —— 避免在 drop 里阻塞。
        // 采集线程会在下一次循环边界看到 flag 后自行退出。
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

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sink::NullSink;
    use crate::source::EncodedFrame;
    use std::time::Duration;

    /// 有限帧源：生成 N 帧后返回 None。
    struct MockSource {
        frames: Vec<EncodedFrame>,
        idx: usize,
    }

    impl MockSource {
        fn new(n: usize) -> Self {
            let frames = (0..n)
                .map(|i| EncodedFrame {
                    data: vec![0xAB; 100],
                    is_keyframe: i == 0, // 第 0 帧是关键帧
                    pts_ms: (i as i64) * 16,
                })
                .collect();
            Self { frames, idx: 0 }
        }
    }

    impl EncodedSource for MockSource {
        fn next_frame_blocking(&mut self) -> Result<Option<EncodedFrame>, MediaError> {
            if self.idx >= self.frames.len() {
                // 模拟阻塞一下（让 sink task 有机会消费）
                std::thread::sleep(Duration::from_millis(5));
                return Ok(None);
            }
            let f = self.frames[self.idx].clone();
            self.idx += 1;
            Ok(Some(f))
        }
        fn stop(&mut self) {}
    }

    #[tokio::test]
    async fn start_encrypts_and_sends_all_frames() {
        let src = MockSource::new(3);
        let key = SframeKey::from_bytes([0x42; 32]);
        let session = MediaSession::start(src, NullSink, key, 1, 1).await.unwrap();

        assert_eq!(session.state(), MediaState::Active);

        // 等采集线程处理完 + sink 消费完
        tokio::time::sleep(Duration::from_millis(100)).await;

        let s = session.stats();
        assert!(s.frames_encrypted >= 3, "encrypted={}", s.frames_encrypted);
        assert!(s.frames_sent >= 3, "sent={}", s.frames_sent);
        assert_eq!(s.last_keyframe_pts_ms, Some(0));
        assert_eq!(s.frames_dropped, 0);
        // 每帧 SFrame 开销：12 header + 16 tag = 28
        assert!(s.bytes_encrypted >= 3 * (100 + 28));

        session.stop().await;
        assert_eq!(session.state(), MediaState::Stopped);
    }

    #[tokio::test]
    async fn stop_is_idempotent() {
        let src = MockSource::new(1);
        let key = SframeKey::from_bytes([0x11; 32]);
        let session = MediaSession::start(src, NullSink, key, 0, 1).await.unwrap();

        tokio::time::sleep(Duration::from_millis(30)).await;
        session.stop().await;
        session.stop().await; // 不应 panic
        assert_eq!(session.state(), MediaState::Stopped);
    }

    /// 「卡死的源」：`next_frame_timeout` 永远返回 Timeout。
    /// 模拟 GStreamer pipeline 崩溃 / 设备被占用 / 屏幕锁屏等场景。
    struct StuckSource {
        stopped: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    impl StuckSource {
        fn new() -> Self {
            Self {
                stopped: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            }
        }
    }

    impl EncodedSource for StuckSource {
        fn next_frame_blocking(&mut self) -> Result<Option<EncodedFrame>, MediaError> {
            // 永远不会走到这（主循环调 timeout 版本）。真被调到就当错误。
            Err(MediaError::Capture(
                "blocking called on stuck source".into(),
            ))
        }
        fn next_frame_timeout(
            &mut self,
            _timeout: Duration,
        ) -> Result<Option<EncodedFrame>, MediaError> {
            if self.stopped.load(std::sync::atomic::Ordering::Relaxed) {
                return Ok(None);
            }
            // 立刻返回 Timeout，模拟「200ms 窗口内一帧都没来」
            Err(MediaError::Timeout)
        }
        fn stop(&mut self) {
            self.stopped
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    #[tokio::test]
    async fn stop_returns_within_timeout_when_source_is_stuck() {
        let src = StuckSource::new();
        let key = SframeKey::from_bytes([0x77; 32]);
        let session = MediaSession::start(src, NullSink, key, 0, 1).await.unwrap();

        // 让主循环转几圈（每圈 Timeout → continue，检查 stop_flag）
        tokio::time::sleep(Duration::from_millis(50)).await;

        // stop 必须 <3 秒返回（内部上限 2s + 缓冲）
        let t0 = std::time::Instant::now();
        tokio::time::timeout(Duration::from_secs(3), session.stop())
            .await
            .expect("stop() must not hang");
        let elapsed = t0.elapsed();

        assert!(
            elapsed < Duration::from_secs(3),
            "stop took {:?}, expected <3s",
            elapsed
        );
        assert_eq!(session.state(), MediaState::Stopped);
        eprintln!("[test] stuck source: stop() returned in {:?}", elapsed);
    }
}

// ============================================================================
// AudioSession
// ============================================================================

use crate::source::AudioSource;

/// 音频媒体会话。
///
/// 与 [`MediaSession`] 结构相同，只是采集源类型换成 [`AudioSource`]，
/// 输出帧用 [`EncryptedFrame`]（`is_keyframe` 恒为 `false`）。
pub struct AudioSession {
    state: Arc<AtomicU8>,
    stop_flag: Arc<AtomicBool>,
    stats: Arc<StatsInner>,
    events_tx: broadcast::Sender<MediaEvent>,
    collector: Mutex<Option<std::thread::JoinHandle<()>>>,
    sink_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl AudioSession {
    ///
    /// # ⚠️ 安全警告（P0-1，2026-10-19 审查发现）
    ///
    /// **SFrame counter 从 0 开始，不跨调用持久化。**
    ///
    /// 若用**相同的 `(sframe_key, key_id, sender_id)`** 启动两次会话，
    /// 两次会话产生的 nonce 序列**完全重叠** → AES-GCM nonce 复用
    /// → **明文可 XOR 恢复 + 认证可伪造**，是密码学灾难。
    ///
    /// 调用方 MUST 保证以下**至少一条**成立：
    ///
    /// 1. **两次调用之间 `key_id` 或 `sframe_key` 变化**
    ///    （推荐：用 `kaede_media::ephemeral_key_id()` 生成独立 key_id）
    /// 2. 或者调用方自己持久化 counter 并从上次位置继续
    ///    （当前 API 不支持，需未来加 `counter_seed: Option<u64>` 参数）
    ///
    /// **UI 层语音走 `kaede-media-session` 的 LiveKit 内建 E2EE，不触发此风险。**
    /// 触发场景：离线落盘、自建 SFU、直接调用本 crate 的 examples。
    pub async fn start<S, K>(
        mut source: S,
        sink: K,
        sframe_key: SframeKey,
        key_id: u32,
        sender_id: u32,
    ) -> Result<Arc<Self>, MediaError>
    where
        S: AudioSource,
        K: MediaSink,
    {
        let state = Arc::new(AtomicU8::new(MediaState::Starting.as_u8()));
        let stop_flag = Arc::new(AtomicBool::new(false));
        let stats = Arc::new(StatsInner::new());
        let (events_tx, _) = broadcast::channel::<MediaEvent>(EVENTS_CHANNEL_CAPACITY);
        let (frame_tx, frame_rx) = async_channel::bounded::<EncryptedFrame>(QUEUE_CAPACITY);

        let collector = {
            let stop = stop_flag.clone();
            let stats = stats.clone();
            let events = events_tx.clone();
            let state = state.clone();
            std::thread::spawn(move || {
                let mut counter = kaede_crypto::sframe::make_counter_start(sender_id);
                while !stop.load(Ordering::Relaxed) {
                    match source.next_audio_timeout(std::time::Duration::from_millis(200)) {
                        Ok(Some(f)) => {
                            let enc = match encrypt(
                                &sframe_key,
                                KeyId(key_id),
                                Counter(counter),
                                &f.data,
                            ) {
                                Ok(e) => e,
                                Err(e) => {
                                    let _ = events.send(MediaEvent::Error {
                                        message: format!("sframe encrypt: {}", e),
                                    });
                                    state.store(MediaState::Failed.as_u8(), Ordering::Relaxed);
                                    break;
                                }
                            };
                            let next = counter.wrapping_add(1);
                            if kaede_crypto::sframe::sender_id_of(Counter(next))
                                != kaede_crypto::sframe::sender_id_of(Counter(counter))
                            {
                                let _ = events.send(MediaEvent::Error {
                                    message: format!(
                                        "sframe counter overflow: sender={:#010x}, stopping track to preserve nonce safety",
                                        kaede_crypto::sframe::sender_id_of(Counter(counter))
                                    ),
                                });
                                state.store(MediaState::Failed.as_u8(), Ordering::Relaxed);
                                break;
                            }
                            counter = next;
                            let enc_len = enc.len();
                            let pts = f.pts_ms;

                            stats.frames_encrypted.fetch_add(1, Ordering::Relaxed);
                            stats
                                .bytes_encrypted
                                .fetch_add(enc_len as u64, Ordering::Relaxed);

                            let _ = events.send(MediaEvent::FrameEncrypted {
                                pts_ms: pts,
                                bytes: enc_len,
                                is_keyframe: false,
                            });

                            let ef = EncryptedFrame {
                                data: enc,
                                is_keyframe: false,
                                pts_ms: pts,
                            };
                            match frame_tx.try_send(ef) {
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
                        Err(MediaError::Timeout) => {
                            // 200ms 没帧，回循环头再检查一次 stop_flag。
                            // 不报错——采集源正常静默时不产出帧。
                            continue;
                        }
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

        let sink_task = {
            let stats = stats.clone();
            let events = events_tx.clone();
            let state = state.clone();
            tokio::spawn(async move {
                while let Ok(frame) = frame_rx.recv().await {
                    let bytes = frame.data.len() as u64;
                    match sink.send(frame).await {
                        Ok(()) => {
                            stats.frames_sent.fetch_add(1, Ordering::Relaxed);
                            stats.bytes_sent.fetch_add(bytes, Ordering::Relaxed);
                        }
                        Err(e) => {
                            let _ = events.send(MediaEvent::Error {
                                message: e.to_string(),
                            });
                            state.store(MediaState::Failed.as_u8(), Ordering::Relaxed);
                            break;
                        }
                    }
                }
                let _ = sink.close().await;
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
            sink_task: Mutex::new(Some(sink_task)),
        }))
    }

    pub fn state(&self) -> MediaState {
        MediaState::from_u8(self.state.load(Ordering::Relaxed))
    }

    pub fn stats(&self) -> MediaStats {
        let mut s = self.stats.snapshot();
        s.state = self.state();
        s
    }

    /// 订阅事件流。**支持多订阅者**（fanout）——每次调用返回独立 receiver，
    /// 每个 receiver 都能拿到全部事件。
    ///
    /// 慢订阅者会看到 `RecvError::Lagged(n)`，表示丢了 n 条事件（通常是
    /// 高频的 `FrameEncrypted` / `FrameDropped`）。慢订阅者应该忽略 Lagged 继续。
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
        // H21: detached std::thread join（同 MediaSession::stop）
        let collector_task = self.collector.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(h) = collector_task {
            std::thread::spawn(move || {
                let _ = h.join();
            });
        }
        // H22: abort 而非 timeout
        let sink_task = self.sink_task.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(h) = sink_task {
            h.abort();
            let _ = h.await;
        }
        self.state
            .store(MediaState::Stopped.as_u8(), Ordering::Relaxed);
        let _ = self.events_tx.send(MediaEvent::Stopped {
            reason: StopReason::UserRequest,
        });
    }
}

impl Drop for AudioSession {
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

#[cfg(test)]
mod audio_tests {
    use super::*;
    use crate::sink::NullSink;
    use crate::source::{AudioFrame, AudioSource};
    use std::time::Duration;

    struct MockAudioSource {
        frames: Vec<AudioFrame>,
        idx: usize,
    }

    impl MockAudioSource {
        fn new(n: usize) -> Self {
            let frames = (0..n)
                .map(|i| AudioFrame {
                    data: vec![0u8; 960 * 4], // 20ms @ 48kHz stereo S16LE
                    pts_ms: (i as i64) * 20,
                    sample_rate: 48000,
                    channels: 2,
                })
                .collect();
            Self { frames, idx: 0 }
        }
    }

    impl AudioSource for MockAudioSource {
        fn next_audio_blocking(&mut self) -> Result<Option<AudioFrame>, MediaError> {
            if self.idx >= self.frames.len() {
                std::thread::sleep(Duration::from_millis(5));
                return Ok(None);
            }
            let f = self.frames[self.idx].clone();
            self.idx += 1;
            Ok(Some(f))
        }
        fn stop(&mut self) {}
    }

    #[tokio::test]
    async fn audio_session_encrypts_and_sends() {
        let src = MockAudioSource::new(5);
        let key = SframeKey::from_bytes([0x42; 32]);
        let session = AudioSession::start(src, NullSink, key, 1, 1).await.unwrap();

        assert_eq!(session.state(), MediaState::Active);
        tokio::time::sleep(Duration::from_millis(80)).await;

        let s = session.stats();
        assert!(s.frames_encrypted >= 5, "encrypted={}", s.frames_encrypted);
        assert!(s.frames_sent >= 5, "sent={}", s.frames_sent);
        assert_eq!(s.frames_dropped, 0);
        // 每帧：960*4 = 3840 bytes plaintext + 12 header + 16 tag = 3868
        assert!(s.bytes_encrypted >= 5 * 3868);

        session.stop().await;
        assert_eq!(session.state(), MediaState::Stopped);
    }

    #[tokio::test]
    async fn audio_session_stop_idempotent() {
        let src = MockAudioSource::new(2);
        let key = SframeKey::from_bytes([0x11; 32]);
        let session = AudioSession::start(src, NullSink, key, 0, 1).await.unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        session.stop().await;
        session.stop().await;
        assert_eq!(session.state(), MediaState::Stopped);
    }

    /// 音频的「卡死源」回归测试（同 `StuckSource`）。
    struct StuckAudioSource {
        stopped: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    impl StuckAudioSource {
        fn new() -> Self {
            Self {
                stopped: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            }
        }
    }

    impl AudioSource for StuckAudioSource {
        fn next_audio_blocking(&mut self) -> Result<Option<AudioFrame>, MediaError> {
            Err(MediaError::Capture(
                "blocking called on stuck source".into(),
            ))
        }
        fn next_audio_timeout(
            &mut self,
            _timeout: Duration,
        ) -> Result<Option<AudioFrame>, MediaError> {
            if self.stopped.load(std::sync::atomic::Ordering::Relaxed) {
                return Ok(None);
            }
            Err(MediaError::Timeout)
        }
        fn stop(&mut self) {
            self.stopped
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    #[tokio::test]
    async fn audio_stop_returns_within_timeout_when_source_is_stuck() {
        let src = StuckAudioSource::new();
        let key = SframeKey::from_bytes([0x88; 32]);
        let session = AudioSession::start(src, NullSink, key, 0, 1).await.unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;

        let t0 = std::time::Instant::now();
        tokio::time::timeout(Duration::from_secs(3), session.stop())
            .await
            .expect("AudioSession::stop() must not hang");
        let elapsed = t0.elapsed();

        assert!(elapsed < Duration::from_secs(3));
        assert_eq!(session.state(), MediaState::Stopped);
        eprintln!(
            "[test] stuck audio source: stop() returned in {:?}",
            elapsed
        );
    }
}
