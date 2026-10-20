//! 房间会话：把 HTTP 登录 + WS 连接 + MLS 建组 + 持久化 串成一条路径。

use std::sync::Arc;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use kaede_protocol::UserId;
use kaede_storage::Store;

use crate::ClientEvent;
use crate::ClientObserver;
use crate::connection::ServerConnection;
use crate::error::ClientError;
use crate::http::{HttpApi, LoginReq, RegisterReq};
use crate::transport::WebSocketTransport;

/// 默认 channel（临时，直到频道系统落地）。
pub const DEFAULT_CHANNEL: &str = "general";

/// 每次上传的 KeyPackage 数量。
pub const DEFAULT_KP_BATCH_SIZE: usize = 5;

/// 从历史拉取的一条消息（已解密）。
#[derive(Debug, Clone)]
pub struct HistoryMessage {
    pub sender: String,
    pub ts: i64,
    pub plaintext: String,
    pub attachments: Vec<kaede_protocol::Attachment>,
    pub reply_to: Option<kaede_protocol::ReplyRef>,
}

/// 一个房间会话。
pub struct RoomSession {
    uid: UserId,
    http: HttpApi,
    conn: Arc<ServerConnection>,
    /// 可选的持久化存储。`None` 表示不持久化。
    storage: Option<Arc<Store>>,
    /// MLS 快照在 storage 里的 key。
    storage_key: String,
    /// `<http_url>::<ws_url>`，用于派生 snapshot cipher key。
    server_url: String,
}

impl RoomSession {
    /// 高层封装（不持久化）：登录 / 注册 → WS → 认证 → MLS 初始化。
    pub async fn connect(
        http_url: &str,
        ws_url: &str,
        login_id: &str,
        password: &str,
        observer: Option<Arc<dyn ClientObserver>>,
    ) -> Result<Self, ClientError> {
        Self::connect_with_storage(http_url, ws_url, login_id, password, None, observer).await
    }

