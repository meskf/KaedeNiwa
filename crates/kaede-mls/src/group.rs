//! MLS 群组封装。

use std::sync::Arc;

use crate::provider::KaedeMlsProvider;
use openmls::prelude::tls_codec::{Deserialize, Serialize};
use openmls::prelude::*;

use crate::error::{MlsError, mls_err};
use crate::identity::MlsIdentity;
use crate::{SFRAME_EXPORT_LABEL, SFRAME_KEY_LEN, SframePurpose};

/// 保留多少个 past epoch 的消息密钥。
///
/// 默认 0 会导致“任何成员变更后，旧 epoch 的聊天历史全部解不开”。
/// 产品需要可回放历史，因此这里显式保留 64 个 epoch。
///
/// **安全权衡**：past epoch secrets 会被持久化，削弱前向保密
/// （拿到当前 state 的攻击者可解密最近 64 个 epoch 的消息）。
/// 这是为了历史可读性做的明确取舍；不要在同一 epoch 内无限累积。
pub const MAX_PAST_EPOCHS: usize = 64;

/// MLS 群组句柄。
pub struct MlsGroupHandle {
    group: MlsGroup,
    identity: Arc<MlsIdentity>,
}

impl MlsGroupHandle {
    /// 创建新群组（当前用户是唯一成员）。
    pub fn create(
        identity: Arc<MlsIdentity>,
        provider: &KaedeMlsProvider,
    ) -> Result<Self, MlsError> {
        let config = MlsGroupCreateConfig::builder()
            .ciphersuite(identity.ciphersuite())
            .use_ratchet_tree_extension(true)
            .max_past_epochs(MAX_PAST_EPOCHS)
            .build();

        let group = MlsGroup::new(
            provider,
            identity.signature_keys(),
            &config,
            identity.credential_with_key().clone(),
        )
        .map_err(mls_err)?;

        Ok(Self { group, identity })
    }

    /// 通过 Welcome 加入已有群组。
    pub fn join(
        identity: Arc<MlsIdentity>,
        provider: &KaedeMlsProvider,
        welcome_bytes: &[u8],
    ) -> Result<Self, MlsError> {
        let msg = MlsMessageIn::tls_deserialize(&mut &welcome_bytes[..])
            .map_err(|e| MlsError::TlsCodec(e.to_string()))?;
        let welcome = match msg.extract() {
            MlsMessageBodyIn::Welcome(w) => w,
            _ => return Err(MlsError::OpenMls("expected Welcome message".into())),
        };

        let join_config = MlsGroupJoinConfig::builder()
            .use_ratchet_tree_extension(true)
            .max_past_epochs(MAX_PAST_EPOCHS)
            .build();

        let staged = StagedWelcome::new_from_welcome(provider, &join_config, welcome, None)
            .map_err(mls_err)?;
        let group = staged.into_group(provider).map_err(mls_err)?;

        Ok(Self { group, identity })
    }

    /// 添加成员。返回 `(commit_bytes, welcome_bytes)`。
    pub fn add_member(
        &mut self,
        provider: &KaedeMlsProvider,
        key_package_bytes: &[u8],
    ) -> Result<(Vec<u8>, Vec<u8>), MlsError> {
        // KeyPackage 是公开数据，独立于 MLSMessage 封装传输。
        // 发送方传的是 KeyPackage::tls_serialize_detached() 的裸字节。
        let kp_in = KeyPackageIn::tls_deserialize(&mut &key_package_bytes[..])
            .map_err(|e| MlsError::TlsCodec(e.to_string()))?;
        let kp = kp_in
            .validate(provider.crypto(), ProtocolVersion::Mls10)
            .map_err(mls_err)?;

        // P0-A: reject members whose sender_id collides with an existing one.
        // SFrame uses SHA256(uid)[0..4] as the high 32 bits of the counter,
        // so collision means shared nonce space => AES-GCM catastrophic failure.
        //
        // MM3: 严格 UTF-8（不再用 from_utf8_lossy）。恶意 KP 用非 UTF-8
        // credential 会导致:
        //   - candidate_uid 变成 "alice�"，不与 "alice" 碰撞
        //   - 但 member_uids() 里也有 mangling → remove_member_by_uid("alice") 找不到
        let cred_bytes = kp.leaf_node().credential().serialized_content();
        let candidate_uid = std::str::from_utf8(cred_bytes)
            .map_err(|e| MlsError::OpenMls(format!("credential not UTF-8: {}", e)))?
            .to_string();
        let existing: Vec<String> = self.member_uids();
        // MLS 不要求 credential 唯一；同一 uid 重复 Add 会产生两个 leaf，
        // remove_member_by_uid 只会删一个，且两个 leaf 共享 sender_id。
        if existing.iter().any(|u| u == &candidate_uid) {
            return Err(MlsError::OpenMls(format!(
                "member already in group: {}",
                candidate_uid
            )));
        }
        if let Some(other) = kaede_crypto::find_sender_id_collision(
            &candidate_uid,
            existing.iter().map(String::as_str),
        ) {
            return Err(MlsError::SenderIdCollision {
                candidate: candidate_uid,
                existing: other,
            });
        }

        let (commit, welcome, _group_info) = self
            .group
            .add_members(provider, self.identity.signature_keys(), &[kp])
            .map_err(mls_err)?;
        self.group.merge_pending_commit(provider).map_err(mls_err)?;

        Ok((
            commit
                .tls_serialize_detached()
                .map_err(|e| MlsError::TlsCodec(e.to_string()))?,
            welcome
                .tls_serialize_detached()
                .map_err(|e| MlsError::TlsCodec(e.to_string()))?,
        ))
    }

