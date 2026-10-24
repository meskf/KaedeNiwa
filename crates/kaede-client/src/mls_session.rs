//! MLS 会话封装。
//!
//! # 生命周期
//!
//! 1. [`MlsSession::new`] —— 生成签名身份
//! 2. [`MlsSession::create_group`] / [`MlsSession::join_with_welcome`]
//! 3. [`MlsSession::add_member`] / [`MlsSession::encrypt`] / [`MlsSession::decrypt`]
//! 4. [`MlsSession::snapshot`] 保存整个状态；[`MlsSession::from_snapshot`] 恢复
//!
//! # 快照格式
//!
//! ```text
//! [ version: u8 = 1 ]
//! [ uid_len: u16 BE ][ uid bytes ]
//! [ sig_scheme: u16 BE ]
//! [ sig_priv_len: u32 BE ][ sig_priv bytes ]
//! [ sig_pub_len: u32 BE ][ sig_pub bytes ]
//! [ group_id_len: u16 BE ][ group_id bytes ]  (0 = 无群组)
//! [ storage_len: u32 BE ][ storage snapshot bytes ]
//! ```
//!
//! # 存储
//!
//! 使用 [`KaedeMlsProvider`]（`RustCrypto` + `MemoryStorage`）。
//! 快照可以存到 SQLite / IndexedDB。

use std::sync::Arc;

use kaede_mls::{KaedeMlsProvider, MlsGroupHandle, MlsIdentity};
use kaede_protocol::UserId;
use openmls_traits::types::SignatureScheme;

use crate::error::ClientError;

const SNAPSHOT_VERSION: u8 = 1;
/// 外层 cipher 头（区别于内层 version=1 / Provider 的 0x02）。
const SNAPSHOT_CIPHER_HEADER: u8 = 0x03;

/// 单个 server 的 MLS 会话。
pub struct MlsSession {
    identity: Arc<MlsIdentity>,
    /// 共享 provider：同一用户的 main / DM / channel group 必须共享 provider，
    /// 因为 KeyPackageBundle 的私钥存在 provider 里（Welcome 解密需要）。
    provider: Arc<KaedeMlsProvider>,
    group: Option<MlsGroupHandle>,
    /// 当前群组 ID（来自 openmls）。`None` 表示未建组。
    group_id: Option<Vec<u8>>,
}

impl MlsSession {
    /// 生成新身份，但尚未建组。
    pub fn new(uid: impl Into<UserId>) -> Result<Self, ClientError> {
        let uid = uid.into();
        let identity = Arc::new(MlsIdentity::generate(uid.as_str())?);
        let provider = Arc::new(KaedeMlsProvider::new());
        Ok(Self {
            identity,
            provider,
            group: None,
            group_id: None,
        })
    }

    pub fn is_ready(&self) -> bool {
        self.group.is_some()
    }

    pub fn uid(&self) -> &str {
        self.identity.uid()
    }

    /// 当前 MLS 群组所有成员 uid（从 credential 解析）。
    ///
    /// **P0-5**：用于 `find_sender_id_collision` 检查。未建组时返回空。
    pub fn member_uids(&self) -> Vec<String> {
        self.group
            .as_ref()
            .map(|g| g.member_uids())
            .unwrap_or_default()
    }

    pub fn epoch(&self) -> u64 {
        self.group.as_ref().map(|g| g.epoch()).unwrap_or(0)
    }

    pub fn member_count(&self) -> usize {
        self.group.as_ref().map(|g| g.member_count()).unwrap_or(0)
    }

    pub fn group_id(&self) -> Option<&[u8]> {
        self.group_id.as_deref()
    }

    // ---------------------------------------------------------------------
    // 群组生命周期
    // ---------------------------------------------------------------------

    /// 克隆当前会话的身份和 provider，但清空 group 状态。
    /// 用于创建独立的 DM group，复用同一个 MLS 身份。
    pub fn fork_for_new_group(&self) -> Self {
        Self {
            identity: self.identity.clone(),
            provider: self.provider.clone(),
            group: None,
            group_id: None,
        }
    }

    /// 从 DM 独立群组导出 SFrame 密钥。
    pub fn export_dm_key(&self, conv_id: &str) -> Result<[u8; 32], ClientError> {
        self.export_sframe_key_for(conv_id, kaede_mls::SframePurpose::Dm)
    }
    pub fn create_group(&mut self) -> Result<(), ClientError> {
        if self.group.is_some() {
            return Err(ClientError::Mls("group already exists".into()));
        }
        let g = MlsGroupHandle::create(self.identity.clone(), &self.provider)?;
        self.group_id = Some(g.group_id());
        self.group = Some(g);
        Ok(())
    }

    pub fn join_with_welcome(&mut self, welcome: &[u8]) -> Result<(), ClientError> {
        if self.group.is_some() {
            return Err(ClientError::Mls("group already exists".into()));
        }
        let g = MlsGroupHandle::join(self.identity.clone(), &self.provider, welcome)?;
        self.group_id = Some(g.group_id());
        self.group = Some(g);
        Ok(())
    }