    /// 高层封装（可持久化）。
    ///
    /// 1. HTTP 登录（失败则注册）
    /// 2. 若 `storage` 提供，尝试从 `mls/{server}/{uid}` 加载旧快照
    ///    - 加载成功 → 用快照恢复身份 + 群组
    ///    - 加载失败/为空 → 生成新身份
    /// 3. 建立 WebSocket + 认证
    pub async fn connect_with_storage(
        http_url: &str,
        ws_url: &str,
        login_id: &str,
        password: &str,
        storage: Option<Arc<Store>>,
        observer: Option<Arc<dyn ClientObserver>>,
    ) -> Result<Self, ClientError> {
        let http = HttpApi::new(http_url)?;

        let auth = match http
            .login(LoginReq {
                login_id: login_id.into(),
                password: password.into(),
            })
            .await
        {
            Ok(a) => a,
            Err(e) => {
                let err_str = format!("{:?}", e);
                // 只有明确用户不存在（401/404）才尝试注册
                if err_str.contains("401") || err_str.contains("404") {
                    http.register(RegisterReq {
                        login_id: login_id.into(),
                        password: password.into(),
                        nickname: String::new(),
                    })
                    .await?
                } else {
                    return Err(e);
                }
            }
        };

        let uid = UserId::from(auth.login_id.clone());

        let transport = Arc::new(WebSocketTransport::connect(ws_url).await?);
        let server_id = format!("{}::{}", http_url, ws_url);
        let conn = Arc::new(ServerConnection::new(server_id, transport, observer));

        conn.connect_with_token(
            "kaede-client",
            env!("CARGO_PKG_VERSION"),
            Some(auth.session_token),
        )
        .await?;
        conn.authenticate()?;

        let server_url = format!("{}::{}", http_url, ws_url);
        let storage_key = format!("mls/{}/{}", server_url, auth.login_id);

        // 尝试从快照恢复
        let mut restored = false;
        if let Some(store) = &storage {
            match store.get("mls", &storage_key).await {
                Ok(Some(snap)) => {
                    // 按 snapshot 外层头的 alg_id 选 cipher：
                    // - 老明文格式（无 0x03 头）→ NoOpCipher
                    // - AES-GCM（0x03, 0x01）    → fallback SHA256 派生
                    // - DPAPI（0x03, 0x10）      → Windows DPAPI
                    let cipher = crate::keystore::device_cipher_for_snapshot(
                        &auth.login_id,
                        &server_url,
                        &snap,
                    );
                    match conn.mls_restore_from_snapshot_with_cipher(&snap, Some(cipher.as_ref())) {
                        Ok(()) => {
                            tracing::info!(
                                "[room] restored MLS from snapshot ({} bytes)",
                                snap.len()
                            );
                            restored = true;
                        }
                        Err(ClientError::SnapshotIncompatible { alg }) => {
                            // P1-γ: snapshot 是另一个平台创建的，当前环境解不开。
                            // 这不是"损坏"，是"平台不兼容"——必须显式打 ERROR，
                            // 因为后续会 mls_init 生成新身份，用户旧加密历史不可恢复。
                            tracing::error!(
                                "[room] snapshot incompatible (alg_id={:#04x}); \
                                 rebuilding MLS identity — old encrypted history NOT recoverable. \
                                 This happens when a snapshot created on another OS is opened here.",
                                alg
                            );
                        }
                        Err(e) => {
                            tracing::warn!("[room] restore failed, starting fresh: {}", e);
                        }
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!("[room] storage read failed: {}", e);
                }
            }
        }

        if !restored {
            conn.mls_init(uid.clone())?;
        }

        Ok(Self {
            uid,
            http,
            conn,
            storage,
            storage_key,
            server_url,
        })
    }

    pub fn uid(&self) -> &UserId {
        &self.uid
    }

    pub fn connection(&self) -> &Arc<ServerConnection> {
        &self.conn
    }

    /// 用已有 token + storage 建连（UI 场景：登录页已拿到 token）。
    ///
    /// - 若有旧快照：恢复身份 + 群组
    /// - 否则：生成新身份
    pub async fn connect_with_token_and_storage(
        http_url: &str,
        ws_url: &str,
        login_id: &str,
        session_token: &str,
        storage: Option<Arc<Store>>,
        observer: Option<Arc<dyn ClientObserver>>,
    ) -> Result<Self, ClientError> {
        let http = HttpApi::new(http_url)?;
        http.set_token(session_token);

        let uid = UserId::from(login_id);

        let transport = Arc::new(WebSocketTransport::connect(ws_url).await?);
        let server_id = format!("{}::{}", http_url, ws_url);
        let conn = Arc::new(ServerConnection::new(server_id, transport, observer));

        conn.connect_with_token(
            "kaede-client",
            env!("CARGO_PKG_VERSION"),
            Some(session_token.into()),
        )
        .await?;
        conn.authenticate()?;

        let server_url = format!("{}::{}", http_url, ws_url);
        let storage_key = format!("mls/{}/{}", server_url, login_id);

        // 尝试从快照恢复
        let mut restored = false;
        if let Some(store) = &storage {
            match store.get("mls", &storage_key).await {
                Ok(Some(snap)) => {
                    let cipher =
                        crate::keystore::device_cipher_for_snapshot(login_id, &server_url, &snap);
                    match conn.mls_restore_from_snapshot_with_cipher(&snap, Some(cipher.as_ref())) {
                        Ok(()) => {
                            tracing::info!(
                                "[room] restored MLS from snapshot ({} bytes)",
                                snap.len()
                            );
                            restored = true;
                        }
                        Err(ClientError::SnapshotIncompatible { alg }) => {
                            // P1-γ: snapshot 是另一个平台创建的，当前环境解不开。
                            // 这不是"损坏"，是"平台不兼容"——必须显式打 ERROR，
                            // 因为后续会 mls_init 生成新身份，用户旧加密历史不可恢复。
                            tracing::error!(
                                "[room] snapshot incompatible (alg_id={:#04x}); \
                                 rebuilding MLS identity — old encrypted history NOT recoverable. \
                                 This happens when a snapshot created on another OS is opened here.",
                                alg
                            );
                        }
                        Err(e) => {
                            tracing::warn!("[room] restore failed, starting fresh: {}", e);
                        }
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!("[room] storage read failed: {}", e);
                }
            }
        }

        if !restored {
            conn.mls_init(uid.clone())?;
        }

        Ok(Self {
            uid,
            http,
            conn,
            storage,
            storage_key,
            server_url,
        })
    }

    /// 起后台事件循环。
    pub fn spawn_event_loop(&self) {
        let conn = self.conn.clone();
        spawn_detached(conn.run_event_loop());
    }

    // ---------------------------------------------------------------------
    // MLS 生命周期
    // ---------------------------------------------------------------------

    /// 房主：建 MLS 组 + 上传 KeyPackage 队列。
    pub async fn become_host(&self) -> Result<(), ClientError> {
        self.conn.mls_create_group()?;
        self.publish_key_packages().await?;
        self.save_snapshot().await?;
        Ok(())
    }

    /// 新成员：生成 + 上传 KeyPackage 队列。
    pub async fn become_guest(&self) -> Result<(), ClientError> {
        self.publish_key_packages().await?;
        self.save_snapshot().await?;
        Ok(())
    }

    async fn publish_key_packages(&self) -> Result<(), ClientError> {
        let mut kps = Vec::with_capacity(DEFAULT_KP_BATCH_SIZE);
        for _ in 0..DEFAULT_KP_BATCH_SIZE {
            kps.push(self.conn.mls_generate_key_package()?);
        }
        self.http.publish_key_packages(kps).await
    }

    /// 通知服务器"我进房间了"。
    pub async fn join_room(&self) -> Result<(), ClientError> {
        self.conn.join().await
    }

    /// 房主：把某个 peer 加进 MLS 组。
    ///
    /// **P0-5**：添加前检查 sender_id 碰撞（同一 32 位哈希 → AES-GCM nonce 复用 → 灾难）。
    /// **P1-4**：fetch KP / send welcome / send commit 都带指数退避重试；幂等（已在群里则 no-op）。
    /// **P1-5**：碰撞错误信息改为可操作的用户指引（不再建议用户"改 login_id"）。
    /// **P1-15**：本地 epoch 已前进但广播失败 → 返回 Err。
    pub async fn host_add_peer(&self, peer: &UserId) -> Result<(), ClientError> {
        // 0. 幂等：peer 已在 group → 直接 Ok（重复 PeerJoined / 网络重试安全）
        let existing = self.conn.mls_member_uids();
        if existing.iter().any(|u| u == peer.as_str()) {
            tracing::debug!("[host_add_peer] {} already in group, no-op", peer);
            return Ok(());
        }

        // 1. sender_id 碰撞检查
        if let Some(other) = kaede_crypto::find_sender_id_collision(
            peer.as_str(),
            existing.iter().map(String::as_str),
        ) {
            tracing::error!(
                "[host_add_peer] sender_id collision: {} vs existing member {}",
                peer,
                other
            );
            return Err(ClientError::Protocol(format!(
                "Sender ID collision between '{peer}' and existing member '{other}'. \
                 Both hash to the same 32-bit SFrame sender ID, which would cause \
                 catastrophic AES-GCM nonce reuse if added. \
                 Please contact the server administrator to resolve this — the affected \
                 user must register with a different login_id."
            )));
        }

        // 2. 拉 KeyPackage（带重试：网络抖动 / guest 尚未 publish KP 时不要永久失败）
        let kp = self.fetch_kp_with_retry(peer).await?;

        // 3. 本地加成员（本地 epoch 前进）
        let (commit, welcome) = self.conn.mls_add_member(&kp)?;

        // 4. 先发 welcome（新成员必须先收到才能进群）——带重试
        self.send_with_retry("welcome", peer, &welcome).await?;

        // 5. 再发 commit（其他成员 epoch 前进）——带重试
        self.send_with_retry("commit", peer, &commit).await?;

        // 6. 快照保存（best effort）
        if let Err(e) = self.save_snapshot().await {
            tracing::warn!("[host_add_peer] snapshot save failed (non-fatal): {}", e);
        }
        tracing::info!("[host_add_peer] added {} to MLS group", peer);
        Ok(())
    }

    /// P1-4：拉 KP 带指数退避重试（总耗时 ~2.9s）。
    ///
    /// 失败通常是：guest 还没 become_guest 完成（404）、瞬时网络错误。
    async fn fetch_kp_with_retry(&self, peer: &UserId) -> Result<Vec<u8>, ClientError> {
        const DELAYS_MS: &[u64] = &[200, 400, 800, 1500];
        let mut last_err: Option<ClientError> = None;
        for (attempt, delay_ms) in DELAYS_MS.iter().enumerate() {
            match self.http.fetch_key_package(peer).await {
                Ok(kp) => {
                    if attempt > 0 {
                        tracing::info!(
                            "[host_add_peer] fetch KP for {} succeeded on attempt {}",
                            peer,
                            attempt + 1
                        );
                    }
                    return Ok(kp);
                }
                Err(e) => {
                    tracing::warn!(
                        "[host_add_peer] fetch KP for {} failed (attempt {}/{}): {}",
                        peer,
                        attempt + 1,
                        DELAYS_MS.len(),
                        e
                    );
                    last_err = Some(e);
                    tokio::time::sleep(std::time::Duration::from_millis(*delay_ms)).await;
                }
            }
        }
        Err(last_err.expect("fetch_kp_with_retry: DELAYS_MS empty"))
    }

    /// P1-4：广播（welcome / commit）带重试。
    ///
    /// welcome/commit 广播失败 = 本地 epoch 已前进但对方没收到，无法回退，
    /// 必须重试到成功或返回致命错误。
    async fn send_with_retry(
        &self,
        what: &'static str,
        peer: &UserId,
        payload: &[u8],
    ) -> Result<(), ClientError> {
        const DELAYS_MS: &[u64] = &[200, 400, 800, 1500];
        let mut last_err: Option<ClientError> = None;
        for (attempt, delay_ms) in DELAYS_MS.iter().enumerate() {
            let result = match what {
                "welcome" => self.conn.send_welcome(peer.clone(), payload.to_vec()).await,
                "commit" => self.conn.send_commit(payload.to_vec()).await,
                _ => unreachable!("send_with_retry: unknown broadcast type {}", what),
            };
            match result {
                Ok(()) => {
                    if attempt > 0 {
                        tracing::info!(
                            "[host_add_peer] {} to {} succeeded on attempt {}",
                            what,
                            peer,
                            attempt + 1
                        );
                    }
                    return Ok(());
                }
                Err(e) => {
                    tracing::warn!(
                        "[host_add_peer] {} to {} failed (attempt {}/{}): {}",
                        what,
                        peer,
                        attempt + 1,
                        DELAYS_MS.len(),
                        e
                    );
                    last_err = Some(e);
                    tokio::time::sleep(std::time::Duration::from_millis(*delay_ms)).await;
                }
            }
        }
        let err = last_err.expect("send_with_retry: DELAYS_MS empty");
        tracing::error!(
            "[host_add_peer] {} to {} gave up after {} attempts: {}",
            what,
            peer,
            DELAYS_MS.len(),
            err
        );
        self.conn.emit_local_event(ClientEvent::MlsSyncError {
            server_id: self.conn.server_id().to_string(),
            user_message: format!(
                "Failed to broadcast {} after {} retries: MLS epoch advanced locally but peers are out of sync. \
                 Please reconnect to the server.",
                what,
                DELAYS_MS.len()
            ),
            recoverable: false,
            detail: err.to_string(),
        });
        Err(ClientError::Protocol(format!(
            "{} broadcast to {} failed after {} retries; local MLS epoch advanced but peers out of sync: {}",
            what,
            peer,
            DELAYS_MS.len(),
            err
        )))
    }
    // ---------------------------------------------------------------------
    // 持久化
    // ---------------------------------------------------------------------

    /// 序列化整个 MLS 状态（明文，含签名私钥）。
    ///
    /// 兼容 API：保留公开签名供旧调用方使用。新代码请用 `save_snapshot()`。
    #[allow(deprecated)]
    pub fn snapshot(&self) -> Result<Vec<u8>, ClientError> {
        self.conn.mls_snapshot()
    }

    /// 保存快照到 storage（如果有）。
    ///
    /// **加密后存储** —— 用当前平台的 snapshot cipher：
    /// - Windows → DPAPI（`CryptProtectData`）
    /// - 其它平台 → SHA256 派生 key + AES-256-GCM
    ///
    /// snapshot 里含签名私钥，明文存储等于身份泄漏。
    pub async fn save_snapshot(&self) -> Result<(), ClientError> {
        let Some(store) = &self.storage else {
            return Ok(());
        };
        let cipher = crate::keystore::device_cipher(self.uid.as_str(), &self.server_url);
        let snap = self.conn.mls_snapshot_with_cipher(cipher.as_ref())?;
        store
            .put("mls", &self.storage_key, &snap)
            .await
            .map_err(|e| ClientError::Storage(e.to_string()))?;
        Ok(())
    }

    /// 手动删除快照（例如登出时）。
    pub async fn clear_snapshot(&self) -> Result<(), ClientError> {
        if let Some(store) = &self.storage {
            store
                .delete("mls", &self.storage_key)
                .await
                .map_err(|e| ClientError::Storage(e.to_string()))?;
        }
        Ok(())
    }

    // ---------------------------------------------------------------------
    // 消息
    // ---------------------------------------------------------------------

    /// 加密 → 广播 → 保存到服务端历史。
    pub async fn send_text(&self, text: &str) -> Result<(), ClientError> {
        self.send_text_to(DEFAULT_CHANNEL, text).await
    }

    /// 指定 channel。
    pub async fn send_text_to(&self, channel_id: &str, text: &str) -> Result<(), ClientError> {
        self.send_text_with_attachments(channel_id, text, &[]).await
    }

    /// 从某个 ts 之前拉历史（用于分页加载）。
    pub async fn load_history_before(
        &self,
        channel_id: &str,
        before_ts: i64,
        limit: usize,
    ) -> Result<Vec<HistoryMessage>, ClientError> {
        let records = self
            .http
            .load_messages(channel_id, Some(before_ts), limit)
            .await?;
        let mut out = Vec::with_capacity(records.len());
        for r in records {
            let ct = match base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(&r.ciphertext) {
                Ok(c) => c,
                Err(_) => continue,
            };
            if let Ok(pt) = self.conn.mls_decrypt(&ct) {
                match crate::payload::decode(&pt) {
                    crate::payload::DecodedPayload::Message(p) => {
                        out.push(HistoryMessage {
                            sender: r.sender,
                            ts: r.ts,
                            plaintext: p.t,
                            attachments: p.a,
                            reply_to: p.r,
                        });
                    }
                    crate::payload::DecodedPayload::Raw(b) => {
                        out.push(HistoryMessage {
                            sender: r.sender,
                            ts: r.ts,
                            plaintext: String::from_utf8_lossy(&b).to_string(),
                            attachments: Vec::new(),
                            reply_to: None,
                        });
                    }
                    _ => {}
                }
            }
        }
        Ok(out)
    }

    /// 拉历史 → MLS 解密。返回按时间升序。
    pub async fn load_history(
        &self,
        channel_id: &str,
        limit: usize,
    ) -> Result<Vec<HistoryMessage>, ClientError> {
        let records = self.http.load_messages(channel_id, None, limit).await?;
        let mut out = Vec::with_capacity(records.len());
        for r in records {
            let ct = match URL_SAFE_NO_PAD.decode(&r.ciphertext) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("[room] bad base64 at ts={}: {}", r.ts, e);
                    continue;
                }
            };
            // 私密频道消息：首字节 0x03 → channel group AES-GCM
            let decrypted: Option<Vec<u8>> = if ct.first() == Some(&0x03) {
                match self.conn.channel_export_key(channel_id) {
                    Ok(key) => kaede_crypto::dm_cipher::open(&key, channel_id, &ct[1..]).ok(),
                    Err(_) => None,
                }
            } else {
                self.conn.mls_decrypt(&ct).ok()
            };
            match decrypted {
                Some(pt) => {
                    match crate::payload::decode(&pt) {
                        crate::payload::DecodedPayload::Message(p) => {
                            out.push(HistoryMessage {
                                sender: r.sender,
                                ts: r.ts,
                                plaintext: p.t,
                                attachments: p.a,
                                reply_to: p.r,
                            });
                        }
                        crate::payload::DecodedPayload::Reaction(_) => {
                            // 反应不入历史
                        }
                        crate::payload::DecodedPayload::Raw(b) => {
                            out.push(HistoryMessage {
                                sender: r.sender,
                                ts: r.ts,
                                plaintext: String::from_utf8_lossy(&b).to_string(),
                                attachments: Vec::new(),
                                reply_to: None,
                            });
                        }
                        // 媒体信令不入历史（只广播）
                        crate::payload::DecodedPayload::MediaMeta(_) => {}
                    }
                }
                None => {
                    tracing::debug!("[room] history decrypt skipped at ts={}", r.ts);
                }
            }
        }
        Ok(out)
    }

