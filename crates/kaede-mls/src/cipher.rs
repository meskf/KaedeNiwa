//! Snapshot 加密抽象。
//!
//! # 为什么需要
//!
//! `MlsSession::snapshot()` 里包含**签名私钥**。默认存明文的话，
//! 任何能读 SQLite 文件 / IndexedDB 的攻击者都能拿到身份。
//!
//! # 设计
//!
//! cipher 的职责只有加密 / 解密字节流。**key 从哪来不归它管**：
//! - Native：由 OS keystore（DPAPI）取
//! - Web：由 WebCrypto non-extractable key 取
//! - 测试：固定字节
//!
//! # 向后兼容
//!
//! 老 snapshot 第一字节是 `0x01`（明文格式）。新格式第一字节 `0x02`，
//! 第二字节是算法 id。这样读时自动识别，不需要额外版本切换。

use crate::MlsError;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// 算法 id。
pub const ALG_NOOP: u8 = 0x00;
pub const ALG_AES_GCM: u8 = 0x01;

/// Windows DPAPI（CryptProtectData）。
///
/// 用当前 Windows 用户的 master key 加密，与用户账户绑定。
/// 实现放在 `kaede-client::keystore::dpapi`。
pub const ALG_DPAPI: u8 = 0x10;

/// snapshot 加密器。
pub trait SnapshotCipher: Send + Sync {
    /// 算法 id（写进 snapshot 头）。
    fn alg_id(&self) -> u8;

    /// 加密。输入明文，输出密文。
    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, MlsError>;

    /// 解密。
    fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, MlsError>;
}

/// 空加密：直接透传。老 snapshot 和未加密场景用。
#[derive(Debug, Clone, Copy, Default)]
pub struct NoOpCipher;

impl SnapshotCipher for NoOpCipher {
    fn alg_id(&self) -> u8 {
        ALG_NOOP
    }
    fn encrypt(&self, pt: &[u8]) -> Result<Vec<u8>, MlsError> {
        Ok(pt.to_vec())
    }
    fn decrypt(&self, ct: &[u8]) -> Result<Vec<u8>, MlsError> {
        Ok(ct.to_vec())
    }
}

/// 从字节流构造 cipher。用于恢复时按 alg_id 选对解密器。
///
/// `key_loader`：根据 alg_id 返回对应 cipher。
/// 如果不知道 alg_id 对应的算法，返回 `Err`。
pub fn cipher_for_alg(
    alg_id: u8,
    key_loader: &dyn Fn(u8) -> Option<Box<dyn SnapshotCipher>>,
) -> Result<Box<dyn SnapshotCipher>, MlsError> {
    match alg_id {
        ALG_NOOP => Ok(Box::new(NoOpCipher)),
        other => key_loader(other)
            .ok_or_else(|| MlsError::OpenMls(format!("unsupported cipher alg: {}", other))),
    }
}

// ============================================================================
// AES-256-GCM 实现
// ============================================================================

/// AES-256-GCM 加密器。
///
/// 每次 `encrypt` 生成新的 12 字节随机 nonce，输出格式：
///
/// ```text
/// [nonce: 12][ciphertext || tag: 16]
/// ```
///
/// `decrypt` 从头部取 nonce，其余作为 `ct||tag` 交给 AEAD。
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct AesGcmCipher {
    key: [u8; 32],
}

impl AesGcmCipher {
    pub fn from_bytes(key: [u8; 32]) -> Self {
        // Z8: 入参 [u8; 32] 在栈上，构造后原参数不清零会残留。
        // 内部拷贝一份到 self.key（Self 已 derive ZeroizeOnDrop），原 key 清零。
        let mut k = key;
        let result = Self { key: k };
        zeroize::Zeroize::zeroize(&mut k);
        result
    }
}

impl SnapshotCipher for AesGcmCipher {
    fn alg_id(&self) -> u8 {
        ALG_AES_GCM
    }

    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, MlsError> {
        let nonce: [u8; 12] = kaede_crypto::random_bytes();
        let ct =
            kaede_crypto::aes256gcm_encrypt(&self.key, &nonce, b"kaede-mls-snapshot", plaintext)
                .map_err(|e| MlsError::OpenMls(format!("snapshot encrypt: {}", e)))?;
        let mut out = Vec::with_capacity(12 + ct.len());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        Ok(out)
    }

    fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, MlsError> {
        if ciphertext.len() < 12 + 16 {
            return Err(MlsError::OpenMls(format!(
                "snapshot too short: {} bytes",
                ciphertext.len()
            )));
        }
        let (nonce, body) = ciphertext.split_at(12);
        kaede_crypto::aes256gcm_decrypt(&self.key, nonce, b"kaede-mls-snapshot", body)
            .map_err(|e| MlsError::OpenMls(format!("snapshot decrypt: {}", e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_roundtrip() {
        let c = NoOpCipher;
        let pt = b"hello snapshot";
        let ct = c.encrypt(pt).unwrap();
        assert_eq!(ct, pt);
        assert_eq!(c.decrypt(&ct).unwrap(), pt);
        assert_eq!(c.alg_id(), ALG_NOOP);
    }

    #[test]
    fn aes_gcm_roundtrip() {
        let c = AesGcmCipher::from_bytes([0x42; 32]);
        let pt = b"the quick brown fox jumps over the lazy dog";
        let ct = c.encrypt(pt).unwrap();
        // 12 nonce + pt + 16 tag
        assert_eq!(ct.len(), 12 + pt.len() + 16);
        assert_ne!(&ct[12..12 + pt.len()], pt); // 确实加密了
        let back = c.decrypt(&ct).unwrap();
        assert_eq!(back, pt);
        assert_eq!(c.alg_id(), ALG_AES_GCM);
    }

    #[test]
    fn aes_gcm_wrong_key_fails() {
        let c1 = AesGcmCipher::from_bytes([0x11; 32]);
        let c2 = AesGcmCipher::from_bytes([0x22; 32]);
        let ct = c1.encrypt(b"secret").unwrap();
        assert!(c2.decrypt(&ct).is_err());
    }

    #[test]
    fn aes_gcm_tamper_fails() {
        let c = AesGcmCipher::from_bytes([0x33; 32]);
        let mut ct = c.encrypt(b"secret").unwrap();
        ct[15] ^= 0x01;
        assert!(c.decrypt(&ct).is_err());
    }

    #[test]
    fn aes_gcm_short_input_fails() {
        let c = AesGcmCipher::from_bytes([0x44; 32]);
        assert!(c.decrypt(&[0u8; 10]).is_err());
    }

    #[test]
    fn aes_gcm_nonce_is_random() {
        let c = AesGcmCipher::from_bytes([0x55; 32]);
        let a = c.encrypt(b"x").unwrap();
        let b = c.encrypt(b"x").unwrap();
        // 相同明文两次加密结果应不同（nonce 随机）
        assert_ne!(a, b);
        assert_eq!(a.len(), b.len());
    }
}