    /// 按 uid 找 leaf index。
    ///
    /// 用于踢人：从 group 里按 credential 找对应的 leaf。
    /// 找不到返回 [`MlsError::OpenMls`]。
    pub fn leaf_index_of(&self, uid: &str) -> Result<LeafNodeIndex, MlsError> {
        let cred: Credential = BasicCredential::new(uid.as_bytes().to_vec()).into();
        self.group
            .member_leaf_index(&cred)
            .ok_or_else(|| MlsError::OpenMls(format!("member not in group: {}", uid)))
    }

    /// 按 uid 移除成员。返回 commit bytes。
    ///
    /// **调用方负责广播 commit**（`ClientMessage::MlsCommit`）。
    /// 其它成员收到 commit 后 epoch 前进，SFrame key 随之轮换。
    pub fn remove_member_by_uid(
        &mut self,
        provider: &KaedeMlsProvider,
        uid: &str,
    ) -> Result<Vec<u8>, MlsError> {
        let leaf = self.leaf_index_of(uid)?;
        self.remove_member(provider, leaf)
    }
    /// 移除成员（按 leaf index）。返回 commit bytes。
    pub fn remove_member(
        &mut self,
        provider: &KaedeMlsProvider,
        leaf: LeafNodeIndex,
    ) -> Result<Vec<u8>, MlsError> {
        let (commit, _, _) = self
            .group
            .remove_members(provider, self.identity.signature_keys(), &[leaf])
            .map_err(mls_err)?;
        self.group.merge_pending_commit(provider).map_err(mls_err)?;
        commit
            .tls_serialize_detached()
            .map_err(|e| MlsError::TlsCodec(e.to_string()))
    }

    /// MLS 自更新（sender chain 前进）。返回 `(commit_bytes, welcome_bytes)`。
    ///
    /// welcome 通常为空（没人被 Add）。
    pub fn update(&mut self, provider: &KaedeMlsProvider) -> Result<(Vec<u8>, Vec<u8>), MlsError> {
        let leaf_node_params = LeafNodeParameters::default();
        let bundle = self
            .group
            .self_update(provider, self.identity.signature_keys(), leaf_node_params)
            .map_err(mls_err)?;
        self.group.merge_pending_commit(provider).map_err(mls_err)?;

        // CommitMessageBundle 有借用 getter：.commit() / .welcome()
        let welcome_bytes = match bundle.welcome() {
            Some(w) => w
                .tls_serialize_detached()
                .map_err(|e| MlsError::TlsCodec(e.to_string()))?,
            None => Vec::new(),
        };
        let commit_bytes = bundle
            .commit()
            .tls_serialize_detached()
            .map_err(|e| MlsError::TlsCodec(e.to_string()))?;

        Ok((commit_bytes, welcome_bytes))
    }

    /// 从 provider 的 storage 加载已有 group。
    ///
    /// 用于持久化恢复：`snapshot()` 保存后，重建 provider 时用此方法。
    pub fn load_from_storage(
        identity: Arc<MlsIdentity>,
        provider: &KaedeMlsProvider,
        group_id: &[u8],
    ) -> Result<Self, MlsError> {
        use openmls::prelude::GroupId;
        let gid = GroupId::from_slice(group_id);
        let mut group = MlsGroup::load(provider.storage(), &gid)
            .map_err(mls_err)?
            .ok_or_else(|| MlsError::GroupNotFound(format!("{:?}", group_id)))?;
        // 旧 snapshot 里的 join config 可能是 max_past_epochs=0（历史解不开）。
        // 恢复时显式升级，让历史可读。
        group
            .set_past_epoch_deletion_policy(
                provider,
                PastEpochDeletionPolicy::MaxEpochs(MAX_PAST_EPOCHS),
            )
            .map_err(|e| MlsError::OpenMls(format!("set past epoch policy: {:?}", e)))?;
        Ok(Self { group, identity })
    }