    // ---------------------------------------------------------------------
    // 频道 / 成员 API（HTTP 代理）
    // ---------------------------------------------------------------------

    /// 列出频道。
    pub async fn list_channels(&self) -> Result<Vec<kaede_protocol::Channel>, ClientError> {
        self.http.list_channels().await
    }

    /// 创建频道（需 op3+）。
    pub async fn create_channel(
        &self,
        name: &str,
        kind: &str,
    ) -> Result<kaede_protocol::Channel, ClientError> {
        self.http.create_channel(name, kind).await
    }

    /// 创建私密频道（B 方案：服务端准入 + 客户端 MLS 隔离）。
    pub async fn create_channel_with(
        &self,
        name: &str,
        kind: &str,
        is_private: bool,
        password: Option<&str>,
        hash_tier: Option<&str>,
    ) -> Result<kaede_protocol::Channel, ClientError> {
        let ch = self
            .http
            .create_channel_with(name, kind, is_private, password, hash_tier)
            .await?;
        if is_private {
            if let Err(e) = self.conn.channel_ensure_group(&ch.id) {
                tracing::warn!("channel_ensure_group failed for {}: {}", ch.id, e);
            }
        }
        Ok(ch)
    }

    /// 加入私密频道（B2 / A1）。
    ///
    /// 两种场景：
    /// - **已有本地 group**（之前加入过）：只验证密码，不重新走 Welcome
    /// - **没有本地 group**（首次加入）：验证密码 + KP + Welcome 交换
    pub async fn join_channel(&self, channel_id: &str, password: &str) -> Result<(), ClientError> {
        // 已有 group：只验证密码（server 会先验密码，通过后 204 幂等返回）
        if self.conn.channel_has_group(channel_id) {
            self.http.join_channel(channel_id, password, None).await?;
            tracing::info!(
                "[room] join_channel {} verified password (existing group)",
                channel_id
            );
            return Ok(());
        }

        // 首次加入：走完整 Welcome 交换
        let rx = self.conn.channel_register_pending(channel_id);
        let kp = self.conn.mls_generate_key_package()?;
        match self
            .http
            .join_channel(channel_id, password, Some(&kp))
            .await
        {
            Ok(()) => {}
            Err(ClientError::Http { status: 409, .. }) => {
                // P1: 单人频道且本地 group 丢失，server 返回 409 表示"无法补发 Welcome"
                // → 自己重建 group（用当前 KP 重新 create_group）
                tracing::info!(
                    "[room] join_channel {} solo channel rebuild (409)",
                    channel_id
                );
                self.conn.channel_ensure_group(channel_id)?;
                return Ok(());
            }
            Err(e) => return Err(e),
        }

        tracing::info!(
            "[room] join_channel {} KP submitted, waiting for welcome",
            channel_id
        );

        // 10 秒超时：如果 server 认为已加入（204 幂等返回）或无人发 Welcome，
        // 就不会有 ChannelSync 到达。此时必须有超时，否则 UI 卡"加入中..."
        match tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await {
            Ok(Ok(Ok(_welcome))) => {
                tracing::info!("[room] join_channel {} completed via welcome", channel_id);
                Ok(())
            }
            Ok(Ok(Err(e))) => Err(ClientError::Protocol(format!("welcome failed: {}", e))),
            Ok(Err(_)) => Err(ClientError::Protocol("welcome channel closed".into())),
            Err(_timeout) => {
                tracing::warn!(
                    "[room] join_channel {} timeout waiting welcome; \
                     group state lost, no online member to re-add",
                    channel_id
                );
                Err(ClientError::Protocol(
                    "加入超时：本地 group 已丢失，当前无其他在线成员能补发 Welcome。\
                     请联系频道其他成员上线后重试，或删除该频道重建。"
                        .into(),
                ))
            }
        }
    }

