//! 房间会话：把 HTTP 登录 + WS 连接 + MLS 建组 + 持久化 串成一条路径。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use kaede_protocol::UserId;
use kaede_storage::Store;

use crate::ClientEvent;
use crate::ClientObserver;
use crate::connection::{AuxStorageHook, ServerConnection};
use crate::error::ClientError;
use crate::http::{HttpApi, LoginReq, RegisterReq};
use crate::transport::WebSocketTransport;

fn is_retryable_kp_error(e: &ClientError) -> bool {
    match e {
        ClientError::Http { status, .. } => {
            matches!(status, 404 | 408 | 429) || (500..=599).contains(status)
        }
        ClientError::Transport(_) | ClientError::Connection(_) | ClientError::Timeout => true,
        _ => false,
    }
}

#[cfg(not(target_arch = "wasm32"))]
async fn sleep_ms(ms: u64) {
    tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
}
#[cfg(target_arch = "wasm32")]
async fn sleep_ms(ms: u64) {
    gloo_timers::future::TimeoutFuture::new(ms as u32).await;
}

#[cfg(not(target_arch = "wasm32"))]
async fn timeout_recv<T>(
    dur: std::time::Duration,
    fut: impl std::future::Future<Output = T>,
) -> Result<T, ()> {
    tokio::time::timeout(dur, fut).await.map_err(|_| ())
}
#[cfg(target_arch = "wasm32")]
async fn timeout_recv<T>(
    dur: std::time::Duration,
    fut: impl std::future::Future<Output = T>,
) -> Result<T, ()> {
    futures_lite::future::or(async { Ok::<T, ()>(fut.await) }, async {
        gloo_timers::future::TimeoutFuture::new(dur.as_millis() as u32).await;
        Err::<T, ()>(())
    })
    .await
}

/// 默认 channel（临时，直到频道系统落地）。
pub const DEFAULT_CHANNEL: &str = "general";

/// 每次上传的 KeyPackage 数量。
///
/// 每个 KP 只能消费一次；上传更多可以降低“被恶意拉取抽干”后的恢复成本。
pub const DEFAULT_KP_BATCH_SIZE: usize = 20;

/// 从历史拉取的一条消息（已解密）。
#[derive(Debug, Clone)]
pub struct HistoryMessage {
    pub sender: String,
    pub ts: i64,
    pub plaintext: String,
    pub attachments: Vec<kaede_protocol::Attachment>,
    pub reply_to: Option<kaede_protocol::ReplyRef>,
    /// @ 提及的 login_id（E2EE 内部字段）。
    pub mentions: Vec<String>,
}

/// 本地历史搜索命中。
#[derive(Debug, Clone)]
pub struct SearchHit {
    pub channel_id: String,
    pub sender: String,
    pub ts: i64,
    pub text: String,
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
    /// 串行化所有会推进 MLS epoch 的操作（add/remove/self_update）。
    ///
    /// 并发 Add 会让 Commit 的 epoch 标签与实际状态错位，接收方永久分叉。
    mls_op_lock: futures_util::lock::Mutex<()>,
    /// 快照持久化失败后置位：在恢复前拒绝新的发送，避免 ratchet 回退。
    snapshot_broken: AtomicBool,
    /// 频道 id → 是否私密。发送路径用它阻止“私密频道无 group 时回退主群”。
    channel_privacy: Mutex<HashMap<String, bool>>,
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
        // C7: wasm 上必须在 snapshot 恢复前初始化 WebCrypto keystore
        #[cfg(target_arch = "wasm32")]
        {
            let _ = crate::keystore_web::init_web_keystore().await;
        }

        let http = HttpApi::new(http_url)?;

