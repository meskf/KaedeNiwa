//! HKDF-SHA256 密钥派生。
//!
//! 用途：从已协商的高熵密钥（如 MLS exporter 输出）派生子密钥。
//! **不用于**口令派生（那是 Argon2id 的领域）。

use hkdf::Hkdf;
use sha2::Sha256;

/// HKDF 错误。
#[derive(Debug, thiserror::Error)]
pub enum HkdfError {
    /// 输出长度超出 HKDF 上限（255 * hash_len = 8160 字节）。
    #[error("invalid output length: {0}")]
    InvalidLength(usize),
}

/// HKDF-SHA256 扩展。
///
/// `salt` 为 `None` 时用全零填充（RFC 5869 默认）。
pub fn hkdf_sha256(
    ikm: &[u8],
    salt: Option<&[u8]>,
    info: &[u8],
    out: &mut [u8],
) -> Result<(), HkdfError> {
    let hk = Hkdf::<Sha256>::new(salt, ikm);
    hk.expand(info, out)
        .map_err(|_| HkdfError::InvalidLength(out.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic() {
        let mut a = [0u8; 32];
        let mut b = [0u8; 32];
        hkdf_sha256(b"ikm", Some(b"salt"), b"info", &mut a).unwrap();
        hkdf_sha256(b"ikm", Some(b"salt"), b"info", &mut b).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn different_info_differ() {
        let mut a = [0u8; 32];
        let mut b = [0u8; 32];
        hkdf_sha256(b"ikm", Some(b"salt"), b"info1", &mut a).unwrap();
        hkdf_sha256(b"ikm", Some(b"salt"), b"info2", &mut b).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn different_salt_differ() {
        let mut a = [0u8; 32];
        let mut b = [0u8; 32];
        hkdf_sha256(b"ikm", Some(b"salt1"), b"info", &mut a).unwrap();
        hkdf_sha256(b"ikm", Some(b"salt2"), b"info", &mut b).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn none_salt_ok() {
        let mut out = [0u8; 32];
        hkdf_sha256(b"ikm", None, b"info", &mut out).unwrap();
    }

    #[test]
    fn oversize_rejected() {
        // HKDF 上限 255 * 32 = 8160
        let mut out = [0u8; 8161];
        assert!(matches!(
            hkdf_sha256(b"ikm", None, b"info", &mut out),
            Err(HkdfError::InvalidLength(_))
        ));
    }

    #[test]
    fn rfc5869_test_case_1() {
        // RFC 5869 §A.1：SHA-256 测试向量
        let ikm = [0x0bu8; 22];
        let salt: [u8; 13] = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c,
        ];
        let info: [u8; 10] = [0xf0, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9];
        let mut okm = [0u8; 42];
        hkdf_sha256(&ikm, Some(&salt), &info, &mut okm).unwrap();

        let expected: [u8; 42] = [
            0x3c, 0xb2, 0x5f, 0x25, 0xfa, 0xac, 0xd5, 0x7a, 0x90, 0x43, 0x4f, 0x64, 0xd0, 0x36,
            0x2f, 0x2a, 0x2d, 0x2d, 0x0a, 0x90, 0xcf, 0x1a, 0x5a, 0x4c, 0x5d, 0xb0, 0x2d, 0x56,
            0xec, 0xc4, 0xc5, 0xbf, 0x34, 0x00, 0x72, 0x08, 0xd5, 0xb8, 0x87, 0x18, 0x58, 0x65,
        ];
        assert_eq!(okm, expected);
    }
}
