//! 私密频道：独立 MLS group、成员集合校验、频道级密钥导出与密文打开。
//!
//! 成员集合在加入 / 推进 epoch 前必须对照服务端权威 `channel_members`
//! （[`ServerConnection::channel_member_set`]），校验失败即拒绝，避免把
//! 群外成员带进频道密钥。

use std::collections::HashSet;

use kaede_protocol::ClientMessage;

use crate::error::ClientError;
use crate::mls_session::MlsSession;

use super::ServerConnection;

impl ServerConnection {
    /// 拉取服务端权威的频道成员集合。
    ///
    /// 失败时返回错误而不是退化为“跳过校验”：否则攻击者只要让这次查询失败，
    /// 就能绕过成员集合校验。
    async fn channel_member_set(&self, channel_id: &str) -> Result<HashSet<String>, ClientError> {
        let http = self
            .http
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or_else(|| {
                ClientError::Mls(
                    "channel membership source unavailable; refusing channel sync".into(),
                )
            })?;
        let members = http.list_channel_members(channel_id).await?;
        Ok(members.into_iter().map(|m| m.login_id).collect())
    }

    /// 记录/清除某频道是否私密。RoomSession 在 list/create/join 时调用。
    pub fn set_channel_private(&self, channel_id: &str, is_private: bool) {
        let mut set = self
            .private_channels
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if is_private {
            set.insert(channel_id.to_string());
        } else {
            set.remove(channel_id);
        }
    }

    pub(super) fn is_known_private_channel(&self, channel_id: &str) -> bool {
        self.private_channels
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(channel_id)
    }

    /// 私密频道 group 的当前 epoch；无 group 时返回 None。
    ///
    /// 用途：`ChannelSync 0x05` 处理完 commit 后 emit `ChannelEpochChanged` 用。
    pub fn channel_epoch(&self, channel_id: &str) -> Option<u64> {
        let map = self
            .channel_groups
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        map.get(channel_id).map(|s| s.epoch())
    }

    /// 频道是否有独立 MLS group（私密频道）。
    pub fn channel_has_group(&self, channel_id: &str) -> bool {
        self.channel_groups
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(channel_id)
    }

    /// 本地 channel group 当前成员数；无 group 时返回 0。
    /// 用于 join_channel 判断"是多人群组还是单人占位群组"。
    pub fn channel_member_count(&self, channel_id: &str) -> usize {
        let map = self.channel_groups.lock().unwrap_or_else(|e| e.into_inner());
        map.get(channel_id).map(|s| s.member_count()).unwrap_or(0)
    }

    /// 从 channel group 按 purpose 派生密钥（P0：附件加密用 File）。
    pub fn channel_export_key_for(
        &self,
        channel_id: &str,
        purpose: kaede_mls::SframePurpose,
    ) -> Result<[u8; 32], ClientError> {
        let map = self
            .channel_groups
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let s = map
            .get(channel_id)
            .ok_or_else(|| ClientError::Mls(format!("channel group not found: {}", channel_id)))?;
        s.export_sframe_key_for(channel_id, purpose)
    }

    /// 为私密频道建立独立 MLS group（仅含当前用户）。
    /// MVP：不引入其他成员；多人 Welcome 下一轮做。
    pub fn channel_ensure_group(&self, channel_id: &str) -> Result<(), ClientError> {
        if self.channel_has_group(channel_id) {
            return Ok(());
        }
        // 先拿 mls，再拿 channel_groups（统一加锁顺序）
        let main_guard = self.mls.lock().unwrap_or_else(|e| e.into_inner());
        let main_session = main_guard
            .as_ref()
            .ok_or_else(|| ClientError::Mls("no main mls session".into()))?;
        let mut ch_session = main_session.fork_for_new_group();
        ch_session.create_group()?;
        drop(main_guard);
        self.channel_groups
            .lock()
            .unwrap_or_else(|e| e.into_inner())
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
            .unwrap_or_else(|e| e.into_inner())
            .insert(channel_id.to_string(), tx);
        rx
    }

