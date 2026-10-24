//! 共享状态：数据库池 + 房间成员表 + per-uid 限流。

use std::collections::HashMap;
use std::net::IpAddr;
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
    /// 该 WS 连接对应的 HTTP session_id（`sessions.session_id`）。
    ///
    /// 用于“撤销其他设备”时精确断开被撤销的连接，而不是误伤当前设备。
    pub http_session_id: String,
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

/// 单个账号的 WS 字节预算：8 MiB/s，突发 16 MiB。
///
/// 只按条数限流时，30 条/s × 4 MiB 可以打出 120 MiB/s 的单连接上行；
/// 这里给字节数单独一个桶，文本聊天（几百字节）完全不受影响。
pub(crate) const WS_BYTES_PER_SEC: f64 = 8.0 * 1024.0 * 1024.0;
pub(crate) const WS_BYTES_BURST: f64 = 16.0 * 1024.0 * 1024.0;

/// 全房间广播预算：32 MiB/s，突发 64 MiB。
///
/// 成本 = 单条 payload 字节数 × 收件人数，防止“一个大包 × 很多在线用户”
/// 的扇出放大拖垮服务器出口带宽。
pub(crate) const BROADCAST_BYTES_PER_SEC: f64 = 32.0 * 1024.0 * 1024.0;
pub(crate) const BROADCAST_BYTES_BURST: f64 = 64.0 * 1024.0 * 1024.0;

/// 并发 WebSocket 连接上限（同时是 semaphore 与 /metrics 的容量来源）。
pub const MAX_WS_CONNECTIONS: usize = 5000;

/// 单个客户端 IP 的并发 WS 连接上限（含尚未完成认证的握手）。
///
/// 全局 semaphore 只能限制总量；没有 per-IP 上限时，单个来源可以
/// 用大量未认证连接占满 permit（首帧超时 5s），让正常用户收到 503。
pub const MAX_WS_CONNECTIONS_PER_IP: usize = 64;

/// 单 IP WS 连接计数守卫：drop 时递减，保证异常路径也能归还额度。
pub struct WsIpGuard {
    map: Arc<Mutex<HashMap<IpAddr, usize>>>,
    ip: IpAddr,
}

impl Drop for WsIpGuard {
    fn drop(&mut self) {
        let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(n) = map.get_mut(&self.ip) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                map.remove(&self.ip);
            }
        }
    }
}

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
    /// 限流桶：Key = `"{uid}:{scope}"`，见 `check_rate_scoped`。
    pub rate_buckets: Arc<Mutex<HashMap<String, RateBucket>>>,
    /// 房间级广播字节预算（所有用户共享）。
    pub broadcast_budget: Arc<Mutex<RateBucket>>,
    /// P0-1：连接 session id 生成器。
    pub next_session_id: Arc<std::sync::atomic::AtomicU64>,
    /// P1-10：WS 连接并发上限。
    pub ws_semaphore: Arc<tokio::sync::Semaphore>,
    /// 单 IP 并发 WS 连接计数（含握手阶段）。
    pub ws_conns_per_ip: Arc<Mutex<HashMap<IpAddr, usize>>>,
    /// P0-2：login 并发上限（防 Argon2 内存 DoS）。
    pub login_semaphore: Arc<tokio::sync::Semaphore>,
    /// P0-2：login 失败计数 `(uid, 来源 IP)` -> (次数, 首次失败时间)。
    ///
    /// 按 IP 隔离可避免攻击者用 5 次错误密码把任意已知账号锁死 15 分钟。
    #[allow(clippy::type_complexity)]
    pub login_failures: Arc<Mutex<HashMap<(String, IpAddr), (u32, Instant)>>>,
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
    /// 管理面板 token。空字符串表示 admin 未启用或未设置。
    pub admin_token: String,
    /// Prometheus 计数器。
    pub metrics: Arc<crate::metrics::Metrics>,
    /// 进程启动时间，用于 admin dashboard 的 uptime。
    pub started_at: Instant,
    /// 内存日志环形缓冲，admin /api/logs 从这里读。
    pub log_buffer: crate::admin::logs::LogBuffer,
}

impl AppState {
    pub fn new(pool: SqlitePool, cfg: Config) -> Self {
        Self::new_with_log(pool, cfg, crate::admin::logs::LogBuffer::new())
    }

