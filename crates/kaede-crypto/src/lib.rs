//! KaedeNiwa 密码学原语。
//!
//! # 范围
//!
//! - **SFrame**：RFC 9605 风格帧加密（媒体 / 应用数据）
//! - **AEAD**：AES-256-GCM / ChaCha20-Poly1305 封装
//! - **HKDF**：SHA-256 密钥派生
//!
//! # 不包含
//!
//! - 公钥密码 / 密钥协商：由 MLS 内部处理（openmls 走自己的 provider）
//! - 身份密钥存储：见 kaede-storage
//!
//! # 安全边界
//!
//! 本 crate **不做密钥管理**。所有密钥由调用方提供、负责生命周期。
//! `SframeKey` 是唯一例外：它用 `Zeroize` 在 drop 时擦除内存。

#![forbid(unsafe_code)]

pub mod aead;
pub mod collision;
pub mod dm_cipher;
pub mod hkdf;
pub mod sframe;

pub use aead::{AeadError, aes256gcm_decrypt, aes256gcm_encrypt};
pub use collision::find_sender_id_collision;
pub use hkdf::{HkdfError, hkdf_sha256};
pub use sframe::{Counter, KeyId, SFRAME_GCM_TAG_LEN, SFRAME_HEADER_LEN, SframeError, SframeKey};

/// KDF info 前缀。所有 KaedeNiwa 的 HKDF info 必须以这个开头，
/// 用于 domain separation。
pub const KDF_INFO_PREFIX: &str = "kaede-v2";

/// 从 OS RNG 生成 N 字节随机数。
///
/// - WASM：走 `getrandom` 的 js backend
/// - Native：走 OS 提供的 CSPRNG
pub fn random_bytes<const N: usize>() -> [u8; N] {
    use rand_core::RngCore;
    let mut buf = [0u8; N];
    rand_core::OsRng.fill_bytes(&mut buf);
    buf
}

/// crate 版本。
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_bytes_differ() {
        let a: [u8; 32] = random_bytes();
        let b: [u8; 32] = random_bytes();
        assert_ne!(a, b, "two random draws should differ");
    }

    #[test]
    fn version_matches_cargo() {
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
    }
}
