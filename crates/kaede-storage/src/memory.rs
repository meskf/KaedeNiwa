//! 进程内存实现。

use std::collections::HashMap;
use std::sync::RwLock;

use crate::StorageError;

#[derive(Default)]
pub struct MemoryStore {
    data: RwLock<HashMap<(String, String), Vec<u8>>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn put(&self, ns: &str, key: &str, value: &[u8]) -> Result<(), StorageError> {
        self.data
            .write()
            .unwrap()
            .insert((ns.into(), key.into()), value.to_vec());
        Ok(())
    }

    pub async fn get(&self, ns: &str, key: &str) -> Result<Option<Vec<u8>>, StorageError> {
        Ok(self
            .data
            .read()
            .unwrap()
            .get(&(ns.into(), key.into()))
            .cloned())
    }

    pub async fn delete(&self, ns: &str, key: &str) -> Result<(), StorageError> {
        self.data.write().unwrap().remove(&(ns.into(), key.into()));
        Ok(())
    }

    pub async fn list_keys(&self, ns: &str) -> Result<Vec<String>, StorageError> {
        Ok(self
            .data
            .read()
            .unwrap()
            .keys()
            .filter(|(n, _)| n == ns)
            .map(|(_, k)| k.clone())
            .collect())
    }

    pub async fn clear(&self, ns: &str) -> Result<(), StorageError> {
        self.data.write().unwrap().retain(|k, _| k.0.as_str() != ns);
        Ok(())
    }
}
