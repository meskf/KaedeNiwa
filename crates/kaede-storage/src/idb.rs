//! IndexedDB 后端（Web / WASM）。
//!
//! 用 rexie 0.6 的最小 API。所有 key 用 `{ns}::{key}` 拼接作为主键，
//! 通过 `by_ns` 索引支持 `list_keys`。

use rexie::{Index, KeyRange, ObjectStore, Rexie, TransactionMode};
use wasm_bindgen::JsValue;

use crate::StorageError;

pub struct IndexedDbStore {
    db: Rexie,
}

impl IndexedDbStore {
    pub async fn open(name: &str) -> Result<Self, StorageError> {
        let rexie = Rexie::builder(name)
            .version(1)
            .add_object_store(
                ObjectStore::new("kv")
                    .key_path("id")
                    .add_index(Index::new("by_ns", "ns")),
            )
            .build()
            .await
            .map_err(|e| StorageError::Backend(format!("{:?}", e)))?;
        Ok(Self { db: rexie })
    }

    /// P1-11：用长度前缀编码，避免 `a::b`+`c` 与 `a`+`b::c` 碰撞。
    /// 格式：`{ns_len}:{ns}:{key}`
    fn composite_id(ns: &str, key: &str) -> String {
        format!("{}:{}:{}", ns.len(), ns, key)
    }

    /// 反向解析 composite_id。失败返回 None。
    fn split_id(id: &str) -> Option<(&str, &str)> {
        let colon = id.find(':')?;
        let len: usize = id[..colon].parse().ok()?;
        let start = colon + 1;
        let end = start.checked_add(len)?;
        if end > id.len() || id.as_bytes().get(end) != Some(&b':') {
            return None;
        }
        Some((&id[start..end], &id[end + 1..]))
    }

    fn make_record(ns: &str, key: &str, value: &[u8]) -> Result<JsValue, StorageError> {
        let obj = js_sys::Object::new();
        js_sys::Reflect::set(
            &obj,
            &JsValue::from_str("id"),
            &JsValue::from_str(&Self::composite_id(ns, key)),
        )
        .map_err(|e| StorageError::Backend(format!("{:?}", e)))?;
        js_sys::Reflect::set(&obj, &JsValue::from_str("ns"), &JsValue::from_str(ns))
            .map_err(|e| StorageError::Backend(format!("{:?}", e)))?;
        let arr = js_sys::Uint8Array::from(value);
        js_sys::Reflect::set(&obj, &JsValue::from_str("v"), &arr)
            .map_err(|e| StorageError::Backend(format!("{:?}", e)))?;
        Ok(obj.into())
    }

    fn extract_value(record: &JsValue) -> Result<Vec<u8>, StorageError> {
        let v = js_sys::Reflect::get(record, &JsValue::from_str("v"))
            .map_err(|e| StorageError::Backend(format!("{:?}", e)))?;
        let arr = js_sys::Uint8Array::new(&v);
        Ok(arr.to_vec())
    }

    pub async fn put(&self, ns: &str, key: &str, value: &[u8]) -> Result<(), StorageError> {
        let record = Self::make_record(ns, key, value)?;
        let tx = self
            .db
            .transaction(&["kv"], TransactionMode::ReadWrite)
            .map_err(|e| StorageError::Backend(format!("{:?}", e)))?;
        let store = tx
            .store("kv")
            .map_err(|e| StorageError::Backend(format!("{:?}", e)))?;
        store
            .put(&record, None)
            .await
            .map_err(|e| StorageError::Backend(format!("{:?}", e)))?;
        tx.done()
            .await
            .map_err(|e| StorageError::Backend(format!("{:?}", e)))?;
        Ok(())
    }

    pub async fn get(&self, ns: &str, key: &str) -> Result<Option<Vec<u8>>, StorageError> {
        let tx = self
            .db
            .transaction(&["kv"], TransactionMode::ReadOnly)
            .map_err(|e| StorageError::Backend(format!("{:?}", e)))?;
        let store = tx
            .store("kv")
            .map_err(|e| StorageError::Backend(format!("{:?}", e)))?;
        let record = store
            .get(JsValue::from_str(&Self::composite_id(ns, key)))
            .await
            .map_err(|e| StorageError::Backend(format!("{:?}", e)))?;
        tx.done()
            .await
            .map_err(|e| StorageError::Backend(format!("{:?}", e)))?;

        match record {
            Some(r) if !r.is_undefined() && !r.is_null() => Ok(Some(Self::extract_value(&r)?)),
            _ => Ok(None),
        }
    }

    pub async fn delete(&self, ns: &str, key: &str) -> Result<(), StorageError> {
        let tx = self
            .db
            .transaction(&["kv"], TransactionMode::ReadWrite)
            .map_err(|e| StorageError::Backend(format!("{:?}", e)))?;
        let store = tx
            .store("kv")
            .map_err(|e| StorageError::Backend(format!("{:?}", e)))?;
        store
            .delete(JsValue::from_str(&Self::composite_id(ns, key)))
            .await
            .map_err(|e| StorageError::Backend(format!("{:?}", e)))?;
        tx.done()
            .await
            .map_err(|e| StorageError::Backend(format!("{:?}", e)))?;
        Ok(())
    }

    pub async fn list_keys(&self, ns: &str) -> Result<Vec<String>, StorageError> {
        let tx = self
            .db
            .transaction(&["kv"], TransactionMode::ReadOnly)
            .map_err(|e| StorageError::Backend(format!("{:?}", e)))?;
        let store = tx
            .store("kv")
            .map_err(|e| StorageError::Backend(format!("{:?}", e)))?;
        let index = store
            .index("by_ns")
            .map_err(|e| StorageError::Backend(format!("{:?}", e)))?;
        let records = index
            .get_all(
                Some(
                    KeyRange::only(&JsValue::from_str(ns))
                        .map_err(|e| StorageError::Backend(format!("{:?}", e)))?,
                ),
                None,
            )
            .await
            .map_err(|e| StorageError::Backend(format!("{:?}", e)))?;
        tx.done()
            .await
            .map_err(|e| StorageError::Backend(format!("{:?}", e)))?;

        let mut out = Vec::with_capacity(records.len());
        for r in records {
            let id = js_sys::Reflect::get(&r, &JsValue::from_str("id"))
                .map_err(|e| StorageError::Backend(format!("{:?}", e)))?;
            if let Some(s) = id.as_string()
                && let Some((parsed_ns, k)) = Self::split_id(&s)
                && parsed_ns == ns
            {
                out.push(k.to_string());
            }
        }
        Ok(out)
    }

    /// 同一事务内删除整个 namespace（原子；不会留下半清空状态）。
    pub async fn clear(&self, ns: &str) -> Result<(), StorageError> {
        let keys = self.list_keys(ns).await?;
        if keys.is_empty() {
            return Ok(());
        }
        let tx = self
            .db
            .transaction(&["kv"], TransactionMode::ReadWrite)
            .map_err(|e| StorageError::Backend(format!("{:?}", e)))?;
        let store = tx
            .store("kv")
            .map_err(|e| StorageError::Backend(format!("{:?}", e)))?;
        for k in keys {
            store
                .delete(JsValue::from_str(&Self::composite_id(ns, &k)))
                .await
                .map_err(|e| StorageError::Backend(format!("{:?}", e)))?;
        }
        tx.done()
            .await
            .map_err(|e| StorageError::Backend(format!("{:?}", e)))?;
        Ok(())
    }
}
