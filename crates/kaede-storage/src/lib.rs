//! KaedeNiwa 存储抽象。
//!
//! 三种后端：
//! - [`MemoryStore`]：进程内存（测试）
//! - [`SqliteStore`]：native（Desktop / Server）
//! - [`IndexedDbStore`]：WASM（Web）
//!
//! 统一入口是 [`Store`] enum。不用 trait object 的原因是：
//! WASM 端的 `rexie::Rexie` 不是 `Send`，trait 里加 `Send` 会失败，
//! 不加 `Send` 又会让 native 端无法 `tokio::spawn`。enum 分发没有
//! 这个问题。
//!
//! # Namespace 约定
//!
//! - `"mls"`       MLS 群组状态
//! - `"mls_kp"`    本地 KeyPackage 队列
//! - `"identity"`  身份密钥
//! - `"messages"`  消息历史
//! - `"meta"`      应用元数据

#![forbid(unsafe_code)]

use thiserror::Error;

mod memory;
pub use memory::MemoryStore;

#[cfg(feature = "native")]
mod sqlite;
#[cfg(feature = "native")]
pub use sqlite::SqliteStore;

#[cfg(feature = "wasm")]
mod idb;
#[cfg(feature = "wasm")]
pub use idb::IndexedDbStore;

/// 存储错误。
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("backend error: {0}")]
    Backend(String),
    #[error("serialization error: {0}")]
    Serialize(String),
    #[error("not found: {0}")]
    NotFound(String),
}

/// 统一存储句柄。
pub enum Store {
    Memory(MemoryStore),
    #[cfg(feature = "native")]
    Sqlite(SqliteStore),
    #[cfg(feature = "wasm")]
    IndexedDb(IndexedDbStore),
}

impl Store {
    /// 进程内存 store。测试用。
    pub fn memory() -> Self {
        Store::Memory(MemoryStore::new())
    }

    pub async fn put(&self, ns: &str, key: &str, value: &[u8]) -> Result<(), StorageError> {
        match self {
            Store::Memory(s) => s.put(ns, key, value).await,
            #[cfg(feature = "native")]
            Store::Sqlite(s) => s.put(ns, key, value).await,
            #[cfg(feature = "wasm")]
            Store::IndexedDb(s) => s.put(ns, key, value).await,
        }
    }

    pub async fn get(&self, ns: &str, key: &str) -> Result<Option<Vec<u8>>, StorageError> {
        match self {
            Store::Memory(s) => s.get(ns, key).await,
            #[cfg(feature = "native")]
            Store::Sqlite(s) => s.get(ns, key).await,
            #[cfg(feature = "wasm")]
            Store::IndexedDb(s) => s.get(ns, key).await,
        }
    }

    pub async fn delete(&self, ns: &str, key: &str) -> Result<(), StorageError> {
        match self {
            Store::Memory(s) => s.delete(ns, key).await,
            #[cfg(feature = "native")]
            Store::Sqlite(s) => s.delete(ns, key).await,
            #[cfg(feature = "wasm")]
            Store::IndexedDb(s) => s.delete(ns, key).await,
        }
    }

    pub async fn list_keys(&self, ns: &str) -> Result<Vec<String>, StorageError> {
        match self {
            Store::Memory(s) => s.list_keys(ns).await,
            #[cfg(feature = "native")]
            Store::Sqlite(s) => s.list_keys(ns).await,
            #[cfg(feature = "wasm")]
            Store::IndexedDb(s) => s.list_keys(ns).await,
        }
    }

    pub async fn clear(&self, ns: &str) -> Result<(), StorageError> {
        match self {
            Store::Memory(s) => s.clear(ns).await,
            #[cfg(feature = "native")]
            Store::Sqlite(s) => s.clear(ns).await,
            #[cfg(feature = "wasm")]
            Store::IndexedDb(s) => s.clear(ns).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn memory_store_crud() {
        let s = Store::memory();
        s.put("ns", "k", b"v").await.unwrap();
        assert_eq!(s.get("ns", "k").await.unwrap(), Some(b"v".to_vec()));
        assert_eq!(s.get("ns", "x").await.unwrap(), None);
        s.delete("ns", "k").await.unwrap();
        assert_eq!(s.get("ns", "k").await.unwrap(), None);
    }

    #[tokio::test]
    async fn memory_namespaces_isolated() {
        let s = Store::memory();
        s.put("a", "k", b"1").await.unwrap();
        s.put("b", "k", b"2").await.unwrap();
        assert_eq!(s.get("a", "k").await.unwrap(), Some(b"1".to_vec()));
        assert_eq!(s.get("b", "k").await.unwrap(), Some(b"2".to_vec()));
    }

    #[tokio::test]
    async fn memory_list_and_clear() {
        let s = Store::memory();
        s.put("ns", "a", b"1").await.unwrap();
        s.put("ns", "b", b"2").await.unwrap();
        s.put("other", "c", b"3").await.unwrap();
        let mut keys = s.list_keys("ns").await.unwrap();
        keys.sort();
        assert_eq!(keys, vec!["a", "b"]);
        s.clear("ns").await.unwrap();
        assert_eq!(s.list_keys("ns").await.unwrap().len(), 0);
        assert_eq!(s.list_keys("other").await.unwrap().len(), 1);
    }
}
