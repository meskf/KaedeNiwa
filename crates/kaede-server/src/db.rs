//! SQLite 初始化 + 工具函数。

use anyhow::Result;
use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};

pub async fn init(url: &str) -> Result<SqlitePool> {
    let pool = SqlitePoolOptions::new()
        .after_connect(|conn, _meta| {
            Box::pin(async move {
                // &mut *conn 是 reborrow：直接把 &mut SqliteConnection
                // 传给 execute 会 move（非 Copy），第二次调用无法使用。
                sqlx::query("PRAGMA foreign_keys = ON;")
                    .execute(&mut *conn)
                    .await?;
                // SQLITE_BUSY 等待时间：SQLite 默认立即返回 BUSY。
                // 5000ms 与主流框架一致，避免高并发写时频繁报
                // "database is locked"。
                sqlx::query("PRAGMA busy_timeout = 5000;")
                    .execute(&mut *conn)
                    .await?;
                // synchronous 是**连接级**设置，必须放在 after_connect；
                // 只在池上执行一次的话，其余连接会保持默认 FULL。
                sqlx::query("PRAGMA synchronous = NORMAL;")
                    .execute(&mut *conn)
                    .await?;
                Ok(())
            })
        })
        // SQLite 写是单锁串行。10 个连接不会提升写吞吐，只会增加
        // BUSY 概率和锁升级开销。4 个足够覆盖并发读 + 少量写。
        .max_connections(4)
        // 池耗尽时不要无限 await：5s 内拿不到连接直接报 PoolTimedOut，
        // 让上层能返回 503 而不是卡死 handler。
        .acquire_timeout(std::time::Duration::from_secs(5))
        .idle_timeout(std::time::Duration::from_secs(300))
        .connect(url)
        .await?;

    sqlx::query("PRAGMA journal_mode = WAL;")
        .execute(&pool)
        .await?;
    sqlx::query("PRAGMA foreign_keys = ON;")
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
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
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
    // 单事务完成 INSERT + 双重配额裁剪：
    //   1. 按条数保留最新 MAX_PENDING_PER_USER 条
    //   2. 按字节总量从最旧开始删，直到 <= MAX_PENDING_BYTES
    let mut tx = pool.begin().await?;
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
    .execute(&mut *tx)
    .await?;

    // 按条数裁剪：保留最新 N 条
    sqlx::query(
        "DELETE FROM pending_envelopes \
         WHERE to_uid = ? AND id NOT IN ( \
           SELECT id FROM pending_envelopes \
           WHERE to_uid = ? ORDER BY id DESC LIMIT ? \
         )",
    )
    .bind(to_uid)
    .bind(to_uid)
    .bind(MAX_PENDING_PER_USER)
    .execute(&mut *tx)
    .await?;

    // 按字节裁剪
    sqlx::query(
        "DELETE FROM pending_envelopes WHERE id IN ( \
           SELECT id FROM ( \
             SELECT id, \
                    SUM(length(payload)) OVER (ORDER BY id DESC) AS cum \
             FROM pending_envelopes WHERE to_uid = ? \
           ) WHERE cum > ? \
         )",
    )
    .bind(to_uid)
    .bind(MAX_PENDING_BYTES)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

/// 读取信封但不删除（投递成功后再删）。
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

/// 清理过期数据（session / KeyPackage / 离线信封 / Commit 历史 / 审计 / 孤儿附件）。
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

    // mls_commits 保留最近 1024 条。
    // 按 rowid 保留而非 epoch 值——epoch 受客户端控制，按值排序会被
    // epoch 注入清空。
    sqlx::query(
        "DELETE FROM mls_commits WHERE rowid NOT IN \
         (SELECT rowid FROM mls_commits ORDER BY rowid DESC LIMIT 1024)",
    )
    .execute(pool)
    .await?;

    // admin_audit 保留最近 180 天。
    sqlx::query("DELETE FROM admin_audit WHERE ts < ?")
        .bind(now - 180i64 * 24 * 3600 * 1000)
        .execute(pool)
        .await?;

    // P1-13：清理孤儿 attachments 行。
    //
    // 两种孤儿来源：
    //   1. 文件写成功但进程崩溃在 INSERT 之前（正常路径有回滚，崩溃路径无）
    //   2. 频道删除时 purge_target 删 DB 但删文件时文件已不存在/权限错误
    //
    // 保守策略：只删"7 天前"且 target_id 指向不存在的频道（不含 DM）。
    // 不按文件系统状态判断——文件可能在冷存储/迁移盘上，误删代价大。
    sqlx::query(
        "DELETE FROM attachments \
         WHERE created_at < ? \
           AND target_id IS NOT NULL \
           AND target_id NOT IN (SELECT id FROM nodes) \
           AND target_id NOT LIKE 'dm:%'",
    )
    .bind(now - 7 * 24 * 3600 * 1000)
    .execute(pool)
    .await?;

    Ok(())
}

/// 单个账号的消息密文累计上限（终身）。
pub const MAX_USER_MESSAGE_BYTES: i64 = 512 * 1024 * 1024;

/// 单次历史拉取返回的**密文总字节**上限。
///
/// 单个请求最多 `limit=500` 条、单条密文 4 MiB，若不设总量上限，
/// 一次 `fetch_all` 可把约 2 GiB 密文读进内存再 base64 放大。
/// 8 MiB 足够覆盖正常的一页历史，同时把最坏内存占用压到可控范围。
pub(crate) const MAX_HISTORY_RESPONSE_BYTES: i64 = 8 * 1024 * 1024;

/// 在事务内预占消息字节配额。返回 false 表示超额。
pub async fn reserve_message_bytes(
    conn: &mut sqlx::SqliteConnection,
    login_id: &str,
    bytes: i64,
) -> Result<bool, sqlx::Error> {
    // 单条 UPSERT 完成"检查 + 预占"，避免 SELECT 后再 INSERT 的锁升级
    // 死锁（SQLite 并发写事务常见问题）。超额时 WHERE 不成立，无行返回。
    let row: Option<(i64,)> = sqlx::query_as(
        "INSERT INTO message_usage (login_id, bytes) VALUES (?, ?) \
         ON CONFLICT(login_id) DO UPDATE SET bytes = message_usage.bytes + excluded.bytes \
         WHERE message_usage.bytes + excluded.bytes <= ? \
         RETURNING bytes",
    )
    .bind(login_id)
    .bind(bytes)
    .bind(MAX_USER_MESSAGE_BYTES)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.is_some())
}

/// 把 token 变成不可逆的存储值（DB 泄漏时不直接暴露可用 token）。
pub fn hash_token(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let h = Sha256::digest(token.as_bytes());
    base64_url(&h)
}


pub async fn init_nodes_table(pool: &sqlx::SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS nodes (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            parent_id TEXT,
            kind TEXT NOT NULL,
            position INTEGER NOT NULL DEFAULT 0,
            is_private INTEGER NOT NULL DEFAULT 0,
            password_hash TEXT,
            hash_tier TEXT,
            created_at INTEGER NOT NULL
        );"
    ).execute(pool).await?;
    
    // 【平滑迁移】把旧 channels 表的数据同步到 nodes 表（幂等）
    let _ = sqlx::query(
        "INSERT OR IGNORE INTO nodes (id, name, parent_id, kind, position, is_private, password_hash, hash_tier, created_at)
         SELECT id, name, category_id, kind, position, is_private, password_hash, hash_tier, created_at FROM channels"
    ).execute(pool).await;

    Ok(())
}