    /// 加入 Welcome，并校验群成员包含 `required` 中的全部 uid。
    ///
    /// MLS 本身只保证 Welcome 的签名与加密正确，**不保证群的成员集合符合业务预期**：
    /// 恶意对端/频道权威可以构造一个把第三方也 Add 进来的群，受害者加入后会把
    /// 后续消息加密给这个第三方。因此所有业务群在 join 时必须显式校验成员集合。
    ///
    /// 校验在赋值给 `self.group` 之前完成；失败时内存中的会话状态不变
    /// （已消费的 KeyPackage 私钥无法回收，这是 MLS 一次性凭证的固有限制）。
    pub fn join_with_welcome_checked(
        &mut self,
        welcome: &[u8],
        required: &[&str],
    ) -> Result<(), ClientError> {
        if self.group.is_some() {
            return Err(ClientError::Mls("group already exists".into()));
        }
        let g = MlsGroupHandle::join(self.identity.clone(), &self.provider, welcome)?;
        let members: std::collections::HashSet<String> = g.member_uids().into_iter().collect();
        for uid in required {
            if !members.contains(*uid) {
                return Err(ClientError::Mls(format!(
                    "welcome group does not contain required member {uid}; refusing"
                )));
            }
        }
        self.group_id = Some(g.group_id());
        self.group = Some(g);
        Ok(())
    }

    /// 丢弃当前主群，改用 Welcome 指向的新群。
    ///
    /// 用于“房主丢失快照后重建主群”的恢复路径：已有本地群的一方
    /// 不能再简单忽略新 Welcome，否则双方永久分裂。
    ///
    /// **会丢失旧群的 epoch/历史密钥**；调用方必须提示用户。
    pub fn reset_and_join(&mut self, welcome: &[u8]) -> Result<(), ClientError> {
        self.group = None;
        self.group_id = None;
        self.join_with_welcome(welcome)
    }

    /// 重建路径的校验版本：先解析并校验新群，成功后才替换旧群。
    ///
    /// 校验失败时保留原群，避免攻击者用一个非法 Welcome 把本地状态打空。
    pub fn reset_and_join_checked(
        &mut self,
        welcome: &[u8],
        required: &[&str],
    ) -> Result<(), ClientError> {
        let old_group = self.group.take();
        let old_group_id = self.group_id.take();
        match self.join_with_welcome_checked(welcome, required) {
            Ok(()) => Ok(()),
            Err(e) => {
                self.group = old_group;
                self.group_id = old_group_id;
                Err(e)
            }
        }
    }

    pub fn add_member(&mut self, key_package: &[u8]) -> Result<(Vec<u8>, Vec<u8>), ClientError> {
        let g = self
            .group
            .as_mut()
            .ok_or_else(|| ClientError::Mls("no group".into()))?;
        Ok(g.add_member(&self.provider, key_package)?)
    }

    /// 添加成员并校验 KeyPackage 的 credential 与 `expected_uid` 一致。
    pub fn add_member_for(
        &mut self,
        key_package: &[u8],
        expected_uid: &str,
    ) -> Result<(Vec<u8>, Vec<u8>), ClientError> {
        let g = self
            .group
            .as_mut()
            .ok_or_else(|| ClientError::Mls("no group".into()))?;
        Ok(g.add_member_for(&self.provider, key_package, expected_uid)?)
    }

    pub fn remove_member(
        &mut self,
        leaf: openmls::prelude::LeafNodeIndex,
    ) -> Result<Vec<u8>, ClientError> {
        let g = self
            .group
            .as_mut()
            .ok_or_else(|| ClientError::Mls("no group".into()))?;
        Ok(g.remove_member(&self.provider, leaf)?)
    }

    /// 按 uid 移除成员。返回 commit bytes。
    ///
    /// **调用方负责广播 commit**（`ClientMessage::MlsCommit`）。
    pub fn remove_member_by_uid(&mut self, uid: &str) -> Result<Vec<u8>, ClientError> {
        let g = self
            .group
            .as_mut()
            .ok_or_else(|| ClientError::Mls("no group".into()))?;
        Ok(g.remove_member_by_uid(&self.provider, uid)?)
    }
    pub fn self_update(&mut self) -> Result<Vec<u8>, ClientError> {
        let g = self
            .group
            .as_mut()
            .ok_or_else(|| ClientError::Mls("no group".into()))?;
        let (commit, _welcome) = g.update(&self.provider)?;
        Ok(commit)
    }

    pub fn generate_key_package(&self) -> Result<Vec<u8>, ClientError> {
        Ok(self.identity.generate_key_package(&self.provider)?)
    }

    // ---------------------------------------------------------------------
    // 消息
    // ---------------------------------------------------------------------

    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, ClientError> {
        let g = self
            .group
            .as_mut()
            .ok_or_else(|| ClientError::Mls("no group".into()))?;
        Ok(g.encrypt_app(&self.provider, plaintext)?)
    }

