//! MLS Provider：自带快照能力的 `RustCrypto` + `MemoryStorage`。
//!
//! 与 openmls 自带的 `OpenMlsRustCrypto` 结构相同，但暴露
//! [`KaedeMlsProvider::snapshot`] / [`KaedeMlsProvider::from_snapshot`]。
//!
//! # 快照格式
//!
//! 自定义格式（**不用 openmls 的 `test-utils` feature**）：
//!
//! ```text
//! [ count: u64 BE ]
//! repeat count:
//!   [ k_len: u64 BE ][ v_len: u64 BE ][ key bytes ][ value bytes ]
//! ```
//!
//! 快照包含整个 storage（所有 group state / keypackage / signature key）。
//! 恢复后 `MlsGroup::load(storage, group_id)` 可以正常工作。

use std::collections::HashMap;
use std::io::{Cursor, Read};

/// snapshot 内层格式版本。与外层 `[0x02][alg][u32]` header 独立，
/// 用于兼容内层结构变化。
///
/// **警告**：`from_snapshot_with` 用 `bytes[0] == 0x02` 判定新格式。
/// `INNER_VERSION` 不能为 0x02，否则旧格式（直接是 inner）会与新格式
/// 判型歧义。
const INNER_VERSION: u8 = 1;

// P0-C: hard caps on deserialization to prevent malicious snapshot DoS.
const MAX_SNAPSHOT_BYTES: usize = 64 * 1024 * 1024; // 64 MiB
const MAX_ENTRIES: usize = 10_000;
const MAX_KEY_BYTES: usize = 4096;
const MAX_VALUE_BYTES: usize = 16 * 1024 * 1024;

use openmls_memory_storage::MemoryStorage;
use openmls_rust_crypto::RustCrypto;
use openmls_traits::OpenMlsProvider;

/// MLS Provider。跨平台可用。
pub struct KaedeMlsProvider {
    crypto: RustCrypto,
    storage: MemoryStorage,
}

impl KaedeMlsProvider {
    /// 全新的空 provider。
    pub fn new() -> Self {
        Self {
            crypto: RustCrypto::default(),
            storage: MemoryStorage::default(),
        }
    }

    /// 从快照恢复（不加密路径，向后兼容）。
    pub fn from_snapshot(bytes: &[u8]) -> Result<Self, ProviderError> {
        Self::from_snapshot_with(bytes, None)
    }

