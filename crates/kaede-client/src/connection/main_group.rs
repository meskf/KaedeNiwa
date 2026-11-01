//! 主群 MLS 生命周期：建群 / 入群 / 加人 / 删人 / 自更新 / exporter / 快照。
//!
//! 所有方法都只操作 `ServerConnection.mls`（主群 session）；DM 与私密频道
//! 的独立 group 见 [`super::dm`] 与 [`super::channel`]。

use kaede_protocol::{ClientMessage, UserId};

use crate::error::ClientError;
use crate::mls_session::MlsSession;
use crate::state::ClientState;

use super::ServerConnection;

impl ServerConnection {
    /// 初始化 MLS 会话（生成签名身份）。每个连接一次。
    pub fn mls_init(&self, uid: impl Into<UserId>) -> Result<(), ClientError> {
        let mut guard = self.mls.lock().unwrap_or_else(|e| e.into_inner());
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
            let mut guard = self.mls.lock().unwrap_or_else(|e| e.into_inner());
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
            let mut guard = self.mls.lock().unwrap_or_else(|e| e.into_inner());
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
        sender: &str,
    ) -> Result<bool, ClientError> {
        // 必须先取 uid，避免在持有 mls 锁时再次加锁。
        let me = {
            let guard = self.mls.lock().unwrap_or_else(|e| e.into_inner());
            guard
                .as_ref()
                .map(|s| s.uid().to_string())
                .ok_or_else(|| ClientError::Mls("mls not initialized".into()))?
        };
        // 主群 Welcome 必须包含自己和发送者（host）。否则恶意方可以构造一个
        // 不含 host 的群替换本地主群，让后续公开频道消息只加密给攻击者。
        let required = [me.as_str(), sender];

        let mut guard = self.mls.lock().unwrap_or_else(|e| e.into_inner());
        let s = guard
            .as_mut()
            .ok_or_else(|| ClientError::Mls("mls not initialized".into()))?;

        if !s.is_ready() {
            s.join_with_welcome_checked(welcome, &required)?;
            self.mark_main_dirty();
            return Ok(true);
        }

        match expected_group_id {
            Some(gid) if Some(gid) == s.group_id() => Ok(false),
            Some(_) => {
                s.reset_and_join_checked(welcome, &required)?;
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
        let kp = {
            let guard = self.mls.lock().unwrap_or_else(|e| e.into_inner());
            let s = guard
                .as_ref()
                .ok_or_else(|| ClientError::Mls("mls not initialized".into()))?;
            s.generate_key_package()?
        };
        // KP 私钥存在 provider storage 里；不落盘的话重启后服务端队列里的
        // 这 20 个 KP 全部无法解密 Welcome。
        self.mark_main_dirty();
        Ok(kp)
    }

    /// 房主 Add 新成员。
    ///
    /// **调用方负责发送返回的 `(commit, welcome)`**：
    /// - commit → `transport.send(ClientMessage::MlsCommit { payload })`（广播）
    /// - welcome → `transport.send(ClientMessage::MlsWelcome { to, payload })`（定向）
    pub fn mls_add_member(&self, kp: &[u8]) -> Result<(Vec<u8>, Vec<u8>), ClientError> {
        let before = self.mls_epoch();
        let (out, backup, new_epoch) = {
            let mut guard = self.mls.lock().unwrap_or_else(|e| e.into_inner());
            let s = guard
                .as_mut()
                .ok_or_else(|| ClientError::Mls("mls not initialized".into()))?;
            // 提交前状态：若服务端同 epoch 已有胜出 Commit，用它回滚本地分支。
            let backup = s.recovery_snapshot()?;
            let out = s.add_member(kp)?;
            (out, backup, s.epoch())
        };
        self.store_commit_backup(new_epoch, backup);
        self.mark_main_dirty();
        self.emit_epoch_if_changed(before);
        Ok(out)
    }

    /// 只加密一条应用消息（推进 sender ratchet），**不发送**。
    ///
    /// 调用方必须在广播前把快照落盘；否则重启后 ratchet 回退会导致
    /// 同一 key + generation 再次加密（AES-GCM nonce 复用）。
    pub fn mls_encrypt_app(&self, plaintext: &[u8]) -> Result<Vec<u8>, ClientError> {
        let ciphertext = {
            let mut guard = self.mls.lock().unwrap_or_else(|e| e.into_inner());
            let s = guard
                .as_mut()
                .ok_or_else(|| ClientError::Mls("mls not initialized".into()))?;
            s.encrypt(plaintext)?
        };
        self.mark_main_dirty();
        Ok(ciphertext)
    }

    /// 广播一条已经加密好的 MLS 应用消息。
    pub async fn mls_send_ciphertext(
        &self,
        channel_id: String,
        ciphertext: Vec<u8>,
        id: i64,
        ts: i64,
    ) -> Result<(), ClientError> {
        self.transport
            .send(ClientMessage::MlsAppMessage {
                channel_id,
                payload: ciphertext,
                id,
                ts,
            })
            .await
    }

    /// 加密并广播一条应用消息。返回密文。
    ///
    /// **仅测试可用**：此方法先发送后返回，调用方无法在广播前落盘，
    /// 重启后 ratchet 回退会导致 AES-GCM nonce 复用。
    /// 生产发送路径必须走 [`Self::mls_encrypt_app`] + 落盘 +
    /// [`Self::mls_send_ciphertext`]。
    #[cfg(test)]
    pub async fn mls_send_app(
        &self,
        channel_id: &str,
        plaintext: &[u8],
    ) -> Result<Vec<u8>, ClientError> {
        let ciphertext = self.mls_encrypt_app(plaintext)?;
        self.mls_send_ciphertext(channel_id.to_string(), ciphertext.clone(), 0, 0)
            .await?;
        Ok(ciphertext)
    }

    /// MLS 解密应用消息，返回 `(plaintext, MLS 认证的发送者 uid)`。
    ///
    /// 服务器注入的 `from` 不可信；调用方必须比较两者，不一致时丢弃。
    pub fn mls_decrypt_with_sender(
        &self,
        ciphertext: &[u8],
    ) -> Result<(Vec<u8>, String), ClientError> {
        let mut guard = self.mls.lock().unwrap_or_else(|e| e.into_inner());
        let s = guard
            .as_mut()
            .ok_or_else(|| ClientError::Mls("mls not initialized".into()))?;
        let out = s.decrypt_with_sender(ciphertext)?;
        self.mark_main_dirty();
        Ok(out)
    }

    /// MLS 解密应用消息（丢弃发送者）。仅用于不关心归属的内部路径。
    pub fn mls_decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, ClientError> {
        self.mls_decrypt_with_sender(ciphertext)
            .map(|(plaintext, _sender)| plaintext)
    }

    /// 移除一个成员。返回 commit bytes。
    pub fn mls_remove_member(
        &self,
        leaf: openmls::prelude::LeafNodeIndex,
    ) -> Result<Vec<u8>, ClientError> {
        let before = self.mls_epoch();
        let (out, backup, new_epoch) = {
            let mut guard = self.mls.lock().unwrap_or_else(|e| e.into_inner());
            let s = guard
                .as_mut()
                .ok_or_else(|| ClientError::Mls("mls not initialized".into()))?;
            let backup = s.recovery_snapshot()?;
            let out = s.remove_member(leaf)?;
            (out, backup, s.epoch())
        };
        self.store_commit_backup(new_epoch, backup);
        self.mark_main_dirty();
        self.emit_epoch_if_changed(before);
        Ok(out)
    }

    /// 按 uid 移除成员。返回 commit bytes。
    ///
    /// **调用方负责广播 commit**（`Self::send_commit`）。
    pub fn mls_remove_member_by_uid(&self, uid: &str) -> Result<Vec<u8>, ClientError> {
        let before = self.mls_epoch();
        let (out, backup, new_epoch) = {
            let mut guard = self.mls.lock().unwrap_or_else(|e| e.into_inner());
            let s = guard
                .as_mut()
                .ok_or_else(|| ClientError::Mls("mls not initialized".into()))?;
            let backup = s.recovery_snapshot()?;
            let out = s.remove_member_by_uid(uid)?;
            (out, backup, s.epoch())
        };
        self.store_commit_backup(new_epoch, backup);
        self.mark_main_dirty();
        self.emit_epoch_if_changed(before);
        Ok(out)
    }
    /// MLS 自更新。返回 commit bytes。
    pub fn mls_self_update(&self) -> Result<Vec<u8>, ClientError> {
        let before = self.mls_epoch();
        let (out, backup, new_epoch) = {
            let mut guard = self.mls.lock().unwrap_or_else(|e| e.into_inner());
            let s = guard
                .as_mut()
                .ok_or_else(|| ClientError::Mls("mls not initialized".into()))?;
            let backup = s.recovery_snapshot()?;
            let out = s.self_update()?;
            (out, backup, s.epoch())
        };
        self.store_commit_backup(new_epoch, backup);
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
        let guard = self.mls.lock().unwrap_or_else(|e| e.into_inner());
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
        let guard = self.mls.lock().unwrap_or_else(|e| e.into_inner());
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
        let guard = self.mls.lock().unwrap_or_else(|e| e.into_inner());
        guard.as_ref().map(|s| s.member_uids()).unwrap_or_default()
    }

    pub fn mls_epoch(&self) -> u64 {
        self.mls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|s| s.epoch())
            .unwrap_or(0)
    }

    /// 当前主群 group_id。未建组返回 `None`。
    pub fn mls_group_id(&self) -> Option<Vec<u8>> {
        self.mls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|s| s.group_id().map(|g| g.to_vec()))
    }

