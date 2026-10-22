//! DM 消息的 per-conv AEAD 封装（B3 方案）。
//!
//! # 格式
//!
//! ```text
//! sealed = ver(1) || msg_id(16) || nonce(12) || ciphertext_with_tag(N + 16)
//! ```
//!
//! - `ver = 0x05`（当前格式）；`0x01` 是历史格式，仅用于读取升级前历史
//! - `nonce` 每条消息随机生成（12 字节），不依赖计数器
//! - `ciphertext_with_tag` 是 AES-256-GCM 输出
//!
//! # AAD
//!
//! 当前格式 AAD = `len(conv_id) || conv_id || len(from) || from || len(to) || to || msg_id`
//!
//! 理由：
//! 1. **绑定会话**：key 已由 exporter context 绑定 conv_id，AAD 是第二道防线
//! 2. **防篡改**：攻击者把密文从 conv A 挪到 conv B 会 AAD 校验失败
//! 3. **绑定发送者/接收者**：服务器反射密文并篡改 `from`/`to` 会解密失败
//! 4. **绑定消息 id**：调用方按 `msg_id` 维护重放窗口
//!
//! 旧 v1 的 AAD 只有 `conv_id`，仅用于读取升级前历史（[`open_legacy`]）。
//!
//! # 密钥来源
//!
//! 由 `ServerConnection::dm_derive_key(conv_id)` 从 MLS exporter 派生：
//! `export_secret("kaede-sframe-v1", conv_id || 0x00 || "dm", 32)`
//!
//! # 已知局限
//!
//! B3 的密钥域隔离不等于密码学隔离。任何 MLS group 成员都能从 group state
//! export 出 DM key。若需密码学隔离，升级为每对好友独立 2 人 MLS group（B2）。

use rand_core::{OsRng, RngCore};

use crate::aead::{AeadError, aes256gcm_decrypt, aes256gcm_encrypt};

/// 当前密文格式版本（v2：绑定发送者/接收者/消息 id）。
///
/// 不能复用 0x02：DM Welcome 信封也用 `[0x02][generation][welcome]` 开头，
/// 复用会让客户端把普通密文误判成 Welcome。0x03/0x04 已分别被频道消息
/// 与附件占用。
pub const DM_CIPHER_VERSION: u8 = 0x05;

/// 旧格式（仅 conv_id 作为 AAD，无发送者/重放保护）。
pub const DM_CIPHER_VERSION_V1: u8 = 0x01;

/// nonce 长度。
pub const DM_NONCE_LEN: usize = 12;

/// v2 消息 id 长度。
pub const DM_MSG_ID_LEN: usize = 16;