    /// 从快照恢复。
    ///
    /// - `key_loader`：`Some` 时用于按 alg_id 取 cipher；`None` 时非 NoOp 会报错
    /// - 老的 `0x01` 明文格式（无外层包头）也会被识别并走老路径
    pub fn from_snapshot_with(
        bytes: &[u8],
        cipher: Option<&dyn crate::SnapshotCipher>,
    ) -> Result<Self, ProviderError> {
        // P0-C: reject oversized inputs before any parsing.
        if bytes.len() > MAX_SNAPSHOT_BYTES {
            return Err(ProviderError::Deserialize(format!(
                "snapshot too large: {} > {}",
                bytes.len(),
                MAX_SNAPSHOT_BYTES
            )));
        }
        // 判断是否是新格式：[0x02][alg][u32]
        let (inner_bytes, _alg) = if bytes.len() >= 6 && bytes[0] == 0x02 {
            let alg = bytes[1];
            let ct_len = u32::from_be_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]) as usize;
            let ct = &bytes[6..];
            if ct.len() != ct_len {
                return Err(ProviderError::Deserialize(format!(
                    "ciphertext length mismatch: header={} actual={}",
                    ct_len,
                    ct.len()
                )));
            }
            let pt = if alg == crate::ALG_NOOP {
                ct.to_vec()
            } else {
                let c = cipher.ok_or_else(|| {
                    ProviderError::Deserialize(format!(
                        "cipher alg {} requires a cipher instance",
                        alg
                    ))
                })?;
                if c.alg_id() != alg {
                    return Err(ProviderError::Deserialize(format!(
                        "cipher alg mismatch: snapshot={} supplied={}",
                        alg,
                        c.alg_id()
                    )));
                }
                c.decrypt(ct)
                    .map_err(|e| ProviderError::Deserialize(e.to_string()))?
            };
            (pt, Some(alg))
        } else {
            // 老格式：直接是 inner
            (bytes.to_vec(), None)
        };

        let mut cursor = Cursor::new(inner_bytes.as_slice());

        // inner 版本检查
        let inner_version = {
            let mut b = [0u8; 1];
            cursor
                .read_exact(&mut b)
                .map_err(|e| ProviderError::Deserialize(e.to_string()))?;
            b[0]
        };
        if inner_version != INNER_VERSION {
            return Err(ProviderError::Deserialize(format!(
                "unsupported inner version: {}",
                inner_version
            )));
        }

        let count_raw = read_u64(&mut cursor)?;
        let count: usize = count_raw.try_into().map_err(|_| {
            ProviderError::Deserialize(format!("entry count {} overflows usize", count_raw))
        })?;
        // P0-C: cap entry count to prevent memory exhaustion.
        if count > MAX_ENTRIES {
            return Err(ProviderError::Deserialize(format!(
                "too many entries: {} > {}",
                count, MAX_ENTRIES
            )));
        }
        let mut map = HashMap::with_capacity(count.min(1024));

        for _ in 0..count {
            let k_len_raw = read_u64(&mut cursor)?;
            let v_len_raw = read_u64(&mut cursor)?;
            let k_len: usize = k_len_raw.try_into().map_err(|_| {
                ProviderError::Deserialize(format!("key len {} overflows usize", k_len_raw))
            })?;
            let v_len: usize = v_len_raw.try_into().map_err(|_| {
                ProviderError::Deserialize(format!("value len {} overflows usize", v_len_raw))
            })?;
            // P0-C: cap per-entry key/value lengths.
            if k_len > MAX_KEY_BYTES {
                return Err(ProviderError::Deserialize(format!(
                    "key too large: {} > {}",
                    k_len, MAX_KEY_BYTES
                )));
            }
            if v_len > MAX_VALUE_BYTES {
                return Err(ProviderError::Deserialize(format!(
                    "value too large: {} > {}",
                    v_len, MAX_VALUE_BYTES
                )));
            }
            let mut k = vec![0u8; k_len];
            let mut v = vec![0u8; v_len];
            cursor
                .read_exact(&mut k)
                .map_err(|e| ProviderError::Deserialize(e.to_string()))?;
            cursor
                .read_exact(&mut v)
                .map_err(|e| ProviderError::Deserialize(e.to_string()))?;
            if map.insert(k, v).is_some() {
                return Err(ProviderError::Deserialize("duplicate snapshot key".into()));
            }
        }
        if cursor.position() != inner_bytes.len() as u64 {
            return Err(ProviderError::Deserialize(format!(
                "trailing bytes in snapshot: {} != {}",
                cursor.position(),
                inner_bytes.len()
            )));
        }

        // MM5: 直接访问 openmls_memory_storage 的 pub `values` 字段。
        // 升级 openmls_memory_storage 时必须人工核对字段类型是否兼容。
        let storage = MemoryStorage::default();
        *storage.values.write().unwrap_or_else(|e| e.into_inner()) = map;

        Ok(Self {
            crypto: RustCrypto::default(),
            storage,
        })
    }

    /// 序列化整个 storage 到字节（不加密）。
    ///
    /// 生产代码应使用 [`Self::snapshot_with`] 并传入 cipher。
    pub fn snapshot(&self) -> Result<Vec<u8>, ProviderError> {
        self.snapshot_with(&crate::NoOpCipher)
    }

    /// 序列化 + 加密。
    ///
    /// 输出格式：
    /// ```text
    /// [0x02][alg_id:u8][ct_len:u32 BE][ct]
    /// ```
    pub fn snapshot_with(
        &self,
        cipher: &dyn crate::SnapshotCipher,
    ) -> Result<Vec<u8>, ProviderError> {
        // 锁中毒（此前某次 panic）不应让快照功能永久不可用。
        let values = self
            .storage
            .values
            .read()
            .unwrap_or_else(|e| e.into_inner());
        let mut inner = Vec::with_capacity(16 + values.len() * 64);

        // inner 版本号
        inner.push(INNER_VERSION);
        inner.extend_from_slice(&(values.len() as u64).to_be_bytes());
        for (k, v) in values.iter() {
            inner.extend_from_slice(&(k.len() as u64).to_be_bytes());
            inner.extend_from_slice(&(v.len() as u64).to_be_bytes());
            inner.extend_from_slice(k);
            inner.extend_from_slice(v);
        }
        drop(values);

        let ct = cipher
            .encrypt(&inner)
            .map_err(|e| ProviderError::Serialize(e.to_string()))?;

        // 中间缓冲里含 KeyPackage / 群组私钥材料，加密后立刻清零。
        {
            use zeroize::Zeroize;
            inner.zeroize();
        }

        let mut out = Vec::with_capacity(6 + ct.len());
        out.push(0x02); // outer version
        out.push(cipher.alg_id());
        out.extend_from_slice(&(ct.len() as u32).to_be_bytes());
        out.extend_from_slice(&ct);
        Ok(out)
    }
}

