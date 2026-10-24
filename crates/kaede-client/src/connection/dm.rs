//! DM 双人会话：独立 2 人 MLS group + v2 AEAD 密封 + 代际仲裁。
//!
//! conv_id 一律先经 [`canonical_conv_id`] 规范化（`dm:<min_uid>:<max_uid>`），
//! 保证两端派生相同的 exporter context。

use std::collections::HashSet;

use crate::error::ClientError;
use crate::event::ClientEvent;
use crate::mls_session::MlsSession;

use super::ServerConnection;

impl ServerConnection {
    /// 从 MLS exporter 派生指定 DM 会话的 32 字节对称密钥。
    ///
    /// exporter context = `conv_id || 0x00 || "dm"`（见 SframePurpose::Dm）。
    /// 必须在 MLS ready 后调用。
    pub fn dm_derive_key(&self, conv_id: &str) -> Result<[u8; 32], ClientError> {
        let conv_id = &canonical_conv_id(conv_id);
        let map = self.dm_groups.lock().unwrap_or_else(|e| e.into_inner());
        let s = map
            .get(conv_id)
            .ok_or_else(|| ClientError::Mls(format!("DM group not found for {}", conv_id)))?;
        s.export_dm_key(conv_id)
    }

    /// B2：本地是否已持有该 conv 对应的独立 2 人 MLS group。
    pub fn dm_has_group(&self, conv_id: &str) -> bool {
        self.dm_groups
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(&canonical_conv_id(conv_id))
    }

    /// B2 Phase 2：从 dm group 派生 LiveKit E2EE 媒体 key。
    ///
    /// seed 来自 server token（房间级），目的是让 DM 双方对齐 key；
    /// 非 DM 成员即使拿到 seed，没有 dm group state 也算不出 key。
    pub fn dm_export_media_key(&self, conv_id: &str, seed: &[u8]) -> Result<[u8; 32], ClientError> {
        let conv_id = &canonical_conv_id(conv_id);
        let map = self.dm_groups.lock().unwrap_or_else(|e| e.into_inner());
        let s = map
            .get(conv_id)
            .ok_or_else(|| ClientError::Mls(format!("dm group not found: {}", conv_id)))?;
        s.export_sframe_key_for_with_extra(conv_id, kaede_mls::SframePurpose::LivekitE2ee, seed)
    }

