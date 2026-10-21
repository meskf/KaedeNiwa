//! KaedeNiwa MLS 封装（基于 openmls 0.9）。
//!
//! # 范围
//!
//! - [`MlsIdentity`]：签名密钥 + BasicCredential
//! - [`MlsGroupHandle`]：群组生命周期 + App Message 加解密 + SFrame 密钥导出
//!
//! # SFrame 密钥派生
//!
//! 所有 channel 的 SFrame key 通过 MLS exporter 派生：
//!
//! ```text
//! key = group.export_secret(provider, "SFrame 10 MLS", channel_id, 32)
//! ```
//!
//! 这让媒体加密与 MLS epoch 绑定：成员离开触发新 epoch，
//! 旧成员无法解开新 epoch 的媒体。

#![forbid(unsafe_code)]

mod cipher;
mod error;
mod group;
mod identity;
mod provider;
mod purpose;

pub use cipher::{
    ALG_AES_GCM, ALG_DPAPI, ALG_NOOP, AesGcmCipher, NoOpCipher, SnapshotCipher, cipher_for_alg,
};
pub use error::MlsError;
pub use group::MlsGroupHandle;
pub use identity::{MlsIdentity, extract_credential_from_kp};
pub use provider::{KaedeMlsProvider, ProviderError};
pub use purpose::SframePurpose;

/// SFrame 密钥导出的 label。RFC 9605 §5.2 标准。
pub const SFRAME_EXPORT_LABEL: &str = "SFrame 10 MLS";

/// SFrame 密钥长度。
pub const SFRAME_KEY_LEN: usize = 32;

/// 默认 ciphersuite（X25519 + AES-128-GCM + SHA-256 + Ed25519）。
///
/// TODO 阶段 2：评估 PQ ciphersuite。
pub const DEFAULT_CIPHERSUITE: openmls::prelude::Ciphersuite =
    openmls::prelude::Ciphersuite::MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519;

/// crate 版本。
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;

    #[test]
    fn version_is_set() {
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn sframe_label_matches_rfc() {
        assert_eq!(SFRAME_EXPORT_LABEL, "SFrame 10 MLS");
        assert_eq!(SFRAME_KEY_LEN, 32);
    }
}

/// 重导出 `OpenMlsRustCrypto`，让上层 crate 不需要直接依赖
/// `openmls_rust_crypto`。
pub use openmls_rust_crypto::OpenMlsRustCrypto;