    /// 离开频道。
    pub async fn leave_channel(&self, channel_id: &str) -> Result<(), ClientError> {
        self.http.leave_channel(channel_id).await
    }

    /// 改频道。
    pub async fn update_channel(
        &self,
        id: &str,
        patch: crate::http::UpdateChannelReq,
    ) -> Result<kaede_protocol::Channel, ClientError> {
        self.http.update_channel(id, patch).await
    }

    /// 删频道。
    pub async fn delete_channel(&self, id: &str) -> Result<(), ClientError> {
        self.http.delete_channel(id).await
    }

    /// 列出成员。
    pub async fn list_members(&self) -> Result<Vec<kaede_protocol::Member>, ClientError> {
        self.http.list_members().await
    }

    /// 改成员。
    pub async fn update_member(
        &self,
        login_id: &str,
        patch: crate::http::UpdateMemberReq,
    ) -> Result<kaede_protocol::Member, ClientError> {
        self.http.update_member(login_id, patch).await
    }

    /// 踢人（需 op4+）。
    ///
    /// 两个动作：
    /// 1. **HTTP**：服务器删账号 + 广播 `MemberRemoved`
    /// 2. **MLS**：本地从 group 移除该成员 leaf，广播 commit，
    ///    使所有成员的 epoch 前进 → SFrame key 轮换
    ///
    /// 步骤 1 是强制的；步骤 2 是**尽最大努力**——
    /// 如果被踢者不在 MLS group 里（例如刚注册没进群），
    /// MLS 移除会失败，但踢人本身仍视为成功。
    /// 踢人（需 op4+）。
    ///
    /// **P0-6**：顺序调整 + 严格错误传播。原实现"先 HTTP 后 MLS + 全吞错误"
    /// 会导致 MLS 失败时房间永久分裂。新实现：
    /// 1. MLS 本地移除 + 广播 commit（**必须成功**，否则返回 Err）
    /// 2. HTTP 删号（失败可重试，MLS 已踢人）
    /// 3. 快照保存（best effort）
    pub async fn remove_member(&self, login_id: &str) -> Result<(), ClientError> {
        // C-10: 禁止踢自己（否则本地从 MLS 移出自己 + 删除自己账号，但客户端仍在运行）
        if login_id == self.uid.as_str() {
            return Err(ClientError::Protocol(
                "cannot kick yourself; use disconnect instead".into(),
            ));
        }

        // P0-3: 顺序调整——先 HTTP 删号（安全必须成功），再 MLS 移除（尽力而为）。
        // 旧顺序「先 MLS 后 HTTP」在 MLS 广播失败时 return Err，跳过 HTTP 删号，
        // 导致被踢者 session 仍然有效，可以继续通过 HTTP 读频道历史 / 写消息。

        // 1. HTTP 删号（安全必须成功）
        self.http.remove_member(login_id).await?;

        // 2. MLS 移除 + 广播（尽力而为，失败不阻塞踢人）
        if let Ok(commit) = self.conn.mls_remove_member_by_uid(login_id) {
            if let Err(e) = self.conn.send_commit(commit).await {
                tracing::error!("[kick] MLS commit broadcast failed after HTTP kick: {}", e);
                self.conn.emit_local_event(ClientEvent::MlsSyncError {
                    server_id: self.conn.server_id().to_string(),
                    user_message: format!(
                        "踢人 {} 已成功，但 MLS commit 广播失败，其他成员可能仍在旧 epoch。请离开语音频道重进，或重连服务器。",
                        login_id
                    ),
                    recoverable: true,
                    detail: e.to_string(),
                });
            }
        } else {
            tracing::warn!(
                "[kick] MLS remove failed for {} (user may not be in group), HTTP kick succeeded",
                login_id
            );
        }

        // 3. 快照保存（best effort）
        if let Err(e) = self.save_snapshot().await {
            tracing::warn!("[kick] snapshot save failed (non-fatal): {}", e);
        }

        tracing::info!("[kick] {} removed via HTTP + MLS (best effort)", login_id);
        Ok(())
    }