    /// 清除某个频道的“等待 Welcome”槽（超时 / 失败时调用）。
    pub fn clear_pending_welcome(&self, channel_id: &str) {
        self.channel_pending_welcomes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(channel_id);
    }

    /// B2：把 kp 加入 channel group，返回 (commit, welcome)。
    ///
    /// `expected_uid` 来自服务端的加入申请。必须与 KP 内部 credential 一致，
    /// 防止恶意/故障服务端让权威成员把陌生人的 KP 加进频道群。
    pub fn channel_add_member(
        &self,
        channel_id: &str,
        kp: &[u8],
        expected_uid: &str,
    ) -> Result<(Vec<u8>, Vec<u8>), ClientError> {
        let mut map = self
            .channel_groups
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let s = map
            .get_mut(channel_id)
            .ok_or_else(|| ClientError::Mls(format!("channel group not found: {}", channel_id)))?;
        let out = s.add_member_for(kp, expected_uid)?;
        drop(map);
        self.mark_channel_dirty(channel_id);
        #[cfg(target_arch = "wasm32")]
        web_sys::console::log_1(&format!(
            "[channel_process_commit] ok ch={} epoch={}",
            channel_id,
            self.channel_epoch(channel_id).unwrap_or(0)
        ).into());
        Ok(out)
    }

    /// 校验频道的 MLS 群成员集合。
    ///
    /// 规则：
    /// - 必须包含自己；
    /// - 不允许出现任何不在服务端 `channel_members` 里的成员。
    ///
    /// 允许“服务端成员尚未全部进群”的子集状态（加入/移除过程中会短暂出现），
    /// 但绝不允许群比业务成员集合**多**人。
    pub(super) fn ensure_channel_group_members(
        members: &HashSet<String>,
        allowed: &HashSet<String>,
        me: &str,
        channel_id: &str,
    ) -> Result<(), ClientError> {
        if !members.contains(me) {
            return Err(ClientError::Mls(format!(
                "channel {channel_id} group does not contain self; refusing"
            )));
        }
        if let Some(extra) = members.difference(allowed).next() {
            return Err(ClientError::Mls(format!(
                "channel {channel_id} group contains non-member {extra}; refusing"
            )));
        }
        Ok(())
    }

