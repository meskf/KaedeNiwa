//! Snapshot cipher 的平台实现。
//!
//! # 目标
//!
//! MLS snapshot 里包含**签名私钥**。默认存明文的话，任何能读 SQLite
//! 文件 / IndexedDB 的攻击者都能拿到身份。
//!
//! # 平台策略
//! ## ⚠️ S-2 安全边界摘要（2026-10-19 审查）
//!
//! - **Windows**：DPAPI 与用户账户绑定——即使 snapshot 泄露，其他用户/机器也解不开。
//! - **Web（fallback）**：key = SHA256(公开信息)。**任何能读 IndexedDB 的攻击者都能解密**。
//!   详细说明见 [`fallback::derive_device_key`] 的 doc。
//! - **Desktop 优先**：生产部署应引导用户用 Desktop，Web 端只用于低价值场景。
//! - **迁移路径**：S-1 的 KDF 修正 + S-2 的 WebCrypto non-extractable key
//!   必须作为 snapshot 格式升级（`0x03` → `0x04`）一起做。**当前不做**。
//!
//! - **Windows**：DPAPI（`CryptProtectData`）—— 用当前用户的 master key，
//!   与账户绑定，换用户 / 换机器解不开
//! - **macOS / Linux Desktop**：系统钥匙串（Keychain / Secret Service）保存随机 key；
//!   钥匙串不可用时**拒绝持久化**，避免退回公开信息派生 key
//! - **Web**：WebCrypto DEK（可用时）；否则 SHA256 派生 key（fallback，安全有限）
//!
//! 生产 TODO：
//! - Web：WebCrypto `generateKey(non-extractable)` + 存 IndexedDB
//!
//! # 向后兼容
//!
//! snapshot 外层头 `[0x03][alg_id:u8][ct_len:u32 BE][ct]` 里的
//! `alg_id` 决定用哪种 cipher。老 snapshot（AES-GCM，`alg_id=0x01`）仍
//! 可解，新写入用平台 cipher（Windows 用 DPAPI，`alg_id=0x10`）。

use kaede_mls::SnapshotCipher;

// =========================================================================
// Windows: DPAPI
// =========================================================================

#[cfg(windows)]
pub mod dpapi {
    use std::ffi::c_void;

    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };

    use kaede_mls::{MlsError, SnapshotCipher};

    /// DPAPI snapshot cipher。
    ///
    /// 无状态：每次 `encrypt` / `decrypt` 都直接用当前用户的 master key。
    pub struct DpapiCipher;

    impl DpapiCipher {
        pub fn new() -> Self {
            Self
        }
    }

    impl Default for DpapiCipher {
        fn default() -> Self {
            Self::new()
        }
    }

    fn blob_from_slice(s: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: s.len() as u32,
            pbData: s.as_ptr() as *mut u8,
        }
    }

    fn empty_blob() -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        }
    }

    fn free_blob(b: CRYPT_INTEGER_BLOB) {
        if !b.pbData.is_null() {
            unsafe {
                LocalFree(b.pbData as *mut c_void);
            }
        }
    }

    fn blob_to_vec(b: &CRYPT_INTEGER_BLOB) -> Vec<u8> {
        if b.pbData.is_null() || b.cbData == 0 {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(b.pbData, b.cbData as usize).to_vec() }
        }
    }

    impl SnapshotCipher for DpapiCipher {
        fn alg_id(&self) -> u8 {
            kaede_mls::ALG_DPAPI
        }

        fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, MlsError> {
            let in_blob = blob_from_slice(plaintext);
            let mut out_blob = empty_blob();

            let ok = unsafe {
                CryptProtectData(
                    &in_blob,
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut out_blob,
                )
            };
            if ok == 0 {
                let err = std::io::Error::last_os_error();
                free_blob(out_blob);
                return Err(MlsError::OpenMls(format!(
                    "CryptProtectData failed: {}",
                    err
                )));
            }
            let out = blob_to_vec(&out_blob);
            free_blob(out_blob);
            Ok(out)
        }

        fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, MlsError> {
            let in_blob = blob_from_slice(ciphertext);
            let mut out_blob = empty_blob();

            let ok = unsafe {
                CryptUnprotectData(
                    &in_blob,
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut out_blob,
                )
            };
            if ok == 0 {
                let err = std::io::Error::last_os_error();
                free_blob(out_blob);
                return Err(MlsError::OpenMls(format!(
                    "CryptUnprotectData failed: {}",
                    err
                )));
            }
            let out = blob_to_vec(&out_blob);
            free_blob(out_blob);
            Ok(out)
        }
    }
}