    /// 带共享 log buffer 的构造器（main.rs 用：subscriber layer 与 AppState 共享同一 buffer）。
    pub fn new_with_log(
        pool: SqlitePool,
        cfg: Config,
        log_buffer: crate::admin::logs::LogBuffer,
    ) -> Self {
        // 提前提取：cfg 会在结构体构造时被 move 进 Self
        let admin_token_for_state = cfg.admin_token.clone();
        let rate_buckets = Arc::new(Mutex::new(HashMap::<String, RateBucket>::new()));
        // P1-6：后台清理任务，防恶意注册 uid 撑爆 rate_buckets。
        let buckets_clone = rate_buckets.clone();
        let kp_budget = Arc::new(Mutex::new(HashMap::<String, (u32, Instant)>::new()));
        let kp_budget_clone = kp_budget.clone();
        let login_failures = Arc::new(Mutex::new(
            HashMap::<(String, IpAddr), (u32, Instant)>::new(),
        ));
        let login_failures_clone = login_failures.clone();
        let room_media_seeds = Arc::new(Mutex::new(HashMap::<String, RoomMedia>::new()));
        let room_media_clone = room_media_seeds.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(300));
            tick.tick().await;
            loop {
                tick.tick().await;
                {
                    let mut m = buckets_clone.lock().unwrap_or_else(|e| e.into_inner());
                    m.retain(|_, b| b.last.elapsed() < RATE_BUCKET_TTL);
                }
                {
                    let mut m = kp_budget_clone.lock().unwrap_or_else(|e| e.into_inner());
                    m.retain(|_, (_, start)| start.elapsed() < Duration::from_secs(600));
                }
                {
                    let mut m = login_failures_clone
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    m.retain(|_, (_, first)| first.elapsed() < Duration::from_secs(900));
                }
                {
                    // 先剔除超过 TTL 未续期的参与者（崩溃/关页不会调用 /leave），
                    // 再删除空房间 + 1 小时未活动的 seed。
                    let mut m = room_media_clone.lock().unwrap_or_else(|e| e.into_inner());
                    prune_media_rooms(&mut m, MEDIA_PARTICIPANT_TTL, Duration::from_secs(3600));
                }
                // 注意：DB 侧过期数据清理只在 main.rs 的每小时任务里做。
                // 这里再跑一遍会造成两套清理调度互相干扰（同一张表被两个周期扫）。
            }
        });
        Self {
            pool,
            cfg,
            room: Arc::new(Mutex::new(Room::default())),
            rate_buckets,
            broadcast_budget: Arc::new(Mutex::new(RateBucket::new(BROADCAST_BYTES_BURST))),
            next_session_id: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            ws_semaphore: Arc::new(tokio::sync::Semaphore::new(MAX_WS_CONNECTIONS)),
            ws_conns_per_ip: Arc::new(Mutex::new(HashMap::new())),
            login_semaphore: Arc::new(tokio::sync::Semaphore::new(8)),
            login_failures,
            room_media_seeds,
            kp_fetch_budget: kp_budget,
            admin_token: admin_token_for_state,
            metrics: Arc::new(crate::metrics::Metrics::default()),
            started_at: Instant::now(),
            log_buffer,
        }
    }

    /// KeyPackage 拉取限流（P1-11 双桶）。
    ///
    /// 原实现只按 target 限流：任意 10 个账号轮换即可把某个 target 的
    /// 额度打满，让正常用户（房主邀请、新成员进群）拿不到 KP。
    /// 现在同时检查两个维度：
    ///   - caller 维度（每分钟最多 20 次，不限目标）：防多账号轮换；
    ///   - target 维度（每分钟最多 10 次，不限来源）：防集中抽干。
    ///
    /// 好友（friends.is_friend）在 http.rs 层豁免，不进本函数——
    /// 所以合法的高频邀请场景不受影响。
    pub fn check_kp_fetch(&self, caller: &str, target: &str) -> bool {
        const CALLER_LIMIT: u32 = 20;
        const TARGET_LIMIT: u32 = 10;
        const WINDOW: Duration = Duration::from_secs(60);
        let mut m = self
            .kp_fetch_budget
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let caller_key = format!("kfc:{}", caller);
        let target_key = format!("kft:{}", target);

        // 阶段 1：重置 caller 桶（如过期）并检查
        {
            let now = Instant::now();
            let c = m.entry(caller_key.clone()).or_insert((0, now));
            if c.1.elapsed() > WINDOW {
                *c = (0, now);
            }
            if c.0 >= CALLER_LIMIT {
                return false;
            }
        }

        // 阶段 2：重置 target 桶（如过期）并检查
        {
            let now = Instant::now();
            let t = m.entry(target_key.clone()).or_insert((0, now));
            if t.1.elapsed() > WINDOW {
                *t = (0, now);
            }
            if t.0 >= TARGET_LIMIT {
                return false;
            }
        }

        // 阶段 3：两个桶都通过，一起 +1
        if let Some(c) = m.get_mut(&caller_key) {
            c.0 = c.0.saturating_add(1);
        }
        if let Some(t) = m.get_mut(&target_key) {
            t.0 = t.0.saturating_add(1);
        }
        true
    }

    /// 尝试占用一个单 IP WS 连接额度。超限返回 None。
    pub fn try_acquire_ws_ip(&self, ip: IpAddr) -> Option<WsIpGuard> {
        let mut map = self
            .ws_conns_per_ip
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let n = map.entry(ip).or_insert(0);
        if *n >= MAX_WS_CONNECTIONS_PER_IP {
            return None;
        }
        *n += 1;
        Some(WsIpGuard {
            map: self.ws_conns_per_ip.clone(),
            ip,
        })
    }

    /// 当前连接的 session id（若用户在线）。
    pub fn peer_session_id(&self, uid: &UserId) -> Option<u64> {
        self.room
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .peers
            .get(uid)
            .map(|p| p.session_id)
    }

    /// P0-1（单设备止血）：后登连接踢掉前登。返回被踢的旧 handle。
    pub fn add_peer(&self, handle: PeerHandle) -> Option<PeerHandle> {
        let mut room = self.room.lock().unwrap_or_else(|e| e.into_inner());
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
        let mut room = self.room.lock().unwrap_or_else(|e| e.into_inner());
        room.presence.remove(uid);
        room.peers.remove(uid)
    }

    /// 仅当在线连接属于指定 HTTP session 时才移除（撤销单个会话用）。
    pub fn take_peer_if_http_session(
        &self,
        uid: &UserId,
        http_session_id: &str,
    ) -> Option<PeerHandle> {
        let mut room = self.room.lock().unwrap_or_else(|e| e.into_inner());
        let matches = room
            .peers
            .get(uid)
            .is_some_and(|p| p.http_session_id == http_session_id);
        if matches {
            room.presence.remove(uid);
            room.peers.remove(uid)
        } else {
            None
        }
    }

    /// 仅当在线连接不属于当前 HTTP session 时才移除（撤销其他设备用）。
    pub fn take_peer_if_other_http_session(
        &self,
        uid: &UserId,
        current_http_session: &str,
    ) -> Option<PeerHandle> {
        let mut room = self.room.lock().unwrap_or_else(|e| e.into_inner());
        let is_other = room
            .peers
            .get(uid)
            .is_some_and(|p| p.http_session_id != current_http_session);
        if is_other {
            room.presence.remove(uid);
            room.peers.remove(uid)
        } else {
            None
        }
    }

    pub fn remove_peer(&self, uid: &UserId) {
        let mut room = self.room.lock().unwrap_or_else(|e| e.into_inner());
        room.peers.remove(uid);
        room.presence.remove(uid);
    }

    /// P0-X：仅当 stored session_id 匹配时才移除。
    /// 防止旧连接退出清理误删新连接（多设备 / 重连）。
    pub fn remove_peer_if_session(&self, uid: &UserId, session_id: u64) -> bool {
        let mut room = self.room.lock().unwrap_or_else(|e| e.into_inner());
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
            .unwrap_or_else(|e| e.into_inner())
            .presence
            .insert(uid.clone(), status);
    }

    /// 当前所有在线成员的 presence。
    pub fn snapshot_presence(&self) -> Vec<kaede_protocol::Presence> {
        self.room
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .presence
            .iter()
            .map(|(uid, s)| kaede_protocol::Presence {
                login_id: uid.0.clone(),
                status: s.clone(),
            })
            .collect()
    }

    pub fn peer_uids(&self) -> Vec<UserId> {
        self.room
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .peers
            .keys()
            .cloned()
            .collect()
    }

    /// per-uid 限流。返回 true = 允许通过。
    ///
    /// 桶与 uid 绑定：同一 uid 断线重连后速率状态不重置。
    /// 空闲超过 `RATE_BUCKET_TTL` 的桶下次访问时惰性重置（防止 map 无限增长）。
    pub fn check_rate(&self, uid: &UserId, rate: f64, capacity: f64) -> bool {
        self.check_rate_scoped(uid, "default", rate, capacity)
    }

    /// per-uid + per-scope 限流。
    ///
    /// 不同端点（发言 / 拉 KP / 语音 token / 上传）的速率和容量差异很大，
    /// 共用一个桶会让聊天把 KeyPackage 拉取额度挤掉。scope 用于隔离它们。
    pub fn check_rate_scoped(&self, uid: &UserId, scope: &str, rate: f64, capacity: f64) -> bool {
        self.check_rate_scoped_cost(uid, scope, rate, capacity, 1.0)
    }

    /// 按权重消耗 token。`cost` 可以是字节数（字节预算）或消息条数。
    pub fn check_rate_scoped_cost(
        &self,
        uid: &UserId,
        scope: &str,
        rate: f64,
        capacity: f64,
        cost: f64,
    ) -> bool {
        let key = format!("{}:{}", uid.as_str(), scope);
        self.check_rate_key_cost(&key, rate, capacity, cost)
    }

    /// 按任意 key 限流（用于未登录请求，例如按来源 IP 限制注册）。
    pub fn check_rate_key(&self, key: &str, rate: f64, capacity: f64) -> bool {
        self.check_rate_key_cost(key, rate, capacity, 1.0)
    }

    pub fn check_rate_key_cost(&self, key: &str, rate: f64, capacity: f64, cost: f64) -> bool {
        let mut map = self.rate_buckets.lock().unwrap_or_else(|e| e.into_inner());
        let bucket = map
            .entry(key.to_string())
            .or_insert_with(|| RateBucket::new(capacity));
        if bucket.last.elapsed() > RATE_BUCKET_TTL {
            *bucket = RateBucket::new(capacity);
        }
        bucket.try_consume(cost, rate, capacity)
    }

    /// 预留全房间广播带宽（成本 = payload 字节 × 收件人数）。
    pub fn try_reserve_broadcast(&self, payload_bytes: usize, recipients: usize) -> bool {
        let cost = payload_bytes as f64 * recipients.max(1) as f64;
        self.broadcast_budget
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .try_consume(cost, BROADCAST_BYTES_PER_SEC, BROADCAST_BYTES_BURST)
    }

    pub fn peer_count(&self) -> usize {
        self.room
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .peers
            .len()
    }

    /// 参与者离开媒体房间（HTTP `/leave` 或 LiveKit webhook）。
    ///
    /// 最后一人离开后立刻丢弃 seed，保证下一次会话一定是新 key。
    pub fn media_participant_left(&self, room: &str, uid: &str) {
        let mut seeds = self
            .room_media_seeds
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = seeds.get_mut(room) {
            entry.participants.remove(uid);
            entry.last_used = Instant::now();
            if entry.participants.is_empty() {
                seeds.remove(room);
                tracing::info!("[livekit] media session ended, seed dropped room={}", room);
            }
        }
    }

    /// 整个媒体房间结束（LiveKit `room_finished`），无条件丢弃 seed。
    pub fn media_room_finished(&self, room: &str) {
        let mut seeds = self
            .room_media_seeds
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if seeds.remove(room).is_some() {
            tracing::info!("[livekit] room finished, seed dropped room={}", room);
        }
    }

    /// 把某用户从**所有**媒体房间中移除，返回受影响的 LiveKit 房间名列表。
    ///
    /// 用于封禁 / 踢人：这两个场景拿不到具体频道，但必须吊销该账号的媒体访问，
    /// 否则已建立的 LiveKit 连接会一直有效到 token 过期（6 小时）。
    ///
    /// 房间内没有其他参与者时直接销毁 seed，下一次进入会生成全新 e2ee key。
    pub fn media_revoke_user(&self, uid: &str) -> Vec<String> {
        let mut rooms = Vec::new();
        let mut seeds = self
            .room_media_seeds
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        seeds.retain(|room, entry| {
            if entry.participants.remove(uid).is_some() {
                rooms.push(room.clone());
            }
            entry.last_used = Instant::now();
            !entry.participants.is_empty()
        });
        if !rooms.is_empty() {
            tracing::info!(
                "[livekit] revoked media access for uid={} in {} room(s)",
                uid,
                rooms.len()
            );
        }
        rooms
    }

    /// 主动清除某个 uid 的所有限流桶（如踢人时）。
    pub fn clear_rate(&self, uid: &UserId) {
        let prefix = format!("{}:", uid.as_str());
        self.rate_buckets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|k, _| !k.starts_with(&prefix));
    }

    /// 广播给除 `sender` 外的所有成员。
    ///
    /// 先快照出所有 tx，再释放锁逐个 `try_send`（锁内不 await）。
    ///
    /// **丢弃语义**：目标出站缓冲（每连接 1024 条）满时，该成员**静默丢失**
    /// 此消息。仅用于可丢失/可由历史拉取补偿的消息（如 reaction、普通事件）。
    /// 必须可靠到达的控制消息（MlsCommit 等）走 [`Self::broadcast_except_important`]。
    pub fn broadcast_except(&self, sender: &UserId, msg: ServerMessage) {
        let targets: Vec<mpsc::Sender<ServerMessage>> = {
            let room = self.room.lock().unwrap_or_else(|e| e.into_inner());
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

    /// P1-7：关键消息（MlsCommit 等）的广播路径。
    ///
    /// 与 `broadcast_except` 的差异：用 await send 等待缓冲腾出空间，
    /// 单个 slow consumer 最多阻塞 2s（`BROADCAST_SEND_TIMEOUT`），超时后
    /// **该成员仍会丢失此消息**（记 metrics + warn 日志），但不会拖住其他成员。
    /// 所有目标并发发送，总耗时取决于最慢的一个（上限 2s），而非串行 N×2s。
    ///
    /// 注意：这不是"保证送达"。被超时丢弃的成员依赖重连后的离线补投追上状态。
    pub async fn broadcast_except_important(&self, sender: &UserId, msg: ServerMessage) {
        let targets: Vec<mpsc::Sender<ServerMessage>> = {
            let room = self.room.lock().unwrap_or_else(|e| e.into_inner());
            room.peers
                .iter()
                .filter(|(uid, _)| *uid != sender)
                .map(|(_, peer)| peer.tx.clone())
                .collect()
        };
        let sends = targets.into_iter().map(|tx| {
            let msg = msg.clone();
            let metrics = self.metrics.clone();
            async move {
                match tokio::time::timeout(BROADCAST_SEND_TIMEOUT, tx.send(msg)).await {
                    Ok(Ok(())) => {}
                    Ok(Err(_)) => tracing::warn!("broadcast_important: target closed"),
                    Err(_) => {
                        metrics.slow_peer_dropped();
                        tracing::warn!("broadcast_important: target slow (2s), dropped");
                    }
                }
            }
        });
        futures_util::future::join_all(sends).await;
    }

    /// 广播给所有在线成员（含发送者本人）。
    ///
    /// 丢弃语义同 [`Self::broadcast_except`]：缓冲满时静默丢失，
    /// 仅用于可丢失/可补偿的消息。
    pub fn broadcast_all(&self, msg: ServerMessage) {
        let targets: Vec<mpsc::Sender<ServerMessage>> = {
            let room = self.room.lock().unwrap_or_else(|e| e.into_inner());
            room.peers.values().map(|p| p.tx.clone()).collect()
        };
        for tx in targets {
            let _ = tx.try_send(msg.clone());
        }
    }

    /// 定向发送。
    pub fn send_to(&self, target: &UserId, msg: ServerMessage) -> bool {
        let tx = {
            let room = self.room.lock().unwrap_or_else(|e| e.into_inner());
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
            livekit_ws_url: "ws://localhost:18082".into(),
            admin_enabled: false,
            admin_bind: "127.0.0.1:18081".into(),
            admin_token: String::new(),
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
            http_session_id: String::new(),
        });
        state.add_peer(PeerHandle {
            uid: UserId::new("b"),
            tx: tx2,
            session_id: 0,
            http_session_id: String::new(),
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
            http_session_id: String::new(),
        });
        state.add_peer(PeerHandle {
            uid: UserId::new("b"),
            tx: tx_b,
            session_id: 0,
            http_session_id: String::new(),
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
            http_session_id: String::new(),
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

    #[test]
    fn byte_budget_limits_large_messages() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let state = rt.block_on(mk_state());
        let alice = UserId::new("alice");
        let mib = 1024.0 * 1024.0;

        // 16 MiB 突发正好允许 4 个 4 MiB 包。
        for i in 0..4 {
            assert!(
                state.check_rate_scoped_cost(
                    &alice,
                    "ws-bytes",
                    WS_BYTES_PER_SEC,
                    WS_BYTES_BURST,
                    4.0 * mib
                ),
                "4 MiB packet {} should pass",
                i
            );
        }
        assert!(
            !state.check_rate_scoped_cost(
                &alice,
                "ws-bytes",
                WS_BYTES_PER_SEC,
                WS_BYTES_BURST,
                4.0 * mib
            ),
            "5th 4 MiB packet must be limited"
        );
    }

    #[test]
    fn broadcast_budget_accounts_for_fanout() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let state = rt.block_on(mk_state());
        let mib = 1024 * 1024;

        // 4 MiB × 32 收件人 = 128 MiB，超过 64 MiB 突发。
        assert!(
            !state.try_reserve_broadcast(4 * mib, 32),
            "large fanout must be rejected"
        );
        // 普通文本 × 100 人完全在预算内。
        assert!(
            state.try_reserve_broadcast(1024, 100),
            "small broadcast should pass"
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
            http_session_id: String::new(),
        });
        state.add_peer(PeerHandle {
            uid: alice.clone(),
            tx: tx2,
            session_id: 2,
            http_session_id: String::new(),
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

    /// 封禁 / 踢人：必须吊销目标账号在所有媒体房间的访问。
    #[tokio::test]
    async fn media_revoke_user_drops_only_target() {
        let state = mk_state().await;

        let now = Instant::now();
        {
            let mut seeds = state
                .room_media_seeds
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            seeds.insert(
                "kaede-general".into(),
                RoomMedia {
                    seed: [1u8; 32],
                    participants: HashMap::from([
                        ("alice".to_string(), now),
                        ("bob".to_string(), now),
                    ]),
                    last_used: now,
                },
            );
            seeds.insert(
                "kaede-voice".into(),
                RoomMedia {
                    seed: [2u8; 32],
                    participants: HashMap::from([("alice".to_string(), now)]),
                    last_used: now,
                },
            );
        }

        let mut affected = state.media_revoke_user("alice");
        affected.sort();
        assert_eq!(affected, vec!["kaede-general", "kaede-voice"]);

        let seeds = state
            .room_media_seeds
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // alice 独占的房间被销毁；还有 bob 的房间保留但已剔除 alice。
        assert!(!seeds.contains_key("kaede-voice"));
        let general = seeds.get("kaede-general").expect("room kept");
        assert!(!general.participants.contains_key("alice"));
        assert!(general.participants.contains_key("bob"));
    }

    /// 用户不在任何房间时不应误删 seed。
    #[tokio::test]
    async fn media_revoke_unknown_user_is_noop() {
        let state = mk_state().await;
        let now = Instant::now();
        state
            .room_media_seeds
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                "kaede-general".into(),
                RoomMedia {
                    seed: [3u8; 32],
                    participants: HashMap::from([("bob".to_string(), now)]),
                    last_used: now,
                },
            );

        assert!(state.media_revoke_user("alice").is_empty());
        assert!(
            state
                .room_media_seeds
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains_key("kaede-general")
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
            http_session_id: String::new(),
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
            http_session_id: String::new(),
        });

        // healthy peer
        let (tx_healthy, mut rx_healthy) = mpsc::channel::<ServerMessage>(16);
        state.add_peer(PeerHandle {
            uid: UserId::new("healthy"),
            tx: tx_healthy,
            session_id: 2,
            http_session_id: String::new(),
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
                http_session_id: String::new(),
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