    /// B2：删除该 conv 对应的 DM group（Welcome 发送失败时回滚）。
    pub fn dm_remove_group(&self, conv_id: &str) {
        self.dm_groups
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&canonical_conv_id(conv_id));
    }

    /// 从 dm group 按 purpose 派生密钥（P0：附件加密用 File）。
    pub fn dm_export_key_for(
        &self,
        conv_id: &str,
        purpose: kaede_mls::SframePurpose,
    ) -> Result<[u8; 32], ClientError> {
        let conv_id = &canonical_conv_id(conv_id);
        let map = self.dm_groups.lock().unwrap_or_else(|e| e.into_inner());
        let s = map
            .get(conv_id)
            .ok_or_else(|| ClientError::Mls(format!("dm group not found: {}", conv_id)))?;
        s.export_sframe_key_for(conv_id, purpose)
    }

    /// 加密一条 DM 消息，返回 sealed 密文（可直接 POST 到 server）。
    /// 发起方：创建 2 人 DM group 并返回 `(Welcome, generation)`。
    pub fn dm_init_group(
        &self,
        conv_id: &str,
        peer_kp: &[u8],
    ) -> Result<(Vec<u8>, u64), ClientError> {
        let conv_id = &canonical_conv_id(conv_id);
        // 先解析会话双方，避免持有 mls 锁时再次加锁。
        let me = self.self_uid()?;
        let (_, peer) = dm_parties(conv_id, &me)?;
        let expected: HashSet<String> = HashSet::from([me.clone(), peer.clone()]);

        let main_guard = self.mls.lock().unwrap_or_else(|e| e.into_inner());
        let main_session = main_guard
            .as_ref()
            .ok_or_else(|| ClientError::Mls("no main mls session".into()))?;
        let mut dm_session = main_session.fork_for_new_group();
        dm_session.create_group()?;
        // 校验 KP credential == 预期对端，防止服务端返回错误 KP。
        let (_commit, welcome) = dm_session.add_member_for(peer_kp, &peer)?;
        let members: HashSet<String> = dm_session.member_uids().into_iter().collect();
        if members != expected {
            return Err(ClientError::Mls(format!(
                "DM group members {:?} != expected {:?} for {}",
                members, expected, conv_id
            )));
        }
        let mut map = self.dm_groups.lock().unwrap_or_else(|e| e.into_inner());
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
            .unwrap_or_else(|e| e.into_inner())
            .get(&conv_id)
            .copied()
            .unwrap_or(0)
    }

    fn set_dm_generation(&self, conv_id: &str, generation: u64) {
        let conv_id = canonical_conv_id(conv_id);
        self.dm_generations
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(conv_id.clone(), generation);
        if self
            .aux_storage
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
        {
            self.dirty_dm_meta
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(conv_id);
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

        // DM 群必须恰好是会话双方。MLS Welcome 只证明密码学正确，
        // 不证明群成员符合业务语义；对端（或恶意服务器）可以把第三方
        // Add 进来，让受害者后续消息加密给第三方。
        let me = self.self_uid()?;
        let (_, peer) = dm_parties(conv_id, &me)?;
        let expected: HashSet<String> = HashSet::from([me.clone(), peer.clone()]);

        let main_guard = self.mls.lock().unwrap_or_else(|e| e.into_inner());
        let main_session = main_guard
            .as_ref()
            .ok_or_else(|| ClientError::Mls("no main mls session".into()))?;
        let mut dm_session = main_session.fork_for_new_group();
        if let Err(e) = dm_session.join_with_welcome(welcome) {
            // 历史页 + 实时推送会把同一个 Welcome 投递两次；第一次 join 已
            // 消费 KP，第二次必然 NoMatchingKeyPackage。已有可用 group 时
            // 安全忽略，避免把重复投递升级成用户可见的同步错误。
            let has_existing = self
                .dm_groups
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains_key(conv_id);
            if has_existing && generation <= local_gen {
                tracing::debug!(
                    "[dm] duplicate/unusable welcome for {} (gen {} <= {}), keeping existing group: {}",
                    conv_id,
                    generation,
                    local_gen,
                    e
                );
                return Ok(());
            }
            return Err(e);
        }
        let members: HashSet<String> = dm_session.member_uids().into_iter().collect();
        if members != expected {
            return Err(ClientError::Mls(format!(
                "DM group members {:?} != expected {:?} for {}; refusing welcome",
                members, expected, conv_id
            )));
        }
        let new_gid = dm_session.group_id().map(|g| g.to_vec());
        drop(main_guard);

        let mut map = self.dm_groups.lock().unwrap_or_else(|e| e.into_inner());
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
        if generation > local_gen && local_gen > 0 {
            self.set_dm_generation(conv_id, generation);
            self.emit_local_event(ClientEvent::MlsSyncError {
                server_id: self.server_id.clone(),
                user_message: format!("私聊 {} 的对端已重建会话，本地旧消息不可恢复", conv_id),
                recoverable: true,
                detail: format!("dm generation {} -> {}", local_gen, generation),
            });
        } else if generation > local_gen {
            // 首次建立（0 -> 1）不是“重建”，不需要告警。
            self.set_dm_generation(conv_id, generation);
        }
        self.mark_dm_dirty(conv_id);
        Ok(())
    }

    fn mark_dm_dirty(&self, conv_id: &str) {
        if self
            .aux_storage
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
        {
            self.dirty_dm_groups
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(conv_id.to_string());
        }
    }

    /// B3+: 序列化 + 加密某个 DM group 的 snapshot。
    pub fn dm_snapshot_with_cipher(
        &self,
        conv_id: &str,
        cipher: &dyn kaede_mls::SnapshotCipher,
    ) -> Result<Vec<u8>, ClientError> {
        let conv_id = &canonical_conv_id(conv_id);
        let map = self.dm_groups.lock().unwrap_or_else(|e| e.into_inner());
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
            .unwrap_or_else(|e| e.into_inner())
            .insert(conv_id.to_string(), session);
        Ok(())
    }

    pub fn dm_seal(&self, conv_id: &str, plaintext: &[u8]) -> Result<Vec<u8>, ClientError> {
        let conv_id = &canonical_conv_id(conv_id);
        let me = self.self_uid()?;
        let key = self.dm_derive_key(conv_id)?;
        let (from, to) = dm_parties(conv_id, &me)?;
        kaede_crypto::dm_cipher::seal(&key, conv_id, &from, &to, plaintext)
            .map_err(|e| ClientError::Protocol(format!("dm seal: {e}")))
    }

    /// 解密一条实时 DM 消息（仅 v2；带重放检测）。
    ///
    /// 实时路径**不再接受 v1**：v1 的 AAD 只有 conv_id，不含发送者/方向/消息 id，
    /// 恶意服务器可以重放历史 v1 密文并伪装成任意发送者。v1 只允许在
    /// [`Self::dm_open_history`] 里读取升级前历史。
    pub fn dm_open(
        &self,
        conv_id: &str,
        from: &str,
        ciphertext: &[u8],
    ) -> Result<Vec<u8>, ClientError> {
        let conv_id = &canonical_conv_id(conv_id);
        let me = self.self_uid()?;
        let key = self.dm_derive_key(conv_id)?;
        match kaede_crypto::dm_cipher::open(&key, conv_id, from, &me, ciphertext) {
            Ok((pt, msg_id)) => {
                if !self
                    .dm_replay
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .accept(conv_id, msg_id)
                {
                    return Err(ClientError::Protocol("replayed DM message".into()));
                }
                Ok(pt)
            }
            Err(e) => Err(ClientError::Protocol(format!("dm open: {e}"))),
        }
    }

    /// 读取 DM 历史：不参与实时重放窗口（同一消息可能被重复拉取）。
    pub fn dm_open_history(
        &self,
        conv_id: &str,
        from: &str,
        ciphertext: &[u8],
    ) -> Result<Vec<u8>, ClientError> {
        let conv_id = &canonical_conv_id(conv_id);
        let me = self.self_uid()?;
        let key = self.dm_derive_key(conv_id)?;
        // AAD 必须与实际发送时的方向一致：
        // - 对方发的：from=peer, to=me
        // - 自己发的：from=me, to=peer（实时路径不会推回 sender，
        //   只会在重启后从历史里读到）
        let (_, peer) = dm_parties(conv_id, &me)?;
        let (aad_from, aad_to) = if from == me.as_str() {
            (me.clone(), peer)
        } else {
            (from.to_string(), me.clone())
        };
        match kaede_crypto::dm_cipher::open(&key, conv_id, &aad_from, &aad_to, ciphertext) {
            Ok((pt, _)) => Ok(pt),
            Err(kaede_crypto::dm_cipher::DmCipherError::UnknownVersion(_)) => {
                kaede_crypto::dm_cipher::open_legacy(&key, conv_id, ciphertext)
                    .map_err(|e| ClientError::Protocol(format!("dm open: {e}")))
            }
            Err(e) => Err(ClientError::Protocol(format!("dm open: {e}"))),
        }
    }
}

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