#[cfg(windows)]
pub use dpapi::DpapiCipher;

// =========================================================================
// Fallback: SHA256 派生 + AES-256-GCM
// =========================================================================

pub mod fallback {
    use kaede_mls::AesGcmCipher;

    /// ## ⚠️ S-1 历史遗留 bug（2026-10-19 审查发现）
    ///
    /// 本函数实际计算 `SHA256("kaede-snapshot-v1:" || uid || ":" || server_url || ":" || server_url)`
    /// —— `server_url` 被 update **两次**。看起来是 copy-paste 后忘了改，
    /// 语义上应该是 `uid || ":" || server_url`。
    ///
    /// **为什么不能修**：任何现有 Web 用户的 snapshot 都是用"重复 server_url"
    /// 版 KDF 加密的。改成正确版会让**所有现有 Web 用户丢失本地 MLS 身份**
    /// （签名私钥）→ 必须重新加入所有群组。
    ///
    /// **修复条件**：必须同时升级 snapshot 格式版本（外层头 `0x03` → `0x04`），
    /// 保留"双路回退"（`0x04` 用正确 KDF，`0x03` 用重复 server_url KDF）→
    /// 用户下次登录时**自动迁移**（读取 0x03 → 解密 → 写 0x04）。
    /// 该迁移尚未实现。**当前保留重复 server_url 的 KDF 是有意为之**。
    ///
    /// ## ⚠️ S-2 Web 端 fallback cipher 的安全边界
    ///
    /// 本函数的两个输入（`uid` + `server_url`）都是**公开信息**：
    /// - `uid` = login_id（服务器可见、UI 显示）
    /// - `server_url` = 服务器地址（客户端配置、DNS 可查）
    ///
    /// 因此任何能读 IndexedDB 的攻击者（浏览器扩展 / XSS / 物理访问）
    /// 都能**用公开信息重算 key → 解密 snapshot → 拿到签名私钥**。
    ///
    /// **防御范围**：
    /// - ✅ 随手打开 IndexedDB 的偶然窥探
    /// - ❌ **不防御**：浏览器扩展、XSS、物理访问、恶意本地程序
    ///
    /// **生产建议**：
    /// - Web 端**只用于低价值账号**，不要承载生产身份
    /// - 生产部署优先 Desktop（Windows 走 DPAPI，与用户账户绑定）
    /// - 未来升级路径：WebCrypto `generateKey(non-extractable)` + IndexedDB 存 handle
    ///   （已列入 TODO，但涉及 snapshot 格式升级，与 S-1 迁移一起做）
    ///
    /// 从 `uid + server_url` 派生 32 字节 AES key。
    ///
    /// ⚠️ 安全边界（诚实说明）：
    ///   - 防御：sqlite3 / IndexedDB 随手打开的偶然窥探
    ///   - 不防御：攻击者能读本 crate 源码（salt 和 KDF 是公开的）
    ///
    /// 生产环境应换成平台 keystore（Windows DPAPI / macOS Keychain /
    /// Web WebCrypto）。
    pub fn derive_device_key(uid: &str, server_url: &str) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(b"kaede-snapshot-v1:");
        h.update(uid.as_bytes());
        h.update(b":");
        h.update(server_url.as_bytes());
        h.update(b":");
        h.update(server_url.as_bytes());
        let out = h.finalize();
        let mut k = [0u8; 32];
        k.copy_from_slice(&out);
        k
    }

    /// 构造 fallback cipher。
    pub fn cipher(uid: &str, server_url: &str) -> AesGcmCipher {
        AesGcmCipher::from_bytes(derive_device_key(uid, server_url))
    }
}