    /// 加密应用消息。
    pub fn encrypt_app(
        &mut self,
        provider: &KaedeMlsProvider,
        plaintext: &[u8],
    ) -> Result<Vec<u8>, MlsError> {
        let out = self
            .group
            .create_message(provider, self.identity.signature_keys(), plaintext)
            .map_err(mls_err)?;
        out.tls_serialize_detached()
            .map_err(|e| MlsError::TlsCodec(e.to_string()))
    }

    /// 解密应用消息，并返回 **MLS 认证的发送者 uid**。
    ///
    /// `from` 字段由服务器注入，恶意服务器可以改写；真正的发送者身份
    /// 必须取自 MLS 验证过的 sender credential。调用方应比较两者，
    /// 不一致时丢弃消息。
    pub fn decrypt_app_with_sender(
        &mut self,
        provider: &KaedeMlsProvider,
        ciphertext: &[u8],
    ) -> Result<(Vec<u8>, String), MlsError> {
        let msg = MlsMessageIn::tls_deserialize(&mut &ciphertext[..])
            .map_err(|e| MlsError::TlsCodec(e.to_string()))?;
        let proto: ProtocolMessage = msg
            .try_into_protocol_message()
            .map_err(|e| MlsError::OpenMls(format!("{:?}", e)))?;
        let processed = self
            .group
            .process_message(provider, proto)
            .map_err(mls_err)?;

        // sender credential 已被 MLS 签名验证；在 into_content 之前取出。
        let sender = processed.credential().serialized_content().to_vec();
        let sender = String::from_utf8(sender)
            .map_err(|e| MlsError::OpenMls(format!("sender credential not UTF-8: {}", e)))?;

        match processed.into_content() {
            ProcessedMessageContent::ApplicationMessage(app) => Ok((app.into_bytes(), sender)),
            _ => Err(MlsError::NotApplicationMessage),
        }
    }

    /// 解密应用消息。非 ApplicationMessage 会被拒绝。
    ///
    /// 不返回发送者；安全敏感路径请用 [`Self::decrypt_app_with_sender`]。
    pub fn decrypt_app(
        &mut self,
        provider: &KaedeMlsProvider,
        ciphertext: &[u8],
    ) -> Result<Vec<u8>, MlsError> {
        self.decrypt_app_with_sender(provider, ciphertext)
            .map(|(plaintext, _sender)| plaintext)
    }

    /// 处理握手消息（Commit / Proposal）。成功返回 true。
    pub fn process_handshake(
        &mut self,
        provider: &KaedeMlsProvider,
        msg_bytes: &[u8],
    ) -> Result<bool, MlsError> {
        let msg = MlsMessageIn::tls_deserialize(&mut &msg_bytes[..])
            .map_err(|e| MlsError::TlsCodec(e.to_string()))?;
        let proto: ProtocolMessage = msg
            .try_into_protocol_message()
            .map_err(|e| MlsError::OpenMls(format!("{:?}", e)))?;
        let processed = self
            .group
            .process_message(provider, proto)
            .map_err(mls_err)?;

        match processed.into_content() {
            ProcessedMessageContent::StagedCommitMessage(sc) => {
                self.group
                    .merge_staged_commit(provider, *sc)
                    .map_err(mls_err)?;
                Ok(true)
            }
            ProcessedMessageContent::ProposalMessage(_) => Err(MlsError::OpenMls(
                "proposal flow not supported; use add_members / remove_members directly".into(),
            )),
            ProcessedMessageContent::ExternalJoinProposalMessage(_) => Err(MlsError::OpenMls(
                "external join proposal not supported".into(),
            )),
            ProcessedMessageContent::OwnPendingCommit => Ok(true),
            ProcessedMessageContent::OwnPrivateMessage => Err(MlsError::NotApplicationMessage),
            ProcessedMessageContent::ApplicationMessage(_) => Err(MlsError::NotApplicationMessage),
        }
    }

    /// 导出 SFrame 密钥。channel_id 作为 exporter context。
    ///
    /// 等价于 `export_sframe_key_for(provider, channel_id, SframePurpose::Default)`。
    /// 保留此签名是为了向后兼容（旧测试 / 旧调用点）。
    pub fn export_sframe_key(
        &self,
        provider: &KaedeMlsProvider,
        channel_id: &str,
    ) -> Result<[u8; SFRAME_KEY_LEN], MlsError> {
        self.export_sframe_key_for(provider, channel_id, SframePurpose::Default)
    }