    pub fn decrypt(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>, ClientError> {
        self.decrypt_with_sender(ciphertext)
            .map(|(plaintext, _sender)| plaintext)
    }

    /// 解密并返回 MLS 认证的发送者 uid。
    pub fn decrypt_with_sender(
        &mut self,
        ciphertext: &[u8],
    ) -> Result<(Vec<u8>, String), ClientError> {
        let g = self
            .group
            .as_mut()
            .ok_or_else(|| ClientError::Mls("no group".into()))?;
        Ok(g.decrypt_app_with_sender(&self.provider, ciphertext)?)
    }

    pub fn process_handshake(&mut self, msg: &[u8]) -> Result<bool, ClientError> {
        let g = self
            .group
            .as_mut()
            .ok_or_else(|| ClientError::Mls("no group".into()))?;
        Ok(g.process_handshake(&self.provider, msg)?)
    }

    pub fn export_sframe_key(&self, channel_id: &str) -> Result<[u8; 32], ClientError> {
        self.export_sframe_key_for(channel_id, kaede_mls::SframePurpose::Default)
    }

    /// 导出 SFrame 密钥（带 purpose）。
    ///
    /// 不同 purpose 派生独立 key，消除 AES-GCM nonce 复用。
    /// 派生 LiveKit E2EE 密钥（32 字节 AES-256）。
    ///
    /// 与 server 的 `HMAC(LIVEKIT_API_SECRET, "kaede-e2ee-v1:" || channel_id)`
    /// 不同：这个 key 只依赖 MLS group 状态，server 不参与。
    ///
    /// 与 `export_sframe_key_for(..., SframePurpose::Default)` 也不同，
    /// 使用独立的 `LivekitE2ee` purpose 域。
    pub fn export_livekit_e2ee_key(&self, channel_id: &str) -> Result<[u8; 32], ClientError> {
        self.export_sframe_key_for(channel_id, kaede_mls::SframePurpose::LivekitE2ee)
    }

    pub fn export_sframe_key_for(
        &self,
        channel_id: &str,
        purpose: kaede_mls::SframePurpose,
    ) -> Result<[u8; 32], ClientError> {
        let g = self
            .group
            .as_ref()
            .ok_or_else(|| ClientError::Mls("no group".into()))?;
        Ok(g.export_sframe_key_for(&self.provider, channel_id, purpose)?)
    }

    /// P0-2: 带额外上下文派生 SFrame key（房间级 seed 注入）。
    pub fn export_sframe_key_for_with_extra(
        &self,
        channel_id: &str,
        purpose: kaede_mls::SframePurpose,
        extra: &[u8],
    ) -> Result<[u8; 32], ClientError> {
        let g = self
            .group
            .as_ref()
            .ok_or_else(|| ClientError::Mls("no group".into()))?;
        Ok(g.export_sframe_key_for_with_extra(&self.provider, channel_id, purpose, extra)?)
    }

    // ---------------------------------------------------------------------
    // 快照 / 恢复
    // ---------------------------------------------------------------------

    /// 序列化整个 MLS 状态（不加密）。
    ///
    /// **包含签名私钥**。生产代码应使用 [`Self::snapshot_with`]。
    #[deprecated(
        since = "0.2.0",
        note = "plaintext snapshot leaks signing private key to disk; use snapshot_with(cipher)"
    )]
    pub fn snapshot(&self) -> Result<Vec<u8>, ClientError> {
        self.snapshot_inner()
    }

    /// 序列化 + 加密。
    ///
    /// 输出外层格式：`[0x03][alg_id:u8][ct_len:u32 BE][ct]`
    /// 其中 `ct` 是 [`Self::snapshot_inner`] 的输出加密结果。
    pub fn snapshot_with(
        &self,
        cipher: &dyn kaede_mls::SnapshotCipher,
    ) -> Result<Vec<u8>, ClientError> {
        let mut inner = self.snapshot_inner()?;
        let ct = cipher
            .encrypt(&inner)
            .map_err(|e| ClientError::Mls(format!("snapshot encrypt: {}", e)))?;

        // S-9: 明文（含签名私钥）在加密后立刻清零
        use zeroize::Zeroize;
        inner.zeroize();
        // P0-H：ct.len() as u32 在 4GB+ 时静默回绕
        if ct.len() > u32::MAX as usize {
            return Err(ClientError::Mls(format!(
                "encrypted snapshot too large: {} bytes (max {})",
                ct.len(),
                u32::MAX
            )));
        }
        let mut out = Vec::with_capacity(6 + ct.len());
        out.push(SNAPSHOT_CIPHER_HEADER);
        out.push(cipher.alg_id());
        out.extend_from_slice(&(ct.len() as u32).to_be_bytes());
        out.extend_from_slice(&ct);
        Ok(out)
    }

    fn snapshot_inner(&self) -> Result<Vec<u8>, ClientError> {
        let mut buf = Vec::with_capacity(512);

        // version
        buf.push(SNAPSHOT_VERSION);

        // uid
        let uid = self.identity.uid().as_bytes();
        buf.extend_from_slice(&(uid.len() as u16).to_be_bytes());
        buf.extend_from_slice(uid);

        // sig scheme
        let scheme_u16: u16 = self.identity.signature_keys().scheme() as u16;
        buf.extend_from_slice(&scheme_u16.to_be_bytes());

        // sig priv
        let priv_bytes = self.identity.signature_keys().private_bytes();
        buf.extend_from_slice(&(priv_bytes.len() as u32).to_be_bytes());
        buf.extend_from_slice(priv_bytes);

        // sig pub
        let pub_bytes = self.identity.signature_keys().public_bytes();
        buf.extend_from_slice(&(pub_bytes.len() as u32).to_be_bytes());
        buf.extend_from_slice(pub_bytes);

        // group_id
        match &self.group_id {
            Some(gid) => {
                buf.extend_from_slice(&(gid.len() as u16).to_be_bytes());
                buf.extend_from_slice(gid);
            }
            None => {
                buf.extend_from_slice(&0u16.to_be_bytes());
            }
        }

        // storage
        let mut storage_bytes = self
            .provider
            .snapshot()
            .map_err(|e| ClientError::Mls(format!("provider snapshot: {}", e)))?;
        buf.extend_from_slice(&(storage_bytes.len() as u32).to_be_bytes());
        buf.extend_from_slice(&storage_bytes);

        // H7: storage_bytes 含 KeyPackage 私钥，中间缓冲必须显式清零
        use zeroize::Zeroize;
        storage_bytes.zeroize();

        Ok(buf)
    }

    /// 从快照恢复（不加密路径）。
    pub fn from_snapshot(snapshot: &[u8]) -> Result<Self, ClientError> {
        Self::from_snapshot_with(snapshot, None)
    }

    /// 内存回滚备份：序列化当前状态（明文，仅保存在内存）。
    ///
    /// 用于 Commit 冲突时回滚本地已应用但未被服务端采纳的 Commit。
    /// **绝不可**直接写盘——写盘必须走 [`Self::snapshot_with`]。
    pub(crate) fn recovery_snapshot(&self) -> Result<Vec<u8>, ClientError> {
        self.snapshot_inner()
    }

    /// 从内存回滚备份恢复。
    pub(crate) fn from_recovery_snapshot(snapshot: &[u8]) -> Result<Self, ClientError> {
        Self::from_snapshot_inner(snapshot)
    }

    /// 从快照恢复。
    ///
    /// **fail-closed**：只接受带 `0x03` 外层头、且 alg 非 NOOP 的加密快照。
    /// 老格式明文快照会被拒绝——否则任何能写本地存储的攻击者都可以把
    /// 加密快照替换成明文快照，绕过平台钥匙串保护并植入自己的签名身份。
    /// 需要旧格式迁移时，必须显式重建身份后重新落盘。
    pub fn from_snapshot_with(
        snapshot: &[u8],
        cipher: Option<&dyn kaede_mls::SnapshotCipher>,
    ) -> Result<Self, ClientError> {
        if snapshot.len() < 6 || snapshot[0] != SNAPSHOT_CIPHER_HEADER {
            return Err(ClientError::Mls(
                "legacy plaintext snapshot rejected; re-login to rebuild identity".into(),
            ));
        }
        let alg = snapshot[1];
        if alg == kaede_mls::ALG_NOOP {
            return Err(ClientError::Mls(
                "plaintext (NOOP) snapshot rejected; snapshot must be encrypted".into(),
            ));
        }
        let ct_len =
            u32::from_be_bytes([snapshot[2], snapshot[3], snapshot[4], snapshot[5]]) as usize;
        let ct = &snapshot[6..];
        // 数据长度不符 = 数据损坏，不是平台不兼容
        if ct.len() != ct_len {
            return Err(ClientError::Mls(format!(
                "ciphertext length mismatch: header={} actual={}",
                ct_len,
                ct.len()
            )));
        }
        // cipher 缺失 = 平台不兼容（典型：Windows 上创建的 DPAPI snapshot 在 Linux/Web 打开）
        let c = cipher.ok_or(ClientError::SnapshotIncompatible { alg })?;
        // cipher 与 snapshot 声明的 alg 不符 = 调用方传错 cipher（防御性）
        if c.alg_id() != alg {
            return Err(ClientError::SnapshotIncompatible { alg });
        }
        // 解密失败 = 数据损坏或密钥错误
        let inner_bytes = c
            .decrypt(ct)
            .map_err(|e| ClientError::Mls(format!("snapshot decrypt: {}", e)))?;

        Self::from_snapshot_inner(&inner_bytes)
    }

    fn from_snapshot_inner(snapshot: &[u8]) -> Result<Self, ClientError> {
        let mut c = Cursor::new(snapshot);

        let version = read_u8(&mut c)?;
        if version != SNAPSHOT_VERSION {
            return Err(ClientError::Mls(format!(
                "unsupported snapshot version: {}",
                version
            )));
        }

        // uid
        let uid_len = read_u16(&mut c)? as usize;
        let uid_bytes = read_bytes_bounded(&mut c, uid_len, MAX_UID_LEN, "uid")?;
        let uid = String::from_utf8(uid_bytes)
            .map_err(|e| ClientError::Mls(format!("bad uid: {}", e)))?;

        // sig scheme
        let scheme_u16 = read_u16(&mut c)?;
        let scheme = SignatureScheme::try_from(scheme_u16)
            .map_err(|e| ClientError::Mls(format!("bad scheme {}: {}", scheme_u16, e)))?;

        // sig priv
        let priv_len = read_u32(&mut c)? as usize;
        let priv_bytes = read_bytes_bounded(&mut c, priv_len, MAX_SIG_PRIV_LEN, "sig_priv")?;

        // sig pub
        let pub_len = read_u32(&mut c)? as usize;
        let pub_bytes = read_bytes_bounded(&mut c, pub_len, MAX_SIG_PUB_LEN, "sig_pub")?;

        let identity = Arc::new(MlsIdentity::from_raw(
            uid.as_str(),
            scheme,
            priv_bytes,
            pub_bytes,
        )?);

        // group_id
        let gid_len = read_u16(&mut c)? as usize;
        let group_id = if gid_len == 0 {
            None
        } else {
            Some(read_bytes_bounded(
                &mut c,
                gid_len,
                MAX_GROUP_ID_LEN,
                "group_id",
            )?)
        };

        // storage
        let storage_len = read_u32(&mut c)? as usize;
        let storage_bytes = read_bytes_bounded(&mut c, storage_len, MAX_STORAGE_LEN, "storage")?;

        let provider = KaedeMlsProvider::from_snapshot(&storage_bytes)
            .map_err(|e| ClientError::Mls(format!("provider restore: {}", e)))?;

        // 尝试从 storage 加载 group
        // C4: 若 snapshot 声明有 group 但 load 失败 → 直接 Err。
        // 静默降级会诱导用户重建 group，导致群分裂（其他成员收不到 commit）。
        let group = match &group_id {
            Some(gid) => Some(
                MlsGroupHandle::load_from_storage(identity.clone(), &provider, gid).map_err(
                    |e| {
                        ClientError::Mls(format!(
                            "snapshot declares group {:?} but load_from_storage failed: {}",
                            gid, e
                        ))
                    },
                )?,
            ),
            None => None,
        };

        Ok(Self {
            identity,
            provider: std::sync::Arc::new(provider),
            group,
            group_id,
        })
    }
}