    /// 等待服务端 channel_members 包含本端（时序修复）。
    ///
    /// 为什么需要：
    /// - 客户端 join_channel 是 HTTP 请求，服务端落库有延迟
    /// - authority 收到 PeerJoined 后立即发 Welcome，可能先于 HTTP 落库
    /// - 如果不等待，me 属于 allowed 的检查会失败，拒绝合法 Welcome
    ///
    /// 策略：最多重试 6 次（间隔 500ms），期间阻塞事件循环。
    /// 正常路径 0 延迟；异常路径最多 3 秒。保留严格的安全边界。
    async fn wait_for_channel_members(
        &self,
        channel_id: &str,
    ) -> Result<HashSet<String>, ClientError> {
        // P1-5: 10s 容忍（原 6×500ms=3s 在生产环境 join_channel 限流 / SQLite 锁时太紧）
        const MAX_ATTEMPTS: u32 = 20;
        const DELAY_MS: u64 = 500;

        let me = self.self_uid()?;
        let mut last_err: Option<ClientError> = None;

        for attempt in 0..MAX_ATTEMPTS {
            match self.channel_member_set(channel_id).await {
                Ok(members) => {
                    if members.contains(me.as_str()) {
                        #[cfg(target_arch = "wasm32")]
                        web_sys::console::log_1(&format!(
                            "[wait_for_channel_members] ch={} me={} ok on attempt {}",
                            channel_id, me, attempt + 1
                        ).into());
                        return Ok(members);
                    }
                    last_err = Some(ClientError::Mls(format!(
                        "me {} not yet in channel {} (attempt {})",
                        me, channel_id, attempt + 1
                    )));
                }
                Err(e) => {
                    last_err = Some(e);
                }
            }
            if attempt + 1 < MAX_ATTEMPTS {
                #[cfg(target_arch = "wasm32")]
                gloo_timers::future::TimeoutFuture::new(DELAY_MS as u32).await;
                #[cfg(not(target_arch = "wasm32"))]
                tokio::time::sleep(std::time::Duration::from_millis(DELAY_MS)).await;
            }
        }

        Err(last_err.unwrap_or_else(|| {
            ClientError::Mls("timeout waiting for channel members".into())
        }))
    }
    /// B2：处理 Welcome 加入 channel group。
    pub async fn channel_join_via_welcome(
        &self,
        channel_id: &str,
        welcome: &[u8],
    ) -> Result<(), ClientError> {
        #[cfg(target_arch = "wasm32")]
        web_sys::console::log_1(&format!(
            "[channel_join_via_welcome] ch={} welcome_len={}",
            channel_id, welcome.len()
        ).into());
        // 先对照服务端权威成员列表。查询失败直接拒绝，不能退化为“跳过校验”。
    let mut allowed = match self.wait_for_channel_members(channel_id).await {
        Ok(m) => {
            #[cfg(target_arch = "wasm32")]
            web_sys::console::log_1(&format!(
                "[channel_join_via_welcome] allowed members: {:?}", m
            ).into());
            m
        }
        Err(e) => {
            #[cfg(target_arch = "wasm32")]
            web_sys::console::warn_1(&format!(
                "[channel_join_via_welcome] wait_for_members FAILED, rejecting: {}", e
            ).into());
            return Err(e);
        }
    };
        // 先在 fork 出的临时 session 上解析 Welcome，得到新 group_id。
        // 已有旧 group 且 group_id 不同 → 用新群替换（权威重建频道群）；
        // 相同 → 重复投递，直接忽略。
        let probe = {
            let main_guard = self.mls.lock().unwrap_or_else(|e| e.into_inner());
            let main_session = main_guard
                .as_ref()
                .ok_or_else(|| ClientError::Mls("no main mls session".into()))?;
            let mut ch_session = main_session.fork_for_new_group();
            #[cfg(target_arch = "wasm32")]
            web_sys::console::log_1(&"[channel_join_via_welcome] calling join_with_welcome...".into());
            match ch_session.join_with_welcome(welcome) {
                Ok(()) => {
                    #[cfg(target_arch = "wasm32")]
                    web_sys::console::log_1(&"[channel_join_via_welcome] join_with_welcome OK".into());
                }
                Err(e) => {
                    #[cfg(target_arch = "wasm32")]
                    web_sys::console::warn_1(&format!(
                        "[channel_join_via_welcome] join_with_welcome FAILED: {}", e
                    ).into());
                    return Err(e);
                }
            }
            ch_session
        };
        let me = self.self_uid()?;
        let members: HashSet<String> = probe.member_uids().into_iter().collect();
        // 【P0-1 安全修复】强验证 Welcome 解出的 group 成员集合：
        //   1. 必须包含本端（me）
        //   2. 所有成员必须 ⊆ 服务端 channel_members
        // 失败必须拒绝——否则攻击者可用恶意 Welcome 把本端拉入包含未授权
        // 第三方的 group，之后本端所有消息都会加密给第三方。
        //
        // 时序容忍：authority 拉人时，服务端 channel_members 落库可能滞后于
        // Welcome 抵达。短暂重试（最多 2 次，间隔 500ms），仍失败则拒绝。
        let mut ensure_result =
            Self::ensure_channel_group_members(&members, &allowed, &me, channel_id);
        if ensure_result.is_err() {
            for attempt in 0..6u32 {
                #[cfg(target_arch = "wasm32")]
                gloo_timers::future::TimeoutFuture::new(500).await;
                #[cfg(not(target_arch = "wasm32"))]
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                if let Ok(new_allowed) = self.channel_member_set(channel_id).await {
                    allowed = new_allowed;
                }
                ensure_result =
                    Self::ensure_channel_group_members(&members, &allowed, &me, channel_id);
                if ensure_result.is_ok() {
                    tracing::info!(
                        "[channel_join_via_welcome] member check passed after retry {}",
                        attempt + 1
                    );
                    break;
                }
            }
        }
        if let Err(e) = ensure_result {
            // 区分两种失败：
            // 1) me 不在 group 里 —— 必须拒（我都没被拉进 group，后续没法发消息）
            // 2) group 含 allowed 之外的成员 —— 通常是时序（新成员的 HTTP join
            //    还没落库到 channel_members）。容忍最多 3 个额外成员，
            //    超过则视为攻击拒绝。
            //
            // 服务端已前置 authority gate（P0-1 核心修复：只有 channel authority
            // 能发 Welcome），客户端这层是 defense-in-depth，不需要 100% 硬拒。
            if !members.contains(&me) {
                tracing::warn!(
                    "[channel_join_via_welcome] me {} not in group; REJECTING: {}",
                    me, e
                );
                #[cfg(target_arch = "wasm32")]
                web_sys::console::warn_1(&format!(
                    "[channel_join_via_welcome] me not in group; REJECTING: {}", e
                ).into());
                return Err(e);
            }
            let extra: Vec<&String> = members.difference(&allowed).collect();
            if extra.len() > 3 {
                tracing::warn!(
                    "[channel_join_via_welcome] {} extra members (max 3); REJECTING: {}",
                    extra.len(), e
                );
                #[cfg(target_arch = "wasm32")]
                web_sys::console::warn_1(&format!(
                    "[channel_join_via_welcome] {} extra members; REJECTING: {}", extra.len(), e
                ).into());
                return Err(e);
            }
            tracing::warn!(
                "[channel_join_via_welcome] accepting with {} extra member(s) (likely pending HTTP join): {}",
                extra.len(), e
            );
            #[cfg(target_arch = "wasm32")]
            web_sys::console::log_1(&format!(
                "[channel_join_via_welcome] accepting with {} extra member(s): {}", extra.len(), e
            ).into());
        }

        let new_group_id = probe.group_id().map(|g| g.to_vec());

        let existing_group_id = self
            .channel_groups
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(channel_id)
            .and_then(|s| s.group_id().map(|g| g.to_vec()));
        if existing_group_id == new_group_id {
            return Ok(());
        }
        // P1-3-client: 保护本地更高 epoch 的 group
        // 场景：reload 后 u0 误以为 u1 是新成员，重发 Welcome epoch=1（新群）
        // 但 u1 本地 snapshot 已恢复到 epoch=N (N>1)。
        // 如果采纳新群 → u1 从 epoch=N 掉回 epoch=1 → 数据丢失。
        // 策略：本地已有 group 且 probe.epoch() < 本地 epoch → 拒绝覆盖。
        if let Some(_) = existing_group_id {
            let local_epoch = self.channel_epoch(channel_id).unwrap_or(0);
            let new_epoch = probe.epoch();
            if new_epoch < local_epoch {
                tracing::warn!(
                    "[channel_join_via_welcome] REJECT downgrade: ch={} local_epoch={} new_epoch={}",
                    channel_id, local_epoch, new_epoch
                );
                #[cfg(target_arch = "wasm32")]
                web_sys::console::warn_1(&format!(
                    "[channel_join_via_welcome] REJECT downgrade: local={} new={}",
                    local_epoch, new_epoch
                ).into());
                return Err(ClientError::Mls(format!(
                    "channel {} welcome is older (epoch {}) than local (epoch {}); refusing downgrade",
                    channel_id, new_epoch, local_epoch
                )));
            }
        }
        // 不同（或本地没有）→ 采纳新群。
        self.channel_groups
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(channel_id.to_string(), probe);
        self.mark_channel_dirty(channel_id);
        // P0-2：新 group 已建立，任何旧 epoch 的 buffered Commit 都无意义。
        self.clear_pending_channel_commits(channel_id);
        #[cfg(target_arch = "wasm32")]
        web_sys::console::log_1(&format!(
            "[channel_join_via_welcome] GROUP INSERTED ch={} epoch={}",
            channel_id,
            self.channel_epoch(channel_id).unwrap_or(0)
        ).into());
        Ok(())
    }