/// DM 加解密错误。
#[derive(Debug, thiserror::Error)]
pub enum DmCipherError {
    /// 密文过短（< 1 + 12 + 16）。
    #[error("sealed too short: {0}")]
    TooShort(usize),
    /// 未知版本号。
    #[error("unknown version: {0:#x}")]
    UnknownVersion(u8),
    /// 底层 AEAD 失败。
    #[error("aead: {0}")]
    Aead(#[from] AeadError),
}

fn aad(conv_id: &str, from: &str, to: &str, msg_id: &[u8; DM_MSG_ID_LEN]) -> Vec<u8> {
    let mut out =
        Vec::with_capacity(4 + conv_id.len() + 4 + from.len() + 4 + to.len() + DM_MSG_ID_LEN);
    for part in [conv_id, from, to] {
        out.extend_from_slice(&(part.len() as u32).to_be_bytes());
        out.extend_from_slice(part.as_bytes());
    }
    out.extend_from_slice(msg_id);
    out
}

/// 加密一条消息（v2）。
///
/// AAD 绑定 `conv_id + from + to + msg_id`：
/// - 服务器把 A→B 的密文反射给 A 并标 `from=B` 时解密失败；
/// - 同一密文重复投递由调用方按 `msg_id` 拒重放。
pub fn seal(
    key: &[u8; 32],
    conv_id: &str,
    from: &str,
    to: &str,
    plaintext: &[u8],
) -> Result<Vec<u8>, DmCipherError> {
    let mut msg_id = [0u8; DM_MSG_ID_LEN];
    OsRng.fill_bytes(&mut msg_id);
    let mut nonce = [0u8; DM_NONCE_LEN];
    OsRng.fill_bytes(&mut nonce);

    let aad = aad(conv_id, from, to, &msg_id);
    let ct = aes256gcm_encrypt(key, &nonce, &aad, plaintext)?;

    let mut out = Vec::with_capacity(1 + DM_MSG_ID_LEN + DM_NONCE_LEN + ct.len());
    out.push(DM_CIPHER_VERSION);
    out.extend_from_slice(&msg_id);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// 读出 v2 密文里的消息 id（用于重放检测）。
pub fn message_id(sealed: &[u8]) -> Option<[u8; DM_MSG_ID_LEN]> {
    const HDR: usize = 1 + DM_MSG_ID_LEN + DM_NONCE_LEN;
    if sealed.len() < HDR + 16 || sealed[0] != DM_CIPHER_VERSION {
        return None;
    }
    sealed[1..1 + DM_MSG_ID_LEN].try_into().ok()
}

/// 解密一条 v2 消息。返回 `(plaintext, msg_id)`。
pub fn open(
    key: &[u8; 32],
    conv_id: &str,
    from: &str,
    to: &str,
    sealed: &[u8],
) -> Result<(Vec<u8>, [u8; DM_MSG_ID_LEN]), DmCipherError> {
    const HDR: usize = 1 + DM_MSG_ID_LEN + DM_NONCE_LEN;
    if sealed.len() < HDR + 16 {
        return Err(DmCipherError::TooShort(sealed.len()));
    }
    let ver = sealed[0];
    if ver != DM_CIPHER_VERSION {
        return Err(DmCipherError::UnknownVersion(ver));
    }
    let msg_id: [u8; DM_MSG_ID_LEN] = sealed[1..1 + DM_MSG_ID_LEN]
        .try_into()
        .map_err(|_| DmCipherError::TooShort(sealed.len()))?;
    let nonce = &sealed[1 + DM_MSG_ID_LEN..HDR];
    let ct = &sealed[HDR..];
    let aad = aad(conv_id, from, to, &msg_id);
    let pt = aes256gcm_decrypt(key, nonce, &aad, ct)?;
    Ok((pt, msg_id))
}

/// 解密旧版（v1）消息；仅用于读取升级前的历史。
pub fn open_legacy(key: &[u8; 32], conv_id: &str, sealed: &[u8]) -> Result<Vec<u8>, DmCipherError> {
    const HDR: usize = 1 + DM_NONCE_LEN;
    if sealed.len() < HDR + 16 {
        return Err(DmCipherError::TooShort(sealed.len()));
    }
    let ver = sealed[0];
    if ver != DM_CIPHER_VERSION_V1 {
        return Err(DmCipherError::UnknownVersion(ver));
    }
    let nonce = &sealed[1..HDR];
    let ct = &sealed[HDR..];
    Ok(aes256gcm_decrypt(key, nonce, conv_id.as_bytes(), ct)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let key = [0x42u8; 32];
        let conv = "dm:alice:bob";
        let pt = b"hello bob";

        let sealed = seal(&key, conv, "alice", "bob", pt).unwrap();
        assert_eq!(sealed.len(), 1 + DM_MSG_ID_LEN + 12 + pt.len() + 16);
        assert_eq!(sealed[0], DM_CIPHER_VERSION);

        let (back, id) = open(&key, conv, "alice", "bob", &sealed).unwrap();
        assert_eq!(back, pt);
        assert_eq!(Some(id), message_id(&sealed));
    }

    #[test]
    fn nonce_is_random() {
        let key = [0x42u8; 32];
        let conv = "dm:alice:bob";
        let a = seal(&key, conv, "alice", "bob", b"x").unwrap();
        let b = seal(&key, conv, "alice", "bob", b"x").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn wrong_conv_id_fails() {
        let key = [0x42u8; 32];
        let sealed = seal(&key, "dm:alice:bob", "alice", "bob", b"hi").unwrap();
        assert!(open(&key, "dm:alice:carol", "alice", "bob", &sealed).is_err());
    }

    #[test]
    fn wrong_sender_or_recipient_fails() {
        let key = [0x42u8; 32];
        let conv = "dm:alice:bob";
        let sealed = seal(&key, conv, "alice", "bob", b"hi").unwrap();
        // 服务器反射：同一密文标成 bob 发给 alice
        assert!(open(&key, conv, "bob", "alice", &sealed).is_err());
        // 方向反转
        assert!(open(&key, conv, "alice", "alice", &sealed).is_err());
    }

    #[test]
    fn wrong_key_fails() {
        let sealed = seal(&[1u8; 32], "dm:alice:bob", "alice", "bob", b"hi").unwrap();
        assert!(open(&[2u8; 32], "dm:alice:bob", "alice", "bob", &sealed).is_err());
    }

    #[test]
    fn tamper_fails() {
        let key = [0x42u8; 32];
        let conv = "dm:alice:bob";
        let mut sealed = seal(&key, conv, "alice", "bob", b"hi").unwrap();
        let last = sealed.len() - 1;
        sealed[last] ^= 0xFF;
        assert!(open(&key, conv, "alice", "bob", &sealed).is_err());
    }

    #[test]
    fn bad_version_fails() {
        let key = [0x42u8; 32];
        let conv = "dm:alice:bob";
        let mut sealed = seal(&key, conv, "alice", "bob", b"hi").unwrap();
        sealed[0] = 0xFF;
        match open(&key, conv, "alice", "bob", &sealed) {
            Err(DmCipherError::UnknownVersion(0xFF)) => {}
            other => panic!("expected UnknownVersion, got {:?}", other),
        }
    }

    #[test]
    fn too_short_fails() {
        let key = [0x42u8; 32];
        assert!(matches!(
            open(&key, "x", "a", "b", &[0u8; 10]),
            Err(DmCipherError::TooShort(10))
        ));
        assert!(matches!(
            open_legacy(&key, "x", &[0u8; 10]),
            Err(DmCipherError::TooShort(10))
        ));
    }

    #[test]
    fn legacy_v1_still_decrypts_for_history_only() {
        let key = [0x42u8; 32];
        let conv = "dm:alice:bob";
        let mut nonce = [0u8; DM_NONCE_LEN];
        OsRng.fill_bytes(&mut nonce);
        let ct = aes256gcm_encrypt(&key, &nonce, conv.as_bytes(), b"old").unwrap();
        let mut sealed = vec![DM_CIPHER_VERSION_V1];
        sealed.extend_from_slice(&nonce);
        sealed.extend_from_slice(&ct);
        assert_eq!(open_legacy(&key, conv, &sealed).unwrap(), b"old");
        // 实时 open 必须拒绝 v1（防止服务器重放旧密文并伪造发送者）。
        // v1 比 v2 头短，可能先触发 TooShort；两者都代表“实时路径拒绝”。
        assert!(
            open(&key, conv, "alice", "bob", &sealed).is_err(),
            "live open must reject v1 ciphertext"
        );
    }
}