    // ---------------------------------------------------------------------
    // 附件
    // ---------------------------------------------------------------------

    /// 上传附件。
    /// 按 target_id 派生附件加密 key。
    fn resolve_file_key(&self, target_id: &str) -> Result<[u8; 32], ClientError> {
        if target_id.starts_with("dm:") {
            self.conn
                .dm_export_key_for(target_id, kaede_mls::SframePurpose::File)
        } else if self.conn.channel_has_group(target_id) {
            self.conn
                .channel_export_key_for(target_id, kaede_mls::SframePurpose::File)
        } else {
            self.conn
                .mls_export_sframe_key_for(target_id, kaede_mls::SframePurpose::File)
        }
    }

    /// 上传附件（P0：客户端加密，server 只存密文）。
    ///
    /// 加密格式：[0x04][nonce:12][ciphertext||tag]
    /// AAD = target_id（防密文跨频道重放）。
    pub async fn upload_attachment(
        &self,
        target_id: &str,
        name: &str,
        mime: Option<&str>,
        bytes: Vec<u8>,
    ) -> Result<kaede_protocol::Attachment, ClientError> {
        let key = self.resolve_file_key(target_id)?;
        let nonce: [u8; 12] = kaede_crypto::random_bytes();
        let ct = kaede_crypto::aes256gcm_encrypt(&key, &nonce, target_id.as_bytes(), &bytes)
            .map_err(|e| ClientError::Protocol(format!("attachment encrypt: {e}")))?;
        let mut wrapped = Vec::with_capacity(1 + 12 + ct.len());
        wrapped.push(0x04);
        wrapped.extend_from_slice(&nonce);
        wrapped.extend_from_slice(&ct);
        self.http.upload_attachment(name, mime, wrapped).await
    }

    /// 下载附件（P0：客户端解密；兼容旧明文附件）。
    pub async fn download_attachment(
        &self,
        target_id: &str,
        id: &str,
    ) -> Result<Vec<u8>, ClientError> {
        let raw = self.http.download_attachment(id).await?;
        if raw.is_empty() || raw[0] != 0x04 {
            return Ok(raw);
        }
        if raw.len() < 1 + 12 + 16 {
            return Err(ClientError::Protocol(format!(
                "attachment too short: {} bytes",
                raw.len()
            )));
        }
        let key = self.resolve_file_key(target_id)?;
        let nonce: &[u8; 12] = raw[1..13].try_into().unwrap();
        let ct = &raw[13..];
        kaede_crypto::aes256gcm_decrypt(&key, nonce, target_id.as_bytes(), ct)
            .map_err(|e| ClientError::Protocol(format!("attachment decrypt: {e}")))
    }

    /// 发送一条带回复的消息。
    pub async fn send_text_with_reply(
        &self,
        channel_id: &str,
        text: &str,
        reply_to: kaede_protocol::ReplyRef,
    ) -> Result<(), ClientError> {
        let payload = crate::payload::MessagePayload::new(
            channel_id,
            self.uid.as_str(),
            text,
            Vec::new(),
            Some(reply_to),
        );
        let bytes = payload
            .to_bytes()
            .map_err(|e| ClientError::Protocol(e.to_string()))?;
        let ciphertext = self.conn.mls_send_app(&bytes).await?;

        if let Err(e) = self
            .http
            .post_message(channel_id, now_ms(), self.uid.as_str(), &ciphertext)
            .await
        {
            tracing::warn!("post_message: {}", e);
        }
        Ok(())
    }

