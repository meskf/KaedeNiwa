//! 单个 server 连接。
//!
//! 融合三部分：
//! 1. 状态机（[`ClientStateMachine`]）
//! 2. 网络 I/O（[`Transport`]）
//! 3. MLS 会话（[`MlsSession`]）
//!
//! # 生命周期
//!
//! ```text
//! Idle
//!   └─ connect() ─────────→ Connecting
//!        └─ authenticate() → Authenticated
//!             ├─ join()   → MlsJoining
//!             │    ├─ mls_create_group()        → MlsReady
//!             │    └─ mls_join_with_welcome()   → MlsReady
//!             └─ ...
//! ```
//!
//! # 线程模型
//!
//! `MlsSession` 用 `std::sync::Mutex` 保护。MLS 操作全部是同步的，
//! **不跨 await 持锁**。发送消息时先在锁内加密，释放锁后再 await。
//!
//! # 子模块划分（本文件只保留核心状态 + 传输生命周期）
//!
//! - [`main_group`]：主群 MLS 建群 / 加人 / 删人 / exporter / 快照
//! - [`dm`]：DM 双人 group、v2 密封、代际仲裁
//! - [`channel`]：私密频道 group、成员集合校验、频道密文
//! - [`snapshot`]：DM / 频道 group 快照的 dirty 跟踪与落盘恢复
//! - [`inbound`]：`handle_server_message` 入站分发
//! - [`pending`]：主群 Welcome 的用户确认队列

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use kaede_protocol::{ClientMessage, ServerMessage, UserId};

use crate::ClientObserver;
use crate::error::ClientError;
use crate::event::ClientEvent;
use crate::http::HttpApi;
use crate::mls_session::MlsSession;
use crate::state::{ClientState, ClientStateMachine};
use crate::transport::Transport;

mod channel;
mod dm;
mod inbound;
mod main_group;
mod pending;
mod snapshot;
#[allow(deprecated)]
#[cfg(test)]
mod tests;

pub(crate) use dm::{pack_dm_welcome, unpack_dm_welcome};
use pending::PendingMainWelcome;
pub use pending::PendingWelcomeInfo;

/// B2：join_channel 上传 KP 后，等 Welcome 的异步应答通道类型。
type PendingWelcomeSender = async_channel::Sender<Result<Vec<u8>, String>>;
/// B2：channel_id → 等待 Welcome 的 sender 映射。
type ChannelPendingWelcomes =
    std::sync::Mutex<std::collections::HashMap<String, PendingWelcomeSender>>;

