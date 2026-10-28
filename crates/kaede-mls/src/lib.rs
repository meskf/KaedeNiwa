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
pub const DEFAULT_CIPHERSUITE: openmls::prelude::Ciphersuite =
    openmls::prelude::Ciphersuite::MLS_128_MLKEM768X25519_CHACHA20POLY1305_SHA384_MLDSA44;

pub const SFRAME_KEY_LEN: usize = 32;

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


#[cfg(test)]
mod pq_probe {
    use super::*;
    #[test]
    fn pq_probe_ciphersuite() {
        let cs = DEFAULT_CIPHERSUITE;
        println!("CIPHERSUITE = {:?}", cs);
        println!("KEM        = {:?}", cs.hpke_kem_algorithm());
        println!("AEAD       = {:?}", cs.aead_algorithm());
        println!("HASH       = {:?}", cs.hash_algorithm());
        println!("SIG        = {:?}", cs.signature_algorithm());
        assert_eq!(
            cs,
            openmls::prelude::Ciphersuite::MLS_128_MLKEM768X25519_AES128GCM_SHA256_Ed25519,
            "DEFAULT_CIPHERSUITE must be the PQ hybrid suite"
        );
    }
}

#[cfg(test)]
mod pq_size_probe {
    use super::*;
    #[test]
    fn pq_keypackage_size() {
        let id = MlsIdentity::generate("alice").unwrap();
        let provider = KaedeMlsProvider::new();
        let kp = id.generate_key_package(&provider).unwrap();
        println!("PQ KeyPackage size = {} bytes", kp.len());
        assert!(
            kp.len() > 1000,
            "PQ KeyPackage should be >1000 bytes, got {}",
            kp.len()
        );
    }
}