    /// 发送一条带附件的消息（附件元数据打包进 MLS payload）。
    ///
    /// payload 格式（明文，MLS 加密后传输）：
    /// ```json
    /// { "c": "<channel>", "u": "<uid>", "t": "<text>", "a": [<Attachment>...] }
    /// ```
    pub async fn send_text_with_attachments(
        &self,
        channel_id: &str,
        text: &str,
        attachments: &[kaede_protocol::Attachment],
    ) -> Result<(), ClientError> {
        let payload = crate::payload::MessagePayload::new(
            channel_id,
            self.uid.as_str(),
            text,
            attachments.to_vec(),
            None,
        );
        let bytes = payload
            .to_bytes()
            .map_err(|e| ClientError::Protocol(e.to_string()))?;

        // 私密频道：走 channel group 的 AES-GCM key（首字节 0x03 标记）
        let ciphertext = if self.conn.channel_has_group(channel_id) {
            let key = self.conn.channel_export_key(channel_id)?;
            let sealed = kaede_crypto::dm_cipher::seal(&key, channel_id, &bytes)
                .map_err(|e| ClientError::Protocol(format!("channel seal: {e}")))?;
            let mut out = Vec::with_capacity(1 + sealed.len());
            out.push(0x03u8);
            out.extend_from_slice(&sealed);
            out
        } else {
            self.conn.mls_send_app(&bytes).await?
        };
        if let Err(e) = self
            .http
            .post_message(channel_id, now_ms(), self.uid.as_str(), &ciphertext)
            .await
        {
            tracing::warn!("post_message: {}", e);
        }
        Ok(())
    }

    /// 发送表情回复（独立 MLS 消息）。
    pub async fn send_reaction(
        &self,
        channel_id: &str,
        target_ts: i64,
        emoji: &str,
        action: &str, // "add" | "remove"
    ) -> Result<(), ClientError> {
        let payload = crate::payload::ReactionPayload::new(
            channel_id,
            self.uid.as_str(),
            target_ts,
            emoji,
            action,
        );
        let bytes = payload
            .to_bytes()
            .map_err(|e| ClientError::Protocol(e.to_string()))?;
        let _ = self.conn.mls_send_app(&bytes).await?;
        // reaction 不入历史（只广播）
        Ok(())
    }

    // ---- DM ----

    pub async fn list_dm_conversations(
        &self,
    ) -> Result<Vec<crate::http::DmConversation>, ClientError> {
        self.http.list_dm_conversations().await
    }

    pub async fn open_dm(&self, peer: &str) -> Result<crate::http::DmConversation, ClientError> {
        self.http.open_dm(peer).await
    }

    pub async fn dm_load_history(
        &self,
        conv_id: &str,
        before_ts: Option<i64>,
        limit: usize,
    ) -> Result<Vec<HistoryMessage>, ClientError> {
        let records = self.http.dm_messages(conv_id, before_ts, limit).await?;
        let mut out = Vec::with_capacity(records.len());
        for r in records {
            let ct = match base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(&r.ciphertext) {
                Ok(c) => c,
                Err(_) => continue,
            };
            // B2：Welcome envelope（首字节 0x02）→ 加入独立 group，不入 UI 消息列表
            if ct.first() == Some(&0x02) {
                if let Err(e) = self.conn.dm_join_group(conv_id, &ct[1..]) {
                    tracing::warn!("dm_join_group failed for {}: {}", conv_id, e);
                }
                continue;
            }
            #[allow(clippy::collapsible_if)]
            if let Ok(pt) = self.conn.dm_open(conv_id, &ct) {
                if let crate::payload::DecodedPayload::Message(p) = crate::payload::decode(&pt) {
                    out.push(HistoryMessage {
                        sender: r.sender,
                        ts: r.ts,
                        plaintext: p.t,
                        attachments: p.a,
                        reply_to: p.r,
                    });
                }
            }
        }
        Ok(out)
    }

    /// 发送 DM 消息。
    pub async fn send_dm(&self, conv_id: &str, text: &str) -> Result<(), ClientError> {
        // B2：本地若无 DM group，先拉对端 KP + 建 2 人 group + 发 Welcome
        if !self.conn.dm_has_group(conv_id) {
            self.dm_ensure_group(conv_id).await?;
        }

        let payload =
            crate::payload::MessagePayload::new(conv_id, self.uid.as_str(), text, Vec::new(), None);
        let bytes = payload
            .to_bytes()
            .map_err(|e| ClientError::Protocol(e.to_string()))?;

        // B3：per-conv AEAD 加密
        let ciphertext = self.conn.dm_seal(conv_id, &bytes)?;
        let ts = now_ms();

        // 落库（HTTP）
        if let Err(e) = self
            .http
            .dm_post_message(conv_id, ts, self.uid.as_str(), &ciphertext)
            .await
        {
            tracing::warn!("dm_post_message: {}", e);
            return Err(e);
        }

        // 本地回显（服务器只推给对方，不推回自己）
        self.conn.emit_local_event(ClientEvent::DmReceived {
            server_id: self.conn.server_id().to_string(),
            conv_id: conv_id.to_string(),
            from: self.uid.clone(),
            ts,
            plaintext: bytes,
        });
        Ok(())
    }

    /// B2：确保本地存在 conv 对应的 2 人 MLS group；若不存在，发 Welcome。
    async fn dm_ensure_group(&self, conv_id: &str) -> Result<(), ClientError> {
        let peer = conv_id
            .strip_prefix("dm:")
            .and_then(|r| r.split_once(':'))
            .map(|(a, b)| if a == self.uid.as_str() { b } else { a })
            .ok_or_else(|| ClientError::Protocol(format!("invalid conv_id: {}", conv_id)))?;
        let peer_uid = UserId::from(peer.to_string());
        let peer_kp = self.http.fetch_key_package(&peer_uid).await?;

        let welcome = self.conn.dm_init_group(conv_id, &peer_kp)?;
        let ts = now_ms();
        let mut envelope = Vec::with_capacity(1 + welcome.len());
        envelope.push(0x02u8);
        envelope.extend_from_slice(&welcome);

        if let Err(e) = self
            .http
            .dm_post_message(conv_id, ts, self.uid.as_str(), &envelope)
            .await
        {
            // Welcome 发送失败：回滚，避免下次 dm_seal 出乱子
            self.conn.dm_remove_group(conv_id);
            return Err(e);
        }
        Ok(())
    }