impl Default for KaedeMlsProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenMlsProvider for KaedeMlsProvider {
    type CryptoProvider = RustCrypto;
    type RandProvider = RustCrypto;
    type StorageProvider = MemoryStorage;

    fn storage(&self) -> &Self::StorageProvider {
        &self.storage
    }

    fn crypto(&self) -> &Self::CryptoProvider {
        &self.crypto
    }

    fn rand(&self) -> &Self::RandProvider {
        &self.crypto
    }
}

fn read_u64(cursor: &mut Cursor<&[u8]>) -> Result<u64, ProviderError> {
    let mut b = [0u8; 8];
    cursor
        .read_exact(&mut b)
        .map_err(|e| ProviderError::Deserialize(e.to_string()))?;
    Ok(u64::from_be_bytes(b))
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("serialize failed: {0}")]
    Serialize(String),
    #[error("deserialize failed: {0}")]
    Deserialize(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_snapshot_roundtrip() {
        let p = KaedeMlsProvider::new();
        let bytes = p.snapshot().unwrap();
        let p2 = KaedeMlsProvider::from_snapshot(&bytes).unwrap();
        assert_eq!(p2.snapshot().unwrap(), bytes);
    }

    #[test]
    fn populated_snapshot_roundtrip() {
        let p = KaedeMlsProvider::new();
        // 直接往 storage 里塞点数据
        {
            let mut values = p.storage.values.write().unwrap();
            values.insert(b"key1".to_vec(), b"value1".to_vec());
            values.insert(b"key2".to_vec(), b"value2".to_vec());
            values.insert(b"key3".to_vec(), vec![0u8; 1024]);
        }

        let bytes = p.snapshot().unwrap();
        let p2 = KaedeMlsProvider::from_snapshot(&bytes).unwrap();

        let v = p2.storage.values.read().unwrap();
        assert_eq!(v.len(), 3);
        assert_eq!(v.get(b"key1".as_slice()), Some(&b"value1".to_vec()));
        assert_eq!(v.get(b"key3".as_slice()).map(|v| v.len()), Some(1024));
    }

    #[test]
    fn aes_gcm_snapshot_roundtrip() {
        let p = KaedeMlsProvider::new();
        {
            let mut values = p.storage.values.write().unwrap();
            values.insert(b"key1".to_vec(), b"value1".to_vec());
        }
        let cipher = crate::AesGcmCipher::from_bytes([0x77; 32]);
        let bytes = p.snapshot_with(&cipher).unwrap();
        // 头部：0x02 + alg(0x01) + u32 len
        assert_eq!(bytes[0], 0x02);
        assert_eq!(bytes[1], crate::ALG_AES_GCM);

        // 用同一 cipher 恢复
        let same = crate::AesGcmCipher::from_bytes([0x77; 32]);
        let p2 = KaedeMlsProvider::from_snapshot_with(&bytes, Some(&same)).unwrap();
        let v = p2.storage.values.read().unwrap();
        assert_eq!(v.get(b"key1".as_slice()), Some(&b"value1".to_vec()));
    }

    #[test]
    fn aes_gcm_snapshot_wrong_key_rejected() {
        let p = KaedeMlsProvider::new();
        let cipher = crate::AesGcmCipher::from_bytes([0x11; 32]);
        let bytes = p.snapshot_with(&cipher).unwrap();

        let wrong = crate::AesGcmCipher::from_bytes([0x99; 32]);
        let r = KaedeMlsProvider::from_snapshot_with(&bytes, Some(&wrong));
        assert!(r.is_err());
    }

    #[test]
    fn aes_gcm_snapshot_without_loader_rejected() {
        let p = KaedeMlsProvider::new();
        let cipher = crate::AesGcmCipher::from_bytes([0x22; 32]);
        let bytes = p.snapshot_with(&cipher).unwrap();
        // 没有 key_loader → 应该报错
        let r = KaedeMlsProvider::from_snapshot_with(&bytes, None);
        assert!(r.is_err());
    }

    #[test]
    fn noop_snapshot_uses_new_header() {
        let p = KaedeMlsProvider::new();
        let bytes = p.snapshot_with(&crate::NoOpCipher).unwrap();
        assert_eq!(bytes[0], 0x02);
        assert_eq!(bytes[1], crate::ALG_NOOP);
        // NoOp 也能走 from_snapshot_with(None) 恢复
        let p2 = KaedeMlsProvider::from_snapshot_with(&bytes, None).unwrap();
        assert_eq!(p2.snapshot_with(&crate::NoOpCipher).unwrap(), bytes);
    }

    #[test]
    fn reject_oversize_snapshot_bytes() {
        let too_big = vec![0u8; MAX_SNAPSHOT_BYTES + 1];
        assert!(matches!(
            KaedeMlsProvider::from_snapshot(&too_big),
            Err(ProviderError::Deserialize(_))
        ));
    }

    #[test]
    fn reject_too_many_entries() {
        let mut inner = Vec::new();
        inner.push(INNER_VERSION);
        inner.extend_from_slice(&(MAX_ENTRIES as u64 + 1).to_be_bytes());

        let mut buf = Vec::new();
        buf.push(0x02);
        buf.push(0x00);
        buf.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        buf.extend_from_slice(&inner);

        assert!(matches!(
            KaedeMlsProvider::from_snapshot(&buf),
            Err(ProviderError::Deserialize(_))
        ));
    }

    #[test]
    fn reject_oversize_value() {
        let mut inner = Vec::new();
        inner.push(INNER_VERSION);
        inner.extend_from_slice(&1u64.to_be_bytes());
        inner.extend_from_slice(&4u64.to_be_bytes());
        inner.extend_from_slice(&(MAX_VALUE_BYTES as u64 + 1).to_be_bytes());
        inner.extend_from_slice(b"key1");

        let mut buf = Vec::new();
        buf.push(0x02);
        buf.push(0x00);
        buf.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        buf.extend_from_slice(&inner);

        assert!(matches!(
            KaedeMlsProvider::from_snapshot(&buf),
            Err(ProviderError::Deserialize(_))
        ));
    }

    #[test]
    fn corrupt_snapshot_rejected() {
        let r = KaedeMlsProvider::from_snapshot(&[0u8, 1, 2]); // 太短
        assert!(r.is_err());
    }
}