/// 单个 server 的连接句柄。
pub struct ServerConnection {
    server_id: String,
    state: Mutex<ClientStateMachine>,
    transport: Arc<dyn Transport>,
    observer: Option<Arc<dyn ClientObserver>>,
    peers: Mutex<Vec<UserId>>,
    mls: Mutex<Option<MlsSession>>,
    dm_groups: Mutex<std::collections::HashMap<String, MlsSession>>,
    channel_groups: Mutex<std::collections::HashMap<String, MlsSession>>,
    /// B2：join_channel 上传 KP 后，等 Welcome 的 channel 集合。
    channel_pending_welcomes: ChannelPendingWelcomes,
    /// DM 会话代际：每次本地重建 DM group 时 +1，用于解决单侧重建后的分裂。
    dm_generations: Mutex<std::collections::HashMap<String, u64>>,
    /// 待写盘的 DM 代际。
    dirty_dm_meta: Mutex<std::collections::HashSet<String>>,
    /// B3+：DM / 私密频道 group 自动持久化 hook。
    /// 由 `RoomSession::connect_with_storage` 注入；未注入时所有持久化 no-op。
    aux_storage: Mutex<Option<AuxStorageHook>>,
    /// B3+：待写盘的 DM conv_id 集合。
    dirty_dm_groups: Mutex<std::collections::HashSet<String>>,
    /// B3+：待写盘的 channel_id 集合。
    dirty_channel_groups: Mutex<std::collections::HashSet<String>>,
    /// 主群 MLS 状态是否有未落盘的变更（发送/接收/epoch 前进）。
    ///
    /// **必须**在每次 `create_message` / `process_message` / commit merge 后置位：
    /// 否则重启后会回退 sender ratchet，导致 MLS 层 AES-GCM nonce 复用。
    main_dirty: AtomicBool,
    /// 本次是否从本地快照恢复了 MLS 身份。
    ///
    /// 为 false 表示身份是新建的：服务端旧 KeyPackage 对新身份不可解密，
    /// 首次发布 KP 时必须替换而不是追加。
    identity_restored: AtomicBool,
    /// 串行化主群快照写盘。
    ///
    /// `RoomSession::save_snapshot` 与 `ServerConnection::flush_dirty_snapshots`
    /// 会写同一个 `mls/{server}/{uid}` key；没有这把锁时，旧 epoch 的快照
    /// 可能后写覆盖新快照，重启后 ratchet 回退 → nonce 复用。
    snapshot_write: futures_util::lock::Mutex<()>,
    /// 任何 MLS snapshot 持久化失败后置位：DM / 私密频道 group 的 epoch 与
    /// sender ratchet 可能已前进但没有落盘，此时继续发送会在重启后回退到旧状态。
    ///
    /// 与主群的 [`RoomSession::snapshot_broken`] 语义一致，但覆盖 DM / 频道路径
    /// （这些 snapshot 由 [`ServerConnection::flush_dirty_snapshots`] 负责写盘）。
    snapshot_broken: AtomicBool,
    /// 当前用户是否是主群 host（op5）。由 RoomSession 注入。
    is_host: AtomicBool,
    /// DM / 私密频道 v2 密文的 in-memory 重放窗口。
    dm_replay: Mutex<ReplayGuard>,
    channel_replay: Mutex<ReplayGuard>,
    /// 已知私密频道集合。用于在**本地还没有 channel group** 时也拒绝
    /// 主群广播里伪造的私密频道消息 / reaction。
    private_channels: Mutex<HashSet<String>>,
    /// 本地刚生成、尚未确认被服务端采纳的主群 Commit 回滚备份。
    ///
    /// 管理员并发提交同一 epoch 时，服务端只保留第一份；落败者需要
    /// 回滚本地已应用的 Commit，再跟随胜出 Commit，否则永久分叉。
    commit_backup: Mutex<Option<CommitBackup>>,
    /// epoch 缺失时暂存的 Commit：epoch → payload。上限 32 条。
    pending_commits: Mutex<std::collections::BTreeMap<u64, Vec<u8>>>,
    /// 上次 `ReportEpoch` 的 (local_epoch, 时间)。
    ///
    /// 防止服务端返回一份无法应用的陈旧 Commit 时，客户端反复重拉同一份
    /// 数据形成死循环（每次失败都 ReportEpoch → 又拿到同一份 → 又失败）。
    last_epoch_report: Mutex<Option<(u64, web_time::Instant)>>,
    /// 频道成员校验来源。生产由 `RoomSession` 注入；用于在加入/推进频道 group
    /// 前对照服务端 `channel_members`，拒绝把群外成员带进 MLS group。
    http: Mutex<Option<HttpApi>>,
    /// Round 6：等待用户确认的主群 Welcome。key = sender login_id。
    ///
    /// 收到 MlsWelcome 时不立即入群，而是存这里并 emit `PendingWelcome`。
    /// 用户通过 UI 调 `accept_pending_welcome(sender)` 或
    /// `reject_pending_welcome(sender)` 决定。
    ///
    /// **为什么是 HashMap 而不是 Option**：攻击者可以反复发恶意 Welcome，
    /// 用 Option 会让后来的合法 Welcome 覆盖先前的恶意 Welcome（或反之）。
    /// HashMap 允许并存，UI 层负责限流展示。
    pending_main_welcomes: Mutex<std::collections::HashMap<String, PendingMainWelcome>>,
}

