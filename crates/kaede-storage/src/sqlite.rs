//! SQLite 后端（Desktop / Server）。

use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};

use crate::StorageError;

pub struct SqliteStore {
    pool: SqlitePool,
}

impl SqliteStore {
    /// 打开或创建 SQLite 数据库。
    ///
    /// URL 格式：
    /// - `sqlite::memory:`（测试）
    /// - `sqlite://path/to/file.db?mode=rwc`
    pub async fn connect(url: &str) -> Result<Self, StorageError> {
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(url)
            .await
            .map_err(|e| StorageError::Backend(e.to_string()))?;
        Self::init_schema(&pool).await?;
        Ok(Self { pool })
    }

    pub async fn in_memory() -> Result<Self, StorageError> {
        // P1-S1（7b 审查）：SQLite `:memory:` 每个连接是独立数据库。
        // 默认 max_connections(5) 会导致 5 个隔离的 kv 表 → 读写不一致。
        // 测试场景必须单连接。
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .map_err(|e| StorageError::Backend(e.to_string()))?;
        Self::init_schema(&pool).await?;
        Ok(Self { pool })
    }

    async fn init_schema(pool: &SqlitePool) -> Result<(), StorageError> {
        // P0-9: sqlx 多语句非稳定契约，拆两条独立执行。
        // kaede-storage 刻意不依赖 tracing（基础层轻量）。WAL 是性能优化，
        // 失败回退到 DELETE journal mode 不影响正确性；用 stderr 留痕。
        if let Err(e) = sqlx::query("PRAGMA journal_mode = WAL").execute(pool).await {
            eprintln!(
                "kaede-storage: PRAGMA journal_mode = WAL failed (non-fatal): {}",
                e
            );
        }
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS kv (
                ns TEXT NOT NULL,
                k  TEXT NOT NULL,
                v  BLOB NOT NULL,
                PRIMARY KEY (ns, k)
            )",
        )
        .execute(pool)
        .await
        .map_err(|e| StorageError::Backend(e.to_string()))?;
        Ok(())
    }

    pub async fn put(&self, ns: &str, key: &str, value: &[u8]) -> Result<(), StorageError> {
        sqlx::query("INSERT OR REPLACE INTO kv (ns, k, v) VALUES (?1, ?2, ?3)")
            .bind(ns)
            .bind(key)
            .bind(value)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Backend(e.to_string()))?;
        Ok(())
    }

    pub async fn get(&self, ns: &str, key: &str) -> Result<Option<Vec<u8>>, StorageError> {
        let row: Option<(Vec<u8>,)> = sqlx::query_as("SELECT v FROM kv WHERE ns = ?1 AND k = ?2")
            .bind(ns)
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StorageError::Backend(e.to_string()))?;
        Ok(row.map(|(v,)| v))
    }

    pub async fn delete(&self, ns: &str, key: &str) -> Result<(), StorageError> {
        sqlx::query("DELETE FROM kv WHERE ns = ?1 AND k = ?2")
            .bind(ns)
            .bind(key)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Backend(e.to_string()))?;
        Ok(())
    }

    pub async fn list_keys(&self, ns: &str) -> Result<Vec<String>, StorageError> {
        let rows: Vec<(String,)> = sqlx::query_as("SELECT k FROM kv WHERE ns = ?1")
            .bind(ns)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StorageError::Backend(e.to_string()))?;
        Ok(rows.into_iter().map(|(k,)| k).collect())
    }

    pub async fn clear(&self, ns: &str) -> Result<(), StorageError> {
        sqlx::query("DELETE FROM kv WHERE ns = ?1")
            .bind(ns)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Backend(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sqlite_crud() {
        let s = SqliteStore::in_memory().await.unwrap();
        s.put("ns", "k", b"v").await.unwrap();
        assert_eq!(s.get("ns", "k").await.unwrap(), Some(b"v".to_vec()));
        s.delete("ns", "k").await.unwrap();
        assert_eq!(s.get("ns", "k").await.unwrap(), None);
    }

    #[tokio::test]
    async fn sqlite_persists_after_reconnect() {
        let dir = std::env::temp_dir().join(format!(
            "kaede_test_{}_{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let url = format!("sqlite://{}?mode=rwc", dir.display());

        {
            let s = SqliteStore::connect(&url).await.unwrap();
            s.put("ns", "k", b"persisted").await.unwrap();
        }

        {
            let s = SqliteStore::connect(&url).await.unwrap();
            assert_eq!(s.get("ns", "k").await.unwrap(), Some(b"persisted".to_vec()));
        }

        let _ = std::fs::remove_file(&dir);
    }
}