    /// 群组成员数。未初始化返回 0。
    pub fn mls_member_count(&self) -> usize {
        self.mls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|s| s.member_count())
            .unwrap_or(0)
    }

    /// 序列化 + 加密。
    pub fn mls_snapshot_with_cipher(
        &self,
        cipher: &dyn kaede_mls::SnapshotCipher,
    ) -> Result<Vec<u8>, ClientError> {
        let guard = self.mls.lock().unwrap_or_else(|e| e.into_inner());
        let s = guard
            .as_ref()
            .ok_or_else(|| ClientError::Mls("mls not initialized".into()))?;
        s.snapshot_with(cipher)
    }

    /// 从快照恢复 + 解密。
    pub fn mls_restore_from_snapshot_with_cipher(
        &self,
        snapshot: &[u8],
        cipher: Option<&dyn kaede_mls::SnapshotCipher>,
    ) -> Result<(), ClientError> {
        let session = MlsSession::from_snapshot_with(snapshot, cipher)?;
        let mut guard = self.mls.lock().unwrap_or_else(|e| e.into_inner());
        *guard = Some(session);
        Ok(())
    }

    /// 群组是否就绪。
    pub fn mls_is_ready(&self) -> bool {
        self.mls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|s| s.is_ready())
            .unwrap_or(false)
    }
}