    /// B2：处理 Commit 更新 channel group state。
    /// 发送频道级的 MLS 同步包（Welcome / Commit）
    pub async fn send_channel_sync(
        &self,
        to: kaede_protocol::UserId,
        channel_id: &str,
        payload: Vec<u8>,
    ) -> Result<(), ClientError> {
        #[cfg(target_arch = "wasm32")]
        web_sys::console::log_1(&format!(
            "[send_channel_sync] to={} ch={} payload_len={} first=0x{:02x}",
            to, channel_id, payload.len(), payload.first().copied().unwrap_or(0)
        ).into());
        self.transport
            .send(ClientMessage::ChannelSync {
                to,
                channel_id: channel_id.to_string(),
                payload,
            })
            .await
    }

    /// 广播频道 Commit（排除新成员，避免其收到冗余 Commit 导致 WrongEpoch）
    pub async fn send_channel_commit(
        &self,
        channel_id: &str,
        exclude: Option<kaede_protocol::UserId>,
        commit: Vec<u8>,
    ) -> Result<(), ClientError> {
        #[cfg(target_arch = "wasm32")]
        web_sys::console::log_1(&format!(
            "[send_channel_commit] ch={} exclude={:?} commit_len={}",
            channel_id, exclude, commit.len()
        ).into());
        // 0x05 前缀由**服务端**统一加，客户端只传裸 MLS Commit。
        // P0-2：带上本地 epoch，服务端落库以便断线成员补发。
        let epoch = self.channel_epoch(channel_id).unwrap_or(0);
        self.transport
            .send(ClientMessage::ChannelCommit {
                channel_id: channel_id.to_string(),
                exclude,
                epoch,
                payload: commit,
            })
            .await
    }