type ReplayEntry = (HashSet<[u8; 16]>, VecDeque<[u8; 16]>);

/// 本地主群 Commit 的回滚备份（只存在于内存，不写盘）。
struct CommitBackup {
    /// 该 Commit 应用后的 epoch。
    new_epoch: u64,
    /// Commit 应用前的完整 MLS 状态（明文，用后 zeroize）。
    state: zeroize::Zeroizing<Vec<u8>>,
}

/// 每个会话最近 1024 条消息 id 的滑动窗口，用于拒重放。
#[derive(Default)]
struct ReplayGuard {
    seen: HashMap<String, ReplayEntry>,
}

impl ReplayGuard {
    fn accept(&mut self, conv_id: &str, id: [u8; 16]) -> bool {
        const WINDOW: usize = 1024;
        let (set, order) = self
            .seen
            .entry(conv_id.to_string())
            .or_insert_with(|| (HashSet::new(), VecDeque::new()));
        if !set.insert(id) {
            return false;
        }
        order.push_back(id);
        while order.len() > WINDOW {
            if let Some(old) = order.pop_front() {
                set.remove(&old);
            }
        }
        true
    }
}

/// B3+：DM / 私密频道 group 自动持久化 hook。
///
/// 保存格式与主群 snapshot 一致：外层 `[0x03][alg][u32 len][ct]`，
/// 通过 `device_cipher` / `device_cipher_for_snapshot` 加密 / 解密。
///
/// 存储布局：
/// - ns=`"dm"`,      key=`"{server_url}/{conv_id}"`
/// - ns=`"channel"`, key=`"{server_url}/{channel_id}"`
#[derive(Clone)]
pub struct AuxStorageHook {
    pub store: Arc<kaede_storage::Store>,
    pub server_url: String,
    pub uid: String,
}

impl ServerConnection {
    pub fn new(
        server_id: impl Into<String>,
        transport: Arc<dyn Transport>,
        observer: Option<Arc<dyn ClientObserver>>,
    ) -> Self {
        Self {
            server_id: server_id.into(),
            state: Mutex::new(ClientStateMachine::new()),
            transport,
            observer,
            peers: Mutex::new(Vec::new()),
            mls: Mutex::new(None),
            dm_groups: Mutex::new(std::collections::HashMap::new()),
            channel_groups: Mutex::new(std::collections::HashMap::new()),
            channel_pending_welcomes: Mutex::new(std::collections::HashMap::new()),
            dm_generations: Mutex::new(std::collections::HashMap::new()),
            dirty_dm_meta: Mutex::new(std::collections::HashSet::new()),
            aux_storage: Mutex::new(None),
            dirty_dm_groups: Mutex::new(std::collections::HashSet::new()),
            dirty_channel_groups: Mutex::new(std::collections::HashSet::new()),
            main_dirty: AtomicBool::new(false),
            identity_restored: AtomicBool::new(false),
            snapshot_write: futures_util::lock::Mutex::new(()),
            snapshot_broken: AtomicBool::new(false),
            is_host: AtomicBool::new(false),
            dm_replay: Mutex::new(ReplayGuard::default()),
            channel_replay: Mutex::new(ReplayGuard::default()),
            private_channels: Mutex::new(HashSet::new()),
            commit_backup: Mutex::new(None),
            pending_commits: Mutex::new(std::collections::BTreeMap::new()),
            last_epoch_report: Mutex::new(None),
            http: Mutex::new(None),
            pending_main_welcomes: Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// 注入 HTTP 客户端，供频道成员集合校验使用。
    pub fn set_http(&self, http: HttpApi) {
        *self.http.lock().unwrap_or_else(|e| e.into_inner()) = Some(http);
    }

    // =================================================================
    // 主群状态持久化
    // =================================================================

    fn mark_main_dirty(&self) {
        self.main_dirty.store(true, Ordering::Release);
    }

    /// 取走“主群状态已变更”标志。返回 true 表示调用方应立即落盘。
    pub fn take_main_dirty(&self) -> bool {
        self.main_dirty.swap(false, Ordering::AcqRel)
    }

    pub fn set_identity_restored(&self, restored: bool) {
        self.identity_restored.store(restored, Ordering::Release);
    }

    pub fn identity_restored(&self) -> bool {
        self.identity_restored.load(Ordering::Acquire)
    }

    /// 获取主群快照写盘锁。
    ///
    /// 所有写 `mls/{server}/{uid}` key 的路径都必须持有它，并在锁内
    /// **重新取快照**，否则旧快照可能后写覆盖新快照。
    pub async fn lock_snapshot_write(&self) -> futures_util::lock::MutexGuard<'_, ()> {
        self.snapshot_write.lock().await
    }

    /// 保存失败时把标志放回去，下一条消息会再次尝试。
    pub fn restore_main_dirty(&self) {
        self.main_dirty.store(true, Ordering::Release);
    }

    /// 记录本地刚生成的 Commit 的回滚备份。
    fn store_commit_backup(&self, new_epoch: u64, state: Vec<u8>) {
        *self.commit_backup.lock().unwrap_or_else(|e| e.into_inner()) = Some(CommitBackup {
            new_epoch,
            state: zeroize::Zeroizing::new(state),
        });
    }

    fn commit_backup_epoch(&self) -> Option<u64> {
        self.commit_backup
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|b| b.new_epoch)
    }

