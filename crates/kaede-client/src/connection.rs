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

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use kaede_protocol::{ClientMessage, ServerMessage, UserId};

use crate::ClientObserver;
use crate::error::ClientError;
use crate::event::ClientEvent;
use crate::mls_session::MlsSession;
use crate::state::{ClientState, ClientStateMachine};
use crate::transport::Transport;

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
    /// 当前用户是否是主群 host（op5）。由 RoomSession 注入。
    is_host: AtomicBool,
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
            is_host: AtomicBool::new(false),
        }
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

    /// 保存失败时把标志放回去，下一条消息会再次尝试。
    pub fn restore_main_dirty(&self) {
        self.main_dirty.store(true, Ordering::Release);
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

    pub fn server_id(&self) -> &str {
        &self.server_id
    }

    pub fn state(&self) -> ClientState {
        self.state.lock().unwrap().state()
    }

    pub fn peers(&self) -> Vec<UserId> {
        self.peers.lock().unwrap().clone()
    }

    fn transition(&self, next: ClientState) -> Result<(), ClientError> {
        let mut m = self.state.lock().unwrap();
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

    // =================================================================
    // 网络生命周期
    // =================================================================

    /// 步骤 1：连接并握手（无 token）。
    pub async fn connect(
        &self,
        client_name: impl Into<String>,
        client_version: impl Into<String>,
    ) -> Result<(), ClientError> {
        self.connect_with_token(client_name, client_version, None)
            .await
    }

    /// 带 session token 的握手（登录后调用）。
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

    // =================================================================
    // MLS 生命周期
    // =================================================================

    /// 初始化 MLS 会话（生成签名身份）。每个连接一次。
    pub fn mls_init(&self, uid: impl Into<UserId>) -> Result<(), ClientError> {
        let mut guard = self.mls.lock().unwrap();
        if guard.is_some() {
            return Err(ClientError::Mls("mls already initialized".into()));
        }
        *guard = Some(MlsSession::new(uid)?);
        self.mark_main_dirty();
        Ok(())
    }

    /// 房主建群。成功后状态 → `MlsReady`。
    ///
    /// 若当前处于 `Authenticated`，自动补一次 `MlsJoining` 转场，
    /// 调用方无需关心 join_room 的先后顺序。
    pub fn mls_create_group(&self) -> Result<(), ClientError> {
        {
            let mut guard = self.mls.lock().unwrap();
            let s = guard
                .as_mut()
                .ok_or_else(|| ClientError::Mls("mls not initialized".into()))?;
            s.create_group()?;
        }
        self.mark_main_dirty();
        if self.state() == ClientState::Authenticated {
            let _ = self.transition(ClientState::MlsJoining);
        }
        self.transition(ClientState::MlsReady)?;
        Ok(())
    }

    /// 用 Welcome 加入群组。成功后状态 → `MlsReady`。
    ///
    /// 若当前处于 `Authenticated`，自动补一次 `MlsJoining` 转场。
    pub fn mls_join_with_welcome(&self, welcome: &[u8]) -> Result<(), ClientError> {
        {
            let mut guard = self.mls.lock().unwrap();
            let s = guard
                .as_mut()
                .ok_or_else(|| ClientError::Mls("mls not initialized".into()))?;
            s.join_with_welcome(welcome)?;
        }
        self.mark_main_dirty();
        if self.state() == ClientState::Authenticated {
            let _ = self.transition(ClientState::MlsJoining);
        }
        self.transition(ClientState::MlsReady)?;
        Ok(())
    }

    /// 处理收到的 Welcome；若本地已有**不同**的主群，则重建为新群。
    ///
    /// `expected_group_id` 由发送方在 Welcome 帧里附带（`send_welcome` 自动加）。
    /// 老客户端/老路径没有该字段时传 `None`，此时已就绪方保守忽略。
    ///
    /// 返回：
    /// - `Ok(true)`  已加入（首次或重建）
    /// - `Ok(false)` 重复 Welcome，无需处理
    pub fn mls_join_or_reset_welcome(
        &self,
        welcome: &[u8],
        expected_group_id: Option<&[u8]>,
    ) -> Result<bool, ClientError> {
        let mut guard = self.mls.lock().unwrap();
        let s = guard
            .as_mut()
            .ok_or_else(|| ClientError::Mls("mls not initialized".into()))?;

        if !s.is_ready() {
            s.join_with_welcome(welcome)?;
            self.mark_main_dirty();
            return Ok(true);
        }

        match expected_group_id {
            Some(gid) if Some(gid) == s.group_id() => Ok(false),
            Some(_) => {
                s.reset_and_join(welcome)?;
                self.mark_main_dirty();
                Ok(true)
            }
            None => {
                tracing::warn!(
                    "[client] welcome without group_id while already ready; ignoring (legacy)"
                );
                Ok(false)
            }
        }
    }

    /// 生成自己的 KeyPackage（供别人 Add）。
    pub fn mls_generate_key_package(&self) -> Result<Vec<u8>, ClientError> {
        let guard = self.mls.lock().unwrap();
        let s = guard
            .as_ref()
            .ok_or_else(|| ClientError::Mls("mls not initialized".into()))?;
        s.generate_key_package()
    }

    /// 房主 Add 新成员。
    ///
    /// **调用方负责发送返回的 `(commit, welcome)`**：
    /// - commit → `transport.send(ClientMessage::MlsCommit { payload })`（广播）
    /// - welcome → `transport.send(ClientMessage::MlsWelcome { to, payload })`（定向）
    pub fn mls_add_member(&self, kp: &[u8]) -> Result<(Vec<u8>, Vec<u8>), ClientError> {
        let before = self.mls_epoch();
        let out = {
            let mut guard = self.mls.lock().unwrap();
            let s = guard
                .as_mut()
                .ok_or_else(|| ClientError::Mls("mls not initialized".into()))?;
            s.add_member(kp)?
        };
        self.mark_main_dirty();
        self.emit_epoch_if_changed(before);
        Ok(out)
    }

    /// 加密并广播一条应用消息。返回密文（供上层持久化）。
    pub async fn mls_send_app(&self, plaintext: &[u8]) -> Result<Vec<u8>, ClientError> {
        let ciphertext = {
            let mut guard = self.mls.lock().unwrap();
            let s = guard
                .as_mut()
                .ok_or_else(|| ClientError::Mls("mls not initialized".into()))?;
            s.encrypt(plaintext)?
        };
        // 发送会推进 sender ratchet：必须尽快落盘，否则重启后 nonce 复用。
        self.mark_main_dirty();
        let ct_for_send = ciphertext.clone();
        self.transport
            .send(ClientMessage::MlsAppMessage {
                payload: ct_for_send,
            })
            .await?;
        Ok(ciphertext)
    }

    /// MLS 解密应用消息。供上层拉历史时使用。
    pub fn mls_decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, ClientError> {
        let mut guard = self.mls.lock().unwrap();
        let s = guard
            .as_mut()
            .ok_or_else(|| ClientError::Mls("mls not initialized".into()))?;
        let out = s.decrypt(ciphertext)?;
        self.mark_main_dirty();
        Ok(out)
    }
    // =====================================================================
    // DM（B3 方案：per-conv AEAD）
    // =====================================================================

    /// 从 MLS exporter 派生指定 DM 会话的 32 字节对称密钥。
    ///
    /// exporter context = `conv_id || 0x00 || "dm"`（见 SframePurpose::Dm）。
    /// 必须在 MLS ready 后调用。
    pub fn dm_derive_key(&self, conv_id: &str) -> Result<[u8; 32], ClientError> {
        let conv_id = &canonical_conv_id(conv_id);
        let map = self.dm_groups.lock().unwrap();
        let s = map
            .get(conv_id)
            .ok_or_else(|| ClientError::Mls(format!("DM group not found for {}", conv_id)))?;
        s.export_dm_key(conv_id)
    }

    /// B2：本地是否已持有该 conv 对应的独立 2 人 MLS group。
    pub fn dm_has_group(&self, conv_id: &str) -> bool {
        self.dm_groups
            .lock()
            .unwrap()
            .contains_key(&canonical_conv_id(conv_id))
    }

    /// B2 Phase 2：从 dm group 派生 LiveKit E2EE 媒体 key。
    ///
    /// seed 来自 server token（房间级），目的是让 DM 双方对齐 key；
    /// 非 DM 成员即使拿到 seed，没有 dm group state 也算不出 key。
    pub fn dm_export_media_key(&self, conv_id: &str, seed: &[u8]) -> Result<[u8; 32], ClientError> {
        let conv_id = &canonical_conv_id(conv_id);
        let map = self.dm_groups.lock().unwrap();
        let s = map
            .get(conv_id)
            .ok_or_else(|| ClientError::Mls(format!("dm group not found: {}", conv_id)))?;
        s.export_sframe_key_for_with_extra(conv_id, kaede_mls::SframePurpose::LivekitE2ee, seed)
    }

    /// B2：删除该 conv 对应的 DM group（Welcome 发送失败时回滚）。
    pub fn dm_remove_group(&self, conv_id: &str) {
        self.dm_groups
            .lock()
            .unwrap()
            .remove(&canonical_conv_id(conv_id));
    }

    /// 私密频道 group 的当前 epoch；无 group 时返回 None。
    ///
    /// 用途：`ChannelSync 0x05` 处理完 commit 后 emit `ChannelEpochChanged` 用。
    pub fn channel_epoch(&self, channel_id: &str) -> Option<u64> {
        let map = self.channel_groups.lock().unwrap();
        map.get(channel_id).map(|s| s.epoch())
    }

    /// 频道是否有独立 MLS group（私密频道）。
    pub fn channel_has_group(&self, channel_id: &str) -> bool {
        self.channel_groups.lock().unwrap().contains_key(channel_id)
    }

    /// 从 channel group 按 purpose 派生密钥（P0：附件加密用 File）。
    pub fn channel_export_key_for(
        &self,
        channel_id: &str,
        purpose: kaede_mls::SframePurpose,
    ) -> Result<[u8; 32], ClientError> {
        let map = self.channel_groups.lock().unwrap();
        let s = map
            .get(channel_id)
            .ok_or_else(|| ClientError::Mls(format!("channel group not found: {}", channel_id)))?;
        s.export_sframe_key_for(channel_id, purpose)
    }

    /// 从 dm group 按 purpose 派生密钥（P0：附件加密用 File）。
    pub fn dm_export_key_for(
        &self,
        conv_id: &str,
        purpose: kaede_mls::SframePurpose,
    ) -> Result<[u8; 32], ClientError> {
        let conv_id = &canonical_conv_id(conv_id);
        let map = self.dm_groups.lock().unwrap();
        let s = map
            .get(conv_id)
            .ok_or_else(|| ClientError::Mls(format!("dm group not found: {}", conv_id)))?;
        s.export_sframe_key_for(conv_id, purpose)
    }

    /// 为私密频道建立独立 MLS group（仅含当前用户）。
    /// MVP：不引入其他成员；多人 Welcome 下一轮做。
    pub fn channel_ensure_group(&self, channel_id: &str) -> Result<(), ClientError> {
        if self.channel_has_group(channel_id) {
            return Ok(());
        }
        // 先拿 mls，再拿 channel_groups（统一加锁顺序）
        let main_guard = self.mls.lock().unwrap();
        let main_session = main_guard
            .as_ref()
            .ok_or_else(|| ClientError::Mls("no main mls session".into()))?;
        let mut ch_session = main_session.fork_for_new_group();
        ch_session.create_group()?;
        drop(main_guard);
        self.channel_groups
            .lock()
            .unwrap()
            .insert(channel_id.to_string(), ch_session);
        self.mark_channel_dirty(channel_id);
        Ok(())
    }

    /// B2：注册等待 Welcome 的 receiver。Bob join_channel 前先调这个。
    pub fn channel_register_pending(
        &self,
        channel_id: &str,
    ) -> async_channel::Receiver<Result<Vec<u8>, String>> {
        let (tx, rx) = async_channel::bounded(1);
        self.channel_pending_welcomes
            .lock()
            .unwrap()
            .insert(channel_id.to_string(), tx);
        rx
    }

    /// 清除某个频道的“等待 Welcome”槽（超时 / 失败时调用）。
    pub fn clear_pending_welcome(&self, channel_id: &str) {
        self.channel_pending_welcomes
            .lock()
            .unwrap()
            .remove(channel_id);
    }

    /// B2：把 kp 加入 channel group，返回 (commit, welcome)。
    pub fn channel_add_member(
        &self,
        channel_id: &str,
        kp: &[u8],
    ) -> Result<(Vec<u8>, Vec<u8>), ClientError> {
        let mut map = self.channel_groups.lock().unwrap();
        let s = map
            .get_mut(channel_id)
            .ok_or_else(|| ClientError::Mls(format!("channel group not found: {}", channel_id)))?;
        let out = s.add_member(kp)?;
        drop(map);
        self.mark_channel_dirty(channel_id);
        Ok(out)
    }

    /// B2：处理 Welcome 加入 channel group。
    pub fn channel_join_via_welcome(
        &self,
        channel_id: &str,
        welcome: &[u8],
    ) -> Result<(), ClientError> {
        if self.channel_has_group(channel_id) {
            return Err(ClientError::Mls("channel group already exists".into()));
        }
        let main_guard = self.mls.lock().unwrap();
        let main_session = main_guard
            .as_ref()
            .ok_or_else(|| ClientError::Mls("no main mls session".into()))?;
        let mut ch_session = main_session.fork_for_new_group();
        ch_session.join_with_welcome(welcome)?;
        drop(main_guard);
        self.channel_groups
            .lock()
            .unwrap()
            .insert(channel_id.to_string(), ch_session);
        self.mark_channel_dirty(channel_id);
        Ok(())
    }

    /// B2：处理 Commit 更新 channel group state。
    pub fn channel_process_commit(
        &self,
        channel_id: &str,
        commit: &[u8],
    ) -> Result<bool, ClientError> {
        let mut map = self.channel_groups.lock().unwrap();
        let s = map
            .get_mut(channel_id)
            .ok_or_else(|| ClientError::Mls(format!("channel group not found: {}", channel_id)))?;
        let out = s.process_handshake(commit)?;
        drop(map);
        self.mark_channel_dirty(channel_id);
        Ok(out)
    }

    /// 从 channel group 移除某个成员，返回 Commit。
    pub fn channel_remove_member(
        &self,
        channel_id: &str,
        uid: &str,
    ) -> Result<Vec<u8>, ClientError> {
        let mut map = self.channel_groups.lock().unwrap();
        let s = map
            .get_mut(channel_id)
            .ok_or_else(|| ClientError::Mls(format!("channel group not found: {}", channel_id)))?;
        let out = s.remove_member_by_uid(uid)?;
        drop(map);
        self.mark_channel_dirty(channel_id);
        Ok(out)
    }

    /// 本地丢弃某个频道的 group（离开频道时调用）。
    pub fn channel_drop_group(&self, channel_id: &str) {
        if self
            .channel_groups
            .lock()
            .unwrap()
            .remove(channel_id)
            .is_some()
        {
            self.dirty_channel_groups.lock().unwrap().remove(channel_id);
        }
    }

    /// 从频道独立 group 导出 SFrame key。
    pub fn channel_export_key(&self, channel_id: &str) -> Result<[u8; 32], ClientError> {
        let map = self.channel_groups.lock().unwrap();
        let s = map
            .get(channel_id)
            .ok_or_else(|| ClientError::Mls(format!("channel group not found: {}", channel_id)))?;
        s.export_dm_key(channel_id)
    }

    /// B2 Phase 1：从 channel group 派生 LiveKit E2EE 媒体 key。
    ///
    /// seed 来自 server token（房间级），目的是让同房间成员对齐 key；
    /// 非成员即使拿到 seed，没有 channel group state 也算不出 key。
    pub fn channel_export_media_key(
        &self,
        channel_id: &str,
        seed: &[u8],
    ) -> Result<[u8; 32], ClientError> {
        let map = self.channel_groups.lock().unwrap();
        let s = map
            .get(channel_id)
            .ok_or_else(|| ClientError::Mls(format!("channel group not found: {}", channel_id)))?;
        s.export_sframe_key_for_with_extra(channel_id, kaede_mls::SframePurpose::LivekitE2ee, seed)
    }

    /// 加密一条 DM 消息，返回 sealed 密文（可直接 POST 到 server）。
    /// 发起方：创建 2 人 DM group 并返回 `(Welcome, generation)`。
    pub fn dm_init_group(
        &self,
        conv_id: &str,
        peer_kp: &[u8],
    ) -> Result<(Vec<u8>, u64), ClientError> {
        let conv_id = &canonical_conv_id(conv_id);
        let main_guard = self.mls.lock().unwrap();
        let main_session = main_guard
            .as_ref()
            .ok_or_else(|| ClientError::Mls("no main mls session".into()))?;
        let mut dm_session = main_session.fork_for_new_group();
        dm_session.create_group()?;
        let (_commit, welcome) = dm_session.add_member(peer_kp)?;
        let mut map = self.dm_groups.lock().unwrap();
        if map.contains_key(conv_id) {
            // 双方同时发起时，另一个 Welcome 可能已经先到并落库。
            // 不要覆盖已有 group，交给 dm_join_group 的确定性仲裁。
            return Err(ClientError::Mls(format!(
                "DM group already exists for {}",
                conv_id
            )));
        }
        map.insert(conv_id.to_string(), dm_session);
        drop(map);
        let generation = self.dm_generation(conv_id) + 1;
        self.set_dm_generation(conv_id, generation);
        self.mark_dm_dirty(conv_id);
        Ok((welcome, generation))
    }

    /// 当前 DM 会话的代际（本地重建次数）。未知会话为 0。
    pub fn dm_generation(&self, conv_id: &str) -> u64 {
        let conv_id = canonical_conv_id(conv_id);
        self.dm_generations
            .lock()
            .unwrap()
            .get(&conv_id)
            .copied()
            .unwrap_or(0)
    }

    fn set_dm_generation(&self, conv_id: &str, generation: u64) {
        let conv_id = canonical_conv_id(conv_id);
        self.dm_generations
            .lock()
            .unwrap()
            .insert(conv_id.clone(), generation);
        if self.aux_storage.lock().unwrap().is_some() {
            self.dirty_dm_meta.lock().unwrap().insert(conv_id);
        }
    }

    /// 接收方：处理 Welcome 消息
    ///
    /// # 代际仲裁
    ///
    /// 每次本地重建 DM group，generation +1 并随 Welcome 发送。
    /// 接收方规则：
    /// 1. `generation > local` → 采纳新群（单侧丢失状态后的恢复路径）；
    /// 2. `generation < local` → 忽略（旧 Welcome / 历史重放）；
    /// 3. 相等 → 双方并发首次建群，按 group_id 字典序最小者胜出。
    pub fn dm_join_group(
        &self,
        conv_id: &str,
        welcome: &[u8],
        generation: u64,
    ) -> Result<(), ClientError> {
        let conv_id = &canonical_conv_id(conv_id);
        let local_gen = self.dm_generation(conv_id);
        if generation < local_gen {
            tracing::info!(
                "[dm] ignore stale welcome for {} (gen {} < {})",
                conv_id,
                generation,
                local_gen
            );
            return Ok(());
        }

        let main_guard = self.mls.lock().unwrap();
        let main_session = main_guard
            .as_ref()
            .ok_or_else(|| ClientError::Mls("no main mls session".into()))?;
        let mut dm_session = main_session.fork_for_new_group();
        dm_session.join_with_welcome(welcome)?;
        let new_gid = dm_session.group_id().map(|g| g.to_vec());
        drop(main_guard);

        let mut map = self.dm_groups.lock().unwrap();
        if generation == local_gen
            && let Some(existing) = map.get(conv_id)
            && let (Some(old), Some(new)) =
                (existing.group_id().map(|g| g.to_vec()), new_gid.as_ref())
            && new.as_slice() >= old.as_slice()
        {
            // 保留字典序更小的已有 group；丢弃刚 join 的这份。
            tracing::info!(
                "[dm] concurrent initiator detected for {}; keeping group {}",
                conv_id,
                String::from_utf8_lossy(&old)
            );
            return Ok(());
        }
        map.insert(conv_id.to_string(), dm_session);
        drop(map);
        if generation > local_gen {
            self.set_dm_generation(conv_id, generation);
            self.emit_local_event(ClientEvent::MlsSyncError {
                server_id: self.server_id.clone(),
                user_message: format!("私聊 {} 的对端已重建会话，本地旧消息不可恢复", conv_id),
                recoverable: true,
                detail: format!("dm generation {} -> {}", local_gen, generation),
            });
        }
        self.mark_dm_dirty(conv_id);
        Ok(())
    }
    /// B3+: 注入 aux storage hook。由 RoomSession 在构造后调用。
    pub fn set_aux_storage(&self, hook: AuxStorageHook) {
        *self.aux_storage.lock().unwrap() = Some(hook);
    }

    fn mark_dm_dirty(&self, conv_id: &str) {
        if self.aux_storage.lock().unwrap().is_some() {
            self.dirty_dm_groups
                .lock()
                .unwrap()
                .insert(conv_id.to_string());
        }
    }

    fn mark_channel_dirty(&self, channel_id: &str) {
        if self.aux_storage.lock().unwrap().is_some() {
            self.dirty_channel_groups
                .lock()
                .unwrap()
                .insert(channel_id.to_string());
        }
    }

    /// B3+: 序列化 + 加密某个 DM group 的 snapshot。
    pub fn dm_snapshot_with_cipher(
        &self,
        conv_id: &str,
        cipher: &dyn kaede_mls::SnapshotCipher,
    ) -> Result<Vec<u8>, ClientError> {
        let conv_id = &canonical_conv_id(conv_id);
        let map = self.dm_groups.lock().unwrap();
        let s = map
            .get(conv_id)
            .ok_or_else(|| ClientError::Mls(format!("DM group not found for {}", conv_id)))?;
        s.snapshot_with(cipher)
    }

    /// B3+: 从 snapshot 恢复某个 DM group。覆盖已有的同名 group。
    pub fn dm_restore_from_snapshot_with_cipher(
        &self,
        conv_id: &str,
        snapshot: &[u8],
        cipher: Option<&dyn kaede_mls::SnapshotCipher>,
    ) -> Result<(), ClientError> {
        let conv_id = &canonical_conv_id(conv_id);
        let session = MlsSession::from_snapshot_with(snapshot, cipher)?;
        self.dm_groups
            .lock()
            .unwrap()
            .insert(conv_id.to_string(), session);
        Ok(())
    }

    /// B3+: 序列化 + 加密某个 channel group 的 snapshot。
    pub fn channel_snapshot_with_cipher(
        &self,
        channel_id: &str,
        cipher: &dyn kaede_mls::SnapshotCipher,
    ) -> Result<Vec<u8>, ClientError> {
        let map = self.channel_groups.lock().unwrap();
        let s = map
            .get(channel_id)
            .ok_or_else(|| ClientError::Mls(format!("channel group not found: {}", channel_id)))?;
        s.snapshot_with(cipher)
    }

    /// B3+: 从 snapshot 恢复某个 channel group。覆盖已有的同名 group。
    pub fn channel_restore_from_snapshot_with_cipher(
        &self,
        channel_id: &str,
        snapshot: &[u8],
        cipher: Option<&dyn kaede_mls::SnapshotCipher>,
    ) -> Result<(), ClientError> {
        let session = MlsSession::from_snapshot_with(snapshot, cipher)?;
        self.channel_groups
            .lock()
            .unwrap()
            .insert(channel_id.to_string(), session);
        Ok(())
    }

    /// B3+: 把所有 dirty snapshot flush 到 storage。没有 hook 时 no-op。
    pub async fn flush_dirty_snapshots(&self) {
        let hook = match self.aux_storage.lock().unwrap().clone() {
            Some(h) => h,
            None => return,
        };
        let cipher = crate::keystore::device_cipher(&hook.uid, &hook.server_url);

        // 主群状态优先落盘：发送/接收消息都会推进 ratchet，
        // 不保存会导致重启后 generation/nonce 复用。
        if self.take_main_dirty() {
            match self.mls_snapshot_with_cipher(cipher.as_ref()) {
                Ok(snap) => {
                    let key = format!("{}/{}", hook.server_url, hook.uid);
                    match hook.store.put("mls", &key, &snap).await {
                        Ok(()) => {}
                        Err(e) => {
                            tracing::warn!("[main-persist] put failed: {} (will retry)", e);
                            self.restore_main_dirty();
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("[main-persist] snapshot failed: {}", e);
                    self.restore_main_dirty();
                }
            }
        }

        let dm_ids: Vec<String> = self
            .dirty_dm_groups
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .collect();
        for conv_id in dm_ids {
            match self.dm_snapshot_with_cipher(&conv_id, cipher.as_ref()) {
                Ok(snap) => {
                    let key = format!("{}/{}", hook.server_url, conv_id);
                    match hook.store.put("dm", &key, &snap).await {
                        Ok(()) => {
                            self.dirty_dm_groups.lock().unwrap().remove(&conv_id);
                        }
                        Err(e) => {
                            tracing::warn!(
                                "[aux-persist] dm {} put failed: {} (will retry)",
                                conv_id,
                                e
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "[aux-persist] dm {} snapshot failed: {} (dropping dirty)",
                        conv_id,
                        e
                    );
                    self.dirty_dm_groups.lock().unwrap().remove(&conv_id);
                }
            }
        }

        let ch_ids: Vec<String> = self
            .dirty_channel_groups
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .collect();
        for channel_id in ch_ids {
            match self.channel_snapshot_with_cipher(&channel_id, cipher.as_ref()) {
                Ok(snap) => {
                    let key = format!("{}/{}", hook.server_url, channel_id);
                    match hook.store.put("channel", &key, &snap).await {
                        Ok(()) => {
                            self.dirty_channel_groups
                                .lock()
                                .unwrap()
                                .remove(&channel_id);
                        }
                        Err(e) => {
                            tracing::warn!(
                                "[aux-persist] channel {} put failed: {} (will retry)",
                                channel_id,
                                e
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "[aux-persist] channel {} snapshot failed: {} (dropping dirty)",
                        channel_id,
                        e
                    );
                    self.dirty_channel_groups
                        .lock()
                        .unwrap()
                        .remove(&channel_id);
                }
            }
        }

        // DM 代际元数据：用于单侧重建后的仲裁与崩溃恢复。
        let meta_ids: Vec<String> = self.dirty_dm_meta.lock().unwrap().iter().cloned().collect();
        for conv_id in meta_ids {
            let generation = self.dm_generation(&conv_id);
            let key = format!("{}/{}", hook.server_url, conv_id);
            match hook
                .store
                .put("dm_meta", &key, &generation.to_be_bytes())
                .await
            {
                Ok(()) => {
                    self.dirty_dm_meta.lock().unwrap().remove(&conv_id);
                }
                Err(e) => {
                    tracing::warn!(
                        "[aux-persist] dm_meta {} put failed: {} (will retry)",
                        conv_id,
                        e
                    );
                }
            }
        }
    }

    /// B3+: 从 storage 恢复所有 DM / 私密频道 snapshot。
    ///
    /// 返回需要 emit 到 UI 的 `MlsSyncError` 列表。恢复失败的 group 不会进 map，
    /// 下次 `dm_ensure_group` / `channel_ensure_group` 会重建。
    pub async fn load_aux_snapshots_from_storage(&self, hook: &AuxStorageHook) -> Vec<ClientEvent> {
        let mut errors = Vec::new();
        let prefix = format!("{}/", hook.server_url);

        // 先恢复 DM 代际（必须在 group 恢复前/后都可以，独立命名空间）。
        if let Ok(keys) = hook.store.list_keys("dm_meta").await {
            for key in keys {
                let Some(conv_id) = key.strip_prefix(&prefix) else {
                    continue;
                };
                if let Ok(Some(bytes)) = hook.store.get("dm_meta", &key).await
                    && bytes.len() == 8
                {
                    let mut buf = [0u8; 8];
                    buf.copy_from_slice(&bytes);
                    let generation = u64::from_be_bytes(buf);
                    self.dm_generations
                        .lock()
                        .unwrap()
                        .insert(conv_id.to_string(), generation);
                }
            }
        }

        let dm_keys = match hook.store.list_keys("dm").await {
            Ok(k) => k,
            Err(e) => {
                tracing::warn!("[aux-restore] list dm keys failed: {}", e);
                Vec::new()
            }
        };
        for key in dm_keys {
            let Some(conv_id) = key.strip_prefix(&prefix) else {
                continue;
            };
            let conv_id = conv_id.to_string();
            let snap = match hook.store.get("dm", &key).await {
                Ok(Some(s)) => s,
                Ok(None) => continue,
                Err(e) => {
                    tracing::warn!("[aux-restore] get dm {} failed: {}", key, e);
                    continue;
                }
            };
            let cipher =
                crate::keystore::device_cipher_for_snapshot(&hook.uid, &hook.server_url, &snap);
            match self.dm_restore_from_snapshot_with_cipher(&conv_id, &snap, Some(cipher.as_ref()))
            {
                Ok(()) => {
                    tracing::info!("[aux-restore] dm {} restored", conv_id);
                }
                Err(ClientError::SnapshotIncompatible { alg }) => {
                    tracing::error!(
                        "[aux-restore] dm {} snapshot incompatible (alg={:#04x})",
                        conv_id,
                        alg
                    );
                    errors.push(ClientEvent::MlsSyncError {
                        server_id: self.server_id.clone(),
                        user_message: format!(
                            "DM 会话 {} 状态因平台不兼容丢失，请让对方重新发送一条消息",
                            conv_id
                        ),
                        recoverable: true,
                        detail: format!("SnapshotIncompatible alg={:#04x}", alg),
                    });
                }
                Err(e) => {
                    tracing::warn!("[aux-restore] dm {} restore failed: {}", conv_id, e);
                    errors.push(ClientEvent::MlsSyncError {
                        server_id: self.server_id.clone(),
                        user_message: format!(
                            "DM 会话 {} 状态恢复失败，请让对方重新发送一条消息",
                            conv_id
                        ),
                        recoverable: true,
                        detail: e.to_string(),
                    });
                }
            }
        }

        let ch_keys = match hook.store.list_keys("channel").await {
            Ok(k) => k,
            Err(e) => {
                tracing::warn!("[aux-restore] list channel keys failed: {}", e);
                Vec::new()
            }
        };
        for key in ch_keys {
            let Some(channel_id) = key.strip_prefix(&prefix) else {
                continue;
            };
            let channel_id = channel_id.to_string();
            let snap = match hook.store.get("channel", &key).await {
                Ok(Some(s)) => s,
                Ok(None) => continue,
                Err(e) => {
                    tracing::warn!("[aux-restore] get channel {} failed: {}", key, e);
                    continue;
                }
            };
            let cipher =
                crate::keystore::device_cipher_for_snapshot(&hook.uid, &hook.server_url, &snap);
            match self.channel_restore_from_snapshot_with_cipher(
                &channel_id,
                &snap,
                Some(cipher.as_ref()),
            ) {
                Ok(()) => {
                    tracing::info!("[aux-restore] channel {} restored", channel_id);
                }
                Err(ClientError::SnapshotIncompatible { alg }) => {
                    tracing::error!(
                        "[aux-restore] channel {} snapshot incompatible (alg={:#04x})",
                        channel_id,
                        alg
                    );
                    errors.push(ClientEvent::MlsSyncError {
                        server_id: self.server_id.clone(),
                        user_message: format!(
                            "私密频道 {} 状态因平台不兼容丢失，请重新加入",
                            channel_id
                        ),
                        recoverable: true,
                        detail: format!("SnapshotIncompatible alg={:#04x}", alg),
                    });
                }
                Err(e) => {
                    tracing::warn!("[aux-restore] channel {} restore failed: {}", channel_id, e);
                    errors.push(ClientEvent::MlsSyncError {
                        server_id: self.server_id.clone(),
                        user_message: format!("私密频道 {} 状态恢复失败，请重新加入", channel_id),
                        recoverable: true,
                        detail: e.to_string(),
                    });
                }
            }
        }

        errors
    }

    pub fn dm_seal(&self, conv_id: &str, plaintext: &[u8]) -> Result<Vec<u8>, ClientError> {
        let conv_id = &canonical_conv_id(conv_id);
        let key = self.dm_derive_key(conv_id)?;
        kaede_crypto::dm_cipher::seal(&key, conv_id, plaintext)
            .map_err(|e| ClientError::Protocol(format!("dm seal: {e}")))
    }

    /// 解密一条 DM 消息。
    pub fn dm_open(&self, conv_id: &str, ciphertext: &[u8]) -> Result<Vec<u8>, ClientError> {
        let conv_id = &canonical_conv_id(conv_id);
        let key = self.dm_derive_key(conv_id)?;
        kaede_crypto::dm_cipher::open(&key, conv_id, ciphertext)
            .map_err(|e| ClientError::Protocol(format!("dm open: {e}")))
    }

    /// 主动 emit 一条事件到本地 observer（用于本地回显等场景）。
    pub fn emit_local_event(&self, event: ClientEvent) {
        if let Some(obs) = &self.observer {
            obs.on_event(event);
        }
    }

    /// 移除一个成员。返回 commit bytes。
    pub fn mls_remove_member(
        &self,
        leaf: openmls::prelude::LeafNodeIndex,
    ) -> Result<Vec<u8>, ClientError> {
        let before = self.mls_epoch();
        let out = {
            let mut guard = self.mls.lock().unwrap();
            let s = guard
                .as_mut()
                .ok_or_else(|| ClientError::Mls("mls not initialized".into()))?;
            s.remove_member(leaf)?
        };
        self.mark_main_dirty();
        self.emit_epoch_if_changed(before);
        Ok(out)
    }

    /// 按 uid 移除成员。返回 commit bytes。
    ///
    /// **调用方负责广播 commit**（`Self::send_commit`）。
    pub fn mls_remove_member_by_uid(&self, uid: &str) -> Result<Vec<u8>, ClientError> {
        let before = self.mls_epoch();
        let out = {
            let mut guard = self.mls.lock().unwrap();
            let s = guard
                .as_mut()
                .ok_or_else(|| ClientError::Mls("mls not initialized".into()))?;
            s.remove_member_by_uid(uid)?
        };
        self.mark_main_dirty();
        self.emit_epoch_if_changed(before);
        Ok(out)
    }
    /// MLS 自更新。返回 commit bytes。
    pub fn mls_self_update(&self) -> Result<Vec<u8>, ClientError> {
        let before = self.mls_epoch();
        let out = {
            let mut guard = self.mls.lock().unwrap();
            let s = guard
                .as_mut()
                .ok_or_else(|| ClientError::Mls("mls not initialized".into()))?;
            s.self_update()?
        };
        self.mark_main_dirty();
        self.emit_epoch_if_changed(before);
        Ok(out)
    }

    /// 从 MLS exporter 派生 channel 的 SFrame 密钥（旧 API，等价于 Default purpose）。
    pub fn mls_export_sframe_key(&self, channel_id: &str) -> Result<[u8; 32], ClientError> {
        self.mls_export_sframe_key_for(channel_id, kaede_mls::SframePurpose::Default)
    }

    /// 派生 LiveKit E2EE 密钥（从 MLS exporter）。
    pub fn mls_export_livekit_e2ee_key(&self, channel_id: &str) -> Result<[u8; 32], ClientError> {
        self.mls_export_sframe_key_for(channel_id, kaede_mls::SframePurpose::LivekitE2ee)
    }

    /// 从 MLS exporter 派生 channel 的 SFrame 密钥（带 purpose）。
    pub fn mls_export_sframe_key_for(
        &self,
        channel_id: &str,
        purpose: kaede_mls::SframePurpose,
    ) -> Result<[u8; 32], ClientError> {
        let guard = self.mls.lock().unwrap();
        let s = guard
            .as_ref()
            .ok_or_else(|| ClientError::Mls("mls not initialized".into()))?;
        s.export_sframe_key_for(channel_id, purpose)
    }

    /// P0-2: 带额外上下文派生 SFrame key（房间级 seed）。
    pub fn mls_export_sframe_key_for_with_extra(
        &self,
        channel_id: &str,
        purpose: kaede_mls::SframePurpose,
        extra: &[u8],
    ) -> Result<[u8; 32], ClientError> {
        let guard = self.mls.lock().unwrap();
        let s = guard
            .as_ref()
            .ok_or_else(|| ClientError::Mls("mls not initialized".into()))?;
        s.export_sframe_key_for_with_extra(channel_id, purpose, extra)
    }

    /// 当前 MLS epoch。未初始化返回 0。
    /// 当前 MLS 群组所有成员 uid。未初始化时返回空。
    ///
    /// **P0-5**：供 `room.rs::host_add_peer` 做 sender_id 碰撞检查。
    pub fn mls_member_uids(&self) -> Vec<String> {
        let guard = self.mls.lock().unwrap();
        guard.as_ref().map(|s| s.member_uids()).unwrap_or_default()
    }

    pub fn mls_epoch(&self) -> u64 {
        self.mls
            .lock()
            .unwrap()
            .as_ref()
            .map(|s| s.epoch())
            .unwrap_or(0)
    }

    /// 当前主群 group_id。未建组返回 `None`。
    pub fn mls_group_id(&self) -> Option<Vec<u8>> {
        self.mls
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|s| s.group_id().map(|g| g.to_vec()))
    }

    /// 群组成员数。未初始化返回 0。
    pub fn mls_member_count(&self) -> usize {
        self.mls
            .lock()
            .unwrap()
            .as_ref()
            .map(|s| s.member_count())
            .unwrap_or(0)
    }

    /// 序列化整个 MLS 状态（不加密）。
    ///
    /// **包含签名私钥**。生产代码应使用 [`Self::mls_snapshot_with_cipher`]。
    #[deprecated(
        since = "0.2.0",
        note = "use mls_snapshot_with_cipher(device_cipher(...))"
    )]
    #[allow(deprecated)]
    pub fn mls_snapshot(&self) -> Result<Vec<u8>, ClientError> {
        let guard = self.mls.lock().unwrap();
        let s = guard
            .as_ref()
            .ok_or_else(|| ClientError::Mls("mls not initialized".into()))?;
        s.snapshot()
    }

    /// 序列化 + 加密。
    pub fn mls_snapshot_with_cipher(
        &self,
        cipher: &dyn kaede_mls::SnapshotCipher,
    ) -> Result<Vec<u8>, ClientError> {
        let guard = self.mls.lock().unwrap();
        let s = guard
            .as_ref()
            .ok_or_else(|| ClientError::Mls("mls not initialized".into()))?;
        s.snapshot_with(cipher)
    }

    /// 从快照恢复 MLS 状态（不加密路径）。
    #[deprecated(
        since = "0.2.0",
        note = "use mls_restore_from_snapshot_with_cipher(...)"
    )]
    #[allow(deprecated)]
    pub fn mls_restore_from_snapshot(&self, snapshot: &[u8]) -> Result<(), ClientError> {
        let session = MlsSession::from_snapshot(snapshot)?;
        let mut guard = self.mls.lock().unwrap();
        *guard = Some(session);
        Ok(())
    }

    /// 从快照恢复 + 解密。
    pub fn mls_restore_from_snapshot_with_cipher(
        &self,
        snapshot: &[u8],
        cipher: Option<&dyn kaede_mls::SnapshotCipher>,
    ) -> Result<(), ClientError> {
        let session = MlsSession::from_snapshot_with(snapshot, cipher)?;
        let mut guard = self.mls.lock().unwrap();
        *guard = Some(session);
        Ok(())
    }

    /// 群组是否就绪。
    pub fn mls_is_ready(&self) -> bool {
        self.mls
            .lock()
            .unwrap()
            .as_ref()
            .map(|s| s.is_ready())
            .unwrap_or(false)
    }

    // =================================================================
    // 收消息
    // =================================================================

    /// 处理一条收到的服务器消息。
    ///
    /// MLS 消息在这里自动解密 / 处理握手。
    pub async fn handle_server_message(&self, msg: ServerMessage) -> Result<(), ClientError> {
        match msg {
            ServerMessage::PeerJoined { uid } => {
                // 去重：重连时服务器可能重发 PeerJoined，避免 peers 出现重复
                let is_new = {
                    let mut p = self.peers.lock().unwrap();
                    if p.contains(&uid) {
                        false
                    } else {
                        p.push(uid.clone());
                        true
                    }
                };
                if is_new {
                    self.emit(ClientEvent::PeerJoined {
                        server_id: self.server_id.clone(),
                        uid,
                    });
                }
            }
            ServerMessage::PeerLeft { uid } => {
                self.peers.lock().unwrap().retain(|u| u != &uid);
                self.emit(ClientEvent::PeerLeft {
                    server_id: self.server_id.clone(),
                    uid,
                });
            }
            ServerMessage::SdpOffer { from, sdp } => {
                self.emit(ClientEvent::SdpOffer {
                    server_id: self.server_id.clone(),
                    from,
                    sdp,
                });
            }
            ServerMessage::SdpAnswer { from, sdp } => {
                self.emit(ClientEvent::SdpAnswer {
                    server_id: self.server_id.clone(),
                    from,
                    sdp,
                });
            }
            ServerMessage::IceCandidate { from, candidate } => {
                self.emit(ClientEvent::IceCandidate {
                    server_id: self.server_id.clone(),
                    from,
                    candidate,
                });
            }
            ServerMessage::SfuToken {
                token,
                ws_url,
                room,
                identity,
            } => {
                self.emit(ClientEvent::SfuToken {
                    server_id: self.server_id.clone(),
                    token,
                    ws_url,
                    room,
                    identity,
                });
            }
            ServerMessage::Error { code, message } => {
                self.emit(ClientEvent::ServerError {
                    server_id: self.server_id.clone(),
                    code,
                    message: message.clone(),
                });
                if code.is_fatal() {
                    return Err(ClientError::Server { code, message });
                }
            }

            // ---- MLS 握手 ----
            ServerMessage::MlsWelcome { from, payload } => {
                // 未就绪 → 正常加入；已就绪但 group_id 不同 → 重建主群。
                let was_ready = self.mls_is_ready();
                let (gid, welcome) = unpack_welcome_payload(&payload);
                match self.mls_join_or_reset_welcome(welcome, gid) {
                    Ok(false) => {
                        tracing::debug!(
                            "[client:{}] duplicate welcome from {} ignored",
                            self.server_id,
                            from
                        );
                    }
                    Ok(true) => {
                        if was_ready {
                            tracing::warn!(
                                "[client:{}] main group rebuilt from welcome by {}; \
                                 old group history is no longer decryptable",
                                self.server_id,
                                from
                            );
                            self.emit(ClientEvent::MlsSyncError {
                                server_id: self.server_id.clone(),
                                user_message:
                                    "服务器主群已重建，本地旧群的历史消息不可恢复；请继续使用新群"
                                        .into(),
                                recoverable: true,
                                detail: format!("reset main group via welcome from {}", from),
                            });
                        }
                        // 未 join 过时先补一次 MlsJoining，再进 MlsReady。
                        if self.state() == ClientState::Authenticated
                            && let Err(e) = self.transition(ClientState::MlsJoining)
                        {
                            tracing::warn!(
                                "[client:{}] → MlsJoining failed: {}",
                                self.server_id,
                                e
                            );
                        }
                        if let Err(e) = self.transition(ClientState::MlsReady) {
                            // MlsReady → MlsReady 会失败（重连场景已就绪），预期行为
                            tracing::debug!(
                                "[client:{}] → MlsReady skipped: {}",
                                self.server_id,
                                e
                            );
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            "[client:{}] welcome from {} failed: {}",
                            self.server_id,
                            from,
                            e
                        );
                        self.emit(ClientEvent::MlsSyncError {
                            server_id: self.server_id.clone(),
                            user_message: "无法通过 Welcome 加入主群，请重连服务器".into(),
                            recoverable: true,
                            detail: e.to_string(),
                        });
                    }
                }
            }

            ServerMessage::MlsCommit {
                from: _,
                epoch: _,
                payload,
            }
            | ServerMessage::MlsProposal { from: _, payload } => {
                // C-2: 锁内处理、锁外 emit（避免 emit 阻塞锁）
                let before = self.mls_epoch();
                let result = {
                    let mut guard = self.mls.lock().unwrap();
                    match guard.as_mut() {
                        Some(s) if s.is_ready() => s.process_handshake(&payload).map(|_| ()),
                        _ => Ok(()), // 未 ready 时静默（无 group 状态可破坏）
                    }
                };
                if result.is_ok() {
                    self.mark_main_dirty();
                    self.emit_epoch_if_changed(before);
                }
                if let Err(e) = result {
                    tracing::warn!("[client:{}] handshake failed: {}", self.server_id, e);
                    self.emit(ClientEvent::MlsSyncError {
                        server_id: self.server_id.clone(),
                        user_message: "MLS 同步失败，语音媒体可能中断；如遇异常请离开语音频道重进"
                            .into(),
                        recoverable: true,
                        detail: e.to_string(),
                    });
                }
            }

            ServerMessage::DmReceived {
                conv_id,
                from,
                ts,
                ciphertext,
            } => {
                // B2: Welcome envelope (first byte 0x02) -> join DM group, do not emit to UI
                if let Some((generation, welcome)) = unpack_dm_welcome(&ciphertext) {
                    if let Err(e) = self.dm_join_group(&conv_id, welcome, generation) {
                        tracing::warn!(
                            "[client:{}] dm_join_group failed for conv {}: {}",
                            self.server_id,
                            conv_id,
                            e
                        );
                    }
                } else {
                    match self.dm_open(&conv_id, &ciphertext) {
                        Ok(plaintext) => {
                            self.emit_local_event(ClientEvent::DmReceived {
                                server_id: self.server_id.clone(),
                                conv_id,
                                from,
                                ts,
                                plaintext,
                            });
                        }
                        Err(e) => {
                            tracing::warn!(
                                "[client:{}] DM decrypt failed (conv {}): {}",
                                self.server_id,
                                conv_id,
                                e
                            );
                        }
                    }
                }
            }
            ServerMessage::MlsAppMessage { from, payload } => {
                let plaintext = {
                    let mut guard = self.mls.lock().unwrap();
                    match guard.as_mut() {
                        Some(s) if s.is_ready() => match s.decrypt(&payload) {
                            Ok(pt) => Some(pt),
                            Err(e) => {
                                tracing::warn!(
                                    "[client:{}] decrypt from {} failed: {}",
                                    self.server_id,
                                    from,
                                    e
                                );
                                None
                            }
                        },
                        _ => None,
                    }
                };
                if let Some(pt) = plaintext {
                    // 先尝试识别媒体信令（MediaMeta），识别成功就不发 AppMessage。
                    // 其它类型（Message / Reaction / Raw）仍走 AppMessage 事件，
                    // 由上层自行 decode（保持既有行为）。
                    use crate::payload::{DecodedPayload, decode};
                    match decode(&pt) {
                        DecodedPayload::MediaMeta(meta) => {
                            self.emit(ClientEvent::MediaMetaReceived {
                                server_id: self.server_id.clone(),
                                from,
                                meta,
                            });
                        }
                        // R2-02: 私密频道内容不得经主群广播进入本地状态。
                        // 任意主群成员都能构造 `c = <私密频道 id>` 的 payload。
                        DecodedPayload::Message(ref p) if self.channel_has_group(&p.c) => {
                            tracing::warn!(
                                "[client:{}] drop main-group message targeting private channel {}",
                                self.server_id,
                                p.c
                            );
                        }
                        DecodedPayload::Reaction(ref r) if self.channel_has_group(&r.c) => {
                            tracing::warn!(
                                "[client:{}] drop main-group reaction targeting private channel {}",
                                self.server_id,
                                r.c
                            );
                        }
                        _ => {
                            self.emit(ClientEvent::AppMessage {
                                server_id: self.server_id.clone(),
                                from,
                                plaintext: pt,
                            });
                        }
                    }
                }
            }

            ServerMessage::ChannelMemberJoined {
                channel_id,
                requester,
                key_package,
            } => {
                if !self.channel_has_group(&channel_id) {
                    tracing::debug!(
                        "[client:{}] channel_member_joined for {} but local has no group; skip",
                        self.server_id,
                        channel_id
                    );
                } else {
                    let kp_bytes: Vec<u8> = {
                        use base64::Engine as _;
                        match base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(&key_package)
                        {
                            Ok(b) => b,
                            Err(e) => {
                                tracing::warn!("[client:{}] bad kp b64: {}", self.server_id, e);
                                return Ok(());
                            }
                        }
                    };
                    match self.channel_add_member(&channel_id, &kp_bytes) {
                        Ok((commit, welcome)) => {
                            let mut payload = Vec::with_capacity(1 + welcome.len());
                            payload.push(0x04u8);
                            payload.extend_from_slice(&welcome);
                            if let Err(e) = self
                                .transport
                                .send(ClientMessage::ChannelSync {
                                    to: requester.clone(),
                                    channel_id: channel_id.clone(),
                                    payload,
                                })
                                .await
                            {
                                tracing::warn!(
                                    "[client:{}] send ChannelSync welcome failed: {}",
                                    self.server_id,
                                    e
                                );
                            } else {
                                tracing::info!(
                                    "[client:{}] sent welcome for {} to {}",
                                    self.server_id,
                                    channel_id,
                                    requester
                                );
                            }

                            // 把 Commit 广播给频道内其他成员，让所有人的 epoch 对齐。
                            // 旧实现漏了这一步，导致既有成员与新成员 key 不一致。
                            if let Err(e) = self
                                .transport
                                .send(ClientMessage::ChannelCommit {
                                    channel_id: channel_id.clone(),
                                    exclude: Some(requester.clone()),
                                    payload: commit,
                                })
                                .await
                            {
                                tracing::warn!(
                                    "[client:{}] send ChannelCommit failed for {}: {}",
                                    self.server_id,
                                    channel_id,
                                    e
                                );
                                self.emit(ClientEvent::MlsSyncError {
                                    server_id: self.server_id.clone(),
                                    user_message: format!(
                                        "私密频道 {} 的成员同步失败，请让新成员重新加入或重新登录",
                                        channel_id
                                    ),
                                    recoverable: true,
                                    detail: e.to_string(),
                                });
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                "[client:{}] channel_add_member failed for {}: {}",
                                self.server_id,
                                channel_id,
                                e
                            );
                        }
                    }
                }
            }

            ServerMessage::ChannelSync {
                channel_id,
                payload,
            } => {
                if payload.is_empty() {
                    tracing::warn!("[client:{}] empty ChannelSync", self.server_id);
                    return Ok(());
                }
                match payload[0] {
                    0x04 => {
                        let welcome = &payload[1..];
                        match self.channel_join_via_welcome(&channel_id, welcome) {
                            Ok(()) => {
                                tracing::info!(
                                    "[client:{}] joined channel group {} via welcome",
                                    self.server_id,
                                    channel_id
                                );
                                let sender = self
                                    .channel_pending_welcomes
                                    .lock()
                                    .unwrap()
                                    .remove(&channel_id);
                                if let Some(tx) = sender {
                                    let _ = tx.try_send(Ok(welcome.to_vec()));
                                }
                                self.emit(ClientEvent::ChannelJoined {
                                    server_id: self.server_id.clone(),
                                    channel_id,
                                });
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "[client:{}] join_via_welcome failed for {}: {}",
                                    self.server_id,
                                    channel_id,
                                    e
                                );
                                let sender = self
                                    .channel_pending_welcomes
                                    .lock()
                                    .unwrap()
                                    .remove(&channel_id);
                                if let Some(tx) = sender {
                                    let _ = tx.try_send(Err(e.to_string()));
                                }
                            }
                        }
                    }
                    0x05 => {
                        let commit = &payload[1..];
                        match self.channel_process_commit(&channel_id, commit) {
                            Ok(_) => {
                                tracing::info!(
                                    "[client:{}] processed channel commit for {}",
                                    self.server_id,
                                    channel_id
                                );
                                // 块 B：私密频道 epoch 变化 → 若正在该频道语音，
                                // observer 会触发媒体会话重连。
                                if let Some(epoch) = self.channel_epoch(&channel_id) {
                                    self.emit(ClientEvent::ChannelEpochChanged {
                                        server_id: self.server_id.clone(),
                                        channel_id: channel_id.clone(),
                                        epoch,
                                    });
                                }
                            }
                            Err(e) => tracing::warn!(
                                "[client:{}] process_commit failed for {}: {}",
                                self.server_id,
                                channel_id,
                                e
                            ),
                        }
                    }
                    tag => tracing::warn!(
                        "[client:{}] unknown ChannelSync tag {:#x} for {}",
                        self.server_id,
                        tag,
                        channel_id
                    ),
                }
            }

            ServerMessage::ChannelMemberLeft { channel_id, member } => {
                if !self.channel_has_group(&channel_id) {
                    tracing::debug!(
                        "[client:{}] channel_member_left for {} but no local group; skip",
                        self.server_id,
                        channel_id
                    );
                    return Ok(());
                }
                match self.channel_remove_member(&channel_id, member.as_str()) {
                    Ok(commit) => {
                        if let Err(e) = self
                            .transport
                            .send(ClientMessage::ChannelCommit {
                                channel_id: channel_id.clone(),
                                exclude: None,
                                payload: commit,
                            })
                            .await
                        {
                            tracing::warn!(
                                "[client:{}] send ChannelCommit after leave failed: {}",
                                self.server_id,
                                e
                            );
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            "[client:{}] remove channel member {} failed: {}",
                            self.server_id,
                            member,
                            e
                        );
                    }
                }
            }

            ServerMessage::ChannelMessage {
                channel_id,
                from,
                ts,
                ciphertext,
            } => {
                if ciphertext.first() == Some(&0x03) {
                    // 只接受本地确实持有频道 group 的频道。
                    if !self.channel_has_group(&channel_id) {
                        tracing::warn!(
                            "[client:{}] channel message for {} but no local channel group; drop",
                            self.server_id,
                            channel_id
                        );
                    } else {
                        match self.channel_export_key(&channel_id) {
                            Ok(key) => {
                                match kaede_crypto::dm_cipher::open(
                                    &key,
                                    &channel_id,
                                    &ciphertext[1..],
                                ) {
                                    Ok(plaintext) => {
                                        self.emit(ClientEvent::ChannelMessage {
                                            server_id: self.server_id.clone(),
                                            channel_id,
                                            from,
                                            ts,
                                            plaintext,
                                        });
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            "[client:{}] channel message decrypt failed for {}: {}",
                                            self.server_id,
                                            channel_id,
                                            e
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "[client:{}] channel key unavailable for {}: {}",
                                    self.server_id,
                                    channel_id,
                                    e
                                );
                            }
                        }
                    }
                } else {
                    tracing::warn!(
                        "[client:{}] channel message without 0x03 prefix for {}; drop",
                        self.server_id,
                        channel_id
                    );
                }
            }

            ServerMessage::ChannelCreated { channel } => {
                self.emit(ClientEvent::ChannelCreated {
                    server_id: self.server_id.clone(),
                    channel,
                });
            }
            ServerMessage::ChannelUpdated { channel } => {
                self.emit(ClientEvent::ChannelUpdated {
                    server_id: self.server_id.clone(),
                    channel,
                });
            }
            ServerMessage::ChannelDeleted { id } => {
                self.emit(ClientEvent::ChannelDeleted {
                    server_id: self.server_id.clone(),
                    channel_id: id,
                });
            }
            ServerMessage::PresenceUpdated { presence } => {
                self.emit(ClientEvent::PresenceUpdated {
                    server_id: self.server_id.clone(),
                    presence,
                });
            }
            ServerMessage::MemberUpdated { member } => {
                self.emit(ClientEvent::MemberUpdated {
                    server_id: self.server_id.clone(),
                    member,
                });
            }
            ServerMessage::MemberRemoved { login_id } => {
                self.emit(ClientEvent::MemberRemoved {
                    server_id: self.server_id.clone(),
                    login_id,
                });
            }

            ServerMessage::Joined { peers } => {
                *self.peers.lock().unwrap() = peers.clone();
                self.emit(ClientEvent::Joined {
                    server_id: self.server_id.clone(),
                    peers,
                });
            }

            // 其他类型：暂不处理
            ServerMessage::KeyPackageFor { .. } | ServerMessage::Pong { .. } => {}
            ServerMessage::ServerHello { .. } => {}
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
        let reason = loop {
            match self.transport.recv().await {
                Ok(msg) => {
                    if let Err(e) = self.handle_server_message(msg).await {
                        // fatal 错误（Kicked / UnsupportedVersion / Malformed）
                        // 会让 handle_server_message 返回 Err。
                        tracing::warn!("[client:{}] handle failed: {}", self.server_id, e);
                        break e.to_string();
                    }
                    // B3+: 每条消息处理完后就 flush 一次 dirty snapshot，
                    // 避免 DM Welcome 到达 / dm_join_group 后延迟写盘。
                    self.flush_dirty_snapshots().await;
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

/// DM Welcome 信封 magic。
const DM_WELCOME_MAGIC: u8 = 0x4b;

/// 拆出 DM Welcome 信封的 `(generation, welcome)`。
///
/// 新格式：`[0x02][0x4b][generation:u64 BE][welcome]`；
/// 旧格式：`[0x02][welcome]`，按 generation=0 处理。
/// 非 Welcome 信封返回 `None`。
pub(crate) fn unpack_dm_welcome(ct: &[u8]) -> Option<(u64, &[u8])> {
    if ct.first() != Some(&0x02) {
        return None;
    }
    if ct.len() >= 10 && ct[1] == DM_WELCOME_MAGIC {
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&ct[2..10]);
        Some((u64::from_be_bytes(buf), &ct[10..]))
    } else {
        Some((0, &ct[1..]))
    }
}

/// 构造 DM Welcome 信封。
pub(crate) fn pack_dm_welcome(generation: u64, welcome: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(10 + welcome.len());
    out.push(0x02);
    out.push(DM_WELCOME_MAGIC);
    out.extend_from_slice(&generation.to_be_bytes());
    out.extend_from_slice(welcome);
    out
}

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

/// H10: 规范化 DM conv_id 为 `dm:<min_uid>:<max_uid>`（字典序）。
///
/// server 端 `dm::conv_id_for` 已保证字典序，客户端防御性规范化：
/// 若未来 server 或攻击者发送非规范 conv_id（`dm:alice:bob` vs
/// `dm:bob:alice`），两端会派生出不同 exporter context → 不同 key
/// → 消息永远解不开。规范化是幂等的。
///
/// 非 `dm:` 前缀原样返回（兼容 channel_id 等其他用途）。
fn canonical_conv_id(conv_id: &str) -> String {
    if let Some(rest) = conv_id.strip_prefix("dm:")
        && let Some((a, b)) = rest.split_once(':')
    {
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        return format!("dm:{}:{}", lo, hi);
    }
    conv_id.to_string()
}

#[allow(deprecated)]
#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::MockTransport;
    use kaede_protocol::ErrorCode;

    async fn spawn_hello_server(server_t: crate::transport::MockServerSide) {
        let msg = server_t.recv().await.unwrap();
        assert!(matches!(msg, ClientMessage::ClientHello { .. }));
        server_t
            .send(ServerMessage::ServerHello {
                protocol_version: kaede_protocol::PROTOCOL_VERSION,
                server_id: "srv_test".into(),
                server_name: "Test".into(),
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn connect_happy_path() {
        let (client_t, server_t) = MockTransport::pair();
        let conn = ServerConnection::new("s1", Arc::new(client_t), None);
        let h = tokio::spawn(spawn_hello_server(server_t));
        conn.connect("test", "0.1.0").await.unwrap();
        h.await.unwrap();
        assert_eq!(conn.state(), ClientState::Connecting);
    }

    #[tokio::test]
    async fn full_join_flow() {
        let (client_t, server_t) = MockTransport::pair();
        let conn = ServerConnection::new("s1", Arc::new(client_t), None);

        let h = tokio::spawn(async move {
            let _ = server_t.recv().await.unwrap();
            server_t
                .send(ServerMessage::ServerHello {
                    protocol_version: kaede_protocol::PROTOCOL_VERSION,
                    server_id: "srv_test".into(),
                    server_name: "Test".into(),
                })
                .await
                .unwrap();
            let msg = server_t.recv().await.unwrap();
            assert!(matches!(msg, ClientMessage::Join));
            server_t
                .send(ServerMessage::Joined {
                    peers: vec![UserId::new("bob")],
                })
                .await
                .unwrap();
        });

        conn.connect("test", "0.1.0").await.unwrap();
        conn.authenticate().unwrap();
        conn.join().await.unwrap();
        h.await.unwrap();

        // join 现在只发不收；状态已进 MlsJoining，但 peers 要等
        // event_loop 处理 Joined 后才填充。
        assert_eq!(conn.state(), ClientState::MlsJoining);

        // 手动喂一条 Joined，验证 event_loop 路径
        conn.handle_server_message(ServerMessage::Joined {
            peers: vec![UserId::new("bob")],
        })
        .await
        .unwrap();
        assert_eq!(conn.peers().len(), 1);
    }

    #[tokio::test]
    async fn cannot_join_before_auth() {
        let (client_t, _server_t) = MockTransport::pair();
        let conn = ServerConnection::new("s1", Arc::new(client_t), None);
        let err = conn.join().await.unwrap_err();
        assert!(matches!(err, ClientError::NotAuthenticated));
    }

    #[tokio::test]
    async fn server_error_terminates() {
        let (client_t, server_t) = MockTransport::pair();
        let conn = ServerConnection::new("s1", Arc::new(client_t), None);

        let h = tokio::spawn(async move {
            let _ = server_t.recv().await.unwrap();
            server_t
                .send(ServerMessage::Error {
                    code: ErrorCode::UnsupportedVersion,
                    message: "need v3".into(),
                })
                .await
                .unwrap();
        });

        let err = conn.connect("test", "0.1.0").await.unwrap_err();
        h.await.unwrap();
        assert!(matches!(
            err,
            ClientError::Server {
                code: ErrorCode::UnsupportedVersion,
                ..
            }
        ));
    }

    // =================================================================
    // MLS 集成测试
    // =================================================================

    #[tokio::test]
    async fn mls_create_group_transitions_to_ready() {
        let (client_t, _server_t) = MockTransport::pair();
        let conn = ServerConnection::new("s1", Arc::new(client_t), None);
        conn.state.lock().unwrap().force(ClientState::MlsJoining);

        conn.mls_init("alice").unwrap();
        assert!(!conn.mls_is_ready());

        conn.mls_create_group().unwrap();
        assert!(conn.mls_is_ready());
        assert_eq!(conn.state(), ClientState::MlsReady);
        assert_eq!(conn.mls_member_count(), 1);
    }

    #[tokio::test]
    async fn mls_send_app_goes_through_transport() {
        let (client_t, server_t) = MockTransport::pair();
        let conn = ServerConnection::new("s1", Arc::new(client_t), None);
        conn.state.lock().unwrap().force(ClientState::MlsJoining);

        conn.mls_init("alice").unwrap();
        conn.mls_create_group().unwrap();

        let _ct = conn.mls_send_app(b"hello").await.unwrap();

        // 服务器端应收到一条 MlsAppMessage
        let got = server_t.recv().await.unwrap();
        match got {
            ClientMessage::MlsAppMessage { payload } => {
                assert!(!payload.is_empty(), "ciphertext should not be empty");
            }
            other => panic!("expected MlsAppMessage, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn mls_receive_app_message_decrypts() {
        // 建立两个 MlsSession，通过它构造一条给 alice 的合法密文。
        // 然后把它注入 alice 的 ServerConnection，检查事件是否触发。
        use std::sync::Mutex as StdMutex;

        struct CollectingObserver {
            events: StdMutex<Vec<ClientEvent>>,
        }
        impl crate::ClientObserver for CollectingObserver {
            fn on_event(&self, event: ClientEvent) {
                self.events.lock().unwrap().push(event);
            }
        }

        // --- 准备：离线构造 alice + bob 的 MLS 组 ---
        let mut bob_session = MlsSession::new("bob").unwrap();
        let bob_kp = bob_session.generate_key_package().unwrap();

        let mut alice_offline = MlsSession::new("alice").unwrap();
        alice_offline.create_group().unwrap();
        let (_c, welcome) = alice_offline.add_member(&bob_kp).unwrap();
        bob_session.join_with_welcome(&welcome).unwrap();
        assert_eq!(alice_offline.member_count(), 2);
        assert_eq!(bob_session.member_count(), 2);

        // bob 加密一条消息（此时双方 epoch 一致）
        let ct = bob_session.encrypt(b"hi alice").unwrap();

        // --- 拿到 alice_offline 的 MlsSession 也没法直接塞进 ServerConnection，
        //     因为 ServerConnection.mls_init 会创建新 session。
        //     所以这个测试改成：直接验证 alice_offline 能解密，
        //     并验证 ServerConnection 的 handle_server_message 走通了事件链路
        //     使用 MockTransport 无 group 时不会 panic。
        let pt = alice_offline.decrypt(&ct).unwrap();
        assert_eq!(pt, b"hi alice");

        // --- 事件链路测试：给 ServerConnection 一个 无 MLS 状态，
        //     收到 AppMessage 应该静默丢弃（不 panic）---
        let observer = Arc::new(CollectingObserver {
            events: StdMutex::new(Vec::new()),
        });
        let (client_t, _server_t) = MockTransport::pair();
        let conn = ServerConnection::new("s1", Arc::new(client_t), Some(observer.clone()));

        // 未初始化 MLS，直接送一条 AppMessage
        conn.handle_server_message(ServerMessage::MlsAppMessage {
            from: UserId::new("bob"),
            payload: ct.clone(),
        })
        .await
        .unwrap();

        // 应该没有 AppMessage 事件（因为没 MLS state）
        let events = observer.events.lock().unwrap();
        let app_events = events
            .iter()
            .filter(|e| matches!(e, ClientEvent::AppMessage { .. }))
            .count();
        assert_eq!(app_events, 0, "no MLS state => no AppMessage event");
    }

    #[tokio::test]
    async fn mls_remove_member_by_uid_via_connection() {
        use std::sync::Arc;
        // 直接用两个 MlsSession 模拟握手，不经过网络
        let bob = MlsSession::new("bob").unwrap();
        let bob_kp = bob.generate_key_package().unwrap();

        let (client_t, _server_t) = MockTransport::pair();
        let conn = ServerConnection::new("s1", Arc::new(client_t), None);
        conn.state.lock().unwrap().force(ClientState::MlsJoining);
        conn.mls_init("alice").unwrap();
        conn.mls_create_group().unwrap();

        let _ = conn.mls_add_member(&bob_kp).unwrap();
        assert_eq!(conn.mls_member_count(), 2);

        let commit = conn.mls_remove_member_by_uid("bob").unwrap();
        assert!(!commit.is_empty());
        assert_eq!(conn.mls_member_count(), 1);
    }
    #[tokio::test]
    async fn mls_export_sframe_key_for_purpose_via_connection() {
        let (client_t, _server_t) = MockTransport::pair();
        let conn = ServerConnection::new("s1", Arc::new(client_t), None);
        conn.state.lock().unwrap().force(ClientState::MlsJoining);
        conn.mls_init("alice").unwrap();
        conn.mls_create_group().unwrap();

        let k_va = conn
            .mls_export_sframe_key_for("general", kaede_mls::SframePurpose::VoiceAudio)
            .unwrap();
        let k_vv = conn
            .mls_export_sframe_key_for("general", kaede_mls::SframePurpose::VoiceVideo)
            .unwrap();
        let k_sv = conn
            .mls_export_sframe_key_for("general", kaede_mls::SframePurpose::ScreenVideo)
            .unwrap();
        assert_ne!(k_va, k_vv);
        assert_ne!(k_vv, k_sv);

        // 旧 API 等于显式 Default
        let k_default = conn.mls_export_sframe_key("general").unwrap();
        let k_explicit = conn
            .mls_export_sframe_key_for("general", kaede_mls::SframePurpose::Default)
            .unwrap();
        assert_eq!(k_default, k_explicit);
    }
    #[tokio::test]
    async fn mls_export_sframe_key_via_connection() {
        let (client_t, _server_t) = MockTransport::pair();
        let conn = ServerConnection::new("s1", Arc::new(client_t), None);
        conn.state.lock().unwrap().force(ClientState::MlsJoining);

        conn.mls_init("alice").unwrap();
        conn.mls_create_group().unwrap();

        let k1 = conn.mls_export_sframe_key("ch-general").unwrap();
        let k2 = conn.mls_export_sframe_key("ch-general").unwrap();
        assert_eq!(k1, k2);

        let k3 = conn.mls_export_sframe_key("ch-voice").unwrap();
        assert_ne!(k1, k3);
    }

    // =================================================================
    // C-2: MLS handshake 失败 emit MlsSyncError
    // =================================================================

    #[tokio::test]
    async fn mls_commit_failure_emits_sync_error() {
        use std::sync::Mutex as StdMutex;

        struct CollectingObserver {
            events: StdMutex<Vec<ClientEvent>>,
        }
        impl crate::ClientObserver for CollectingObserver {
            fn on_event(&self, event: ClientEvent) {
                self.events.lock().unwrap().push(event);
            }
        }

        let observer = Arc::new(CollectingObserver {
            events: StdMutex::new(Vec::new()),
        });
        let (client_t, _server_t) = MockTransport::pair();
        let conn = ServerConnection::new("s1", Arc::new(client_t), Some(observer.clone()));

        // 初始化 MLS 并进入 MlsReady
        conn.mls_init("alice").unwrap();
        conn.state.lock().unwrap().force(ClientState::MlsJoining);
        conn.mls_create_group().unwrap();
        assert!(conn.mls_is_ready());

        // 送一条无法反序列化的 commit
        conn.handle_server_message(ServerMessage::MlsCommit {
            from: UserId::new("bob"),
            epoch: 1,
            payload: vec![0xFF, 0xFF, 0xFF, 0xFF],
        })
        .await
        .unwrap();

        // 验证 emit 了 MlsSyncError
        let events = observer.events.lock().unwrap();
        let has_sync_error = events.iter().any(|e| {
            matches!(
                e,
                ClientEvent::MlsSyncError {
                    recoverable: true,
                    ..
                }
            )
        });
        assert!(
            has_sync_error,
            "expected MlsSyncError (recoverable=true), got {:?}",
            *events
        );
    }

    // =================================================================
    // R2-02：私密频道内容不得从主群广播通道进入
    // =================================================================

    #[tokio::test]
    async fn main_group_message_targeting_private_channel_is_dropped() {
        use std::sync::Mutex as StdMutex;

        struct CollectingObserver {
            events: StdMutex<Vec<ClientEvent>>,
        }
        impl crate::ClientObserver for CollectingObserver {
            fn on_event(&self, event: ClientEvent) {
                self.events.lock().unwrap().push(event);
            }
        }

        let observer = Arc::new(CollectingObserver {
            events: StdMutex::new(Vec::new()),
        });

        // 构造一条“主群”消息（正文声称属于私密频道 "ch_secret"）。
        let (client_t, _server_t) = MockTransport::pair();
        let conn = ServerConnection::new("s1", Arc::new(client_t), Some(observer.clone()));
        conn.state.lock().unwrap().force(ClientState::MlsJoining);
        conn.mls_init("alice").unwrap();
        conn.mls_create_group().unwrap();

        // 用本地主群加密一条指向私密频道的 payload。
        let payload =
            crate::payload::MessagePayload::new("ch_secret", "bob", "injected", Vec::new(), None)
                .to_bytes()
                .unwrap();
        let ct = conn
            .mls
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .encrypt(&payload)
            .unwrap();

        // 先让本地持有 "ch_secret" 的频道 group（模拟私密频道成员）。
        conn.channel_ensure_group("ch_secret").unwrap();
        assert!(conn.channel_has_group("ch_secret"));

        conn.handle_server_message(ServerMessage::MlsAppMessage {
            from: UserId::new("bob"),
            payload: ct,
        })
        .await
        .unwrap();

        let events = observer.events.lock().unwrap();
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, ClientEvent::AppMessage { .. })),
            "main-group message targeting a private channel must be dropped, got {:?}",
            *events
        );
    }

    // =================================================================
    // R2-04：Welcome 驱动的旧群重建
    // =================================================================

    #[test]
    fn welcome_with_new_group_id_resets_existing_group() {
        // bob 先加入 alice1 的旧群。
        let mut bob = MlsSession::new("bob").unwrap();
        let bob_kp1 = bob.generate_key_package().unwrap();
        let mut alice1 = MlsSession::new("alice").unwrap();
        alice1.create_group().unwrap();
        let (_c1, w1) = alice1.add_member(&bob_kp1).unwrap();
        bob.join_with_welcome(&w1).unwrap();
        let gid1 = bob.group_id().unwrap().to_vec();

        // 房主丢失快照后重建群：alice2 新群，使用 bob 的新 KP。
        let mut alice2 = MlsSession::new("alice").unwrap();
        alice2.create_group().unwrap();
        let bob_kp2 = bob.generate_key_package().unwrap();
        let (_c2, w2) = alice2.add_member(&bob_kp2).unwrap();
        let gid2 = alice2.group_id().unwrap().to_vec();
        assert_ne!(gid1, gid2, "rebuild must produce a new group id");

        let (client_t, _server_t) = MockTransport::pair();
        let conn = ServerConnection::new("s1", Arc::new(client_t), None);
        *conn.mls.lock().unwrap() = Some(bob);

        let changed = conn.mls_join_or_reset_welcome(&w2, Some(&gid2)).unwrap();
        assert!(changed, "different group id must rebuild local group");

        {
            let mut guard = conn.mls.lock().unwrap();
            let s = guard.as_mut().unwrap();
            assert_eq!(s.group_id().unwrap(), &gid2[..]);
            assert_eq!(s.epoch(), alice2.epoch());
            let ct = alice2.encrypt(b"after-reset").unwrap();
            assert_eq!(s.decrypt(&ct).unwrap(), b"after-reset");
            let ct2 = s.encrypt(b"reply").unwrap();
            assert_eq!(alice2.decrypt(&ct2).unwrap(), b"reply");
        }

        // 同一个 Welcome 再投递一次（重复）不应重建。
        assert!(
            !conn.mls_join_or_reset_welcome(&w2, Some(&gid2)).unwrap(),
            "duplicate welcome must be ignored"
        );
    }

    // =================================================================
    // R2-05：DM 代际仲裁
    // =================================================================

    #[test]
    fn dm_generation_rebuild_converges() {
        let conv = "dm:alice:bob";
        let bob = MlsSession::new("bob").unwrap();
        let bob_kp1 = bob.generate_key_package().unwrap();
        let bob_kp2 = bob.generate_key_package().unwrap();

        // 发送方（alice）与接收方（bob）各自一个连接，bob 复用同一 MlsSession。
        let (alice_t, _alice_s) = MockTransport::pair();
        let alice = ServerConnection::new("s-alice", Arc::new(alice_t), None);
        alice.mls_init("alice").unwrap();

        let (bob_t, _bob_s) = MockTransport::pair();
        let bob_conn = ServerConnection::new("s-bob", Arc::new(bob_t), None);
        *bob_conn.mls.lock().unwrap() = Some(bob);

        // 第一次建群：gen=1。
        let (w1, gen1) = alice.dm_init_group(conv, &bob_kp1).unwrap();
        assert_eq!(gen1, 1);
        bob_conn.dm_join_group(conv, &w1, gen1).unwrap();
        assert_eq!(bob_conn.dm_generation(conv), 1);

        let ct = alice.dm_seal(conv, b"hello").unwrap();
        assert_eq!(bob_conn.dm_open(conv, &ct).unwrap(), b"hello");

        // alice 丢失本地 DM 状态后重建：gen=2。
        alice.dm_remove_group(conv);
        assert_eq!(alice.dm_generation(conv), 1);
        let (w2, gen2) = alice.dm_init_group(conv, &bob_kp2).unwrap();
        assert_eq!(gen2, 2);

        // bob 收到 gen=2 的 Welcome 必须采纳，而不是继续保留旧群。
        bob_conn.dm_join_group(conv, &w2, gen2).unwrap();
        assert_eq!(bob_conn.dm_generation(conv), 2);

        let ct2 = alice.dm_seal(conv, b"new-group").unwrap();
        assert_eq!(bob_conn.dm_open(conv, &ct2).unwrap(), b"new-group");
        let ct3 = bob_conn.dm_seal(conv, b"reply").unwrap();
        assert_eq!(alice.dm_open(conv, &ct3).unwrap(), b"reply");
    }
}