impl std::fmt::Debug for MlsSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MlsSession")
            .field("uid", &self.identity.uid())
            .field("has_group", &self.group.is_some())
            .field("epoch", &self.epoch())
            .finish()
    }
}

// ---- 读取 helper ----

use std::io::{Cursor, Read};

// =========================================================================
// P0-G：快照读取硬上限（防本地文件篡改触发 OOM）
// =========================================================================
const MAX_UID_LEN: usize = 256;
const MAX_SIG_PRIV_LEN: usize = 4 * 1024;
const MAX_SIG_PUB_LEN: usize = 4 * 1024;
const MAX_GROUP_ID_LEN: usize = 256;
const MAX_STORAGE_LEN: usize = 16 * 1024 * 1024;

fn read_bytes_bounded(
    c: &mut Cursor<&[u8]>,
    len: usize,
    max: usize,
    what: &str,
) -> Result<Vec<u8>, ClientError> {
    if len > max {
        return Err(ClientError::Mls(format!(
            "{} length {} exceeds max {} (corrupt or hostile snapshot)",
            what, len, max
        )));
    }
    read_bytes(c, len)
}

fn read_u8(c: &mut Cursor<&[u8]>) -> Result<u8, ClientError> {
    let mut b = [0u8; 1];
    c.read_exact(&mut b)
        .map_err(|e| ClientError::Mls(format!("read u8: {}", e)))?;
    Ok(b[0])
}