    /// Commit 被服务端拒绝（同 epoch 有胜出者）时，回滚到提交前状态。
    fn rollback_commit_backup(&self) -> Result<(), ClientError> {
        let backup = self
            .commit_backup
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        let Some(backup) = backup else {
            return Ok(());
        };
        let session = MlsSession::from_recovery_snapshot(&backup.state)?;
        *self.mls.lock().unwrap_or_else(|e| e.into_inner()) = Some(session);
        // 回滚后的状态也必须落盘，否则重启会读回被回滚的 Commit。
        self.mark_main_dirty();
        Ok(())
    }

    fn discard_commit_backup(&self) {
        self.commit_backup
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
    }

    /// 暂存 epoch 有缺口的 Commit。返回是否已加入缓冲。
    fn buffer_pending_commit(&self, epoch: u64, payload: Vec<u8>) -> bool {
        const MAX_PENDING_COMMITS: usize = 32;
        let mut buf = self
            .pending_commits
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if buf.contains_key(&epoch) {
            return true;
        }
        if buf.len() >= MAX_PENDING_COMMITS {
            // 只保留最近的 epoch，避免恶意/异常帧把内存撑爆。
            if let Some(oldest) = buf.keys().next().copied()
                && oldest < epoch
            {
                buf.remove(&oldest);
            } else {
                return false;
            }
        }
        buf.insert(epoch, payload);
        true
    }

