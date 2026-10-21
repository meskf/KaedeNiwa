//! 共享状态：数据库池 + 房间成员表 + per-uid 限流。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kaede_protocol::{ServerMessage, UserId};
use sqlx::SqlitePool;
use tokio::sync::mpsc;

use crate::config::Config;

/// 一个在线连接的写句柄。
#[derive(Clone)]
pub struct PeerHandle {
    pub uid: UserId,
    pub tx: mpsc::Sender<ServerMessage>,
    pub session_id: u64,
}

#[derive(Default)]
pub struct Room {
    pub peers: HashMap<UserId, PeerHandle>,
    /// uid → status（"online" / "idle" / "dnd"）。
    pub presence: HashMap<UserId, String>,
}

/// 令牌桶。rate = 每秒补充的令牌数；capacity = 桶容量。
///
/// 桶与 uid 绑定（per-uid 限流），跨连接持久。断线重连不重置速率。
pub struct RateBucket {
    tokens: f64,
    last: Instant,
}

impl RateBucket {
    fn new(capacity: f64) -> Self {
        Self {
            tokens: capacity,
            last: Instant::now(),
        }
    }

    fn try_consume(&mut self, n: f64, rate: f64, capacity: f64) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last).as_secs_f64();
        self.tokens = (self.tokens + elapsed * rate).min(capacity);
        self.last = now;
        if self.tokens >= n {
            self.tokens -= n;
            true
        } else {
            false
        }
    }
}

/// 空闲桶的保留时长：超过这个时间未访问，下次访问时重置。
///
/// 这防止 HashMap 无限增长（恶意客户端可尝试注册无数 uid 填满 map）。
const RATE_BUCKET_TTL: Duration = Duration::from_secs(600);

/// `broadcast_except_important` 对**单个** peer 的发送超时。
///
/// P1-I 修复的核心：单个 slow consumer 的 1024 buffer 满了会阻塞
/// 整个 MlsCommit 广播。超时即放弃该 peer（后续会因 idle 超时被断开）。
///
/// `pub(crate)` 供测试断言使用（`assert!(elapsed >= 0.9 * BROADCAST_SEND_TIMEOUT)`）。
pub(crate) const BROADCAST_SEND_TIMEOUT: Duration = Duration::from_secs(2);

/// 参与者的“最近一次拿 token”的保留时长。
///
/// LiveKit token 有效期 6 小时；超过 8 小时未续期的 participant 视为已离线
/// （关闭页面/崩溃/断网来不及调用 `/leave`），清理任务据此轮换 seed，
/// 避免 seed 永久复用 + `room_media_seeds` 无界增长。
pub const MEDIA_PARTICIPANT_TTL: Duration = Duration::from_secs(8 * 3600);

/// 房间级媒体 seed 状态。
///
/// `participants` 记录每个参与者最近一次获取 token 的时间；最后一个
/// participant 过期/离开后移除条目，下一次进入会生成全新 seed，
/// 避免同一个 key 在 LiveKit SDK ratchet 重启后被复用。
pub struct RoomMedia {
    pub seed: [u8; 32],
    pub participants: HashMap<String, Instant>,
    pub last_used: Instant,
}

/// 清理媒体房间：先剔除 TTL 过期的 participant，再删除空且久未活动的 seed。
pub(crate) fn prune_media_rooms(
    map: &mut HashMap<String, RoomMedia>,
    participant_ttl: Duration,
    empty_ttl: Duration,
) {
    for r in map.values_mut() {
        r.participants
            .retain(|_, last_seen| last_seen.elapsed() < participant_ttl);
    }
    map.retain(|_, r| !r.participants.is_empty() || r.last_used.elapsed() < empty_ttl);
}

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub cfg: Config,
    pub room: Arc<Mutex<Room>>,
    pub rate_buckets: Arc<Mutex<HashMap<UserId, RateBucket>>>,
    /// P0-1：连接 session id 生成器。
    pub next_session_id: Arc<std::sync::atomic::AtomicU64>,
    /// P1-10：WS 连接并发上限。
    pub ws_semaphore: Arc<tokio::sync::Semaphore>,
    /// P0-2：login 并发上限（防 Argon2 内存 DoS）。
    pub login_semaphore: Arc<tokio::sync::Semaphore>,
    /// P0-2：login 失败计数 uid -> (次数, 首次失败时间)。
    pub login_failures: Arc<Mutex<HashMap<String, (u32, Instant)>>>,
    /// P0-2：每个 LiveKit room 的媒体会话状态。
    ///
    /// - 同一媒体会话内所有参与者拿同一个 seed；
    /// - 最后一个参与者 leave 后条目删除，下次进入生成新 seed；
    /// - 进程重启也会自然换 seed。
    ///
    /// 客户端用 `MLS exporter(channel_id, "livekit-e2ee", seed)` 派生真正的
    /// e2ee key——服务端不知道 exporter 输出，仍然是 E2EE。
    pub room_media_seeds: Arc<Mutex<HashMap<String, RoomMedia>>>,
    /// KeyPackage 拉取的 per-target 限流（target -> (次数, 窗口起点)）。
    pub kp_fetch_budget: Arc<Mutex<HashMap<String, (u32, Instant)>>>,
}