fn read_u16(c: &mut Cursor<&[u8]>) -> Result<u16, ClientError> {
    let mut b = [0u8; 2];
    c.read_exact(&mut b)
        .map_err(|e| ClientError::Mls(format!("read u16: {}", e)))?;
    Ok(u16::from_be_bytes(b))
}

fn read_u32(c: &mut Cursor<&[u8]>) -> Result<u32, ClientError> {
    let mut b = [0u8; 4];
    c.read_exact(&mut b)
        .map_err(|e| ClientError::Mls(format!("read u32: {}", e)))?;
    Ok(u32::from_be_bytes(b))
}

fn read_bytes(c: &mut Cursor<&[u8]>, len: usize) -> Result<Vec<u8>, ClientError> {
    let mut buf = vec![0u8; len];
    c.read_exact(&mut buf)
        .map_err(|e| ClientError::Mls(format!("read bytes: {}", e)))?;
    Ok(buf)
}

// ============================================================================
// 测试
// ============================================================================

#[allow(deprecated)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn livekit_e2ee_key_differs_from_sframe_default() {
        let mut s = MlsSession::new("alice").unwrap();
        s.create_group().unwrap();
        let sframe = s.export_sframe_key("ch-a").unwrap();
        let livekit = s.export_livekit_e2ee_key("ch-a").unwrap();
        assert_ne!(
            sframe, livekit,
            "livekit key must differ from SFrame default"
        );

        let livekit2 = s.export_livekit_e2ee_key("ch-a").unwrap();
        assert_eq!(livekit, livekit2, "deterministic");

        let livekit_other = s.export_livekit_e2ee_key("ch-b").unwrap();
        assert_ne!(livekit, livekit_other, "different channel => different key");
    }

    #[test]
    fn alice_bob_encrypted_chat() {
        let mut bob = MlsSession::new("bob").unwrap();
        let bob_kp = bob.generate_key_package().unwrap();

        let mut alice = MlsSession::new("alice").unwrap();
        alice.create_group().unwrap();
        let (_commit, welcome) = alice.add_member(&bob_kp).unwrap();
        bob.join_with_welcome(&welcome).unwrap();

        assert_eq!(alice.member_count(), 2);
        assert_eq!(bob.member_count(), 2);
        assert_eq!(alice.epoch(), bob.epoch());

        let ct = alice.encrypt(b"hello bob").unwrap();
        let pt = bob.decrypt(&ct).unwrap();
        assert_eq!(pt, b"hello bob");

        let ct = bob.encrypt(b"hi alice").unwrap();
        let pt = alice.decrypt(&ct).unwrap();
        assert_eq!(pt, b"hi alice");
    }

    #[test]
    fn remove_member_by_uid_drops_member() {
        let mut bob = MlsSession::new("bob").unwrap();
        let bob_kp = bob.generate_key_package().unwrap();

        let mut alice = MlsSession::new("alice").unwrap();
        alice.create_group().unwrap();
        let (_c, welcome) = alice.add_member(&bob_kp).unwrap();
        bob.join_with_welcome(&welcome).unwrap();

        assert_eq!(alice.member_count(), 2);

        let commit = alice.remove_member_by_uid("bob").unwrap();
        assert!(!commit.is_empty(), "commit must not be empty");
        assert_eq!(alice.member_count(), 1);

        // 再调一次应失败
        assert!(alice.remove_member_by_uid("bob").is_err());
    }
    #[test]
    fn snapshot_restore_preserves_identity_and_group() {
        // 建组
        let mut bob = MlsSession::new("bob").unwrap();
        let bob_kp = bob.generate_key_package().unwrap();

        let mut alice = MlsSession::new("alice").unwrap();
        alice.create_group().unwrap();
        let (_commit, welcome) = alice.add_member(&bob_kp).unwrap();
        bob.join_with_welcome(&welcome).unwrap();

        let alice_epoch = alice.epoch();
        let alice_members = alice.member_count();
        let alice_gid = alice.group_id().unwrap().to_vec();

        // snapshot + restore
        let snap = alice.snapshot().unwrap();
        let mut alice2 = MlsSession::from_snapshot_inner(&snap).unwrap();

        // 恢复后 uid / epoch / 成员数 / group_id 一致
        assert_eq!(alice2.uid(), "alice");
        assert_eq!(alice2.epoch(), alice_epoch);
        assert_eq!(alice2.member_count(), alice_members);
        assert_eq!(alice2.group_id().unwrap(), &alice_gid[..]);

        // bob 也 snapshot
        let bob_snap = bob.snapshot().unwrap();
        let mut bob2 = MlsSession::from_snapshot_inner(&bob_snap).unwrap();
        assert_eq!(bob2.uid(), "bob");
        assert_eq!(bob2.member_count(), 2);

        // alice2 → bob2
        let ct = alice2.encrypt(b"after restore").unwrap();
        let pt = bob2.decrypt(&ct).unwrap();
        assert_eq!(pt, b"after restore");

        // bob2 → alice2
        let ct = bob2.encrypt(b"me too").unwrap();
        let pt = alice2.decrypt(&ct).unwrap();
        assert_eq!(pt, b"me too");
    }

    #[test]
    fn snapshot_restore_without_group() {
        let s = MlsSession::new("alice").unwrap();
        let snap = s.snapshot().unwrap();
        let s2 = MlsSession::from_snapshot_inner(&snap).unwrap();
        assert_eq!(s2.uid(), "alice");
        assert!(!s2.is_ready());
        assert!(s2.group_id().is_none());
    }

    #[test]
    fn export_sframe_key_agrees_after_restore() {
        let mut bob = MlsSession::new("bob").unwrap();
        let bob_kp = bob.generate_key_package().unwrap();

        let mut alice = MlsSession::new("alice").unwrap();
        alice.create_group().unwrap();
        let (_c, welcome) = alice.add_member(&bob_kp).unwrap();
        bob.join_with_welcome(&welcome).unwrap();

        let a_key_orig = alice.export_sframe_key("ch-general").unwrap();

        let snap = alice.snapshot().unwrap();
        let alice2 = MlsSession::from_snapshot_inner(&snap).unwrap();

        let a_key_restored = alice2.export_sframe_key("ch-general").unwrap();
        assert_eq!(
            a_key_orig, a_key_restored,
            "SFrame key must survive restore"
        );
    }

    #[test]
    fn snapshot_with_aes_gcm_roundtrip() {
        let mut bob = MlsSession::new("bob").unwrap();
        let bob_kp = bob.generate_key_package().unwrap();

        let mut alice = MlsSession::new("alice").unwrap();
        alice.create_group().unwrap();
        let (_c, welcome) = alice.add_member(&bob_kp).unwrap();
        bob.join_with_welcome(&welcome).unwrap();

        let cipher = kaede_mls::AesGcmCipher::from_bytes([0x42; 32]);
        let snap = alice.snapshot_with(&cipher).unwrap();

        // 外层头是 0x03
        assert_eq!(snap[0], 0x03);
        assert_eq!(snap[1], kaede_mls::ALG_AES_GCM);

        // 用同 cipher 恢复
        let same = kaede_mls::AesGcmCipher::from_bytes([0x42; 32]);
        let mut alice2 = MlsSession::from_snapshot_with(&snap, Some(&same)).unwrap();
        assert_eq!(alice2.uid(), "alice");
        assert_eq!(alice2.member_count(), 2);

        // 能从恢复后的会话发消息给 bob
        let ct = alice2.encrypt(b"post-restore").unwrap();
        let pt = bob.decrypt(&ct).unwrap();
        assert_eq!(pt, b"post-restore");
    }

    #[test]
    fn snapshot_with_wrong_key_rejected() {
        let mut s = MlsSession::new("alice").unwrap();
        s.create_group().unwrap();

        let cipher = kaede_mls::AesGcmCipher::from_bytes([0x11; 32]);
        let snap = s.snapshot_with(&cipher).unwrap();

        let wrong = kaede_mls::AesGcmCipher::from_bytes([0x99; 32]);
        assert!(MlsSession::from_snapshot_with(&snap, Some(&wrong)).is_err());

        // 不给 cipher 也报错
        assert!(MlsSession::from_snapshot_with(&snap, None).is_err());
    }

    #[test]
    fn plaintext_snapshot_rejected() {
        // fail-closed：明文老格式不再被接受，防止本地存储被替换后绕过钥匙串保护
        let mut s = MlsSession::new("alice").unwrap();
        s.create_group().unwrap();
        let snap = s.snapshot().unwrap(); // 明文
        assert!(MlsSession::from_snapshot_with(&snap, None).is_err());
        assert!(MlsSession::from_snapshot(&snap).is_err());
    }

    #[test]
    fn noop_snapshot_rejected() {
        let mut s = MlsSession::new("alice").unwrap();
        s.create_group().unwrap();
        let snap = s.snapshot_with(&kaede_mls::NoOpCipher).unwrap();
        assert!(MlsSession::from_snapshot_with(&snap, None).is_err());
    }

    #[test]
    fn corrupt_snapshot_rejected() {
        assert!(MlsSession::from_snapshot_inner(&[]).is_err());
        assert!(MlsSession::from_snapshot_inner(&[0xff]).is_err()); // 坏版本号
        assert!(MlsSession::from_snapshot_inner(&[1, 0]).is_err()); // 截断
    }

    #[test]
    fn no_group_returns_error() {
        let mut s = MlsSession::new("alice").unwrap();
        assert!(matches!(s.encrypt(b"x"), Err(ClientError::Mls(_))));
        assert!(matches!(s.export_sframe_key("x"), Err(ClientError::Mls(_))));
    }

    #[test]
    fn double_create_rejected() {
        let mut s = MlsSession::new("alice").unwrap();
        s.create_group().unwrap();
        assert!(matches!(s.create_group(), Err(ClientError::Mls(_))));
    }

    #[test]
    fn export_sframe_key_for_purpose_differs() {
        let mut alice = MlsSession::new("alice").unwrap();
        alice.create_group().unwrap();

        let k_default = alice.export_sframe_key("general").unwrap();
        let k_va = alice
            .export_sframe_key_for("general", kaede_mls::SframePurpose::VoiceAudio)
            .unwrap();
        let k_vv = alice
            .export_sframe_key_for("general", kaede_mls::SframePurpose::VoiceVideo)
            .unwrap();
        let k_sv = alice
            .export_sframe_key_for("general", kaede_mls::SframePurpose::ScreenVideo)
            .unwrap();
        let k_sa = alice
            .export_sframe_key_for("general", kaede_mls::SframePurpose::ScreenAudio)
            .unwrap();

        assert_ne!(k_va, k_vv, "audio and video must not share key");
        assert_ne!(k_vv, k_sv);
        assert_ne!(k_sv, k_sa);
        assert_ne!(k_default, k_va);

        // 旧 API = 显式 Default
        let k_explicit = alice
            .export_sframe_key_for("general", kaede_mls::SframePurpose::Default)
            .unwrap();
        assert_eq!(k_default, k_explicit);
    }

    #[test]
    fn export_sframe_key_for_agrees_across_members() {
        let mut bob = MlsSession::new("bob").unwrap();
        let bob_kp = bob.generate_key_package().unwrap();

        let mut alice = MlsSession::new("alice").unwrap();
        alice.create_group().unwrap();
        let (_c, welcome) = alice.add_member(&bob_kp).unwrap();
        bob.join_with_welcome(&welcome).unwrap();

        for purpose in [
            kaede_mls::SframePurpose::Default,
            kaede_mls::SframePurpose::Text,
            kaede_mls::SframePurpose::VoiceAudio,
            kaede_mls::SframePurpose::VoiceVideo,
            kaede_mls::SframePurpose::ScreenVideo,
            kaede_mls::SframePurpose::ScreenAudio,
            kaede_mls::SframePurpose::File,
        ] {
            let a = alice.export_sframe_key_for("general", purpose).unwrap();
            let b = bob.export_sframe_key_for("general", purpose).unwrap();
            assert_eq!(a, b, "purpose {:?} must agree", purpose);
        }
    }
    #[test]
    fn uid_and_debug() {
        let s = MlsSession::new("alice").unwrap();
        assert_eq!(s.uid(), "alice");
        let dbg = format!("{:?}", s);
        assert!(dbg.contains("alice"));
    }
}