// =========================================================================
// 统一入口
// =========================================================================

/// macOS / Linux Desktop：把随机 snapshot key 放进系统钥匙串。
///
/// key 在进程内缓存，避免每条消息落盘都访问 Keychain / Secret Service。
#[cfg(all(
    not(windows),
    not(target_arch = "wasm32"),
    not(target_os = "android"),
    not(target_os = "ios")
))]
pub mod keyring_store {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use kaede_mls::{AesGcmCipher, MlsError, SnapshotCipher};

    const SERVICE: &str = "kaedeniwa-mls-snapshot";

    fn cache() -> &'static Mutex<HashMap<String, [u8; 32]>> {
        static CACHE: OnceLock<Mutex<HashMap<String, [u8; 32]>>> = OnceLock::new();
        CACHE.get_or_init(|| Mutex::new(HashMap::new()))
    }

    fn entry(uid: &str, server_url: &str) -> Result<keyring::Entry, MlsError> {
        keyring::Entry::new(SERVICE, &format!("{uid}@{server_url}"))
            .map_err(|e| MlsError::OpenMls(format!("keyring entry: {e}")))
    }

    fn load_key(uid: &str, server_url: &str) -> Result<[u8; 32], MlsError> {
        let cache_key = format!("{uid}\n{server_url}");
        if let Some(k) = cache()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&cache_key)
            .copied()
        {
            return Ok(k);
        }

        let entry = entry(uid, server_url)?;
        let key = match entry.get_password() {
            Ok(encoded) => {
                let raw = URL_SAFE_NO_PAD
                    .decode(encoded.as_bytes())
                    .map_err(|e| MlsError::OpenMls(format!("keyring key decode: {e}")))?;
                raw.try_into()
                    .map_err(|_| MlsError::OpenMls("keyring key has wrong length".into()))?
            }
            Err(keyring::Error::NoEntry) => {
                let key: [u8; 32] = kaede_crypto::random_bytes();
                entry
                    .set_password(&URL_SAFE_NO_PAD.encode(key))
                    .map_err(|e| MlsError::OpenMls(format!("keyring set: {e}")))?;
                key
            }
            Err(e) => return Err(MlsError::OpenMls(format!("keyring get: {e}"))),
        };

        cache()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(cache_key, key);
        Ok(key)
    }

    pub struct KeyringCipher {
        inner: AesGcmCipher,
    }

    impl KeyringCipher {
        pub fn open(uid: &str, server_url: &str) -> Result<Self, MlsError> {
            Ok(Self {
                inner: AesGcmCipher::from_bytes(load_key(uid, server_url)?),
            })
        }
    }

    impl SnapshotCipher for KeyringCipher {
        fn alg_id(&self) -> u8 {
            kaede_mls::ALG_AES_GCM
        }

        fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, MlsError> {
            self.inner.encrypt(plaintext)
        }

        fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, MlsError> {
            self.inner.decrypt(ciphertext)
        }
    }

    /// 钥匙串不可用时的显式失败 cipher：让 save_snapshot 失败并阻断发送，
    /// 而不是悄悄退回公开信息派生 key。
    pub struct FailingCipher {
        message: String,
    }

    impl FailingCipher {
        pub fn new(message: impl Into<String>) -> Self {
            Self {
                message: message.into(),
            }
        }
    }

    impl SnapshotCipher for FailingCipher {
        fn alg_id(&self) -> u8 {
            kaede_mls::ALG_AES_GCM
        }

        fn encrypt(&self, _: &[u8]) -> Result<Vec<u8>, MlsError> {
            Err(MlsError::OpenMls(format!(
                "snapshot persistence unavailable: {}",
                self.message
            )))
        }

        fn decrypt(&self, _: &[u8]) -> Result<Vec<u8>, MlsError> {
            Err(MlsError::OpenMls(format!(
                "snapshot persistence unavailable: {}",
                self.message
            )))
        }
    }
}