    fn take_pending_commit(&self, epoch: u64) -> Option<Vec<u8>> {
        self.pending_commits
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&epoch)
    }

    fn has_pending_commits(&self) -> bool {
        !self
            .pending_commits
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty()
    }

    /// 请求服务端补投 epoch > `current` 的 Commit。
    ///
    /// 同一 `current` 在 5 秒内只上报一次：服务端如果返回一份无法应用的
    /// 陈旧 Commit，失败路径不会再触发无限重拉。
    async fn request_epoch_backfill(&self, current: u64) {
        {
            let mut last = self
                .last_epoch_report
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if let Some((epoch, at)) = last.as_ref()
                && *epoch == current
                && at.elapsed() < std::time::Duration::from_secs(5)
            {
                return;
            }
            *last = Some((current, web_time::Instant::now()));
        }
        let _ = self.report_epoch(current).await;
    }

    /// 是否有 MLS snapshot 处于“持久化失败”状态。
    ///
    /// 为 true 时调用方必须暂停发送：group epoch / sender ratchet 已前进但
    /// 没有落盘，重启会回退到旧状态并复用密钥流。
    pub fn snapshot_broken(&self) -> bool {
        self.snapshot_broken.load(Ordering::Acquire)
    }

    /// 标记 snapshot 持久化失败并向 UI 报错（只在状态翻转时 emit 一次）。
    pub fn mark_snapshot_broken(&self, detail: &str) {
        if !self.snapshot_broken.swap(true, Ordering::AcqRel) {
            tracing::error!("[snapshot] persistence broken: {}", detail);
            self.emit(ClientEvent::MlsSyncError {
                server_id: self.server_id.clone(),
                user_message: "本地密钥状态保存失败；为避免 nonce 复用，已暂停发送。\
                               请检查磁盘/存储空间后重新登录"
                    .into(),
                recoverable: false,
                detail: detail.to_string(),
            });
        } else {
            tracing::warn!("[snapshot] persistence still broken: {}", detail);
        }
    }

    /// 一次完整落盘成功后清除失败状态，恢复发送能力。
    pub fn clear_snapshot_broken(&self) {
        if self.snapshot_broken.swap(false, Ordering::AcqRel) {
            tracing::info!("[snapshot] persistence recovered; sending re-enabled");
        }
    }

    /// 设置当前用户是否是 host。
    pub fn set_is_host(&self, is_host: bool) {
        self.is_host.store(is_host, Ordering::Release);
    }

    pub fn is_host(&self) -> bool {
        self.is_host.load(Ordering::Acquire)
    }

    /// 主群 epoch 变化时通知 UI（语音需要重建媒体会话）。
    fn emit_epoch_if_changed(&self, before: u64) {
        let after = self.mls_epoch();
        if after != before {
            self.emit(ClientEvent::MlsEpochChanged {
                server_id: self.server_id.clone(),
                epoch: after,
            });
        }
    }

    fn self_uid(&self) -> Result<String, ClientError> {
        let guard = self.mls.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .as_ref()
            .map(|s| s.uid().to_string())
            .ok_or_else(|| ClientError::Mls("mls not initialized".into()))
    }

    pub fn server_id(&self) -> &str {
        &self.server_id
    }

    pub fn state(&self) -> ClientState {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).state()
    }

    pub fn peers(&self) -> Vec<UserId> {
        self.peers.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn transition(&self, next: ClientState) -> Result<(), ClientError> {
        let mut m = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let from = m.state();
        m.try_transition(next)?;
        drop(m);
        if let Some(obs) = &self.observer {
            obs.on_event(ClientEvent::StateChanged {
                server_id: self.server_id.clone(),
                from,
                to: next,
            });
        }
        Ok(())
    }

    fn emit(&self, ev: ClientEvent) {
        if let Some(obs) = &self.observer {
            obs.on_event(ev);
        }
    }

    /// 主动 emit 一条事件到本地 observer（用于本地回显等场景）。
    pub fn emit_local_event(&self, event: ClientEvent) {
        if let Some(obs) = &self.observer {
            obs.on_event(event);
        }
    }

    // =================================================================
    // 网络生命周期
    // =================================================================

    /// 带 session token 的握手（登录后调用）。
    ///
    /// 生产服务端要求 Hello 必须携带 token；不带 token 的连接会被立即拒绝。
    pub async fn connect_with_token(
        &self,
        client_name: impl Into<String>,
        client_version: impl Into<String>,
        session_token: Option<String>,
    ) -> Result<(), ClientError> {
        self.transition(ClientState::Connecting)?;

        let hello = ClientMessage::ClientHello {
            protocol_version: kaede_protocol::PROTOCOL_VERSION,
            client_name: client_name.into(),
            client_version: client_version.into(),
            session_token,
        };
        self.transport.send(hello).await?;

        #[cfg(not(target_arch = "wasm32"))]
        let reply = tokio::time::timeout(std::time::Duration::from_secs(10), self.transport.recv())
            .await
            .map_err(|_| ClientError::Timeout)??;

        // TODO(wasm-timeout): 浏览器 WebSocket 有自己的连接超时机制，暂不包 timer。
        // 如需主动超时，用 gloo-timers + futures_util::select。
        #[cfg(target_arch = "wasm32")]
        let reply = self.transport.recv().await?;

        match reply {
            ServerMessage::ServerHello {
                protocol_version,
                server_id: _,
                server_name,
            } => {
                self.emit(ClientEvent::HelloAck {
                    server_id: self.server_id.clone(),
                    server_name: server_name.clone(),
                    protocol_version,
                });
                tracing::info!(
                    "[client:{}] hello ok, server={} v={}",
                    self.server_id,
                    server_name,
                    protocol_version
                );
                Ok(())
            }
            ServerMessage::Error { code, message } => {
                self.transition(ClientState::Disconnected)?;
                Err(ClientError::Server { code, message })
            }
            other => {
                self.transition(ClientState::Disconnected)?;
                Err(ClientError::Protocol(format!(
                    "expected ServerHello, got {:?}",
                    other
                )))
            }
        }
    }

    /// 步骤 2：认证。由上层完成 HTTP 登录后调用。
    pub fn authenticate(&self) -> Result<(), ClientError> {
        self.transition(ClientState::Authenticated)
    }

    /// 步骤 3：加入房间。
    ///
    /// **只发送，不接收回执。** 回执 `Joined` 由 [`Self::run_event_loop`]
    /// 统一处理，避免与事件循环抢同一个 WebSocket stream。
    ///
    /// 若调用方尚未启动 event_loop，需自己在启动后处理 `ClientEvent::Joined`。
    pub async fn join(&self) -> Result<(), ClientError> {
        match self.state() {
            ClientState::Authenticated | ClientState::MlsJoining | ClientState::MlsReady => {}
            _ => return Err(ClientError::NotAuthenticated),
        }
        self.transport.send(ClientMessage::Join).await?;

        // 直接进入 MlsJoining（如果还没到 MlsReady）。
        // 服务器返回的 Joined / PeerJoined 由 event_loop 派发。
        if self.state() == ClientState::Authenticated {
            self.transition(ClientState::MlsJoining)?;
        }
        Ok(())
    }

    /// 发送应用层心跳。服务端 idle timeout 只看入站消息，没有心跳会被 5 分钟断开。
    pub async fn send_ping(&self, nonce: u64) -> Result<(), ClientError> {
        self.transport.send(ClientMessage::Ping { nonce }).await
    }

    /// 设置在线状态。
    pub async fn set_presence(&self, status: &str) -> Result<(), ClientError> {
        self.transport
            .send(ClientMessage::SetPresence {
                status: status.into(),
            })
            .await
    }

    /// 发送「正在输入」指示（协议 v7）。纯元数据，服务端不落盘、不补投。
    pub async fn send_typing(&self, channel_id: &str) -> Result<(), ClientError> {
        self.transport
            .send(ClientMessage::Typing {
                channel_id: channel_id.into(),
            })
            .await
    }

    // =================================================================
    // 主动发送（用于 RoomSession 协调 MLS 建组）
    // =================================================================

    /// 定向发送 MLS Welcome。
    ///
    /// 帧格式：`[0x4b][gid_len:u16 BE][group_id][welcome]`。
    /// group_id 让接收方在“已有旧群”时能判断是否需要重建主群
    /// （见 [`Self::mls_join_or_reset_welcome`]）。老格式无 magic 时按裸 Welcome 处理。
    pub async fn send_welcome(&self, to: UserId, payload: Vec<u8>) -> Result<(), ClientError> {
        let mut framed = Vec::with_capacity(3 + payload.len() + 32);
        if let Some(gid) = self.mls_group_id() {
            framed.push(WELCOME_FRAME_MAGIC);
            framed.extend_from_slice(&(gid.len().min(u16::MAX as usize) as u16).to_be_bytes());
            framed.extend_from_slice(&gid[..gid.len().min(u16::MAX as usize)]);
        }
        framed.extend_from_slice(&payload);
        self.transport
            .send(ClientMessage::MlsWelcome {
                to,
                payload: framed,
            })
            .await
    }

    /// 广播 MLS Commit。`epoch` = commit 产生后的新 epoch（用于离线追赶）。
    pub async fn send_commit(&self, epoch: u64, payload: Vec<u8>) -> Result<(), ClientError> {
        self.transport
            .send(ClientMessage::MlsCommit { epoch, payload })
            .await
    }

    /// 上报自己当前的主群 epoch，让服务器补投离线期间错过的 Commit。
    pub async fn report_epoch(&self, epoch: u64) -> Result<(), ClientError> {
        self.transport
            .send(ClientMessage::ReportEpoch { epoch })
            .await
    }

    // =================================================================
    // 事件循环
    // =================================================================

    /// 持续从 transport 收消息并交给 [`handle_server_message`]。
    ///
    /// 调用方负责 spawn：native 用 `tokio::spawn`，wasm 用
    /// `wasm_bindgen_futures::spawn_local`。返回即连接关闭。
    pub async fn run_event_loop(self: Arc<Self>) {
        // 快照写盘做尾随去抖：每条消息都整包加密重写 MLS 状态代价很高
        // （群状态 + 最多 64 个 past epoch）。发送路径仍在广播前显式落盘，
        // 这里只负责接收侧/成员变更的最终持久化。
        let flush_interval = std::time::Duration::from_millis(250);
        let mut last_flush = web_time::Instant::now();
        let reason = loop {
            match self.transport.recv().await {
                Ok(msg) => {
                    if let Err(e) = self.handle_server_message(msg).await {
                        // fatal 错误（Kicked / UnsupportedVersion / Malformed）
                        // 会让 handle_server_message 返回 Err。
                        tracing::warn!("[client:{}] handle failed: {}", self.server_id, e);
                        break e.to_string();
                    }
                    if last_flush.elapsed() >= flush_interval {
                        self.flush_dirty_snapshots().await;
                        last_flush = web_time::Instant::now();
                    }
                }
                Err(ClientError::Closed) => {
                    tracing::info!("[client:{}] ws closed", self.server_id);
                    break "connection closed".to_string();
                }
                Err(e) => {
                    tracing::warn!("[client:{}] recv error: {}", self.server_id, e);
                    break e.to_string();
                }
            }
        };

        // 退出前补一次落盘，避免最后 250ms 内的成员变更丢失。
        self.flush_dirty_snapshots().await;

        // 主动关闭 transport（服务端可能还挂着，需要我们这端发 Close 帧）
        let _ = self.transport.close().await;

        // 状态置 Disconnected（忽略非法转移——比如已经被 close() 过）
        let _ = self.transition(ClientState::Disconnected);

        // 通知 UI
        self.emit(ClientEvent::Disconnected {
            server_id: self.server_id.clone(),
            reason,
        });
    }

    /// 关闭连接。
    pub async fn close(&self) -> Result<(), ClientError> {
        self.transport.close().await?;
        let _ = self.transition(ClientState::Disconnected);
        Ok(())
    }
}

/// Welcome 帧 magic（见 [`ServerConnection::send_welcome`]）。
const WELCOME_FRAME_MAGIC: u8 = 0x4b;

/// 拆出 Welcome 帧里的 group_id 与裸 Welcome。
///
/// 老格式（直接是 MLS Welcome）返回 `(None, payload)`。
fn unpack_welcome_payload(payload: &[u8]) -> (Option<&[u8]>, &[u8]) {
    if payload.len() >= 3 && payload[0] == WELCOME_FRAME_MAGIC {
        let gid_len = u16::from_be_bytes([payload[1], payload[2]]) as usize;
        if payload.len() >= 3 + gid_len {
            return (Some(&payload[3..3 + gid_len]), &payload[3 + gid_len..]);
        }
    }
    (None, payload)
}