    /// 设置在线状态。
    pub async fn set_presence(&self, status: &str) -> Result<(), ClientError> {
        self.conn.set_presence(status).await
    }

    // ---- 好友 ----

    pub async fn list_friends(&self) -> Result<Vec<crate::http::Friend>, ClientError> {
        self.http.list_friends().await
    }
    pub async fn send_friend_request(&self, login_id: &str) -> Result<(), ClientError> {
        self.http.send_friend_request(login_id).await
    }
    pub async fn accept_friend(&self, from_login_id: &str) -> Result<(), ClientError> {
        self.http.accept_friend(from_login_id).await
    }
    pub async fn remove_friend(&self, login_id: &str) -> Result<(), ClientError> {
        self.http.remove_friend(login_id).await
    }

    /// 拿 LiveKit 接入 token。
    pub async fn get_livekit_token(
        &self,
        channel_id: &str,
    ) -> Result<crate::http::LivekitToken, ClientError> {
        self.http.get_livekit_token(channel_id).await
    }

    /// 拿 LiveKit E2EE 共享密钥（32 字节 AES-256）。
    ///
    /// **DEPRECATED (P0-Z)**：走 server HTTP，server 可推导 key，且 key 确定性。
    /// 服务端已返回 410 Gone。请改用 [`Self::begin_media_session`]。
    #[deprecated(
        since = "0.1.0",
        note = "server endpoint returns 410 Gone; use RoomSession::begin_media_session instead"
    )]
    pub async fn get_livekit_e2ee_key(&self, channel_id: &str) -> Result<[u8; 32], ClientError> {
        #[allow(deprecated)]
        self.http.get_e2ee_key(channel_id).await
    }

    // ---------------------------------------------------------------------
    // MLS 状态查询
    // ---------------------------------------------------------------------

    pub fn epoch(&self) -> u64 {
        self.conn.mls_epoch()
    }

    pub fn member_count(&self) -> usize {
        self.conn.mls_member_count()
    }

    pub fn is_mls_ready(&self) -> bool {
        self.conn.mls_is_ready()
    }

    /// 从 MLS exporter 派生 channel 的 SFrame 密钥（旧 API，= Default purpose）。
    pub fn export_sframe_key(&self, channel_id: &str) -> Result<[u8; 32], ClientError> {
        self.conn.mls_export_sframe_key(channel_id)
    }

    /// 从 MLS exporter 派生 channel 的 SFrame 密钥（带 purpose）。
    pub fn export_sframe_key_for(
        &self,
        channel_id: &str,
        purpose: kaede_mls::SframePurpose,
    ) -> Result<[u8; 32], ClientError> {
        self.conn.mls_export_sframe_key_for(channel_id, purpose)
    }

    /// 语音 — 麦克风 key。
    pub fn export_voice_audio_key(&self, channel_id: &str) -> Result<[u8; 32], ClientError> {
        self.export_sframe_key_for(channel_id, kaede_mls::SframePurpose::VoiceAudio)
    }

    /// 语音 — 摄像头 key。
    pub fn export_voice_video_key(&self, channel_id: &str) -> Result<[u8; 32], ClientError> {
        self.export_sframe_key_for(channel_id, kaede_mls::SframePurpose::VoiceVideo)
    }

    /// 屏幕共享 — 画面 key。
    pub fn export_screen_video_key(&self, channel_id: &str) -> Result<[u8; 32], ClientError> {
        self.export_sframe_key_for(channel_id, kaede_mls::SframePurpose::ScreenVideo)
    }

    /// 屏幕共享 — 系统声音 key。
    /// 派生 LiveKit E2EE 密钥（从 MLS exporter，server 不参与）。
    ///
    /// 这是 B 路线（真 E2EE）的入口。老的 `get_livekit_e2ee_key` 走
    /// server HTTP，server 可解密；这个 key 只有 MLS 组成员能算出来。
    pub fn export_livekit_e2ee_key(&self, channel_id: &str) -> Result<[u8; 32], ClientError> {
        self.export_sframe_key_for(channel_id, kaede_mls::SframePurpose::LivekitE2ee)
    }

    /// 进入媒体会话：拿 token + 强制 epoch 前进 + 用房间级 seed 派生 e2ee key。
    ///
    /// 返回 `(token, e2ee_key)`：
    /// - `token` 用于 `ConnectConfig`
    /// - `e2ee_key` 用于 `KeyProvider::with_shared_key`
    ///
    /// # 安全性质（P0-Z + P0-2 联合保证）
    ///
    /// 1. **每次会话 epoch 前进**（`mls_self_update`）：同一用户离开再进
    ///    语音频道 → epoch 不同 → key 不同 → LiveKit SDK 的 PBKDF2 ratchet
    ///    即使从零重启也不会造成 AES-GCM nonce 复用。
    /// 2. **房间级 seed 对齐**：同房间所有客户端从 server `/api/livekit/token`
    ///    拿到相同 seed，并且在同一 epoch 下（commit 已同步）→ 派生出相同
    ///    key → 多人语音能通。
    /// 3. **server 不知道 exporter 输出**：seed 只是 exporter context 的一部分，
    ///    exporter 结果由 MLS group state 决定，server 无法推导 → 仍是 E2EE。
    ///
    /// # 失败处理
    ///
    /// - token 获取失败 → 网络/权限问题，可直接重试
    /// - seed 缺失或长度错误 → server 版本太老，需升级
    /// - `mls_self_update` 失败 → 本地状态异常（如 guest 还没被 host add 进组），
    ///   需等 host 拉你进群后才能开始媒体会话
    /// - commit 广播失败 → **返回错**。本地 epoch 已前进但其他成员收不到 commit，
    ///   房间状态分裂。调用方必须提示用户"离开语音频道并重进"。这是 P0-Z 的
    ///   安全代价：宁可会话失败，也不能让 nonce 复用。
    pub async fn begin_media_session(
        &self,
        channel_id: &str,
    ) -> Result<(crate::http::LivekitToken, [u8; 32]), ClientError> {
        // 1. 拿 token（含房间级 seed）
        let token = self.http.get_livekit_token(channel_id).await?;
        let seed_b64 = token.e2ee_seed.as_deref().ok_or_else(|| {
            ClientError::Protocol(
                "server did not return e2ee_seed; please upgrade the server".into(),
            )
        })?;
        let seed = URL_SAFE_NO_PAD
            .decode(seed_b64)
            .map_err(|e| ClientError::Protocol(format!("bad e2ee_seed base64: {}", e)))?;
        if seed.len() != 32 {
            return Err(ClientError::Protocol(format!(
                "e2ee_seed length {} != 32",
                seed.len()
            )));
        }

        // B2 Phase 2：DM 走 dm group
        if self.conn.dm_has_group(channel_id) {
            let key = self.conn.dm_export_media_key(channel_id, &seed)?;
            tracing::info!("[begin_media_session] DM {} -> dm group key", channel_id);
            return Ok((token, key));
        }

        // B2 Phase 1：私密频道走 channel group，公开频道走主群
        if self.conn.channel_has_group(channel_id) {
            // TODO Phase 1.5：加 channel_self_update 强制 epoch 前进
            // 当前不做 self_update —— 避免多人各自前进导致 epoch 不同步、key 不一致。
            // 同 epoch 内 seed 由 server 房间级生成，key 稳定；LiveKit ratchet 保证帧序号递增。
            let key = self.conn.channel_export_media_key(channel_id, &seed)?;
            tracing::info!(
                "[begin_media_session] private channel {} -> channel group key",
                channel_id
            );
            return Ok((token, key));
        }

        // 公开频道：原逻辑（主群 + epoch 前进 + seed）
        let commit = self.conn.mls_self_update()?;

        if let Err(e) = self.conn.send_commit(commit).await {
            tracing::error!(
                "[begin_media_session] commit broadcast failed after local epoch advanced: {}",
                e
            );
            return Err(ClientError::Protocol(format!(
                "MLS commit broadcast failed; local epoch advanced but peers out of sync. \
                 Leave and rejoin voice channel to recover. err: {}",
                e
            )));
        }

        let key = self.conn.mls_export_sframe_key_for_with_extra(
            channel_id,
            kaede_mls::SframePurpose::LivekitE2ee,
            &seed,
        )?;

        Ok((token, key))
    }

    pub fn export_screen_audio_key(&self, channel_id: &str) -> Result<[u8; 32], ClientError> {
        self.export_sframe_key_for(channel_id, kaede_mls::SframePurpose::ScreenAudio)
    }

    /// 本用户的 SFrame sender_id（从 uid 派生，跨设备一致）。
    ///
    /// 喂给 `MediaSession::start` / `AudioSession::start` 的 `sender_id` 参数。
    /// 同一个 channel 里不同用户拿到不同 sender_id，counter 空间不重叠，
    /// 消除 AES-GCM nonce 复用。
    pub fn sender_id(&self) -> u32 {
        kaede_crypto::sframe::sender_id_from_uid(self.uid.as_str())
    }

    // ---------------------------------------------------------------------
    // 媒体信令（MLS Application Message 广播）
    // ---------------------------------------------------------------------

    /// 广播「开始媒体会话」信令。
    ///
    /// 接收端从 `ClientEvent::MediaMetaReceived` 拿到，
    /// 知道该用哪个 purpose / codec / 采样率解码后续 SFrame 帧。
    ///
    /// **不入服务端历史**（只广播）——信令是短生命周期的。
    #[allow(clippy::too_many_arguments)]
    pub async fn publish_media_meta(
        &self,
        channel_id: &str,
        purpose: kaede_mls::SframePurpose,
        codec: &str,
        sample_rate: Option<u32>,
        channels: Option<u16>,
    ) -> Result<(), ClientError> {
        let payload = crate::payload::MediaMetaPayload::start(
            channel_id,
            self.uid.as_str(),
            purpose.as_str(),
            codec,
            self.epoch() as u32,
            self.sender_id(),
            sample_rate,
            channels,
        );
        let bytes = payload
            .to_bytes()
            .map_err(|e| ClientError::Protocol(e.to_string()))?;
        let _ = self.conn.mls_send_app(&bytes).await?;
        Ok(())
    }

    /// 广播「停止媒体会话」信令。
    pub async fn publish_media_stop(
        &self,
        channel_id: &str,
        purpose: kaede_mls::SframePurpose,
    ) -> Result<(), ClientError> {
        let payload =
            crate::payload::MediaMetaPayload::stop(channel_id, self.uid.as_str(), purpose.as_str());
        let bytes = payload
            .to_bytes()
            .map_err(|e| ClientError::Protocol(e.to_string()))?;
        let _ = self.conn.mls_send_app(&bytes).await?;
        Ok(())
    }
}