// ============================================================================
// P1-γ 测试：snapshot 跨平台降级行为
// ============================================================================
#[cfg(test)]
mod r2a_tests {
    use super::*;
    use crate::error::ClientError;

    #[test]
    fn snapshot_without_cipher_returns_typed_error() {
        let mut s = MlsSession::new("alice").unwrap();
        s.create_group().unwrap();
        let cipher = kaede_mls::AesGcmCipher::from_bytes([0x42; 32]);
        let snap = s.snapshot_with(&cipher).unwrap();

        // 不提供 cipher → 必须返回 SnapshotIncompatible，而不是 Mls
        match MlsSession::from_snapshot_with(&snap, None) {
            Err(ClientError::SnapshotIncompatible { alg }) => {
                assert_eq!(alg, kaede_mls::ALG_AES_GCM);
            }
            other => panic!("expected SnapshotIncompatible, got {:?}", other),
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn dpapi_snapshot_on_non_windows_returns_typed_error() {
        // 手工构造一个"假"DPAPI snapshot（不需要真 DPAPI）
        let mut snap = vec![0x03u8, kaede_mls::ALG_DPAPI];
        snap.extend_from_slice(&4u32.to_be_bytes());
        snap.extend_from_slice(b"fake"); // 4 字节伪 ciphertext

        // 非 Windows 上，device_cipher_for_snapshot 返回 NoOpCipher（alg 不匹配）
        let cipher = crate::keystore::device_cipher_for_snapshot("alice", "http://x", &snap);
        match MlsSession::from_snapshot_with(&snap, Some(cipher.as_ref())) {
            Err(ClientError::SnapshotIncompatible { alg }) => {
                assert_eq!(alg, kaede_mls::ALG_DPAPI);
            }
            other => panic!("expected SnapshotIncompatible, got {:?}", other),
        }
    }

    #[test]
    fn corrupted_ciphertext_still_returns_mls_error() {
        // 真损坏 ≠ 平台不兼容。仍然走 Mls 分支
        let mut s = MlsSession::new("alice").unwrap();
        s.create_group().unwrap();
        let cipher = kaede_mls::AesGcmCipher::from_bytes([0x42; 32]);
        let mut snap = s.snapshot_with(&cipher).unwrap();
        // 篡改 ciphertext 中间一个字节
        let mid = snap.len() / 2;
        snap[mid] ^= 0xFF;

        let same_cipher = kaede_mls::AesGcmCipher::from_bytes([0x42; 32]);
        match MlsSession::from_snapshot_with(&snap, Some(&same_cipher)) {
            Err(ClientError::Mls(msg)) => {
                assert!(
                    msg.contains("decrypt"),
                    "expected decrypt error, got: {}",
                    msg
                );
            }
            other => panic!("expected Mls (corrupt) error, got {:?}", other),
        }
    }
}

// ============================================================================

// ============================================================================
// P0-G / P0-H 回归测试（R6 阶段 2）
// ============================================================================

#[cfg(test)]
#[allow(deprecated)]
mod p0gh_regression {
    use super::*;

    /// 生成一个真实合法的明文 snapshot（含 group）。
    ///
    /// 相比手工拼字节，这样能保证 scheme / priv / pub / storage 字段全部有效，
    /// 篡改目标字段后能真正走到 bounds 检查，而不是在 scheme 解析时提前失败。
    fn make_plain_snapshot() -> Vec<u8> {
        let mut s = MlsSession::new("alice").unwrap();
        s.create_group().unwrap();
        s.snapshot().unwrap()
    }

    /// 明文 snapshot 布局：
    ///   version(1) | uid_len(2) | uid | scheme(2) | priv_len(4) | priv |
    ///   pub_len(4) | pub | gid_len(2) | gid | storage_len(4) | storage
    fn locate_priv_len_offset(snap: &[u8]) -> usize {
        assert!(snap.len() > 3, "snapshot too short");
        assert_eq!(snap[0], 1, "expected plaintext snapshot (version=1)");
        let uid_len = u16::from_be_bytes([snap[1], snap[2]]) as usize;
        1 + 2 + uid_len + 2 // version + uid_len + uid + scheme
    }

    fn locate_storage_len_offset(snap: &[u8]) -> usize {
        let mut off = locate_priv_len_offset(snap);
        let priv_len =
            u32::from_be_bytes([snap[off], snap[off + 1], snap[off + 2], snap[off + 3]]) as usize;
        off += 4 + priv_len;
        let pub_len =
            u32::from_be_bytes([snap[off], snap[off + 1], snap[off + 2], snap[off + 3]]) as usize;
        off += 4 + pub_len;
        let gid_len = u16::from_be_bytes([snap[off], snap[off + 1]]) as usize;
        off += 2 + gid_len;
        off
    }

    /// P0-G：priv_len = 0xFFFFFFFF 必须被拒绝（不尝试分配 4GB）。
    #[test]
    fn p0g_huge_priv_len_rejected() {
        let mut snap = make_plain_snapshot();
        let off = locate_priv_len_offset(&snap);
        assert!(snap.len() > off + 4, "snapshot too short to patch");
        snap[off..off + 4].copy_from_slice(&0xFFFF_FFFFu32.to_be_bytes());

        match MlsSession::from_snapshot_inner(&snap) {
            Err(ClientError::Mls(msg)) => {
                assert!(
                    msg.contains("sig_priv") && msg.contains("exceeds max"),
                    "expected sig_priv length error, got: {}",
                    msg
                );
            }
            other => panic!("expected Mls error, got {:?}", other),
        }
    }

    /// P0-G：uid_len 超限必须被拒绝（手工构造也 OK——uid 读取早于 scheme）。
    #[test]
    fn p0g_huge_uid_len_rejected() {
        let mut snap = Vec::new();
        snap.push(1u8); // version
        snap.extend_from_slice(&0xFFFFu16.to_be_bytes()); // uid_len = 65535 > MAX_UID_LEN(256)

        match MlsSession::from_snapshot_inner(&snap) {
            Err(ClientError::Mls(msg)) => {
                assert!(
                    msg.contains("uid") && msg.contains("exceeds max"),
                    "expected uid length error, got: {}",
                    msg
                );
            }
            other => panic!("expected Mls error, got {:?}", other),
        }
    }

    /// P0-G：storage_len = 0xFFFFFFFF 必须被拒绝（最危险字段）。
    #[test]
    fn p0g_huge_storage_len_rejected() {
        let mut snap = make_plain_snapshot();
        let off = locate_storage_len_offset(&snap);
        assert!(snap.len() > off + 4, "snapshot too short to patch");
        snap[off..off + 4].copy_from_slice(&0xFFFF_FFFFu32.to_be_bytes());

        match MlsSession::from_snapshot_inner(&snap) {
            Err(ClientError::Mls(msg)) => {
                assert!(
                    msg.contains("storage") && msg.contains("exceeds max"),
                    "expected storage length error, got: {}",
                    msg
                );
            }
            other => panic!("expected Mls error, got {:?}", other),
        }
    }

    /// P0-H：正常尺寸 snapshot 不触发 overflow 检查。
    #[test]
    fn p0h_normal_size_does_not_trigger_overflow_check() {
        let mut s = MlsSession::new("alice").unwrap();
        s.create_group().unwrap();
        let cipher = kaede_mls::AesGcmCipher::from_bytes([0x42; 32]);
        let snap = s.snapshot_with(&cipher).unwrap();
        assert!(snap.len() < u32::MAX as usize);
        assert_eq!(snap[0], 0x03); // 外层 header
    }

    /// 正向：未篡改的 snapshot 仍能正常恢复（确保 bounds 检查不过度严格）。
    #[test]
    fn p0g_normal_snapshot_still_loads() {
        let snap = make_plain_snapshot();
        let _s = MlsSession::from_snapshot_inner(&snap).unwrap();
    }

    /// S1：欢迎校验失败时不得改变本地状态。
    #[test]
    fn checked_welcome_rejects_missing_required_member() {
        // 失败路径：Welcome 本身有效，但缺少业务要求的成员。
        let mut alice = MlsSession::new("alice").unwrap();
        alice.create_group().unwrap();
        let mut bob = MlsSession::new("bob").unwrap();
        let bob_kp = bob.generate_key_package().unwrap();
        let (_commit, welcome) = alice.add_member(&bob_kp).unwrap();

        let err = bob
            .join_with_welcome_checked(&welcome, &["alice", "bob", "dave"])
            .expect_err("missing required member must be rejected");
        assert!(err.to_string().contains("dave"), "unexpected error: {err}");
        assert!(
            bob.group_id().is_none(),
            "failed join must not mutate local state"
        );

        // 正向路径使用独立身份 / KP，避免复用已被消费的 Welcome。
        let mut alice2 = MlsSession::new("alice").unwrap();
        alice2.create_group().unwrap();
        let mut bob2 = MlsSession::new("bob").unwrap();
        let bob_kp2 = bob2.generate_key_package().unwrap();
        let (_commit2, welcome2) = alice2.add_member(&bob_kp2).unwrap();
        bob2.join_with_welcome_checked(&welcome2, &["alice", "bob"])
            .unwrap();
        assert!(bob2.group_id().is_some());
    }
}