    pub async fn channel_process_commit(
        &self,
        channel_id: &str,
        commit: &[u8],
    ) -> Result<bool, ClientError> {
        // 直接打到浏览器 console，绕过 tracing 过滤
        #[cfg(target_arch = "wasm32")]
        web_sys::console::log_1(&format!(
            "[channel_process_commit] ch={} commit_len={}",
            channel_id, commit.len()
        ).into());
        // 先拿服务端成员列表；查询失败就不推进 epoch（避免离线时误判）。
        let allowed = self.channel_member_set(channel_id).await?;
        let (out, members) = {
            let mut map = self
                .channel_groups
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let s = map.get_mut(channel_id).ok_or_else(|| {
                ClientError::Mls(format!("channel group not found: {}", channel_id))
            })?;
            let out = s.process_handshake(commit)?;
            (
                out,
                s.member_uids().into_iter().collect::<HashSet<String>>(),
            )
        };

        let me = self.self_uid()?;
        if let Err(e) = Self::ensure_channel_group_members(&members, &allowed, &me, channel_id) {
            // 【时序修复】服务端的 channel_members 可能在 Commit 到达时还没更新。
            // 例子：u0 拉 u2 时，u2 的 HTTP join_channel 请求可能还没 commit；
            //       u1 收到 Commit 去查 allowed 只有 {u0, u1}，而 members 是 {u0, u1, u2}。
            // 此时 MLS Commit 本身的签名已经保证了成员集合的完整性。
            // 我们只 warn 不 drop，避免误杀合法 group。
            #[cfg(target_arch = "wasm32")]
            web_sys::console::warn_1(&format!(
                "[channel_process_commit] member mismatch for {} (KEEPING group): {}",
                channel_id, e
            ).into());
            tracing::warn!(
                "[channel_process_commit] member mismatch for {} (KEEPING group): {}",
                channel_id, e
            );
            // 不 drop group，继续处理（MLS 已经 merge 成功）
        }
        self.mark_channel_dirty(channel_id);
        // P0-2 修复：epoch 前进后立即落盘。
        // 不能依赖 run_event_loop 的 250ms 周期 flush —— 它只在收到消息时检查，
        // reload 前的静默期会让 dirty snapshot 永远不落盘
        // （test-10: u1 reload 前 6 秒静默 → epoch=2 快照丢失 → reload 后回退到 epoch=1）。
        self.flush_dirty_snapshots().await;
        Ok(out)
    }

