//! SQLite 初始化 + 工具函数。

use anyhow::Result;
use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};

pub async fn init(url: &str) -> Result<SqlitePool> {
    let pool = SqlitePoolOptions::new()
        .max_connections(10)
        .connect(url)
        .await?;

    sqlx::query("PRAGMA journal_mode = WAL;")
        .execute(&pool)
        .await?;
    sqlx::query("PRAGMA foreign_keys = ON;")
        .execute(&pool)
        .await?;
    sqlx::query("PRAGMA synchronous = NORMAL;")
        .execute(&pool)
        .await?;

    // Schema 由 migrations/ 目录管理。
    // 首次运行会把 20261015000001_init 记为已应用；
    // 老 DB（无 _sqlx_migrations 表）也能跑，因为 SQL 里用的是
    // CREATE TABLE IF NOT EXISTS，不会重复建表。
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

pub fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

pub fn random_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64_url(&bytes)
}

pub fn base64_url(bytes: &[u8]) -> String {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    URL_SAFE_NO_PAD.encode(bytes)
}

// ===========================================================================
// 离线 MLS 信封
// ===========================================================================

/// 每个用户最多暂存多少条信封（防内存/DB 膨胀）。
const MAX_PENDING_PER_USER: i64 = 100;

/// 单条信封上限。Welcome/Commit 正常只有几十 KB；上限防止
/// 任意已 Join 用户用 16 MiB 帧把离线队列撑爆。
const MAX_ENVELOPE_BYTES: usize = 256 * 1024;

/// 每个用户的信封总字节上限。
const MAX_PENDING_BYTES: i64 = 4 * 1024 * 1024;

/// 暂存一条离线信封。`kind` = `"mls_welcome"` | `"channel_sync"`。
pub async fn enqueue_envelope(
    pool: &SqlitePool,
    to_uid: &str,
    from_uid: &str,
    kind: &str,
    channel_id: Option<&str>,
    payload: &[u8],
) -> Result<(), sqlx::Error> {
    if payload.len() > MAX_ENVELOPE_BYTES {
        return Err(sqlx::Error::Protocol(format!(
            "envelope too large: {} > {}",
            payload.len(),
            MAX_ENVELOPE_BYTES
        )));
    }
    sqlx::query(
        "INSERT INTO pending_envelopes (to_uid, from_uid, kind, channel_id, payload, created_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(to_uid)
    .bind(from_uid)
    .bind(kind)
    .bind(channel_id)
    .bind(payload)
    .bind(now_ms())
    .execute(pool)
    .await?;

    // 按条数 + 总字节双重配额删最旧的信封。
    let rows: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT id, length(payload) FROM pending_envelopes WHERE to_uid = ? ORDER BY id DESC",
    )
    .bind(to_uid)
    .fetch_all(pool)
    .await?;
    let mut total: i64 = 0;
    for (idx, (id, len)) in rows.into_iter().enumerate() {
        total += len;
        if idx as i64 >= MAX_PENDING_PER_USER || total > MAX_PENDING_BYTES {
            sqlx::query("DELETE FROM pending_envelopes WHERE id = ?")
                .bind(id)
                .execute(pool)
                .await?;
        }
    }

    Ok(())
}

/// 读取信封但**不删除**（投递成功后再删）。
pub async fn list_envelopes(
    pool: &SqlitePool,
    to_uid: &str,
) -> Result<Vec<(i64, String, String, Option<String>, Vec<u8>)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, from_uid, kind, channel_id, payload FROM pending_envelopes \
         WHERE to_uid = ? ORDER BY id ASC LIMIT ?",
    )
    .bind(to_uid)
    .bind(MAX_PENDING_PER_USER)
    .fetch_all(pool)
    .await
}

/// 删除已成功投递的信封。
pub async fn delete_envelopes(pool: &SqlitePool, ids: &[i64]) -> Result<(), sqlx::Error> {
    for id in ids {
        sqlx::query("DELETE FROM pending_envelopes WHERE id = ?")
            .bind(id)
            .execute(pool)
            .await?;
    }
    Ok(())
}

/// 离线信封的 DB 行：(id, from_uid, kind, channel_id, payload)。
type PendingRow = (i64, String, String, Option<String>, Vec<u8>);

/// 取出并按入队顺序删除某个用户的所有离线信封。
///
/// 返回 `(from_uid, kind, channel_id, payload)`。
pub async fn take_envelopes(
    pool: &SqlitePool,
    to_uid: &str,
) -> Result<Vec<(String, String, Option<String>, Vec<u8>)>, sqlx::Error> {
    let mut tx = pool.begin().await?;

    let rows: Vec<PendingRow> = sqlx::query_as(
        "SELECT id, from_uid, kind, channel_id, payload FROM pending_envelopes \
         WHERE to_uid = ? ORDER BY id ASC LIMIT ?",
    )
    .bind(to_uid)
    .bind(MAX_PENDING_PER_USER)
    .fetch_all(&mut *tx)
    .await?;

    if rows.is_empty() {
        tx.commit().await?;
        return Ok(Vec::new());
    }

    for (id, ..) in &rows {
        sqlx::query("DELETE FROM pending_envelopes WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;

    Ok(rows
        .into_iter()
        .map(|(_, from, kind, channel_id, payload)| (from, kind, channel_id, payload))
        .collect())
}

/// 清理过期数据（session / KeyPackage / 离线信封）。
pub async fn cleanup_expired(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let now = now_ms();
    sqlx::query("DELETE FROM sessions WHERE expires_at < ?")
        .bind(now)
        .execute(pool)
        .await?;

    // KeyPackage 30 天未使用视为过期。
    sqlx::query("DELETE FROM key_packages WHERE created_at < ?")
        .bind(now - 30 * 24 * 3600 * 1000)
        .execute(pool)
        .await?;

    // 离线信封 7 天过期。
    sqlx::query("DELETE FROM pending_envelopes WHERE created_at < ?")
        .bind(now - 7 * 24 * 3600 * 1000)
        .execute(pool)
        .await?;

    Ok(())
}

/// 把 token 变成不可逆的存储值（DB 泄漏时不直接暴露可用 token）。
pub fn hash_token(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let h = Sha256::digest(token.as_bytes());
    base64_url(&h)
}
