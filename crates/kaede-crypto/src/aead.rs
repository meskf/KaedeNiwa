//! 通用 AEAD 封装。
//!
//! 目前支持 AES-256-GCM（SFrame 和 identity 文件用）。
//! ChaCha20-Poly1305 保留为可选路径，暂未暴露公共 API。
//!
//! # 非目标
//!
//! 这不是通用 AEAD 框架。用途固定：给上层提供"key + nonce + aad +
//! plaintext -> ciphertext_with_tag"的傻瓜接口，避免每个调用点都
//! 自己拼 `Payload`。

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit, Payload},
};

/// AEAD 错误。
#[derive(Debug, thiserror::Error)]
pub enum AeadError {
    /// 密钥长度错误（应为 32 字节）。
    #[error("invalid key length: {0}")]
    InvalidKeyLength(usize),
    /// Nonce 长度错误（应为 12 字节）。
    #[error("invalid nonce length: {0}")]
    InvalidNonceLength(usize),
    /// 加解密失败（GCM tag 校验失败或内部错误）。
    #[error("aead failure: {0}")]
    Failure(String),
}

/// AES-256-GCM 加密。
///
/// 返回 `ciphertext || tag`（16 字节 tag 附加在末尾）。
pub fn aes256gcm_encrypt(
    key: &[u8],
    nonce: &[u8],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, AeadError> {
    if key.len() != 32 {
        return Err(AeadError::InvalidKeyLength(key.len()));
    }
    if nonce.len() != 12 {
        return Err(AeadError::InvalidNonceLength(nonce.len()));
    }

    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| AeadError::Failure(e.to_string()))?;
    let nonce = Nonce::from_slice(nonce);
    let payload = Payload {
        msg: plaintext,
        aad,
    };
    cipher
        .encrypt(nonce, payload)
        .map_err(|e| AeadError::Failure(e.to_string()))
}

/// AES-256-GCM 解密。
///
/// 输入应为 `ciphertext || tag`。
pub fn aes256gcm_decrypt(
    key: &[u8],
    nonce: &[u8],
    aad: &[u8],
    ciphertext_with_tag: &[u8],
) -> Result<Vec<u8>, AeadError> {
    if key.len() != 32 {
        return Err(AeadError::InvalidKeyLength(key.len()));
    }
    if nonce.len() != 12 {
        return Err(AeadError::InvalidNonceLength(nonce.len()));
    }

    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| AeadError::Failure(e.to_string()))?;
    let nonce = Nonce::from_slice(nonce);
    let payload = Payload {
        msg: ciphertext_with_tag,
        aad,
    };
    cipher
        .decrypt(nonce, payload)
        .map_err(|e| AeadError::Failure(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let key = [0x42u8; 32];
        let nonce = [0x11u8; 12];
        let aad = b"header";
        let pt = b"hello world";

        let ct = aes256gcm_encrypt(&key, &nonce, aad, pt).unwrap();
        // 密文长度 = 明文 + 16 tag
        assert_eq!(ct.len(), pt.len() + 16);

        let back = aes256gcm_decrypt(&key, &nonce, aad, &ct).unwrap();
        assert_eq!(back, pt);
    }

    #[test]
    fn aad_mismatch_fails() {
        let key = [0x42u8; 32];
        let nonce = [0x11u8; 12];
        let ct = aes256gcm_encrypt(&key, &nonce, b"aad1", b"msg").unwrap();
        assert!(aes256gcm_decrypt(&key, &nonce, b"aad2", &ct).is_err());
    }

    #[test]
    fn tamper_fails() {
        let key = [0x42u8; 32];
        let nonce = [0x11u8; 12];
        let mut ct = aes256gcm_encrypt(&key, &nonce, b"", b"msg").unwrap();
        ct[0] ^= 0xFF;
        assert!(aes256gcm_decrypt(&key, &nonce, b"", &ct).is_err());
    }

    #[test]
    fn wrong_key_fails() {
        let nonce = [0x11u8; 12];
        let ct = aes256gcm_encrypt(&[1u8; 32], &nonce, b"", b"msg").unwrap();
        assert!(aes256gcm_decrypt(&[2u8; 32], &nonce, b"", &ct).is_err());
    }

    #[test]
    fn bad_key_len() {
        let r = aes256gcm_encrypt(&[0u8; 31], &[0u8; 12], b"", b"");
        assert!(matches!(r, Err(AeadError::InvalidKeyLength(31))));
    }

    #[test]
    fn bad_nonce_len() {
        let r = aes256gcm_encrypt(&[0u8; 32], &[0u8; 13], b"", b"");
        assert!(matches!(r, Err(AeadError::InvalidNonceLength(13))));
    }
}