/// Web 端“拒绝写盘”cipher。
///
/// WebCrypto DEK 不可用时必须暂停落盘/发送，不能退回到由公开信息派生的
/// key（那会让任何能读 IndexedDB 的人解密快照）。
#[cfg(all(not(windows), target_arch = "wasm32"))]
struct UnavailableCipher {
    message: String,
}

#[cfg(all(not(windows), target_arch = "wasm32"))]
impl SnapshotCipher for UnavailableCipher {
    fn alg_id(&self) -> u8 {
        kaede_mls::ALG_AES_GCM
    }

    fn encrypt(&self, _: &[u8]) -> Result<Vec<u8>, kaede_mls::MlsError> {
        Err(kaede_mls::MlsError::OpenMls(format!(
            "snapshot persistence unavailable: {}",
            self.message
        )))
    }

    fn decrypt(&self, _: &[u8]) -> Result<Vec<u8>, kaede_mls::MlsError> {
        Err(kaede_mls::MlsError::OpenMls(format!(
            "snapshot decryption unavailable: {}",
            self.message
        )))
    }
}

/// 返回当前平台的 snapshot cipher。
///
/// - Windows → DPAPI
/// - macOS / Linux Desktop → 系统钥匙串里的随机 key
/// - Web 且 WebCrypto DEK 已就绪 → WebCrypto DEK（与读取端一致）
/// - Web 无 DEK → **拒绝写盘**（不再退回公开信息派生 key）
pub fn device_cipher(uid: &str, server_url: &str) -> Box<dyn SnapshotCipher> {
    #[cfg(windows)]
    {
        let _ = (uid, server_url);
        Box::new(DpapiCipher::new())
    }
    #[cfg(all(not(windows), target_arch = "wasm32"))]
    {
        let _ = (uid, server_url);
        match crate::keystore_web::cached_dek() {
            Some(dek) => Box::new(kaede_mls::AesGcmCipher::from_bytes(dek)),
            None => Box::new(UnavailableCipher {
                message: "WebCrypto DEK not initialized".into(),
            }),
        }
    }
    // 移动端：当前没有可用的系统 Keystore 绑定。使用与 Web 相同的
    // 公开信息派生 key；snapshot 位于 App 私有沙箱内。威胁模型与 Web 一致
    // （防随手读取，不防能执行代码的攻击者）。
    #[cfg(all(
        not(windows),
        not(target_arch = "wasm32"),
        any(target_os = "android", target_os = "ios")
    ))]
    {
        tracing::warn!(
            "[keystore] mobile build uses fallback snapshot cipher (no platform keystore integration)"
        );
        Box::new(fallback::cipher(uid, server_url))
    }
    #[cfg(all(
        not(windows),
        not(target_arch = "wasm32"),
        not(target_os = "android"),
        not(target_os = "ios")
    ))]
    {
        match keyring_store::KeyringCipher::open(uid, server_url) {
            Ok(cipher) => Box::new(cipher),
            Err(e) => {
                tracing::error!(
                    "[keystore] system keyring unavailable; refusing to persist MLS snapshot: {}",
                    e
                );
                Box::new(keyring_store::FailingCipher::new(e.to_string()))
            }
        }
    }
}

/// Web 端迁移期 cipher：优先用 DEK 解密，失败后回退到 SHA256 fallback。
///
/// 历史 snapshot 是用 fallback 写入的，而当前写入已经切到 DEK；读取时
/// 允许两者都试一次，避免升级后一次性丢失旧身份。加密永远走 primary。
#[cfg(all(not(windows), target_arch = "wasm32"))]
struct WebDekWithFallback {
    primary: kaede_mls::AesGcmCipher,
    secondary: kaede_mls::AesGcmCipher,
}

#[cfg(all(not(windows), target_arch = "wasm32"))]
impl SnapshotCipher for WebDekWithFallback {
    fn alg_id(&self) -> u8 {
        kaede_mls::ALG_AES_GCM
    }

    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, kaede_mls::MlsError> {
        self.primary.encrypt(plaintext)
    }

    fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, kaede_mls::MlsError> {
        self.primary
            .decrypt(ciphertext)
            .or_else(|_| self.secondary.decrypt(ciphertext))
    }
}

/// Desktop 迁移期 cipher：优先用系统钥匙串 key 解密，失败后回退到旧 fallback。
#[cfg(all(
    not(windows),
    not(target_arch = "wasm32"),
    not(target_os = "android"),
    not(target_os = "ios")
))]
struct KeyringWithFallback {
    primary: keyring_store::KeyringCipher,
    secondary: kaede_mls::AesGcmCipher,
}

#[cfg(all(
    not(windows),
    not(target_arch = "wasm32"),
    not(target_os = "android"),
    not(target_os = "ios")
))]
impl SnapshotCipher for KeyringWithFallback {
    fn alg_id(&self) -> u8 {
        kaede_mls::ALG_AES_GCM
    }

    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, kaede_mls::MlsError> {
        self.primary.encrypt(plaintext)
    }

    fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, kaede_mls::MlsError> {
        self.primary
            .decrypt(ciphertext)
            .or_else(|_| self.secondary.decrypt(ciphertext))
    }
}

/// 读 snapshot 外层头的 `alg_id`，返回匹配的 cipher。
///
/// 用于**恢复**：老的 AES-GCM snapshot 必须用 fallback cipher 才能解；
/// 新的 DPAPI snapshot 必须用 DPAPI cipher。
///
/// 不是 `0x03` 外层头（老格式明文）→ 返回 `NoOpCipher`。
pub fn device_cipher_for_snapshot(
    uid: &str,
    server_url: &str,
    snapshot: &[u8],
) -> Box<dyn SnapshotCipher> {
    // 外层格式 [version:u8][alg_id:u8][ct_len:u32 BE][ct]
    // P0-4: provider.rs 写 0x02，老代码检查 0x03 → DPAPI 恢复永远失败。
    // 同时接受 0x02 和 0x03 以兼容已有旧 snapshot。
    if snapshot.len() < 6 || (snapshot[0] != 0x02 && snapshot[0] != 0x03) {
        return Box::new(kaede_mls::NoOpCipher);
    }
    let alg = snapshot[1];
    match alg {
        kaede_mls::ALG_NOOP => Box::new(kaede_mls::NoOpCipher),
        #[cfg(target_arch = "wasm32")]
        kaede_mls::ALG_AES_GCM => {
            if let Some(dek) = crate::keystore_web::cached_dek() {
                Box::new(WebDekWithFallback {
                    primary: kaede_mls::AesGcmCipher::from_bytes(dek),
                    secondary: fallback::cipher(uid, server_url),
                })
            } else {
                Box::new(fallback::cipher(uid, server_url))
            }
        }
        #[cfg(windows)]
        kaede_mls::ALG_AES_GCM => Box::new(fallback::cipher(uid, server_url)),
        #[cfg(all(
            not(windows),
            not(target_arch = "wasm32"),
            any(target_os = "android", target_os = "ios")
        ))]
        kaede_mls::ALG_AES_GCM => Box::new(fallback::cipher(uid, server_url)),
        #[cfg(all(
            not(windows),
            not(target_arch = "wasm32"),
            not(target_os = "android"),
            not(target_os = "ios")
        ))]
        kaede_mls::ALG_AES_GCM => {
            let secondary = fallback::cipher(uid, server_url);
            match keyring_store::KeyringCipher::open(uid, server_url) {
                Ok(primary) => Box::new(KeyringWithFallback { primary, secondary }),
                Err(e) => {
                    // 钥匙串暂时不可用：只用旧 fallback 读，避免直接丢身份；
                    // 新写入仍会走 device_cipher 并显式失败。
                    tracing::warn!(
                        "[keystore] keyring unavailable while restoring; legacy fallback only: {}",
                        e
                    );
                    Box::new(secondary)
                }
            }
        }
        kaede_mls::ALG_DPAPI => {
            #[cfg(windows)]
            {
                Box::new(DpapiCipher::new())
            }
            #[cfg(not(windows))]
            {
                // 非 Windows 上遇到 DPAPI snapshot：解不开。
                // 返回 NoOp，调用方 decrypt 时会失败并走"重新建群"路径。
                let _ = (uid, server_url);
                Box::new(kaede_mls::NoOpCipher)
            }
        }
        _ => Box::new(kaede_mls::NoOpCipher),
    }
}

