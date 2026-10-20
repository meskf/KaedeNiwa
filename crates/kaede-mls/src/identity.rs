//! MLS 身份：签名密钥对 + BasicCredential。
//!
//! 不依赖 `openmls_basic_credential`，避免 openmls_traits 版本冲突。

use openmls::prelude::*;
use openmls_rust_crypto::RustCrypto;
use openmls_traits::{
    crypto::OpenMlsCrypto,
    signatures::{Signer, SignerError},
    types::SignatureScheme,
};

use crate::DEFAULT_CIPHERSUITE;
use crate::error::MlsError;
use crate::provider::KaedeMlsProvider;

/// 签名密钥对。
///
/// 只用 `openmls_traits 0.6` 的 `RustCrypto` 生成/签名，
/// 与 openmls 0.9.1 完全兼容。
pub struct SignatureKeyPair {
    private: zeroize::Zeroizing<Vec<u8>>,
    public: Vec<u8>,
    scheme: SignatureScheme,
}

impl SignatureKeyPair {
    /// 生成新密钥对。
    pub fn new(scheme: SignatureScheme) -> Result<Self, MlsError> {
        let crypto = RustCrypto::default();
        let (private, public) = crypto
            .signature_key_gen(scheme)
            .map_err(|e| MlsError::OpenMls(format!("key gen: {:?}", e)))?;
        Ok(Self {
            private: zeroize::Zeroizing::new(private),
            public,
            scheme,
        })
    }

    /// 公钥字节（用于放进 `CredentialWithKey`）。
    pub fn to_public_vec(&self) -> Vec<u8> {
        self.public.clone()
    }

    // ---- 持久化支持 ----

    pub fn private_bytes(&self) -> &[u8] {
        self.private.as_slice()
    }

    pub fn public_bytes(&self) -> &[u8] {
        &self.public
    }

    pub fn scheme(&self) -> SignatureScheme {
        self.scheme
    }

    /// 从原始字节恢复。
    pub fn from_raw(private: Vec<u8>, public: Vec<u8>, scheme: SignatureScheme) -> Self {
        Self {
            private: zeroize::Zeroizing::new(private),
            public,
            scheme,
        }
    }
}

impl Signer for SignatureKeyPair {
    fn sign(&self, payload: &[u8]) -> Result<Vec<u8>, SignerError> {
        RustCrypto::default()
            .sign(self.scheme, payload, self.private.as_slice())
            .map_err(|_| SignerError::SigningError)
    }

    fn signature_scheme(&self) -> SignatureScheme {
        self.scheme
    }
}

/// MLS 身份。每个 (user, server) 一份，永不跨 server 使用。
pub struct MlsIdentity {
    pub uid: String,
    pub ciphersuite: Ciphersuite,
    pub signature_keys: SignatureKeyPair,
    pub credential_with_key: CredentialWithKey,
}

impl MlsIdentity {
    /// 生成新身份。
    pub fn generate(uid: impl Into<String>) -> Result<Self, MlsError> {
        let uid = uid.into();
        let ciphersuite = DEFAULT_CIPHERSUITE;
        let scheme = ciphersuite.signature_algorithm();
        let signature_keys = SignatureKeyPair::new(scheme)?;

        let credential = BasicCredential::new(uid.as_bytes().to_vec());
        let credential_with_key = CredentialWithKey {
            credential: credential.into(),
            signature_key: signature_keys.to_public_vec().into(),
        };

        Ok(Self {
            uid,
            ciphersuite,
            signature_keys,
            credential_with_key,
        })
    }

    /// 从序列化的签名密钥恢复身份（兼容旧调用，硬编码 DEFAULT_CIPHERSUITE）。
    ///
    /// 新代码请用 [`Self::from_raw_with_ciphersuite`]（P1-1：显式 ciphersuite
    /// 防止未来 DEFAULT_CIPHERSUITE 升级后旧身份 break）。
    pub fn from_raw(
        uid: impl Into<String>,
        scheme: SignatureScheme,
        private: Vec<u8>,
        public: Vec<u8>,
    ) -> Result<Self, MlsError> {
        Self::from_raw_with_ciphersuite(uid, DEFAULT_CIPHERSUITE, scheme, private, public)
    }

    /// 从序列化签名密钥恢复身份（显式 ciphersuite）。
    pub fn from_raw_with_ciphersuite(
        uid: impl Into<String>,
        ciphersuite: Ciphersuite,
        scheme: SignatureScheme,
        private: Vec<u8>,
        public: Vec<u8>,
    ) -> Result<Self, MlsError> {
        let uid = uid.into();
        let signature_keys = SignatureKeyPair::from_raw(private, public, scheme);

        let credential = BasicCredential::new(uid.as_bytes().to_vec());
        let credential_with_key = CredentialWithKey {
            credential: credential.into(),
            signature_key: signature_keys.to_public_vec().into(),
        };

        Ok(Self {
            uid,
            ciphersuite,
            signature_keys,
            credential_with_key,
        })
    }

    /// 生成一个 KeyPackage，序列化为 tls 字节。
    ///
    /// 上层（`kaede-client`）不需要直接依赖 `openmls`。
    /// 生产代码应该一次生成多个并上传到服务器；每个 KP 只能用一次。
    pub fn generate_key_package(&self, provider: &KaedeMlsProvider) -> Result<Vec<u8>, MlsError> {
        use openmls::prelude::KeyPackage as OpenMlsKeyPackage;
        use openmls::prelude::tls_codec::Serialize;

        let bundle = OpenMlsKeyPackage::builder()
            .build(
                self.ciphersuite,
                provider,
                &self.signature_keys,
                self.credential_with_key.clone(),
            )
            .map_err(|e| MlsError::OpenMls(format!("{:?}", e)))?;

        bundle
            .key_package()
            .tls_serialize_detached()
            .map_err(|e| MlsError::TlsCodec(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_identity() {
        let id = MlsIdentity::generate("alice").unwrap();
        assert_eq!(id.uid, "alice");
        assert_eq!(id.ciphersuite, DEFAULT_CIPHERSUITE);
    }

    #[test]
    fn different_identities_differ() {
        let a = MlsIdentity::generate("alice").unwrap();
        let b = MlsIdentity::generate("bob").unwrap();
        assert_ne!(
            a.signature_keys.to_public_vec(),
            b.signature_keys.to_public_vec()
        );
    }
}