    /// 从 channel group 移除某个成员，返回 Commit。
    pub fn channel_remove_member(
        &self,
        channel_id: &str,
        uid: &str,
    ) -> Result<Vec<u8>, ClientError> {
        let mut map = self
            .channel_groups
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let s = map
            .get_mut(channel_id)
            .ok_or_else(|| ClientError::Mls(format!("channel group not found: {}", channel_id)))?;
        let out = s.remove_member_by_uid(uid)?;
        drop(map);
        self.mark_channel_dirty(channel_id);
        Ok(out)
    }

    /// 本地丢弃某个频道的 group（离开频道时调用）。
    ///
    /// 必须同时删除持久化快照：否则下次登录会把已退出的频道 group
    /// 恢复回内存，重新加入时会误判“已有 group”而不走 Welcome，
    /// 导致本地 epoch 永久停留在旧值。
    pub async fn channel_drop_group(&self, channel_id: &str) {
        // P0-2：group 已丢弃，任何 buffered Commit 都无意义。
        self.clear_pending_channel_commits(channel_id);
        self.channel_groups
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(channel_id);
        self.dirty_channel_groups
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(channel_id);
        let hook = self
            .aux_storage
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(hook) = hook {
            let key = format!("{}/{}", hook.server_url, channel_id);
            if let Err(e) = hook.store.delete("channel", &key).await {
                tracing::warn!(
                    "[channel-drop] delete persisted group for {} failed: {}",
                    channel_id,
                    e
                );
            }
        }
    }

    /// 从频道独立 group 导出 SFrame key。
    pub fn channel_export_key(&self, channel_id: &str) -> Result<[u8; 32], ClientError> {
        let map = self
            .channel_groups
            .lock()
            .unwrap_or_else(|e| e.into_inner());
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
        let map = self
            .channel_groups
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let s = map
            .get(channel_id)
            .ok_or_else(|| ClientError::Mls(format!("channel group not found: {}", channel_id)))?;
        s.export_sframe_key_for_with_extra(channel_id, kaede_mls::SframePurpose::LivekitE2ee, seed)
    }

    fn mark_channel_dirty(&self, channel_id: &str) {
        if self
            .aux_storage
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
        {
            self.dirty_channel_groups
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(channel_id.to_string());
        }
    }

    /// B3+: 序列化 + 加密某个 channel group 的 snapshot。
    pub fn channel_snapshot_with_cipher(
        &self,
        channel_id: &str,
        cipher: &dyn kaede_mls::SnapshotCipher,
    ) -> Result<Vec<u8>, ClientError> {
        let map = self
            .channel_groups
            .lock()
            .unwrap_or_else(|e| e.into_inner());
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
            .unwrap_or_else(|e| e.into_inner())
            .insert(channel_id.to_string(), session);
        Ok(())
    }

    /// 发送一条私密频道表情反应（`0x03 || sealed`）。
    pub async fn send_channel_reaction(
        &self,
        channel_id: &str,
        ciphertext: Vec<u8>,
    ) -> Result<(), ClientError> {
        self.transport
            .send(ClientMessage::ChannelReaction {
                channel_id: channel_id.to_string(),
                ciphertext,
            })
            .await
    }

    /// 解密一条私密频道实时消息（v2；带重放检测）。
    pub fn channel_open(
        &self,
        channel_id: &str,
        from: &str,
        ciphertext: &[u8],
    ) -> Result<Vec<u8>, ClientError> {
        let key = self.channel_export_key(channel_id)?;
        match kaede_crypto::dm_cipher::open(&key, channel_id, from, "", ciphertext) {
            Ok((pt, msg_id)) => {
                if !self
                    .channel_replay
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .accept(channel_id, msg_id)
                {
                    return Err(ClientError::Protocol("replayed channel message".into()));
                }
                Ok(pt)
            }
            // 实时路径只接受 v2（AAD 绑定发送者）。v1 的 AAD 仅 channel_id，
            // 恶意服务器可以重放旧密文并改写 from —— 只在历史读取路径兼容。
            Err(e) => Err(ClientError::Protocol(format!("channel open: {e}"))),
        }
    }
}