/// H10: 规范化 DM conv_id 为 `dm:<min_uid>:<max_uid>`（字典序）。
///
/// server 端 `dm::conv_id_for` 已保证字典序，客户端防御性规范化：
/// 若未来 server 或攻击者发送非规范 conv_id（`dm:alice:bob` vs
/// `dm:bob:alice`），两端会派生出不同 exporter context → 不同 key
/// → 消息永远解不开。规范化是幂等的。
///
/// 非 `dm:` 前缀原样返回（兼容 channel_id 等其他用途）。
fn dm_parties(conv_id: &str, uid: &str) -> Result<(String, String), ClientError> {
    let rest = conv_id
        .strip_prefix("dm:")
        .ok_or_else(|| ClientError::Protocol(format!("not a DM conv: {conv_id}")))?;
    let (a, b) = rest
        .split_once(':')
        .ok_or_else(|| ClientError::Protocol(format!("bad DM conv: {conv_id}")))?;
    if a == uid {
        Ok((a.to_string(), b.to_string()))
    } else if b == uid {
        Ok((b.to_string(), a.to_string()))
    } else {
        Err(ClientError::Protocol(format!(
            "uid {uid} is not a party of {conv_id}"
        )))
    }
}

fn canonical_conv_id(conv_id: &str) -> String {
    if let Some(rest) = conv_id.strip_prefix("dm:")
        && let Some((a, b)) = rest.split_once(':')
    {
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        return format!("dm:{}:{}", lo, hi);
    }
    conv_id.to_string()
}
