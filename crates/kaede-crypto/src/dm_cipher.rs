//! DM 消息的 per-conv AEAD 封装（B3 方案）。
//!
//! # 格式
//!
//! ```text
//! sealed = ver(1) || nonce(12) || ciphertext_with_tag(N + 16)
//! ```
//!
//! - `ver = 0x01`，未来不兼容时升高
//! - `nonce` 每条消息随机生成（12 字节），不依赖计数器
//! - `ciphertext_with_tag` 是 AES-256-GCM 输出
//!
//! # AAD
//!
//! AAD = `conv_id.as_bytes()`
//!
//! 理由：
//! 1. **绑定会话**：key 已由 exporter context 绑定 conv_id，AAD 是第二道防线
//! 2. **防篡改**：攻击者把密文从 conv A 挪到 conv B 会 AAD 校验失败
//! 3. **与 server 落库的 channel_id 一致**：server 存 `messages.channel_id = conv_id`
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

/// 当前密文格式版本。
pub const DM_CIPHER_VERSION: u8 = 0x01;

/// nonce 长度。
pub const DM_NONCE_LEN: usize = 12;

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

/// 加密一条 DM 消息。
///
/// `key` 必须是 `dm_derive_key(conv_id)` 派生出的 32 字节对称密钥。
/// `conv_id` 用作 AAD，同时被序列化进 exporter context（见模块文档）。
pub fn seal(key: &[u8; 32], conv_id: &str, plaintext: &[u8]) -> Result<Vec<u8>, DmCipherError> {
    let mut nonce = [0u8; DM_NONCE_LEN];
    OsRng.fill_bytes(&mut nonce);

    let ct = aes256gcm_encrypt(key, &nonce, conv_id.as_bytes(), plaintext)?;

    let mut out = Vec::with_capacity(1 + DM_NONCE_LEN + ct.len());
    out.push(DM_CIPHER_VERSION);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// 解密一条 DM 消息。
pub fn open(key: &[u8; 32], conv_id: &str, sealed: &[u8]) -> Result<Vec<u8>, DmCipherError> {
    const HDR: usize = 1 + DM_NONCE_LEN;
    if sealed.len() < HDR + 16 {
        return Err(DmCipherError::TooShort(sealed.len()));
    }

    let ver = sealed[0];
    if ver != DM_CIPHER_VERSION {
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

        let sealed = seal(&key, conv, pt).unwrap();
        assert_eq!(sealed.len(), 1 + 12 + pt.len() + 16);
        assert_eq!(sealed[0], DM_CIPHER_VERSION);

        let back = open(&key, conv, &sealed).unwrap();
        assert_eq!(back, pt);
    }

    #[test]
    fn nonce_is_random() {
        let key = [0x42u8; 32];
        let conv = "dm:alice:bob";
        let a = seal(&key, conv, b"x").unwrap();
        let b = seal(&key, conv, b"x").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn wrong_conv_id_fails() {
        let key = [0x42u8; 32];
        let sealed = seal(&key, "dm:alice:bob", b"hi").unwrap();
        assert!(open(&key, "dm:alice:carol", &sealed).is_err());
    }

    #[test]
    fn wrong_key_fails() {
        let sealed = seal(&[1u8; 32], "dm:alice:bob", b"hi").unwrap();
        assert!(open(&[2u8; 32], "dm:alice:bob", &sealed).is_err());
    }

    #[test]
    fn tamper_fails() {
        let key = [0x42u8; 32];
        let conv = "dm:alice:bob";
        let mut sealed = seal(&key, conv, b"hi").unwrap();
        let last = sealed.len() - 1;
        sealed[last] ^= 0xFF;
        assert!(open(&key, conv, &sealed).is_err());
    }

    #[test]
    fn bad_version_fails() {
        let key = [0x42u8; 32];
        let conv = "dm:alice:bob";
        let mut sealed = seal(&key, conv, b"hi").unwrap();
        sealed[0] = 0xFF;
        match open(&key, conv, &sealed) {
            Err(DmCipherError::UnknownVersion(0xFF)) => {}
            other => panic!("expected UnknownVersion, got {:?}", other),
        }
    }

    #[test]
    fn too_short_fails() {
        let key = [0x42u8; 32];
        assert!(matches!(
            open(&key, "x", &[0u8; 10]),
            Err(DmCipherError::TooShort(10))
        ));
    }
}