// ============================================================================
// 平台无关 spawn
// ============================================================================

#[cfg(not(target_arch = "wasm32"))]
fn spawn_detached<F>(fut: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    tokio::spawn(fut);
}

#[cfg(target_arch = "wasm32")]
fn spawn_detached<F>(fut: F)
where
    F: std::future::Future<Output = ()> + 'static,
{
    wasm_bindgen_futures::spawn_local(fut);
}

/// 拿当前用户的 snapshot cipher（平台相关）。
///
/// - Windows → DPAPI
/// - 其它平台 → SHA256 派生 key + AES-256-GCM
///
/// 兼容说明：老 snapshot（AES-GCM）恢复时**不应**用此函数，而应用
/// [`crate::keystore::device_cipher_for_snapshot`]，让 `alg_id` 决定 cipher。
pub fn device_cipher(uid: &str, server_url: &str) -> Box<dyn kaede_mls::SnapshotCipher> {
    crate::keystore::device_cipher(uid, server_url)
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

// ============================================================================
// 平台默认 store
// ============================================================================

/// 打开当前平台的默认 store。
///
/// - **native**：SQLite 文件（路径由 `path` 指定）
/// - **wasm**：IndexedDB（名字由 `path` 指定，忽略目录语义）
pub async fn open_default_store(path: &str) -> Result<Arc<Store>, ClientError> {
    #[cfg(target_arch = "wasm32")]
    {
        let s = kaede_storage::IndexedDbStore::open(path)
            .await
            .map_err(|e| ClientError::Storage(e.to_string()))?;
        Ok(Arc::new(Store::IndexedDb(s)))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let url = format!("sqlite://{}?mode=rwc", path);
        let s = kaede_storage::SqliteStore::connect(&url)
            .await
            .map_err(|e| ClientError::Storage(e.to_string()))?;
        Ok(Arc::new(Store::Sqlite(s)))
    }
}