// =========================================================================
// 测试
// =========================================================================

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn dpapi_roundtrip() {
        let c = DpapiCipher::new();
        let pt = b"hello snapshot";
        let ct = c.encrypt(pt).unwrap();
        assert_ne!(ct, pt, "DPAPI output should differ from input");
        assert_eq!(c.decrypt(&ct).unwrap(), pt);
    }

    #[test]
    fn dpapi_empty_roundtrip() {
        let c = DpapiCipher::new();
        let ct = c.encrypt(b"").unwrap();
        assert_eq!(c.decrypt(&ct).unwrap(), b"");
    }

    #[test]
    fn dpapi_wrong_input_rejected() {
        let c = DpapiCipher::new();
        assert!(c.decrypt(b"garbage not dpapi blob").is_err());
    }

    #[test]
    fn dpapi_alg_id() {
        assert_eq!(DpapiCipher::new().alg_id(), kaede_mls::ALG_DPAPI);
    }

    #[test]
    fn dpapi_output_is_random() {
        // DPAPI 每次加密应该输出不同（含 salt）
        let c = DpapiCipher::new();
        let a = c.encrypt(b"x").unwrap();
        let b = c.encrypt(b"x").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn device_cipher_for_snapshot_dispatches_by_alg_id() {
        let uid = "alice";
        let url = "http://x";

        // 老格式（明文）→ NoOp
        let plain = device_cipher_for_snapshot(uid, url, &[0x01, 0x02]);
        assert_eq!(plain.alg_id(), kaede_mls::ALG_NOOP);

        // DPAPI alg_id → 返回 DPAPI cipher
        let mut dpapi_snap = vec![0x02, kaede_mls::ALG_DPAPI, 0, 0, 0, 0];
        dpapi_snap.extend_from_slice(b"fake ct");
        let c = device_cipher_for_snapshot(uid, url, &dpapi_snap);
        assert_eq!(c.alg_id(), kaede_mls::ALG_DPAPI);

        // AES-GCM alg_id → 返回 fallback
        let mut aes_snap = vec![0x02, kaede_mls::ALG_AES_GCM, 0, 0, 0, 0];
        aes_snap.extend_from_slice(b"fake ct");
        let c = device_cipher_for_snapshot(uid, url, &aes_snap);
        assert_eq!(c.alg_id(), kaede_mls::ALG_AES_GCM);
    }
}

#[cfg(all(test, not(windows)))]
mod fallback_tests {
    use super::*;

    #[test]
    fn fallback_derives_stable_key() {
        let a = fallback::derive_device_key("alice", "http://x");
        let b = fallback::derive_device_key("alice", "http://x");
        assert_eq!(a, b);
        let c = fallback::derive_device_key("bob", "http://x");
        assert_ne!(a, c);
    }

    #[test]
    fn fallback_roundtrip() {
        let c = fallback::cipher("alice", "http://x");
        let ct = c.encrypt(b"secret").unwrap();
        assert_ne!(ct, b"secret");
        assert_eq!(c.decrypt(&ct).unwrap(), b"secret");
    }

    #[test]
    fn device_cipher_for_snapshot_dispatches_by_alg_id() {
        let uid = "alice";
        let url = "http://x";
        let plain = device_cipher_for_snapshot(uid, url, &[0x01]);
        assert_eq!(plain.alg_id(), kaede_mls::ALG_NOOP);

        let mut aes_snap = vec![0x02, kaede_mls::ALG_AES_GCM, 0, 0, 0, 0];
        aes_snap.extend_from_slice(b"fake ct");
        let c = device_cipher_for_snapshot(uid, url, &aes_snap);
        assert_eq!(c.alg_id(), kaede_mls::ALG_AES_GCM);
    }
}