        let auth = match http
            .login(LoginReq {
                login_id: login_id.into(),
                password: password.into(),
                device_name: device_name(),
            })
            .await
        {
            Ok(a) => a,
            Err(e) => {
                // C5: 只有结构化 401/404 才尝试注册（format! 字符串匹配会误触发）
                if matches!(
                    e,
                    ClientError::Http { status: 401, .. } | ClientError::Http { status: 404, .. }
                ) {
                    http.register(RegisterReq {
                        login_id: login_id.into(),
                        password: password.into(),
                        nickname: String::new(),
                        device_name: device_name(),
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
            // 旧版本的 event loop 曾把主群快照写到 `{server_url}/{uid}`
            // （缺少 `mls/` 前缀）。读取时兼容并自动迁移到正确 Key。
            let legacy_key = format!("{}/{}", server_url, auth.login_id);
            #[allow(clippy::single_match)]
            match load_main_snapshot(store, &storage_key, &legacy_key).await {
                Some(snap) => {
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
                None => {}
            }
        }

        if !restored {
            conn.mls_init(uid.clone())?;
        }

        // B3+: 注入 aux storage hook + 恢复 DM / 私密频道 snapshot
        if let Some(store) = &storage {
            let hook = AuxStorageHook {
                store: Arc::clone(store),
                server_url: server_url.clone(),
                uid: uid.as_str().to_string(),
            };
            conn.set_aux_storage(hook.clone());
            let errors = conn.load_aux_snapshots_from_storage(&hook).await;
            for ev in errors {
                conn.emit_local_event(ev);
            }
        }

        Ok(Self {
            uid,
            http,
            conn,
            storage,
            storage_key,
            server_url,
            mls_op_lock: futures_util::lock::Mutex::new(()),
            snapshot_broken: AtomicBool::new(false),
            channel_privacy: Mutex::new(HashMap::new()),
        })
    }

    pub fn uid(&self) -> &UserId {
        &self.uid
    }

    pub fn connection(&self) -> &Arc<ServerConnection> {
        &self.conn
    }

    /// `http_url::ws_url`（用于自动重连）。
    pub fn server_url(&self) -> &str {
        &self.server_url
    }

    /// 当前 session token（用于自动重连；过期后回退密码登录）。
    pub fn session_token(&self) -> Option<String> {
        self.http.token()
    }

    /// 本地存储句柄（恢复 MLS 状态）。
    pub fn storage(&self) -> Option<Arc<Store>> {
        self.storage.clone()
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
        // C7: wasm 上必须在 snapshot 恢复前初始化 WebCrypto keystore
        #[cfg(target_arch = "wasm32")]
        {
            let _ = crate::keystore_web::init_web_keystore().await;
        }

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
            let legacy_key = format!("{}/{}", server_url, login_id);
            #[allow(clippy::single_match)]
            match load_main_snapshot(store, &storage_key, &legacy_key).await {
                Some(snap) => {
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
                None => {}
            }
        }

        if !restored {
            conn.mls_init(uid.clone())?;
        }

        // B3+: 注入 aux storage hook + 恢复 DM / 私密频道 snapshot
        if let Some(store) = &storage {
            let hook = AuxStorageHook {
                store: Arc::clone(store),
                server_url: server_url.clone(),
                uid: uid.as_str().to_string(),
            };
            conn.set_aux_storage(hook.clone());
            let errors = conn.load_aux_snapshots_from_storage(&hook).await;
            for ev in errors {
                conn.emit_local_event(ev);
            }
        }

        Ok(Self {
            uid,
            http,
            conn,
            storage,
            storage_key,
            server_url,
            mls_op_lock: futures_util::lock::Mutex::new(()),
            snapshot_broken: AtomicBool::new(false),
            channel_privacy: Mutex::new(HashMap::new()),
        })
    }

    /// 起后台事件循环 + 应用层心跳。
    ///
    /// 服务端 `WS_IDLE_TIMEOUT = 300s` 只统计**入站**消息；没有心跳的话，
    /// 用户空闲 5 分钟就会被断开。
    pub fn spawn_event_loop(&self) {
        let conn = self.conn.clone();
        spawn_detached(conn.run_event_loop());

        let hb = self.conn.clone();
        spawn_detached(async move {
            let mut nonce: u64 = 0;
            loop {
                sleep_ms(45_000).await;
                if hb.state() == crate::ClientState::Disconnected {
                    break;
                }
                nonce += 1;
                if hb.send_ping(nonce).await.is_err() {
                    break;
                }
            }
        });
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
    ///
    /// 如果本地已有主群状态，同时上报 epoch：服务器会补投离线期间
    /// 错过的 Commit，避免 epoch 永久落后。
    pub async fn join_room(&self) -> Result<(), ClientError> {
        self.conn.join().await?;
        if self.conn.mls_is_ready() {
            self.conn.report_epoch(self.conn.mls_epoch()).await?;
        }
        Ok(())
    }

    /// 房主：把某个 peer 加进 MLS 组。
    ///
    /// **P0-5**：添加前检查 sender_id 碰撞（同一 32 位哈希 → AES-GCM nonce 复用 → 灾难）。
    /// **P1-4**：fetch KP / send welcome / send commit 都带指数退避重试；幂等（已在群里则 no-op）。
    /// **P1-5**：碰撞错误信息改为可操作的用户指引（不再建议用户"改 login_id"）。
    /// **P1-15**：本地 epoch 已前进但广播失败 → 返回 Err。
    pub async fn host_add_peer(&self, peer: &UserId) -> Result<(), ClientError> {
        // 所有 epoch 变更串行化：否则并发 Add 的 Commit 会带错 epoch 标签。
        let _op_guard = self.mls_op_lock.lock().await;
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
        // 必须在同一临界区内捕获该 Commit 的 epoch，不能 await 之后再读。
        let commit_epoch = self.conn.mls_epoch();

        // 3.5 广播前先落盘：welcome 可能已送达、commit 却重试失败；
        // 若此时不保存，重启会回滚到旧 epoch，新成员与房主永久分裂。
        if let Err(e) = self.save_snapshot().await {
            tracing::error!(
                "[host_add_peer] aborting add for {}: snapshot save failed before broadcast: {}",
                peer,
                e
            );
            return Err(e);
        }

        // 4. 先发 welcome（新成员必须先收到才能进群）——带重试
        self.send_with_retry("welcome", peer, &welcome, None)
            .await?;

        // 5. 再发 commit（其他成员 epoch 前进）——带重试
        self.send_with_retry("commit", peer, &commit, Some(commit_epoch))
            .await?;

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
                    // 校验类错误（credential/target 不匹配）重试只会继续消费
                    // 服务端的一次性 KeyPackage，必须立即失败。
                    if !is_retryable_kp_error(&e) {
                        return Err(e);
                    }
                    last_err = Some(e);
                    sleep_ms(*delay_ms).await;
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
        commit_epoch: Option<u64>,
    ) -> Result<(), ClientError> {
        const DELAYS_MS: &[u64] = &[200, 400, 800, 1500];
        let mut last_err: Option<ClientError> = None;
        for (attempt, delay_ms) in DELAYS_MS.iter().enumerate() {
            let result = match what {
                "welcome" => self.conn.send_welcome(peer.clone(), payload.to_vec()).await,
                "commit" => {
                    let epoch = commit_epoch.unwrap_or_else(|| self.conn.mls_epoch());
                    self.conn.send_commit(epoch, payload.to_vec()).await
                }
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
                    sleep_ms(*delay_ms).await;
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
        // 没有变更就不写：避免每条无关消息都重写整包快照。
        // 但 broken 状态下要强制重试一次，存储恢复后能自动解除发送阻断。
        let dirty = self.conn.take_main_dirty();
        if !dirty && !self.snapshot_broken.load(Ordering::Acquire) {
            return Ok(());
        }
        let cipher = crate::keystore::device_cipher(self.uid.as_str(), &self.server_url);
        let snap = match self.conn.mls_snapshot_with_cipher(cipher.as_ref()) {
            Ok(s) => s,
            Err(e) => {
                self.conn.restore_main_dirty();
                self.report_snapshot_failure(&e);
                return Err(e);
            }
        };
        match store.put("mls", &self.storage_key, &snap).await {
            Ok(()) => {
                self.snapshot_broken.store(false, Ordering::Release);
                Ok(())
            }
            Err(e) => {
                self.conn.restore_main_dirty();
                let err = ClientError::Storage(e.to_string());
                self.report_snapshot_failure(&err);
                Err(err)
            }
        }
    }

    /// 快照持续失败时向 UI 报错，并阻止后续发送。
    fn report_snapshot_failure(&self, err: &ClientError) {
        self.snapshot_broken.store(true, Ordering::Release);
        self.conn.emit_local_event(ClientEvent::MlsSyncError {
            server_id: self.conn.server_id().to_string(),
            user_message: "本地密钥状态保存失败；为避免 nonce 复用，已暂停发送。\
                           请检查磁盘/存储空间后重新登录"
                .into(),
            recoverable: false,
            detail: err.to_string(),
        });
    }

    fn ensure_snapshot_ok(&self) -> Result<(), ClientError> {
        if self.snapshot_broken.load(Ordering::Acquire) {
            return Err(ClientError::Storage(
                "MLS snapshot persistence is broken; refusing to send".into(),
            ));
        }
        Ok(())
    }

    fn note_channel_privacy(&self, channel_id: &str, is_private: bool) {
        self.channel_privacy
            .lock()
            .unwrap()
            .insert(channel_id.to_string(), is_private);
    }

    /// 已知私密频道？（未知频道按公开处理，兼容旧调用方）
    fn is_private_channel(&self, channel_id: &str) -> bool {
        self.channel_privacy
            .lock()
            .unwrap()
            .get(channel_id)
            .copied()
            .unwrap_or(false)
    }

    /// 私密频道没有本地 group 时必须拒绝，绝不能退回主群加密。
    fn ensure_channel_sendable(&self, channel_id: &str) -> Result<(), ClientError> {
        if self.is_private_channel(channel_id) && !self.conn.channel_has_group(channel_id) {
            return Err(ClientError::Protocol(
                "private channel key missing; re-join the channel before sending".into(),
            ));
        }
        if !self.conn.channel_has_group(channel_id) {
            self.ensure_snapshot_ok()?;
        }
        Ok(())
    }

    // ---------------------------------------------------------------------
    // 本地“自己发出的消息”持久化
    //
    // MLS 不允许解密自己发出的 Application Message（openmls 返回
    // OwnPrivateMessage），服务器也不会把消息回声给发送者。因此自己的
    // 历史只能由本地保存，否则重启后永远看不到。
    // ---------------------------------------------------------------------

    fn local_msg_prefix(&self, channel_id: &str) -> String {
        // 必须带 uid：同一浏览器/机器换账号后不能读到上一个账号的消息。
        format!("{}|{}|{}|", self.uid, self.server_url, channel_id)
    }

    fn local_msg_key(&self, channel_id: &str, ts: i64) -> String {
        // 13 位定长 ts 保证 list_keys 字典序 = 时间序；随机后缀避免同毫秒覆盖。
        let nonce: [u8; 4] = kaede_crypto::random_bytes();
        let suffix = u32::from_be_bytes(nonce);
        format!(
            "{}{:013}-{:08x}",
            self.local_msg_prefix(channel_id),
            ts,
            suffix
        )
    }

    /// 保存一条自己发出的消息（明文 payload）。失败只 warning，不影响发送。
    async fn persist_local_message(&self, channel_id: &str, ts: i64, bytes: &[u8]) {
        let Some(store) = &self.storage else {
            return;
        };
        let key = self.local_msg_key(channel_id, ts);
        // 本地消息也含明文；与 snapshot 使用同一设备 cipher 落盘。
        let Some(sealed) = self.encrypt_cache(bytes) else {
            tracing::warn!("[local-history] encrypt failed; skip persist");
            return;
        };
        if let Err(e) = store.put("messages", &key, &sealed).await {
            tracing::warn!("[local-history] put failed: {}", e);
            return;
        }
        // 每个频道最多保留 2000 条，防无限增长。
        if let Ok(keys) = store.list_keys("messages").await {
            let prefix = self.local_msg_prefix(channel_id);
            let mut mine: Vec<&String> = keys.iter().filter(|k| k.starts_with(&prefix)).collect();
            if mine.len() > 2000 {
                mine.sort();
                let remove_count = mine.len() - 2000;
                for k in mine.into_iter().take(remove_count) {
                    let _ = store.delete("messages", k).await;
                }
            }
        }
    }

    /// 读取某个频道本地保存的自己的消息（按 ts 升序）。
    async fn load_local_messages(&self, channel_id: &str) -> Vec<HistoryMessage> {
        let Some(store) = &self.storage else {
            return Vec::new();
        };
        let prefix = self.local_msg_prefix(channel_id);
        let keys = match store.list_keys("messages").await {
            Ok(k) => k,
            Err(_) => return Vec::new(),
        };
        let mut out = Vec::new();
        for key in keys {
            let Some(ts_str) = key.strip_prefix(&prefix) else {
                continue;
            };
            // key = "{ts:013}-{nonce:08x}"；后 9 字节是随机后缀。
            let Some(ts_part) = ts_str.split('-').next() else {
                continue;
            };
            let Ok(ts) = ts_part.parse::<i64>() else {
                continue;
            };
            let Ok(Some(raw)) = store.get("messages", &key).await else {
                continue;
            };
            let Some(bytes) = self.decrypt_cache(&raw) else {
                continue;
            };
            if let crate::payload::DecodedPayload::Message(p) = crate::payload::decode(&bytes) {
                out.push(HistoryMessage {
                    sender: self.uid.as_str().to_string(),
                    ts,
                    plaintext: p.t,
                    attachments: p.a,
                    reply_to: p.r,
                    mentions: p.m,
                });
            }
        }
        out.sort_by_key(|m| m.ts);
        out
    }

    // ---------------------------------------------------------------------
    // 已解密历史缓存
    //
    // MLS 的 SecretTree 会记录“已用 generation”，同一密文二次解密会被
    // 当作重放拒绝（SecretReuseError）。而快照现在会在每条消息后落盘，
    // 意味着重启后 secret tree 也带着“已用”标记 → 直接重新解密历史会全部失败。
    // 因此解密成功一次后必须把明文缓存下来，下次直接读缓存。
    // ---------------------------------------------------------------------

    const HISTORY_CACHE_NS: &'static str = "history_cache";

    /// 本地缓存加密头：`[0xCA,0xDE,0x01]`。不带头的旧数据按明文读取。
    const CACHE_MAGIC: [u8; 3] = [0xCA, 0xDE, 0x01];

    fn encrypt_cache(&self, plaintext: &[u8]) -> Option<Vec<u8>> {
        let cipher = crate::keystore::device_cipher(self.uid.as_str(), &self.server_url);
        let ct = cipher.encrypt(plaintext).ok()?;
        let mut out = Vec::with_capacity(Self::CACHE_MAGIC.len() + ct.len());
        out.extend_from_slice(&Self::CACHE_MAGIC);
        out.extend_from_slice(&ct);
        Some(out)
    }

    fn decrypt_cache(&self, raw: &[u8]) -> Option<Vec<u8>> {
        if raw.starts_with(&Self::CACHE_MAGIC) {
            let cipher = crate::keystore::device_cipher(self.uid.as_str(), &self.server_url);
            cipher.decrypt(&raw[Self::CACHE_MAGIC.len()..]).ok()
        } else {
            // 兼容旧版明文缓存
            Some(raw.to_vec())
        }
    }

    fn history_cache_prefix(&self, channel_id: &str) -> String {
        format!("{}|{}|{}|", self.uid, self.server_url, channel_id)
    }

    async fn load_cached_history(&self, channel_id: &str) -> Vec<(i64, String, Vec<u8>)> {
        let Some(store) = &self.storage else {
            return Vec::new();
        };
        let prefix = self.history_cache_prefix(channel_id);
        let keys = match store.list_keys(Self::HISTORY_CACHE_NS).await {
            Ok(k) => k,
            Err(_) => return Vec::new(),
        };
        let mut out = Vec::new();
        for key in keys {
            let Some(rest) = key.strip_prefix(&prefix) else {
                continue;
            };
            // rest = "{ts:013}|{sender}"
            let Some((ts_str, sender)) = rest.split_once('|') else {
                continue;
            };
            let Ok(ts) = ts_str.parse::<i64>() else {
                continue;
            };
            if let Ok(Some(raw)) = store.get(Self::HISTORY_CACHE_NS, &key).await
                && let Some(bytes) = self.decrypt_cache(&raw)
            {
                out.push((ts, sender.to_string(), bytes));
            }
        }
        out
    }

    async fn persist_cached_history(&self, channel_id: &str, ts: i64, sender: &str, bytes: &[u8]) {
        let Some(store) = &self.storage else {
            return;
        };
        let key = format!(
            "{}{:013}|{}",
            self.history_cache_prefix(channel_id),
            ts,
            sender
        );
        let Some(sealed) = self.encrypt_cache(bytes) else {
            tracing::warn!("[history-cache] encrypt failed; skip persist");
            return;
        };
        if let Err(e) = store.put(Self::HISTORY_CACHE_NS, &key, &sealed).await {
            tracing::warn!("[history-cache] put failed: {}", e);
        }
    }

    /// 清理历史缓存，每个频道最多保留 2000 条（按 ts 排序保留最新）。
    async fn prune_history_cache(&self, channel_id: &str) {
        let Some(store) = &self.storage else {
            return;
        };
        let prefix = self.history_cache_prefix(channel_id);
        if let Ok(keys) = store.list_keys(Self::HISTORY_CACHE_NS).await {
            let mut mine: Vec<&String> = keys.iter().filter(|k| k.starts_with(&prefix)).collect();
            mine.sort();
            if mine.len() > 2000 {
                let remove = mine.len() - 2000;
                for k in mine.into_iter().take(remove) {
                    let _ = store.delete(Self::HISTORY_CACHE_NS, k).await;
                }
            }
        }
    }

    /// 解密一条历史记录：私密频道走 channel group（0x03），否则走主群 MLS。
    fn decrypt_history_record(&self, channel_id: &str, sender: &str, ct: &[u8]) -> Option<Vec<u8>> {
        if ct.first() == Some(&0x03) {
            let key = self.conn.channel_export_key(channel_id).ok()?;
            let body = &ct[1..];
            match kaede_crypto::dm_cipher::open(&key, channel_id, sender, "", body) {
                Ok((pt, _)) => Some(pt),
                Err(kaede_crypto::dm_cipher::DmCipherError::UnknownVersion(_)) => {
                    kaede_crypto::dm_cipher::open_legacy(&key, channel_id, body).ok()
                }
                Err(_) => None,
            }
        } else {
            self.conn.mls_decrypt(ct).ok()
        }
    }

    fn decode_cached(ts: i64, sender: String, bytes: &[u8]) -> Option<HistoryMessage> {
        match crate::payload::decode(bytes) {
            crate::payload::DecodedPayload::Message(p) => Some(HistoryMessage {
                sender,
                ts,
                plaintext: p.t,
                attachments: p.a,
                reply_to: p.r,
                mentions: p.m,
            }),
            crate::payload::DecodedPayload::Raw(b) => Some(HistoryMessage {
                sender,
                ts,
                plaintext: String::from_utf8_lossy(&b).to_string(),
                attachments: Vec::new(),
                reply_to: None,
                mentions: Vec::new(),
            }),
            // 反应 / 媒体信令不入历史
            _ => None,
        }
    }

    /// 手动删除快照（例如登出时）。
    pub async fn clear_snapshot(&self) -> Result<(), ClientError> {
        if let Some(store) = &self.storage {
            store
                .delete("mls", &self.storage_key)
                .await
                .map_err(|e| ClientError::Storage(e.to_string()))?;
            // 旧版本可能把同一快照写在无前缀 Key 下。
            let legacy_key = format!("{}/{}", self.server_url, self.uid);
            let _ = store.delete("mls", &legacy_key).await;
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

    /// 使用调用方指定的 ts 发送；UI 用它保证乐观消息与广播 payload 的 ts 一致。
    pub async fn send_text_to_at(
        &self,
        channel_id: &str,
        text: &str,
        ts: i64,
    ) -> Result<(), ClientError> {
        self.send_text_with_attachments_at(channel_id, text, &[], ts)
            .await
    }

    /// 指定 channel、ts 与 @ 提及的纯文本消息。
    pub async fn send_text_with_mentions_at(
        &self,
        channel_id: &str,
        text: &str,
        mentions: &[String],
        ts: i64,
    ) -> Result<(), ClientError> {
        self.send_text_with_attachments_mentions_at(channel_id, text, &[], mentions, ts)
            .await
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
        let cached: std::collections::HashMap<i64, (String, Vec<u8>)> = self
            .load_cached_history(channel_id)
            .await
            .into_iter()
            .map(|(ts, sender, bytes)| (ts, (sender, bytes)))
            .collect();

        let mut out = Vec::with_capacity(records.len());
        for r in records {
            if let Some((sender, bytes)) = cached.get(&r.ts) {
                if let Some(m) = Self::decode_cached(r.ts, sender.clone(), bytes) {
                    out.push(m);
                }
                continue;
            }
            let ct = match base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(&r.ciphertext) {
                Ok(c) => c,
                Err(_) => continue,
            };
            if let Some(pt) = self.decrypt_history_record(channel_id, &r.sender, &ct) {
                self.persist_cached_history(channel_id, r.ts, &r.sender, &pt)
                    .await;
                if let Some(m) = Self::decode_cached(r.ts, r.sender.clone(), &pt) {
                    out.push(m);
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
        // 本地保存的“自己发出的消息”必须合并进来：
        // MLS 无法解密自己的消息，服务器也不会回声给发送者。
        // 私密频道例外：频道密钥可以重复解密自己的消息，且旧版本可能留下
        // 客户端 ts 的本地副本（与服务器 ts 不一致 → 重载后重复）。
        let mut local = if self.conn.channel_has_group(channel_id) {
            Vec::new()
        } else {
            self.load_local_messages(channel_id).await
        };

        let cached: std::collections::HashMap<i64, (String, Vec<u8>)> = self
            .load_cached_history(channel_id)
            .await
            .into_iter()
            .map(|(ts, sender, bytes)| (ts, (sender, bytes)))
            .collect();

        let records = self.http.load_messages(channel_id, None, limit).await?;
        let mut out = Vec::with_capacity(records.len());
        for r in records {
            // 已解密过的消息直接读缓存：SecretTree 不允许同一密文二次解密。
            if let Some((sender, bytes)) = cached.get(&r.ts) {
                if let Some(m) = Self::decode_cached(r.ts, sender.clone(), bytes) {
                    out.push(m);
                }
                continue;
            }
            let ct = match URL_SAFE_NO_PAD.decode(&r.ciphertext) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("[room] bad base64 at ts={}: {}", r.ts, e);
                    continue;
                }
            };
            match self.decrypt_history_record(channel_id, &r.sender, &ct) {
                Some(pt) => {
                    self.persist_cached_history(channel_id, r.ts, &r.sender, &pt)
                        .await;
                    if let Some(m) = Self::decode_cached(r.ts, r.sender.clone(), &pt) {
                        out.push(m);
                    }
                }
                None => {
                    tracing::debug!("[room] history decrypt skipped at ts={}", r.ts);
                }
            }
        }

        out.append(&mut local);
        out.sort_by_key(|m| m.ts);
        out.dedup_by(|a, b| a.ts == b.ts && a.sender == b.sender && a.plaintext == b.plaintext);
        self.prune_history_cache(channel_id).await;
        Ok(out)
    }

    // ---------------------------------------------------------------------
    // 频道 / 成员 API（HTTP 代理）
    // ---------------------------------------------------------------------

    /// 列出频道。
    pub async fn list_channels(&self) -> Result<Vec<kaede_protocol::Channel>, ClientError> {
        let list = self.http.list_channels().await?;
        {
            let mut map = self.channel_privacy.lock().unwrap_or_else(|e| e.into_inner());
            for ch in &list {
                map.insert(ch.id.clone(), ch.is_private);
            }
        }
        Ok(list)
    }

    /// 创建频道（需 op3+）。
    pub async fn create_channel(
        &self,
        name: &str,
        kind: &str,
    ) -> Result<kaede_protocol::Channel, ClientError> {
        let ch = self.http.create_channel(name, kind).await?;
        self.note_channel_privacy(&ch.id, false);
        Ok(ch)
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
        self.note_channel_privacy(&ch.id, is_private);
        if is_private && let Err(e) = self.conn.channel_ensure_group(&ch.id) {
            tracing::warn!("channel_ensure_group failed for {}: {}", ch.id, e);
        }
        Ok(ch)
    }

    /// 加入私密频道（B2 / A1）。
    ///
    /// 两种场景：
    /// - **已有本地 group**（之前加入过）：只验证密码，不重新走 Welcome
    /// - **没有本地 group**（首次加入）：验证密码 + KP + Welcome 交换
    pub async fn join_channel(&self, channel_id: &str, password: &str) -> Result<(), ClientError> {
        // 该 API 只用于私密频道；标记后即使 Welcome 失败也不会回退主群加密。
        self.note_channel_privacy(channel_id, true);
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
        match timeout_recv(std::time::Duration::from_secs(10), rx.recv()).await {
            Ok(Ok(Ok(_welcome))) => {
                tracing::info!("[room] join_channel {} completed via welcome", channel_id);
                self.conn.flush_dirty_snapshots().await;
                Ok(())
            }
            Ok(Ok(Err(e))) => Err(ClientError::Protocol(format!("welcome failed: {}", e))),
            Ok(Err(_)) => Err(ClientError::Protocol("welcome channel closed".into())),
            Err(_timeout) => {
                // 不清理等待槽：权威成员上线后补投的 Welcome 仍会加入 group，
                // 并通过 ClientEvent::ChannelJoined 通知 UI 刷新。
                tracing::warn!(
                    "[room] join_channel {} timeout waiting welcome; \
                     request may be completed when channel authority comes online",
                    channel_id
                );
                Err(ClientError::Protocol(
                    "当前无在线成员可授权，加入请求已排队；待频道成员上线后会自动完成。\
                     如长时间无响应，请让频道成员上线后重试。"
                        .into(),
                ))
            }
        }
    }

    /// 频道成员列表。
    pub async fn list_channel_members(
        &self,
        channel_id: &str,
    ) -> Result<Vec<crate::http::ChannelMember>, ClientError> {
        self.http.list_channel_members(channel_id).await
    }

    /// 把成员移出私密频道（仅 op5）。
    pub async fn remove_channel_member(
        &self,
        channel_id: &str,
        login_id: &str,
    ) -> Result<(), ClientError> {
        self.http.remove_channel_member(channel_id, login_id).await
    }

    /// 上报频道最后已读位置。
    pub async fn mark_channel_read(&self, channel_id: &str, ts: i64) -> Result<(), ClientError> {
        self.http.mark_read(Some(channel_id), None, ts).await
    }

    /// 上报 DM 最后已读位置。
    pub async fn mark_dm_read(&self, conv_id: &str, ts: i64) -> Result<(), ClientError> {
        self.http.mark_read(None, Some(conv_id), ts).await
    }

    /// 拉取未读计数。
    pub async fn unread_counts(&self) -> Result<crate::http::UnreadCounts, ClientError> {
        self.http.get_unread().await
    }

    /// 列出当前账号的登录会话（设备）。
    pub async fn list_sessions(&self) -> Result<Vec<crate::http::SessionInfo>, ClientError> {
        self.http.list_sessions().await
    }

    /// 撤销指定会话。
    pub async fn revoke_session(&self, session_id: &str) -> Result<(), ClientError> {
        self.http.revoke_session(session_id).await
    }

    /// 撤销其他所有会话。
    pub async fn revoke_other_sessions(&self) -> Result<(), ClientError> {
        self.http.revoke_other_sessions().await
    }

    /// 在本地已解密历史中搜索（大小写不敏感子串）。
    ///
    /// 搜索范围：`history_cache` + 本地已发送消息缓存；不请求服务器。
    /// 返回按 ts 倒序，最多 `limit` 条。
    pub async fn search_local(
        &self,
        query: &str,
        channel_ids: &[String],
        limit: usize,
    ) -> Vec<SearchHit> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }

        let mut hits = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for ch in channel_ids {
            let mut push = |ts: i64, sender: &str, text: &str| {
                if !text.to_lowercase().contains(&q) {
                    return;
                }
                if !seen.insert((ch.clone(), ts, sender.to_string())) {
                    return;
                }
                hits.push(SearchHit {
                    channel_id: ch.clone(),
                    sender: sender.to_string(),
                    ts,
                    text: text.to_string(),
                });
            };

            for (ts, sender, bytes) in self.load_cached_history(ch).await {
                if let Some(m) = Self::decode_cached(ts, sender, &bytes) {
                    push(m.ts, &m.sender, &m.plaintext);
                }
            }
            for m in self.load_local_messages(ch).await {
                push(m.ts, &m.sender, &m.plaintext);
            }
        }

        hits.sort_by_key(|a| std::cmp::Reverse(a.ts));
        hits.truncate(limit);
        hits
    }

    /// 离开频道。
    ///
    /// 私密频道离开后立刻删除本地 channel group，避免继续持有频道密钥。
    pub async fn leave_channel(&self, channel_id: &str) -> Result<(), ClientError> {
        self.http.leave_channel(channel_id).await?;
        self.conn.channel_drop_group(channel_id).await;
        Ok(())
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
        let _op_guard = self.mls_op_lock.lock().await;
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
            let commit_epoch = self.conn.mls_epoch();
            if let Err(e) = self.conn.send_commit(commit_epoch, commit).await {
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
        if self.is_private_channel(target_id) && !self.conn.channel_has_group(target_id) {
            return Err(ClientError::Protocol(
                "private channel key missing; cannot encrypt attachment on main group".into(),
            ));
        }
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

        // 上传进度事件：UI 据此显示进度条。
        let conn = self.conn.clone();
        let progress_name = name.to_string();
        let progress: crate::http::UploadProgress = std::sync::Arc::new(move |sent, total| {
            conn.emit_local_event(ClientEvent::AttachmentProgress {
                server_id: conn.server_id().to_string(),
                name: progress_name.clone(),
                sent,
                total,
            });
        });
        self.http
            .upload_attachment_with_progress(name, mime, wrapped, Some(progress))
            .await
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
        self.send_text_with_reply_at(channel_id, text, reply_to, now_ms())
            .await
    }

    /// 指定 ts 的回复消息。
    pub async fn send_text_with_reply_at(
        &self,
        channel_id: &str,
        text: &str,
        reply_to: kaede_protocol::ReplyRef,
        ts: i64,
    ) -> Result<(), ClientError> {
        self.send_text_with_reply_mentions_at(channel_id, text, reply_to, &[], ts)
            .await
    }

    /// 指定 ts、带 @ 提及的回复消息。
    pub async fn send_text_with_reply_mentions_at(
        &self,
        channel_id: &str,
        text: &str,
        reply_to: kaede_protocol::ReplyRef,
        mentions: &[String],
        ts: i64,
    ) -> Result<(), ClientError> {
        self.ensure_channel_sendable(channel_id)?;
        let mut payload = crate::payload::MessagePayload::new(
            channel_id,
            self.uid.as_str(),
            text,
            Vec::new(),
            Some(reply_to),
        );
        payload.ts = ts;
        payload.m = mentions.to_vec();
        let bytes = payload
            .to_bytes()
            .map_err(|e| ClientError::Protocol(e.to_string()))?;
        let local_ts = payload.ts;
        // 私密频道必须用 channel group key（0x03），不能退回主群；
        // 服务端会拒绝主群密文，避免私密内容被全服解密。
        let ciphertext = if self.conn.channel_has_group(channel_id) {
            let key = self.conn.channel_export_key(channel_id)?;
            let sealed =
                kaede_crypto::dm_cipher::seal(&key, channel_id, self.uid.as_str(), "", &bytes)
                    .map_err(|e| ClientError::Protocol(format!("channel seal: {e}")))?;
            let mut out = Vec::with_capacity(1 + sealed.len());
            out.push(0x03u8);
            out.extend_from_slice(&sealed);
            out
        } else {
            self.conn.mls_send_app(&bytes).await?
        };
        // 发送推进 MLS ratchet：立即持久化，防重启后 nonce 复用。
        if let Err(e) = self.save_snapshot().await {
            tracing::warn!("[send] snapshot save failed: {}", e);
        }

        match self
            .http
            .post_message(channel_id, now_ms(), self.uid.as_str(), &ciphertext)
            .await
        {
            Ok(()) => {
                // 私密频道的历史用频道密钥可重复解密，不需要本地副本；
                // 再存一份本地消息会在重载后与服务器记录重复。
                if !self.conn.channel_has_group(channel_id) {
                    self.persist_local_message(channel_id, local_ts, &bytes)
                        .await;
                }
            }
            Err(e) => {
                tracing::warn!("post_message: {}", e);
                return Err(e);
            }
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
        self.send_text_with_attachments_at(channel_id, text, attachments, now_ms())
            .await
    }

    /// 指定 ts 的附件消息。
    pub async fn send_text_with_attachments_at(
        &self,
        channel_id: &str,
        text: &str,
        attachments: &[kaede_protocol::Attachment],
        ts: i64,
    ) -> Result<(), ClientError> {
        self.send_text_with_attachments_mentions_at(channel_id, text, attachments, &[], ts)
            .await
    }

    /// 指定 ts、带 @ 提及的附件消息。
    pub async fn send_text_with_attachments_mentions_at(
        &self,
        channel_id: &str,
        text: &str,
        attachments: &[kaede_protocol::Attachment],
        mentions: &[String],
        ts: i64,
    ) -> Result<(), ClientError> {
        self.ensure_channel_sendable(channel_id)?;
        let mut payload = crate::payload::MessagePayload::new(
            channel_id,
            self.uid.as_str(),
            text,
            attachments.to_vec(),
            None,
        );
        payload.ts = ts;
        payload.m = mentions.to_vec();
        let bytes = payload
            .to_bytes()
            .map_err(|e| ClientError::Protocol(e.to_string()))?;
        let local_ts = payload.ts;

        // 私密频道：走 channel group 的 AES-GCM key（首字节 0x03 标记）
        let ciphertext = if self.conn.channel_has_group(channel_id) {
            let key = self.conn.channel_export_key(channel_id)?;
            let sealed =
                kaede_crypto::dm_cipher::seal(&key, channel_id, self.uid.as_str(), "", &bytes)
                    .map_err(|e| ClientError::Protocol(format!("channel seal: {e}")))?;
            let mut out = Vec::with_capacity(1 + sealed.len());
            out.push(0x03u8);
            out.extend_from_slice(&sealed);
            out
        } else {
            self.conn.mls_send_app(&bytes).await?
        };
        // 主群发送推进 ratchet；channel group 分支没有主群状态变化（dirty=false 时无操作）。
        if let Err(e) = self.save_snapshot().await {
            tracing::warn!("[send] snapshot save failed: {}", e);
        }
        match self
            .http
            .post_message(channel_id, now_ms(), self.uid.as_str(), &ciphertext)
            .await
        {
            Ok(()) => {
                if !self.conn.channel_has_group(channel_id) {
                    self.persist_local_message(channel_id, local_ts, &bytes)
                        .await;
                }
            }
            Err(e) => {
                tracing::warn!("post_message: {}", e);
                return Err(e);
            }
        }
        Ok(())
    }

    /// 发送表情回复（独立 MLS 消息）。
    ///
    /// 私密频道暂不支持：表情走主群 Application Message 会向所有主群成员
    /// 泄露频道 id / 目标 ts / 表情内容，必须显式拒绝。
    pub async fn send_reaction(
        &self,
        channel_id: &str,
        target_ts: i64,
        emoji: &str,
        action: &str, // "add" | "remove"
    ) -> Result<(), ClientError> {
        if self.conn.channel_has_group(channel_id) || self.is_private_channel(channel_id) {
            return Err(ClientError::Protocol(
                "private channel reactions are not supported yet".into(),
            ));
        }
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
        if let Err(e) = self.save_snapshot().await {
            tracing::warn!("[reaction] snapshot save failed: {}", e);
        }
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
            // B2：Welcome 信封（首字节 0x02）→ 加入独立 group，不入 UI 消息列表
            if let Some((generation, welcome)) = crate::connection::unpack_dm_welcome(&ct) {
                if let Err(e) = self.conn.dm_join_group(conv_id, welcome, generation) {
                    tracing::warn!("dm_join_group failed for {}: {}", conv_id, e);
                }
                continue;
            }
            #[allow(clippy::collapsible_if)]
            if let Ok(pt) = self.conn.dm_open_history(conv_id, &r.sender, &ct) {
                if let crate::payload::DecodedPayload::Message(p) = crate::payload::decode(&pt) {
                    out.push(HistoryMessage {
                        sender: r.sender,
                        ts: r.ts,
                        plaintext: p.t,
                        attachments: p.a,
                        reply_to: p.r,
                        mentions: p.m,
                    });
                }
            }
        }
        self.conn.flush_dirty_snapshots().await;
        Ok(out)
    }

    /// 发送 DM 消息。
    ///
    /// M53: 若本地无 DM group，先拉对端 KP + 建 2 人 group + 发 Welcome。
    ///
    /// **并发发起**：双方同时创建 group 时，`dm_join_group` 用
    /// group_id 字典序最小者胜出，两端会做出相同选择，不再群分裂。
    pub async fn send_dm(&self, conv_id: &str, text: &str) -> Result<(), ClientError> {
        self.send_dm_with_attachments(conv_id, text, &[]).await
    }

    /// 确保本地已有该 DM 会话的 2 人 group（附件上传前需要 File key）。
    pub async fn ensure_dm_group(&self, conv_id: &str) -> Result<(), ClientError> {
        if !self.conn.dm_has_group(conv_id) {
            self.dm_ensure_group(conv_id).await?;
        }
        Ok(())
    }

    /// 发送带附件的 DM。
    ///
    /// 附件本身已由 `upload_attachment(conv_id, ...)` 用 DM 的 File purpose key
    /// 加密；这里只把附件元数据放进 per-conv AEAD payload。
    pub async fn send_dm_with_attachments(
        &self,
        conv_id: &str,
        text: &str,
        attachments: &[kaede_protocol::Attachment],
    ) -> Result<(), ClientError> {
        self.send_dm_with_attachments_mentions(conv_id, text, attachments, &[])
            .await
    }

    /// 带 @ 提及的 DM。
    pub async fn send_dm_with_attachments_mentions(
        &self,
        conv_id: &str,
        text: &str,
        attachments: &[kaede_protocol::Attachment],
        mentions: &[String],
    ) -> Result<(), ClientError> {
        if !self.conn.dm_has_group(conv_id) {
            self.dm_ensure_group(conv_id).await?;
        }

        let mut payload = crate::payload::MessagePayload::new(
            conv_id,
            self.uid.as_str(),
            text,
            attachments.to_vec(),
            None,
        );
        payload.m = mentions.to_vec();
        let bytes = payload
            .to_bytes()
            .map_err(|e| ClientError::Protocol(e.to_string()))?;

        // B3：per-conv AEAD 加密
        let ciphertext = self.conn.dm_seal(conv_id, &bytes)?;

        // 落库（HTTP）。用服务器返回的权威 ts 做本地回显，
        // 这样重新打开会话读取历史时能按 ts 与服务器记录去重。
        let ts = match self
            .http
            .dm_post_message(conv_id, now_ms(), self.uid.as_str(), &ciphertext)
            .await
        {
            Ok(ts) => ts,
            Err(e) => {
                tracing::warn!("dm_post_message: {}", e);
                return Err(e);
            }
        };

        // 本地回显（服务器只推给对方，不推回自己）
        self.conn.emit_local_event(ClientEvent::DmReceived {
            server_id: self.conn.server_id().to_string(),
            conv_id: conv_id.to_string(),
            from: self.uid.clone(),
            ts,
            plaintext: bytes,
        });
        self.conn.flush_dirty_snapshots().await;
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

        let (welcome, generation) = self.conn.dm_init_group(conv_id, &peer_kp)?;
        let ts = now_ms();
        let envelope = crate::connection::pack_dm_welcome(generation, &welcome);

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

    /// 进入媒体会话：拿 token + 用房间级 seed 派生 e2ee key。
    ///
    /// 返回 `(token, e2ee_key)`：
    /// - `token` 用于 `ConnectConfig`
    /// - `e2ee_key` 用于 `KeyProvider::with_shared_key`
    ///
    /// # 为什么不再每次进入都 `self_update`
    ///
    /// 旧实现让每个参与者进入时都推进 MLS epoch，然后立刻派生 key：
    /// A 先进得到 epoch N 的 key，B 后进得到 epoch N+1 的 key → 两人 key
    /// 永远不同，多人语音必然不通。
    ///
    /// 现在改为：**同一媒体会话内所有人使用同一 epoch + 同一 server seed**。
    /// 跨会话的 nonce 复用由 server 侧 seed 生命周期保证：
    /// 最后一个参与者调用 `/api/livekit/leave` 后 seed 被丢弃，下次进入
    /// 生成新 seed → 即使 LiveKit SDK ratchet 从 0 重启也是新 key。
    ///
    /// # 已知限制
    ///
    /// - 通话过程中如果有成员变更（epoch 前进），新 key 不会热更新到已连接
    ///   的 LiveKit 会话；需要离开重进。
    ///   - **主群**：由 `ClientEvent::MlsEpochChanged` 触发 UI 自动重连
    ///   - **DM / 私密频道**：group epoch 变化暂不触发自动重连（块 B，
    ///     计划 alpha.3 补上）。通话中若发生成员变更，需手动离开重进。
    /// - DM / 私密频道 **已开启 E2EE**（v0.0.1-alpha.2）：三条路径分别从
    ///   `dm_groups` / `channel_groups` / 主群派生 key，统一注入 room-level seed。
    /// - group 存在但 exporter 失败时**拒绝连接**，绝不静默降级到明文媒体。
    pub async fn begin_media_session(
        &self,
        channel_id: &str,
    ) -> Result<
        (
            crate::http::LivekitToken,
            Option<zeroize::Zeroizing<[u8; 32]>>,
        ),
        ClientError,
    > {
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

        // C14 已开启（v0.0.1-alpha.2）：DM / 私密频道 / 主群三条路径
        // 各自从本地 group state 派生 LiveKit E2EE key。三方均注入
        // room-level seed，保证同一媒体会话内所有参与者拿到同一个 key。
        //
        // 优先级：DM → 私密频道 → 主群。三者 channel_id 语义独立：
        // - DM：conv_id 形如 "dm:a:b"
        // - 私密频道：channel_id 与公开频道共享 namespace，由 channel_has_group 区分
        // - 主群：以上两者都不命中时的默认路径
        //
        // 降级路径（保留 C14 语义）：group 缺失时返回 None —— 这是**显式**降级，
        // 调用方 UI 会根据 None 提示"E2EE 未启用"，不是静默失败。

        if self.conn.dm_has_group(channel_id) {
            match self.conn.dm_export_media_key(channel_id, &seed) {
                Ok(k) => {
                    tracing::info!(
                        "[begin_media_session] DM {} -> E2EE enabled (dm group)",
                        channel_id
                    );
                    return Ok((token, Some(zeroize::Zeroizing::new(k))));
                }
                Err(e) => {
                    tracing::error!(
                        "[begin_media_session] DM {} group present but export failed: {}; refusing plaintext media",
                        channel_id,
                        e
                    );
                    return Err(ClientError::Protocol(format!(
                        "媒体 E2EE 密钥派生失败（DM {}）：{e}",
                        channel_id
                    )));
                }
            }
        }

        if self.conn.channel_has_group(channel_id) {
            match self.conn.channel_export_media_key(channel_id, &seed) {
                Ok(k) => {
                    tracing::info!(
                        "[begin_media_session] private channel {} -> E2EE enabled (channel group)",
                        channel_id
                    );
                    return Ok((token, Some(zeroize::Zeroizing::new(k))));
                }
                Err(e) => {
                    tracing::error!(
                        "[begin_media_session] channel {} group present but export failed: {}; refusing plaintext media",
                        channel_id,
                        e
                    );
                    return Err(ClientError::Protocol(format!(
                        "媒体 E2EE 密钥派生失败（频道 {}）：{e}",
                        channel_id
                    )));
                }
            }
        }

        // 主群路径（默认）
        let key = self.conn.mls_export_sframe_key_for_with_extra(
            channel_id,
            kaede_mls::SframePurpose::LivekitE2ee,
            &seed,
        )?;

        Ok((token, Some(zeroize::Zeroizing::new(key))))
    }

    /// 告诉服务器本用户已离开媒体房间。
    ///
    /// 服务器据此维护 seed 生命周期：最后一个参与者离开后删除 seed，
    /// 下一次进入生成新 seed（防 LiveKit ratchet 重置导致的 nonce 复用）。
    pub async fn leave_media_room(&self, channel_id: &str) -> Result<(), ClientError> {
        self.http.leave_livekit_room(channel_id).await
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
        // 私密频道 / DM：媒体信令不走主群广播（会泄露频道 id 与媒体元数据）。
        if self.conn.channel_has_group(channel_id) || self.conn.dm_has_group(channel_id) {
            tracing::debug!(
                "[media-meta] skip broadcast for private target {}",
                channel_id
            );
            return Ok(());
        }
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
        if self.conn.channel_has_group(channel_id) || self.conn.dm_has_group(channel_id) {
            return Ok(());
        }
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

pub(crate) fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

/// 当前平台的设备名，用于“登录设备”列表。
pub fn device_name() -> String {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let host = std::env::var("COMPUTERNAME")
            .or_else(|_| std::env::var("HOSTNAME"))
            .unwrap_or_else(|_| "unknown".to_string());
        format!("{} ({})", host, std::env::consts::OS)
    }
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()
            .and_then(|w| w.navigator().user_agent().ok())
            .map(|ua| ua.chars().take(64).collect())
            .unwrap_or_else(|| "web".to_string())
    }
}

// ============================================================================
// 平台默认 store
// ============================================================================

/// 读取主群快照；兼容旧版本写错的 `{server_url}/{uid}` Key，并迁移到新 Key。
async fn load_main_snapshot(store: &Store, storage_key: &str, legacy_key: &str) -> Option<Vec<u8>> {
    match store.get("mls", storage_key).await {
        Ok(Some(snap)) => Some(snap),
        Ok(None) => match store.get("mls", legacy_key).await {
            Ok(Some(snap)) => {
                tracing::info!("[room] migrating main MLS snapshot to namespaced key");
                if let Err(e) = store.put("mls", storage_key, &snap).await {
                    tracing::warn!("[room] snapshot migration failed: {}", e);
                }
                Some(snap)
            }
            Ok(None) => None,
            Err(e) => {
                tracing::warn!("[room] storage read failed: {}", e);
                None
            }
        },
        Err(e) => {
            tracing::warn!("[room] storage read failed: {}", e);
            None
        }
    }
}

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