    /// 导出 SFrame 密钥（带 purpose）。
    ///
    /// # exporter context
    ///
    /// ```text
    /// context = channel_id || 0x00 || purpose.as_bytes()
    /// ```
    ///
    /// 同一 (channel_id, purpose) 派生的 key 只被这一个用途使用，
    /// 不同 purpose 的 key 空间天然分离，消除 AES-GCM nonce 复用。
    ///
    /// # 为什么用 0x00 分隔
    ///
    /// `::` 等文本分隔符有歧义：
    /// `("a::b", "c")` 和 `("a", "b::c")` 拼出同一个 context。
    /// `0x00` 不会出现在 channel_id 中（用户输入），
    /// 也不会出现在 purpose 常量中，所以 (channel_id, purpose) -> context 是单射。
    pub fn export_sframe_key_for(
        &self,
        provider: &KaedeMlsProvider,
        channel_id: &str,
        purpose: SframePurpose,
    ) -> Result<[u8; SFRAME_KEY_LEN], MlsError> {
        let mut ctx = Vec::with_capacity(channel_id.len() + 1 + purpose.as_bytes().len());
        ctx.extend_from_slice(channel_id.as_bytes());
        ctx.push(0x00);
        ctx.extend_from_slice(purpose.as_bytes());

        let mut secret = self
            .group
            .export_secret(provider.crypto(), SFRAME_EXPORT_LABEL, &ctx, SFRAME_KEY_LEN)
            .map_err(mls_err)?;
        let mut key = [0u8; SFRAME_KEY_LEN];
        key.copy_from_slice(&secret);
        zeroize::Zeroize::zeroize(&mut secret);
        Ok(key)
    }

    /// P0-2: 导出 SFrame 密钥（带 purpose + 额外上下文）。
    ///
    /// exporter context = `channel_id || 0x00 || purpose || 0x00 || extra`。
    /// `extra` 用于「房间级 seed」——同一房间所有客户端传同一个 seed（从
    /// server `/api/livekit/token` 拿），派生出同一个 e2ee key；两次会话
    /// seed 不同则 key 不同，避免 LiveKit SDK ratchet 重启导致的 nonce 复用。
    /// 服务端不知道 exporter 输出（仍是 E2EE）。
    ///
    /// # 与 `export_sframe_key_for` 的关系
    ///
    /// **两者返回不同 key**。本方法的 exporter context 比 [`Self::export_sframe_key_for`]
    /// 多了一段 `0x00 || extra`，因此同 group / 同 channel / 同 purpose 下：
    ///
    /// ```text
    /// export_sframe_key_for(...)                     -> key_A
    /// export_sframe_key_for_with_extra(..., &seed)   -> key_B (!= key_A)
    /// ```
    ///
    /// 调用方必须前后一致：同一用途（如 `LivekitE2ee`）只能用其一。
    pub fn export_sframe_key_for_with_extra(
        &self,
        provider: &KaedeMlsProvider,
        channel_id: &str,
        purpose: SframePurpose,
        extra: &[u8],
    ) -> Result<[u8; SFRAME_KEY_LEN], MlsError> {
        let mut ctx =
            Vec::with_capacity(channel_id.len() + 1 + purpose.as_bytes().len() + 1 + extra.len());
        ctx.extend_from_slice(channel_id.as_bytes());
        ctx.push(0x00);
        ctx.extend_from_slice(purpose.as_bytes());
        ctx.push(0x00);
        ctx.extend_from_slice(extra);

        let mut secret = self
            .group
            .export_secret(provider.crypto(), SFRAME_EXPORT_LABEL, &ctx, SFRAME_KEY_LEN)
            .map_err(mls_err)?;
        let mut key = [0u8; SFRAME_KEY_LEN];
        key.copy_from_slice(&secret);
        zeroize::Zeroize::zeroize(&mut secret);
        Ok(key)
    }

    /// 群组 ID（tls 字节）。
    pub fn group_id(&self) -> Vec<u8> {
        self.group.group_id().as_slice().to_vec()
    }

    /// 当前 epoch。
    pub fn epoch(&self) -> u64 {
        self.group.epoch().as_u64()
    }

    /// 成员数。
    pub fn member_count(&self) -> usize {
        self.group.members().count()
    }

    /// 当前群组所有成员 uid（从 credential 解析），用于 sender_id 碰撞检查。
    ///
    /// MM3: 使用 `from_utf8_lossy` —— 这里的 credential 已经通过 `add_member`
    /// 的严格 UTF-8 校验，非 UTF-8 成员不可能进群。保留 lossy 作为兜底，
    /// 避免 group state 异常时整个函数 panic。
    pub fn member_uids(&self) -> Vec<String> {
        self.group
            .members()
            .map(|m| String::from_utf8_lossy(m.credential.serialized_content()).to_string())
            .collect()
    }