impl AppState {
    pub fn new(pool: SqlitePool, cfg: Config) -> Self {
        let rate_buckets = Arc::new(Mutex::new(HashMap::<UserId, RateBucket>::new()));
        // P1-6：后台清理任务，防恶意注册 uid 撑爆 rate_buckets。
        let buckets_clone = rate_buckets.clone();
        let kp_budget = Arc::new(Mutex::new(HashMap::<String, (u32, Instant)>::new()));
        let kp_budget_clone = kp_budget.clone();
        let room_media_seeds = Arc::new(Mutex::new(HashMap::<String, RoomMedia>::new()));
        let room_media_clone = room_media_seeds.clone();
        let pool_clone = pool.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(300));
            tick.tick().await;
            loop {
                tick.tick().await;
                {
                    let mut m = buckets_clone.lock().unwrap();
                    m.retain(|_, b| b.last.elapsed() < RATE_BUCKET_TTL);
                }
                {
                    let mut m = kp_budget_clone.lock().unwrap();
                    m.retain(|_, (_, start)| start.elapsed() < Duration::from_secs(600));
                }
                {
                    // 先剔除超过 TTL 未续期的参与者（崩溃/关页不会调用 /leave），
                    // 再删除空房间 + 1 小时未活动的 seed。
                    let mut m = room_media_clone.lock().unwrap();
                    prune_media_rooms(&mut m, MEDIA_PARTICIPANT_TTL, Duration::from_secs(3600));
                }
                if let Err(e) = crate::db::cleanup_expired(&pool_clone).await {
                    tracing::warn!("cleanup_expired failed: {}", e);
                }
            }
        });
        Self {
            pool,
            cfg,
            room: Arc::new(Mutex::new(Room::default())),
            rate_buckets,
            next_session_id: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            ws_semaphore: Arc::new(tokio::sync::Semaphore::new(5000)),
            login_semaphore: Arc::new(tokio::sync::Semaphore::new(8)),
            login_failures: Arc::new(Mutex::new(HashMap::new())),
            room_media_seeds,
            kp_fetch_budget: kp_budget,
        }
    }

    /// 某个 target 的 KeyPackage 是否还可以被拉取。
    ///
    /// 限流目的：防止任意登录用户把别人的 KeyPackage 队列抽干
    /// （每个 KP 只能消费一次）。10 次/分钟对一个 target 足够宽松。
    pub fn check_kp_fetch(&self, target: &str) -> bool {
        const LIMIT: u32 = 10;
        const WINDOW: Duration = Duration::from_secs(60);
        let mut m = self.kp_fetch_budget.lock().unwrap();
        let entry = m.entry(target.to_string()).or_insert((0, Instant::now()));
        if entry.1.elapsed() > WINDOW {
            *entry = (0, Instant::now());
        }
        if entry.0 >= LIMIT {
            return false;
        }
        entry.0 += 1;
        true
    }

    /// 当前连接的 session id（若用户在线）。
    pub fn peer_session_id(&self, uid: &UserId) -> Option<u64> {
        self.room
            .lock()
            .unwrap()
            .peers
            .get(uid)
            .map(|p| p.session_id)
    }

    /// P0-1（单设备止血）：后登连接踢掉前登。返回被踢的旧 handle。
    pub fn add_peer(&self, handle: PeerHandle) -> Option<PeerHandle> {
        let mut room = self.room.lock().unwrap();
        room.presence
            .insert(handle.uid.clone(), "online".to_string());
        let old = room.peers.insert(handle.uid.clone(), handle);
        if let Some(prev) = &old {
            let _ = prev.tx.try_send(ServerMessage::Error {
                code: kaede_protocol::ErrorCode::Kicked,
                message: "another connection logged in with the same account".into(),
            });
        }
        old
    }

    /// 移除并返回 peer handle。调用方负责向该 handle 发最后一条消息
    /// （例如 Kicked 错误），之后再 drop 掉。
    pub fn take_peer(&self, uid: &UserId) -> Option<PeerHandle> {
        let mut room = self.room.lock().unwrap();
        room.presence.remove(uid);
        room.peers.remove(uid)
    }

    pub fn remove_peer(&self, uid: &UserId) {
        let mut room = self.room.lock().unwrap();
        room.peers.remove(uid);
        room.presence.remove(uid);
    }

    /// P0-X：仅当 stored session_id 匹配时才移除。
    /// 防止旧连接退出清理误删新连接（多设备 / 重连）。
    pub fn remove_peer_if_session(&self, uid: &UserId, session_id: u64) -> bool {
        let mut room = self.room.lock().unwrap();
        if room.peers.get(uid).map(|p| p.session_id) == Some(session_id) {
            room.peers.remove(uid);
            room.presence.remove(uid);
            true
        } else {
            false
        }
    }

    /// 设置 presence。
    pub fn set_presence(&self, uid: &UserId, status: String) {
        self.room
            .lock()
            .unwrap()
            .presence
            .insert(uid.clone(), status);
    }

    /// 当前所有在线成员的 presence。
    pub fn snapshot_presence(&self) -> Vec<kaede_protocol::Presence> {
        self.room
            .lock()
            .unwrap()
            .presence
            .iter()
            .map(|(uid, s)| kaede_protocol::Presence {
                login_id: uid.0.clone(),
                status: s.clone(),
            })
            .collect()
    }

    pub fn peer_uids(&self) -> Vec<UserId> {
        self.room.lock().unwrap().peers.keys().cloned().collect()
    }

    /// per-uid 限流。返回 true = 允许通过。
    ///
    /// 桶与 uid 绑定：同一 uid 断线重连后速率状态不重置。
    /// 空闲超过 `RATE_BUCKET_TTL` 的桶下次访问时惰性重置（防止 map 无限增长）。
    pub fn check_rate(&self, uid: &UserId, rate: f64, capacity: f64) -> bool {
        let mut map = self.rate_buckets.lock().unwrap();
        let bucket = map
            .entry(uid.clone())
            .or_insert_with(|| RateBucket::new(capacity));
        if bucket.last.elapsed() > RATE_BUCKET_TTL {
            *bucket = RateBucket::new(capacity);
        }
        bucket.try_consume(1.0, rate, capacity)
    }

    /// 主动清除某个 uid 的限流桶（如踢人时）。
    pub fn clear_rate(&self, uid: &UserId) {
        self.rate_buckets.lock().unwrap().remove(uid);
    }

    /// 广播给除 `sender` 外的所有成员。
    ///
    /// 先快照出所有 tx，再释放锁逐个 send。
    /// 这样将来即使换成带 backpressure 的 sender，也不会在锁内 await 造成死锁。
    pub fn broadcast_except(&self, sender: &UserId, msg: ServerMessage) {
        let targets: Vec<mpsc::Sender<ServerMessage>> = {
            let room = self.room.lock().unwrap();
            room.peers
                .iter()
                .filter(|(uid, _)| *uid != sender)
                .map(|(_, peer)| peer.tx.clone())
                .collect()
        };
        for tx in targets {
            let _ = tx.try_send(msg.clone());
        }
    }

    /// P1-7：关键消息（MlsCommit 等）用 await send 有背压，避免 try_send 静默丢弃。
    ///
    /// 所有目标**并发发送**：单个 slow consumer 最多拖 2s，而不是串行 N×2s。
    pub async fn broadcast_except_important(&self, sender: &UserId, msg: ServerMessage) {
        let targets: Vec<mpsc::Sender<ServerMessage>> = {
            let room = self.room.lock().unwrap();
            room.peers
                .iter()
                .filter(|(uid, _)| *uid != sender)
                .map(|(_, peer)| peer.tx.clone())
                .collect()
        };
        let sends = targets.into_iter().map(|tx| {
            let msg = msg.clone();
            async move {
                match tokio::time::timeout(BROADCAST_SEND_TIMEOUT, tx.send(msg)).await {
                    Ok(Ok(())) => {}
                    Ok(Err(_)) => tracing::warn!("broadcast_important: target closed"),
                    Err(_) => tracing::warn!("broadcast_important: target slow (2s), dropped"),
                }
            }
        });
        futures_util::future::join_all(sends).await;
    }

    /// 广播给所有在线成员（不过滤发送者）。
    pub fn broadcast_all(&self, msg: ServerMessage) {
        let targets: Vec<mpsc::Sender<ServerMessage>> = {
            let room = self.room.lock().unwrap();
            room.peers.values().map(|p| p.tx.clone()).collect()
        };
        for tx in targets {
            let _ = tx.try_send(msg.clone());
        }
    }

    /// 定向发送。
    pub fn send_to(&self, target: &UserId, msg: ServerMessage) -> bool {
        let tx = {
            let room = self.room.lock().unwrap();
            room.peers.get(target).map(|p| p.tx.clone())
        };
        if let Some(tx) = tx {
            tx.try_send(msg).is_ok()
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mk_state() -> AppState {
        use sqlx::sqlite::SqlitePoolOptions;
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let cfg = Config {
            bind: "127.0.0.1:0".into(),
            database_url: "sqlite::memory:".into(),
            session_ttl_hours: 1,
            livekit_api_key: "testkey".into(),
            livekit_api_secret: "testsecret".into(),
            livekit_ws_url: "ws://localhost:7880".into(),
        };
        AppState::new(pool, cfg)
    }

    #[tokio::test]
    async fn broadcast_all_reaches_every_peer() {
        let state = mk_state().await;
        let (tx1, mut rx1) = mpsc::channel::<ServerMessage>(16);
        let (tx2, mut rx2) = mpsc::channel::<ServerMessage>(16);
        state.add_peer(PeerHandle {
            uid: UserId::new("a"),
            tx: tx1,
            session_id: 0,
        });
        state.add_peer(PeerHandle {
            uid: UserId::new("b"),
            tx: tx2,
            session_id: 0,
        });

        state.broadcast_all(ServerMessage::PeerLeft {
            uid: UserId::new("x"),
        });

        assert!(rx1.try_recv().is_ok(), "peer a should receive");
        assert!(rx2.try_recv().is_ok(), "peer b should receive");
    }

    #[tokio::test]
    async fn broadcast_except_skips_sender() {
        let state = mk_state().await;
        let (tx_a, mut rx_a) = mpsc::channel::<ServerMessage>(16);
        let (tx_b, mut rx_b) = mpsc::channel::<ServerMessage>(16);
        let uid_a = UserId::new("a");
        state.add_peer(PeerHandle {
            uid: uid_a.clone(),
            tx: tx_a,
            session_id: 0,
        });
        state.add_peer(PeerHandle {
            uid: UserId::new("b"),
            tx: tx_b,
            session_id: 0,
        });

        state.broadcast_except(
            &uid_a,
            ServerMessage::PeerLeft {
                uid: UserId::new("x"),
            },
        );

        assert!(rx_a.try_recv().is_err(), "sender should not receive");
        assert!(rx_b.try_recv().is_ok(), "other should receive");
    }

    #[tokio::test]
    async fn send_to_unknown_returns_false() {
        let state = mk_state().await;
        let ok = state.send_to(
            &UserId::new("nobody"),
            ServerMessage::PeerLeft {
                uid: UserId::new("x"),
            },
        );
        assert!(!ok);
    }

    #[tokio::test]
    async fn send_to_known_returns_true() {
        let state = mk_state().await;
        let (tx, mut rx) = mpsc::channel::<ServerMessage>(16);
        let uid = UserId::new("a");
        state.add_peer(PeerHandle {
            uid: uid.clone(),
            tx,
            session_id: 0,
        });
        let ok = state.send_to(&uid, ServerMessage::Pong { nonce: 42 });
        assert!(ok);
        match rx.try_recv().unwrap() {
            ServerMessage::Pong { nonce } => assert_eq!(nonce, 42),
            _ => panic!("expected Pong"),
        }
    }

    // ---- per-uid 限流测试 ----

    #[test]
    fn rate_allows_burst_up_to_capacity() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let state = rt.block_on(mk_state());
        let uid = UserId::new("alice");
        for i in 0..10 {
            assert!(
                state.check_rate(&uid, 1.0, 10.0),
                "message {} should pass",
                i
            );
        }
        assert!(
            !state.check_rate(&uid, 1.0, 10.0),
            "11th should be rate limited"
        );
    }

    #[test]
    fn rate_is_per_uid_not_per_connection() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let state = rt.block_on(mk_state());
        let alice = UserId::new("alice");
        let bob = UserId::new("bob");

        for _ in 0..5 {
            assert!(state.check_rate(&alice, 1.0, 5.0));
        }
        assert!(!state.check_rate(&alice, 1.0, 5.0), "alice exhausted");
        assert!(state.check_rate(&bob, 1.0, 5.0), "bob independent");
    }

    #[test]
    fn rate_bucket_survives_disconnect() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let state = rt.block_on(mk_state());
        let alice = UserId::new("alice");

        for _ in 0..5 {
            assert!(state.check_rate(&alice, 1.0, 5.0));
        }
        assert!(!state.check_rate(&alice, 1.0, 5.0));
        // 没调 clear_rate → 模拟"断线不清桶"
        assert!(
            !state.check_rate(&alice, 1.0, 5.0),
            "should survive disconnect"
        );
    }

    #[test]
    fn clear_rate_resets_bucket() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let state = rt.block_on(mk_state());
        let alice = UserId::new("alice");

        for _ in 0..5 {
            assert!(state.check_rate(&alice, 1.0, 5.0));
        }
        assert!(!state.check_rate(&alice, 1.0, 5.0));

        state.clear_rate(&alice);
        assert!(
            state.check_rate(&alice, 1.0, 5.0),
            "after clear should pass"
        );
    }

    #[tokio::test]
    async fn remove_peer_if_session_rejects_stale_session() {
        let state = mk_state().await;
        let alice = UserId::new("alice");
        let (tx1, _rx1) = mpsc::channel::<ServerMessage>(16);
        let (tx2, _rx2) = mpsc::channel::<ServerMessage>(16);

        state.add_peer(PeerHandle {
            uid: alice.clone(),
            tx: tx1,
            session_id: 1,
        });
        state.add_peer(PeerHandle {
            uid: alice.clone(),
            tx: tx2,
            session_id: 2,
        });

        // 旧 session 退出，不应删除新 session 的 handle
        assert!(
            !state.remove_peer_if_session(&alice, 1),
            "stale session must not remove"
        );
        assert!(
            state.peer_uids().contains(&alice),
            "new session still present"
        );

        // 当前 session 退出，应该删除
        assert!(
            state.remove_peer_if_session(&alice, 2),
            "current session should remove"
        );
        assert!(!state.peer_uids().contains(&alice));
    }

    /// R2-06：参与者长期不再取 token 后，seed 必须被回收（下次进入换新 seed）。
    #[test]
    fn media_seed_rotates_after_participant_ttl() {
        use std::collections::HashMap;
        use std::time::Instant;

        let mut rooms = HashMap::new();
        let mut participants = HashMap::new();
        participants.insert("alice".to_string(), Instant::now());
        rooms.insert(
            "kaede-general".to_string(),
            RoomMedia {
                seed: [7u8; 32],
                participants,
                last_used: Instant::now(),
            },
        );

        // 未超过 TTL：房间保留。
        prune_media_rooms(
            &mut rooms,
            Duration::from_secs(3600),
            Duration::from_secs(3600),
        );
        assert!(rooms.contains_key("kaede-general"));

        // TTL=0：participant 立即过期 → 房间被删除 → 下次重新生成 seed。
        prune_media_rooms(&mut rooms, Duration::ZERO, Duration::ZERO);
        assert!(
            rooms.is_empty(),
            "stale media room/seed must be dropped after participant TTL"
        );
    }

    // ============================================================
    // Batch 5：P1-I（broadcast_except_important 超时）回归测试
    // ============================================================
    //
    // 修复位置：`broadcast_except_important` 的 `tx.send().await` 外层
    // 加了 `tokio::time::timeout(BROADCAST_SEND_TIMEOUT, ...)`。
    //
    // 测试策略：直接构造"buffer 已满"的 fake peer（`mpsc::channel(1)` 被
    // `try_send` 填满），跳过 HTTP/WS 层——`tx.send().await` 的阻塞路径与
    // 真实 slow client 完全相同，但可控、不 flaky、快。

    /// **P1-I #1**：单个 slow consumer 必须被 2s 超时放弃（不无限阻塞）。
    ///
    /// 修复前：`tx.send().await` 永远等待（rx 不消费，buffer 满）
    /// 修复后：2s 超时后放弃，broadcast 返回
    #[tokio::test]
    async fn p1i_slow_consumer_dropped_after_timeout() {
        let state = mk_state().await;

        // slow peer：channel 容量 1，已被 try_send 填满
        let (tx_slow, _rx_slow) = mpsc::channel::<ServerMessage>(1);
        tx_slow
            .try_send(ServerMessage::Pong { nonce: 0 })
            .expect("fill slow buffer");
        // 此时 tx_slow.send() 会阻塞（buffer 满 + rx 不消费）

        state.add_peer(PeerHandle {
            uid: UserId::new("slow"),
            tx: tx_slow,
            session_id: 1,
        });

        let sender = UserId::new("sender");
        let msg = ServerMessage::MlsCommit {
            from: sender.clone(),
            epoch: 1,
            payload: b"test".to_vec(),
        };

        let start = Instant::now();
        let result = tokio::time::timeout(
            Duration::from_secs(5), // 外层 5s 安全边界（修复前会超这里）
            state.broadcast_except_important(&sender, msg),
        )
        .await;
        let elapsed = start.elapsed();

        assert!(
            result.is_ok(),
            "P1-I regression: broadcast_except_important blocked forever (missing timeout)"
        );
        assert!(
            elapsed >= BROADCAST_SEND_TIMEOUT.mul_f64(0.9),
            "sanity: expected ~{:?}, got {:?} (was timeout actually triggered?)",
            BROADCAST_SEND_TIMEOUT,
            elapsed
        );
        assert!(
            elapsed < Duration::from_secs(4),
            "P1-I regression: took {:?}, expected ~{:?} + overhead",
            elapsed,
            BROADCAST_SEND_TIMEOUT
        );
    }

    /// **P1-I #2**：一个 slow peer 不阻止 healthy peer 收到消息。
    #[tokio::test]
    async fn p1i_healthy_peer_still_receives_after_slow_peer() {
        let state = mk_state().await;

        // slow peer
        let (tx_slow, _rx_slow) = mpsc::channel::<ServerMessage>(1);
        tx_slow
            .try_send(ServerMessage::Pong { nonce: 0 })
            .expect("fill slow buffer");
        state.add_peer(PeerHandle {
            uid: UserId::new("slow"),
            tx: tx_slow,
            session_id: 1,
        });

        // healthy peer
        let (tx_healthy, mut rx_healthy) = mpsc::channel::<ServerMessage>(16);
        state.add_peer(PeerHandle {
            uid: UserId::new("healthy"),
            tx: tx_healthy,
            session_id: 2,
        });

        let sender = UserId::new("sender");
        let msg = ServerMessage::MlsCommit {
            from: sender.clone(),
            epoch: 1,
            payload: b"important".to_vec(),
        };

        tokio::time::timeout(
            Duration::from_secs(5),
            state.broadcast_except_important(&sender, msg),
        )
        .await
        .expect("broadcast should complete (not block forever)");

        // healthy peer 必须收到
        match rx_healthy
            .try_recv()
            .expect("healthy peer should have received the message")
        {
            ServerMessage::MlsCommit { payload, .. } => {
                assert_eq!(payload, b"important");
            }
            other => panic!("expected MlsCommit, got {:?}", other),
        }
    }

    /// **P1-I #3（正向控制）**：全 healthy 时 broadcast 必须快速返回（性能不变）。
    ///
    /// 防止修复"过度"——如果有人把 timeout 改成"永远 sleep 2s 再返回"，
    /// 这个测试会红。
    #[tokio::test]
    async fn p1i_all_healthy_broadcast_is_fast() {
        let state = mk_state().await;

        let mut rxs = Vec::new();
        for i in 0..3u64 {
            let (tx, rx) = mpsc::channel::<ServerMessage>(16);
            state.add_peer(PeerHandle {
                uid: UserId::new(format!("peer-{}", i)),
                tx,
                session_id: i + 1,
            });
            rxs.push(rx);
        }

        let sender = UserId::new("sender");
        let msg = ServerMessage::MlsCommit {
            from: sender.clone(),
            epoch: 1,
            payload: b"fast".to_vec(),
        };

        let start = Instant::now();
        tokio::time::timeout(
            Duration::from_secs(1),
            state.broadcast_except_important(&sender, msg),
        )
        .await
        .expect("all-healthy broadcast should be fast");
        let elapsed = start.elapsed();

        assert!(
            elapsed < Duration::from_millis(200),
            "all-healthy broadcast took {:?}, expected < 200ms",
            elapsed
        );

        for mut rx in rxs {
            match rx.try_recv() {
                Ok(ServerMessage::MlsCommit { .. }) => {}
                other => panic!("peer should receive, got {:?}", other),
            }
        }
    }
}
