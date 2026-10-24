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

    /// B2：处理 Welcome 加入 channel group。
    pub async fn channel_join_via_welcome(
        &self,
        channel_id: &str,
        welcome: &[u8],
    ) -> Result<(), ClientError> {
        // 先对照服务端权威成员列表。查询失败直接拒绝，不能退化为“跳过校验”。
        let allowed = self.channel_member_set(channel_id).await?;
        // 先在 fork 出的临时 session 上解析 Welcome，得到新 group_id。
        // 已有旧 group 且 group_id 不同 → 用新群替换（权威重建频道群）；
        // 相同 → 重复投递，直接忽略。
        let probe = {
            let main_guard = self.mls.lock().unwrap_or_else(|e| e.into_inner());
            let main_session = main_guard
                .as_ref()
                .ok_or_else(|| ClientError::Mls("no main mls session".into()))?;
            let mut ch_session = main_session.fork_for_new_group();
            ch_session.join_with_welcome(welcome)?;
            ch_session
        };
        let me = self.self_uid()?;
        let members: HashSet<String> = probe.member_uids().into_iter().collect();
        Self::ensure_channel_group_members(&members, &allowed, &me, channel_id)?;

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
        // 不同（或本地没有）→ 采纳新群。
        self.channel_groups
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(channel_id.to_string(), probe);
        self.mark_channel_dirty(channel_id);
        Ok(())
    }

    /// B2：处理 Commit 更新 channel group state。
    pub async fn channel_process_commit(
        &self,
        channel_id: &str,
        commit: &[u8],
    ) -> Result<bool, ClientError> {
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
            // 已经 merge 的非法 commit 无法回滚：直接丢弃整个频道 group，
            // 让后续发送因“无 group”而被拒绝，再引导用户重新加入频道。
            self.channel_drop_group(channel_id).await;
            return Err(e);
        }
        self.mark_channel_dirty(channel_id);
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