    /// HM2: 添加成员并**校验 KeyPackage 的 credential 与 `expected_uid` 一致**。
    ///
    /// `add_member` 不校验 credential —— 恶意 KeyPackage 可让 attacker
    /// 冒充任意 uid 加入群（C8/H8）。本方法在 `add_member` 之前先验证
    /// credential，是防御"KP 与身份脱钩"的第一道防线。
    ///
    /// **新代码应优先使用本方法。**
    pub fn add_member_for(
        &mut self,
        provider: &KaedeMlsProvider,
        key_package_bytes: &[u8],
        expected_uid: &str,
    ) -> Result<(Vec<u8>, Vec<u8>), MlsError> {
        let cred = crate::identity::extract_credential_from_kp(key_package_bytes)?;
        if cred != expected_uid {
            return Err(MlsError::CredentialMismatch {
                expected: expected_uid.to_string(),
                got: cred,
            });
        }
        self.add_member(provider, key_package_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_provider() -> KaedeMlsProvider {
        KaedeMlsProvider::new()
    }

    #[test]
    fn create_single_member_group() {
        let provider = new_provider();
        let id = Arc::new(MlsIdentity::generate("alice").unwrap());
        let group = MlsGroupHandle::create(id, &provider).unwrap();
        assert_eq!(group.epoch(), 0); // openmls epoch 从 0 开始
        assert_eq!(group.member_count(), 1);
    }

    #[test]
    fn export_sframe_key_deterministic() {
        let provider = new_provider();
        let id = Arc::new(MlsIdentity::generate("alice").unwrap());
        let group = MlsGroupHandle::create(id, &provider).unwrap();

        let k1 = group.export_sframe_key(&provider, "ch-general").unwrap();
        let k2 = group.export_sframe_key(&provider, "ch-general").unwrap();
        assert_eq!(k1, k2);

        let k3 = group.export_sframe_key(&provider, "ch-voice").unwrap();
        assert_ne!(k1, k3);
    }

    #[test]
    fn add_and_message_roundtrip() {
        // Alice 创建群组
        let alice_provider = new_provider();
        let alice_id = Arc::new(MlsIdentity::generate("alice").unwrap());
        let mut alice_group = MlsGroupHandle::create(alice_id.clone(), &alice_provider).unwrap();

        // Bob 生成 KeyPackage
        let bob_provider = new_provider();
        let bob_id = Arc::new(MlsIdentity::generate("bob").unwrap());
        let bob_kp_bundle = KeyPackage::builder()
            .build(
                bob_id.ciphersuite(),
                &bob_provider,
                bob_id.signature_keys(),
                bob_id.credential_with_key().clone(),
            )
            .expect("build kp");
        let bob_kp_bytes = bob_kp_bundle
            .key_package()
            .tls_serialize_detached()
            .unwrap();

        // Alice 添加 Bob
        let (_commit, welcome_bytes) = alice_group
            .add_member(&alice_provider, &bob_kp_bytes)
            .unwrap();
        assert_eq!(alice_group.member_count(), 2);

        // Bob 通过 Welcome 加入
        let mut bob_group =
            MlsGroupHandle::join(bob_id.clone(), &bob_provider, &welcome_bytes).unwrap();
        assert_eq!(bob_group.member_count(), 2);

        // Alice 发消息 → Bob 解
        let plaintext = b"hello bob";
        let ciphertext = alice_group.encrypt_app(&alice_provider, plaintext).unwrap();
        let decrypted = bob_group.decrypt_app(&bob_provider, &ciphertext).unwrap();
        assert_eq!(decrypted, plaintext);

        // Bob 发消息 → Alice 解
        let reply = b"hi alice";
        let ct2 = bob_group.encrypt_app(&bob_provider, reply).unwrap();
        let pt2 = alice_group.decrypt_app(&alice_provider, &ct2).unwrap();
        assert_eq!(pt2, reply);
    }

    #[test]
    fn leaf_index_lookup_and_remove_by_uid() {
        let alice_provider = new_provider();
        let alice_id = Arc::new(MlsIdentity::generate("alice").unwrap());
        let mut alice_group = MlsGroupHandle::create(alice_id.clone(), &alice_provider).unwrap();

        let bob_provider = new_provider();
        let bob_id = Arc::new(MlsIdentity::generate("bob").unwrap());
        let bob_kp_bundle = KeyPackage::builder()
            .build(
                bob_id.ciphersuite(),
                &bob_provider,
                bob_id.signature_keys(),
                bob_id.credential_with_key().clone(),
            )
            .unwrap();
        let bob_kp_bytes = bob_kp_bundle
            .key_package()
            .tls_serialize_detached()
            .unwrap();

        let (_c, welcome) = alice_group
            .add_member(&alice_provider, &bob_kp_bytes)
            .unwrap();
        let _bob_group = MlsGroupHandle::join(bob_id, &bob_provider, &welcome).unwrap();
        assert_eq!(alice_group.member_count(), 2);

        // 按 uid 找 leaf
        let leaf = alice_group.leaf_index_of("bob").unwrap();
        assert!(leaf.u32() > 0, "bob 不是 index 0");

        // 按 uid 移除
        let commit = alice_group
            .remove_member_by_uid(&alice_provider, "bob")
            .unwrap();
        assert!(!commit.is_empty());
        assert_eq!(alice_group.member_count(), 1);

        // 再找应该失败
        assert!(alice_group.leaf_index_of("bob").is_err());
    }
    #[test]
    fn add_member_rejects_sender_id_collision() {
        use kaede_crypto::sframe::sender_id_from_uid;
        use std::collections::HashMap;

        // Birthday-paradox: sample until a collision appears (~99% at 200k).
        let mut seen: HashMap<u32, String> = HashMap::new();
        let mut collision: Option<(String, String)> = None;
        for i in 0..200_000u32 {
            let uid = format!("u{}", i);
            let sid = sender_id_from_uid(&uid);
            if let Some(prev) = seen.get(&sid) {
                collision = Some((prev.clone(), uid));
                break;
            }
            seen.insert(sid, uid);
        }
        let (a, b) = collision.expect("birthday paradox: 200k uids should collide");

        let provider_a = new_provider();
        let id_a = Arc::new(MlsIdentity::generate(&a).unwrap());
        let mut group = MlsGroupHandle::create(id_a, &provider_a).unwrap();

        let provider_b = new_provider();
        let id_b = Arc::new(MlsIdentity::generate(&b).unwrap());
        let kp_bundle = KeyPackage::builder()
            .build(
                id_b.ciphersuite(),
                &provider_b,
                id_b.signature_keys(),
                id_b.credential_with_key().clone(),
            )
            .unwrap();
        let kp_bytes = kp_bundle.key_package().tls_serialize_detached().unwrap();

        let r = group.add_member(&provider_a, &kp_bytes);
        assert!(
            matches!(r, Err(MlsError::SenderIdCollision { .. })),
            "expected SenderIdCollision"
        );
        assert_eq!(group.member_count(), 1, "group must not change");
    }

    /// P0-2 回归：成员在 epoch 前进后仍能解密上一个 epoch 的聊天历史。
    ///
    /// `max_past_epochs(64)` 之前默认是 0，导致任何成员变更后旧历史全部丢失。
    #[test]
    fn past_epoch_message_still_decrypts_for_existing_member() {
        let ap = new_provider();
        let ai = Arc::new(MlsIdentity::generate("alice").unwrap());
        let mut alice = MlsGroupHandle::create(ai.clone(), &ap).unwrap();

        let bp = new_provider();
        let bi = Arc::new(MlsIdentity::generate("bob").unwrap());
        let bob_kp = KeyPackage::builder()
            .build(
                bi.ciphersuite(),
                &bp,
                bi.signature_keys(),
                bi.credential_with_key().clone(),
            )
            .unwrap()
            .key_package()
            .tls_serialize_detached()
            .unwrap();
        let (_c1, w1) = alice.add_member(&ap, &bob_kp).unwrap();
        let mut bob = MlsGroupHandle::join(bi.clone(), &bp, &w1).unwrap();

        // epoch 1 的消息
        let ct = alice.encrypt_app(&ap, b"history").unwrap();

        // 加入 carol → epoch 2
        let cp = new_provider();
        let ci = Arc::new(MlsIdentity::generate("carol").unwrap());
        let carol_kp = KeyPackage::builder()
            .build(
                ci.ciphersuite(),
                &cp,
                ci.signature_keys(),
                ci.credential_with_key().clone(),
            )
            .unwrap()
            .key_package()
            .tls_serialize_detached()
            .unwrap();
        let (commit2, w2) = alice.add_member(&ap, &carol_kp).unwrap();
        bob.process_handshake(&bp, &commit2).unwrap();
        let carol = MlsGroupHandle::join(ci, &cp, &w2).unwrap();
        assert_eq!(bob.epoch(), alice.epoch());
        assert_eq!(carol.epoch(), alice.epoch());

        // Bob 是 epoch 1 的成员 → 仍可解密 epoch 1 的历史
        let pt = bob.decrypt_app(&bp, &ct).unwrap();
        assert_eq!(pt, b"history");

        // Carol 是 epoch 2 才加入的 → 没有 epoch 1 的密钥，不能解密（符合 MLS 语义）
        assert!(carol.epoch() >= 2 && bob.epoch() >= 2);
    }

    #[test]
    fn alice_bob_agree_on_sframe_key() {
        let alice_provider = new_provider();
        let alice_id = Arc::new(MlsIdentity::generate("alice").unwrap());
        let mut alice_group = MlsGroupHandle::create(alice_id.clone(), &alice_provider).unwrap();

        let bob_provider = new_provider();
        let bob_id = Arc::new(MlsIdentity::generate("bob").unwrap());
        let bob_kp_bundle = KeyPackage::builder()
            .build(
                bob_id.ciphersuite(),
                &bob_provider,
                bob_id.signature_keys(),
                bob_id.credential_with_key().clone(),
            )
            .unwrap();
        let bob_kp_bytes = bob_kp_bundle
            .key_package()
            .tls_serialize_detached()
            .unwrap();

        let (_c, welcome_bytes) = alice_group
            .add_member(&alice_provider, &bob_kp_bytes)
            .unwrap();
        let bob_group = MlsGroupHandle::join(bob_id, &bob_provider, &welcome_bytes).unwrap();

        let a_key = alice_group
            .export_sframe_key(&alice_provider, "ch-general")
            .unwrap();
        let b_key = bob_group
            .export_sframe_key(&bob_provider, "ch-general")
            .unwrap();
        assert_eq!(a_key, b_key, "both members must derive the same SFrame key");
    }
}

#[cfg(test)]
mod purpose_tests {
    use super::*;
    use crate::SframePurpose;

    fn new_provider() -> KaedeMlsProvider {
        KaedeMlsProvider::new()
    }

    #[test]
    fn different_purposes_yield_different_keys() {
        let provider = new_provider();
        let id = Arc::new(MlsIdentity::generate("alice").unwrap());
        let group = MlsGroupHandle::create(id, &provider).unwrap();

        let k_default = group.export_sframe_key(&provider, "general").unwrap();
        let k_voice_a = group
            .export_sframe_key_for(&provider, "general", SframePurpose::VoiceAudio)
            .unwrap();
        let k_voice_v = group
            .export_sframe_key_for(&provider, "general", SframePurpose::VoiceVideo)
            .unwrap();
        let k_screen_v = group
            .export_sframe_key_for(&provider, "general", SframePurpose::ScreenVideo)
            .unwrap();

        assert_ne!(k_default, k_voice_a);
        assert_ne!(k_voice_a, k_voice_v, "audio and video must not share key");
        assert_ne!(k_voice_v, k_screen_v);
        assert_ne!(k_default, k_screen_v);

        let k_explicit = group
            .export_sframe_key_for(&provider, "general", SframePurpose::Default)
            .unwrap();
        assert_eq!(k_default, k_explicit, "old API must equal explicit Default");
    }

    #[test]
    fn different_channels_same_purpose_differ() {
        let provider = new_provider();
        let id = Arc::new(MlsIdentity::generate("alice").unwrap());
        let group = MlsGroupHandle::create(id, &provider).unwrap();

        let a = group
            .export_sframe_key_for(&provider, "ch-a", SframePurpose::VoiceAudio)
            .unwrap();
        let b = group
            .export_sframe_key_for(&provider, "ch-b", SframePurpose::VoiceAudio)
            .unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn alice_bob_agree_on_every_purpose() {
        let alice_provider = new_provider();
        let alice_id = Arc::new(MlsIdentity::generate("alice").unwrap());
        let mut alice_group = MlsGroupHandle::create(alice_id.clone(), &alice_provider).unwrap();

        let bob_provider = new_provider();
        let bob_id = Arc::new(MlsIdentity::generate("bob").unwrap());
        let bob_kp_bundle = KeyPackage::builder()
            .build(
                bob_id.ciphersuite(),
                &bob_provider,
                bob_id.signature_keys(),
                bob_id.credential_with_key().clone(),
            )
            .unwrap();
        let bob_kp_bytes = bob_kp_bundle
            .key_package()
            .tls_serialize_detached()
            .unwrap();

        let (_c, welcome_bytes) = alice_group
            .add_member(&alice_provider, &bob_kp_bytes)
            .unwrap();
        let bob_group = MlsGroupHandle::join(bob_id, &bob_provider, &welcome_bytes).unwrap();

        for purpose in [
            SframePurpose::Default,
            SframePurpose::Text,
            SframePurpose::VoiceAudio,
            SframePurpose::VoiceVideo,
            SframePurpose::ScreenVideo,
            SframePurpose::ScreenAudio,
            SframePurpose::File,
        ] {
            let a = alice_group
                .export_sframe_key_for(&alice_provider, "general", purpose)
                .unwrap();
            let b = bob_group
                .export_sframe_key_for(&bob_provider, "general", purpose)
                .unwrap();
            assert_eq!(a, b, "purpose {:?} must agree across members", purpose);
        }
    }
}

// ============================================================================
// P0-ε: export_sframe_key_for_with_extra 对齐测试
// ============================================================================
// 守护 P0-α + P0-2 的核心假设：
//   1. extra（房间级 seed）确实参与 key 派生（不是被忽略）
//   2. 同 seed 跨成员派生出同 key（多人语音基础）
//   3. 不同 seed 派生不同 key（seed 是有效熵源）
#[cfg(test)]
mod with_extra_tests {
    use super::*;
    use crate::SframePurpose;
    use crate::identity::MlsIdentity;
    use crate::provider::KaedeMlsProvider;
    use std::sync::Arc;

    fn new_provider() -> KaedeMlsProvider {
        KaedeMlsProvider::new()
    }

    fn setup_two_members() -> (
        MlsGroupHandle,
        KaedeMlsProvider,
        MlsGroupHandle,
        KaedeMlsProvider,
    ) {
        let alice_provider = new_provider();
        let alice_id = Arc::new(MlsIdentity::generate("alice").unwrap());
        let mut alice_group = MlsGroupHandle::create(alice_id.clone(), &alice_provider).unwrap();

        let bob_provider = new_provider();
        let bob_id = Arc::new(MlsIdentity::generate("bob").unwrap());
        let bob_kp_bundle = KeyPackage::builder()
            .build(
                bob_id.ciphersuite(),
                &bob_provider,
                bob_id.signature_keys(),
                bob_id.credential_with_key().clone(),
            )
            .unwrap();
        let bob_kp_bytes = bob_kp_bundle
            .key_package()
            .tls_serialize_detached()
            .unwrap();

        let (_c, welcome_bytes) = alice_group
            .add_member(&alice_provider, &bob_kp_bytes)
            .unwrap();
        let bob_group = MlsGroupHandle::join(bob_id, &bob_provider, &welcome_bytes).unwrap();

        (alice_group, alice_provider, bob_group, bob_provider)
    }

    #[test]
    fn with_extra_differs_from_without() {
        let (alice_group, alice_provider, _, _) = setup_two_members();
        let seed = [0x42u8; 32];

        let without = alice_group
            .export_sframe_key_for(&alice_provider, "ch-general", SframePurpose::LivekitE2ee)
            .unwrap();
        let with = alice_group
            .export_sframe_key_for_with_extra(
                &alice_provider,
                "ch-general",
                SframePurpose::LivekitE2ee,
                &seed,
            )
            .unwrap();

        assert_ne!(
            without, with,
            "extra context must change derived key (P0-2 guard: seed participates in KDF)"
        );
    }

    #[test]
    fn with_extra_agrees_across_members() {
        let (alice_group, alice_provider, bob_group, bob_provider) = setup_two_members();
        let seed = [0x42u8; 32];

        let a = alice_group
            .export_sframe_key_for_with_extra(
                &alice_provider,
                "ch-general",
                SframePurpose::LivekitE2ee,
                &seed,
            )
            .unwrap();
        let b = bob_group
            .export_sframe_key_for_with_extra(
                &bob_provider,
                "ch-general",
                SframePurpose::LivekitE2ee,
                &seed,
            )
            .unwrap();

        assert_eq!(
            a, b,
            "both members with same seed must derive same key (multi-party voice)"
        );
    }

    #[test]
    fn with_extra_differs_by_seed() {
        let (alice_group, alice_provider, _, _) = setup_two_members();
        let seed_a = [0x42u8; 32];
        let seed_b = [0x43u8; 32];

        let k_a = alice_group
            .export_sframe_key_for_with_extra(
                &alice_provider,
                "ch-general",
                SframePurpose::LivekitE2ee,
                &seed_a,
            )
            .unwrap();
        let k_b = alice_group
            .export_sframe_key_for_with_extra(
                &alice_provider,
                "ch-general",
                SframePurpose::LivekitE2ee,
                &seed_b,
            )
            .unwrap();

        assert_ne!(
            k_a, k_b,
            "different seeds must derive different keys (seed is effective entropy)"
        );
    }
}
