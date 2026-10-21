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
///
/// HM1: 字段全部私有，只能通过 `generate` / `from_raw_with_ciphersuite` 构造，
/// 避免外部构造出 uid != credential 或 signature_keys 与 credential 公钥
/// 不一致的"畸形" identity（MLS 会下游校验失败，但错误时序难以诊断）。
pub struct MlsIdentity {
    uid: String,
    ciphersuite: Ciphersuite,
    signature_keys: SignatureKeyPair,
    credential_with_key: CredentialWithKey,
}

impl MlsIdentity {
    /// HM1: 只读访问器
    pub fn uid(&self) -> &str {
        &self.uid
    }

    pub fn ciphersuite(&self) -> Ciphersuite {
        self.ciphersuite
    }

    pub fn signature_keys(&self) -> &SignatureKeyPair {
        &self.signature_keys
    }

    pub fn credential_with_key(&self) -> &CredentialWithKey {
        &self.credential_with_key
    }

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

/// C8: 从 KeyPackage 原始字节中提取 credential（uid）。
///
/// **不做签名验证** —— 本函数只解析结构。用于：
/// - server `/api/keypackages`：校验上传的 KP 是否属于 auth uid
/// - client `fetch_key_package`：校验 server 返回的 KP 是否属于请求的 target
///
/// 真正的 MLS 签名验证由 `MlsGroupHandle::add_member` 里的
/// `kp_in.validate(provider.crypto(), ...)` 负责。
///
/// # 安全说明
///
/// 返回的 credential 使用**严格 UTF-8** 校验（不用 `from_utf8_lossy`）。
/// 非 UTF-8 的 credential 在 `add_member` 里也会被拒（MM3）——本函数
/// 是"上游防呆"，让 server 在存储阶段就拒绝恶意 KP。
pub fn extract_credential_from_kp(kp_bytes: &[u8]) -> Result<String, MlsError> {
    use openmls::prelude::tls_codec::Deserialize;
    use openmls::prelude::{KeyPackageIn, ProtocolVersion};
    use openmls_rust_crypto::RustCrypto;

    let kp_in = KeyPackageIn::tls_deserialize(&mut &kp_bytes[..])
        .map_err(|e| MlsError::TlsCodec(e.to_string()))?;
    // validate 会验证协议版本、ciphersuite、签名。对恶意 KP 是额外防线
    // （attacker 上传非 Mls10 或签名无效的 KP 在 server 端就会被拒）。
    let kp = kp_in
        .validate(&RustCrypto::default(), ProtocolVersion::Mls10)
        .map_err(crate::error::mls_err)?;
    let cred_bytes = kp.leaf_node().credential().serialized_content();
    std::str::from_utf8(cred_bytes)
        .map_err(|e| MlsError::OpenMls(format!("credential not UTF-8: {}", e)))
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_identity() {
        let id = MlsIdentity::generate("alice").unwrap();
        assert_eq!(id.uid(), "alice");
        assert_eq!(id.ciphersuite(), DEFAULT_CIPHERSUITE);
    }

    #[test]
    fn different_identities_differ() {
        let a = MlsIdentity::generate("alice").unwrap();
        let b = MlsIdentity::generate("bob").unwrap();
        assert_ne!(
            a.signature_keys().to_public_vec(),
            b.signature_keys().to_public_vec()
        );
    }
}
