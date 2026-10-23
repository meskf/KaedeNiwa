//! 附件上传 / 下载。
//!
//! 存储：`<data_dir>/attachments/<id>`。
//! **服务端只存密文**：附件由客户端用 MLS exporter 派生的 File key 加密
//! （`0x04` 包装），服务端既看不到内容也看不到文件名/MIME。
//!
//! **单文件上限**：32 MiB（8K60 视频帧不走这里，走媒体流）；
//! 全局 5 GiB / 单用户 512 MiB 配额由 `attachments` 表在事务内预占。

use std::path::PathBuf;

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use kaede_protocol::Attachment;

use crate::db;
use crate::error::ApiError;
use crate::http::authenticate;
use crate::state::AppState;

pub const MAX_ATTACHMENT_BYTES: usize = 32 * 1024 * 1024;

/// 附件目录总容量上限，防止任意登录用户持续上传把磁盘写满。
const MAX_TOTAL_ATTACHMENT_BYTES: i64 = 5 * 1024 * 1024 * 1024;

/// 单用户附件总容量上限（含所有频道/DM）。
const MAX_USER_ATTACHMENT_BYTES: i64 = 512 * 1024 * 1024;

/// 孤儿清理的宽限期：早于该时长的孤儿才会被回收，避免删掉正在上传的文件。
const ORPHAN_GRACE: std::time::Duration = std::time::Duration::from_secs(24 * 3600);

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/attachments", post(upload))
        .route("/api/attachments/{id}", get(download))
        .layer(axum::extract::DefaultBodyLimit::max(MAX_ATTACHMENT_BYTES))
        .with_state(state)
}

#[derive(Deserialize)]
struct UploadQuery {
    /// 上传上下文（频道 id / DM conv_id）。只用于配额、授权与删除回收，
    /// 不参与加密。文件名/MIME 不再上传——它们属于消息明文，
    /// 由客户端放在 E2EE payload 内，避免通过 query 进入代理访问日志。
    #[serde(default)]
    target: Option<String>,
}

#[derive(Serialize)]
struct UploadResp {
    attachment: Attachment,
}

/// 附件存储目录。
///
/// 优先 `KAEDE_DATA_DIR` 环境变量；未设置时回退到 `<cwd>/data/attachments`。
/// 服务器部署时应显式设置，避免工作目录变化导致路径漂移。
fn attachments_dir() -> PathBuf {
    let base = match std::env::var("KAEDE_DATA_DIR") {
        Ok(v) if !v.is_empty() => PathBuf::from(v),
        _ => {
            let mut p = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            p.push("data");
            p
        }
    };
    base.join("attachments")
}

async fn upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<UploadQuery>,
    body: Bytes,
) -> Result<(StatusCode, Json<UploadResp>), ApiError> {
    let uid = authenticate(&state, &headers).await?;

    if body.is_empty() {
        return Err(ApiError::BadRequest("empty body".into()));
    }

    // S-4: per-uid 限流，防附件上传 DoS（每秒 1 次，burst 10）
    if !state.check_rate_scoped(&uid, "upload", 1.0, 10.0) {
        return Err(ApiError::BadRequest("upload rate limited".into()));
    }
    if body.len() > MAX_ATTACHMENT_BYTES {
        return Err(ApiError::BadRequest(format!(
            "attachment too large (max {} bytes)",
            MAX_ATTACHMENT_BYTES
        )));
    }

    let target = sanitize_target(q.target.as_deref());
    // 上传时必须能证明自己有权写入该上下文；否则可以把附件挂到别人的
    // 私密频道/DM 下，或绕开后续下载授权。
    if let Some(t) = target.as_deref() {
        authorize_target(&state, uid.as_str(), t).await?;
    }

    // 每用户 + 全站配额：在同一个事务里"检查 + 预占"。
    //
    // 不能只 SELECT 再 INSERT（并发上传可同时通过检查），也不能每次上传
    // 去遍历整个附件目录做统计（O(文件数) 的 syscall，是上传路径上的热点）。
    // SQLite 写事务串行，因此事务内 SUM + INSERT 即可保证不超额。
    let dir = attachments_dir();
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| ApiError::Internal(format!("mkdir: {}", e)))?;

    // 随机 id：URL-safe 字母表，与 download 的 id 白名单一致。
    let id = format!("att_{}", &db::random_token()[..22]);

    let size = body.len() as i64;
    {
        let mut tx = state.pool.begin().await?;
        let (global_used,): (i64,) =
            sqlx::query_as("SELECT COALESCE(SUM(size), 0) FROM attachments")
                .fetch_one(&mut *tx)
                .await?;
        if global_used.saturating_add(size) > MAX_TOTAL_ATTACHMENT_BYTES {
            tx.rollback().await?;
            return Err(ApiError::BadRequest(
                "attachment storage quota exceeded".into(),
            ));
        }
        let (user_used,): (i64,) =
            sqlx::query_as("SELECT COALESCE(SUM(size), 0) FROM attachments WHERE owner = ?")
                .bind(uid.as_str())
                .fetch_one(&mut *tx)
                .await?;
        if user_used.saturating_add(size) > MAX_USER_ATTACHMENT_BYTES {
            tx.rollback().await?;
            return Err(ApiError::BadRequest(format!(
                "user attachment quota exceeded (max {} MiB)",
                MAX_USER_ATTACHMENT_BYTES / 1024 / 1024
            )));
        }
        sqlx::query(
            "INSERT INTO attachments (id, owner, target_id, size, created_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(uid.as_str())
        .bind(target.as_deref())
        .bind(size)
        .bind(db::now_ms())
        .execute(&mut *tx)
        .await?;
        // 先提交：配额已占，后续并发上传会看到这一行。
        tx.commit().await?;
    }

    // 配额占好之后写盘。失败必须回滚 DB 行，否则会永久占用配额。
    // P2-7: 先写 .tmp 再 rename——磁盘满/进程崩溃时不会留下半写文件。
    let final_path = dir.join(&id);
    let tmp_path = dir.join(format!(".{}.tmp", id));
    if let Err(e) = tokio::fs::write(&tmp_path, &body).await {
        let _ = sqlx::query("DELETE FROM attachments WHERE id = ?")
            .bind(&id)
            .execute(&state.pool)
            .await;
        return Err(ApiError::Internal(format!("write tmp: {}", e)));
    }
    if let Err(e) = tokio::fs::rename(&tmp_path, &final_path).await {
        let _ = tokio::fs::remove_file(&tmp_path).await;
        let _ = sqlx::query("DELETE FROM attachments WHERE id = ?")
            .bind(&id)
            .execute(&state.pool)
            .await;
        return Err(ApiError::Internal(format!("rename: {}", e)));
    }

    let att = Attachment {
        id: id.clone(),
        // 文件名/MIME 由客户端在 E2EE payload 中填写；服务端不接收、不返回。
        name: String::new(),
        mime: "application/octet-stream".into(),
        size: body.len() as u64,
        url: format!("/api/attachments/{}", id),
        thumbnail_url: None,
        sha256: None,
        thumbnail_sha256: None,
    };

    tracing::info!("uploaded attachment {} ({} bytes)", att.id, att.size);

    Ok((StatusCode::CREATED, Json(UploadResp { attachment: att })))
}

async fn download(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let uid = authenticate(&state, &headers).await?;
    // 下载同样消耗带宽：限制单账号的并发/频率，防反复拉取大文件。
    if !state.check_rate_scoped(&uid, "download", 5.0, 20.0) {
        return Err(ApiError::BadRequest(
            "attachment download rate limited".into(),
        ));
    }

    // H17: 严格 id 校验——白名单字符集，防路径穿越 / Windows ADS / 保留名
    if !is_valid_attachment_id(&id) {
        return Err(ApiError::BadRequest("invalid id".into()));
    }

    // 授权：附件必须存在元数据，且当前用户有权访问其频道/DM 上下文。
    // 仅“已登录 + 知道随机 id”不足以读取私密频道/DM 的密文。
    let meta: Option<(Option<String>, String)> =
        sqlx::query_as("SELECT target_id, owner FROM attachments WHERE id = ?")
            .bind(&id)
            .fetch_optional(&state.pool)
            .await?;
    let Some((target_id, owner)) = meta else {
        return Err(ApiError::NotFound("attachment".into()));
    };
    match target_id.as_deref() {
        Some(target) => authorize_target(&state, uid.as_str(), target).await?,
        // 未绑定频道的附件没有人人可以共享它，只允许上传者自己读取；
        // 否则“知道随机 id”就等于读取权限。
        None if owner != uid.as_str() => {
            return Err(ApiError::Forbidden("not the attachment owner".into()));
        }
        None => {}
    }

    let path = attachments_dir().join(&id);
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|_| ApiError::NotFound("attachment".into()))?;

    // 从 id 后缀猜 mime（简化：都当 octet-stream，由前端根据文件名猜）
    let mime = "application/octet-stream";

    Ok((StatusCode::OK, [(header::CONTENT_TYPE, mime)], bytes))
}

/// 附件 id 白名单。
///
/// 上传 id 来自 `URL_SAFE_NO_PAD`，字母表包含 `-` 和 `_`；两者都必须允许，
/// 否则约 29% 的附件生成后永远无法下载。仍然排除 `/`、`\`、`.` 等路径字符。
fn is_valid_attachment_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// 就绪探针：确认附件目录可创建且可写。
///
/// 只检查 DB 会让 `/ready` 在上传路径坏掉（只读卷、磁盘满、权限错误）时
/// 仍然返回 200，负载均衡器继续把写请求打进来。
pub(crate) fn storage_ready() -> Result<(), String> {
    let dir = attachments_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("attachments dir {}: {}", dir.display(), e))?;
    let probe = dir.join(".ready-probe");
    std::fs::write(&probe, b"ok").map_err(|e| format!("attachments dir not writable: {}", e))?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

/// 清理上传上下文（频道/DM id）：去控制字符、限长。只用于配额与回收。
fn sanitize_target(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            s.chars()
                .filter(|c| !c.is_control())
                .take(128)
                .collect::<String>()
        })
        .filter(|s| !s.is_empty())
}

/// 对账附件 DB 行与磁盘文件，清理两类孤儿数据。
///
/// 上传路径改成"先占配额、后写盘"以后，进程在两者之间崩溃会留下：
///   1. DB 有行、磁盘无文件 → 会永久占用配额（且下载 404）
///   2. 磁盘有文件、DB 无行 → 文件系统里不可见的磁盘占用
///
/// 两者都只在超过 [`ORPHAN_GRACE`]（24h）后才回收，避免误删正在上传的文件。
/// 该函数由小时级维护任务调用；**不要**放在上传热路径上。
///
/// 返回 `(删除的孤儿文件数, 删除的孤儿行数)`。
pub async fn sweep_orphans(pool: &sqlx::SqlitePool) -> (usize, usize) {
    let dir = attachments_dir();
    if tokio::fs::create_dir_all(&dir).await.is_err() {
        return (0, 0);
    }
    let now = std::time::SystemTime::now();

    // 已知 id 集合，用于判断磁盘文件是否有 DB 行。
    let known: std::collections::HashSet<String> =
        match sqlx::query_as::<_, (String,)>("SELECT id FROM attachments")
            .fetch_all(pool)
            .await
        {
            Ok(v) => v.into_iter().map(|(id,)| id).collect(),
            Err(e) => {
                tracing::warn!("[attachments] sweep: list rows failed: {}", e);
                return (0, 0);
            }
        };

    // ---- 1. 磁盘文件 → DB 行缺失或文件缺失 ----
    let mut removed_files = 0usize;
    let mut present: std::collections::HashSet<String> = std::collections::HashSet::new();
    match tokio::fs::read_dir(&dir).await {
        Ok(mut entries) => {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let name = entry.file_name().to_string_lossy().into_owned();
                let Ok(meta) = entry.metadata().await else {
                    continue;
                };
                if !meta.is_file() {
                    continue;
                }
                let old_enough = meta
                    .modified()
                    .ok()
                    .and_then(|m| now.duration_since(m).ok())
                    .is_some_and(|age| age >= ORPHAN_GRACE);
                if !old_enough {
                    // 新文件（含上传中的 .tmp 与就绪探针）不动。
                    present.insert(name);
                    continue;
                }
                // 非附件命名 / 没有 DB 行的文件都是孤儿。
                if is_valid_attachment_id(&name) && known.contains(&name) {
                    present.insert(name);
                    continue;
                }
                if tokio::fs::remove_file(entry.path()).await.is_ok() {
                    removed_files += 1;
                    tracing::warn!("[attachments] removed orphan file {}", name);
                }
            }
        }
        Err(e) => tracing::warn!("[attachments] sweep: read_dir failed: {}", e),
    }

    // ---- 2. DB 行 → 磁盘文件缺失（超宽限期）----
    let mut removed_rows = 0usize;
    let cutoff = db::now_ms() - ORPHAN_GRACE.as_millis() as i64;
    let stale: Vec<(String, i64)> =
        match sqlx::query_as("SELECT id, created_at FROM attachments WHERE created_at < ?")
            .bind(cutoff)
            .fetch_all(pool)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("[attachments] sweep: query stale rows failed: {}", e);
                return (removed_files, 0);
            }
        };
    for (id, _) in stale {
        if !is_valid_attachment_id(&id) {
            continue;
        }
        if present.contains(&id) {
            continue;
        }
        if tokio::fs::try_exists(dir.join(&id)).await.unwrap_or(true) {
            continue;
        }
        if sqlx::query("DELETE FROM attachments WHERE id = ?")
            .bind(&id)
            .execute(pool)
            .await
            .is_ok()
        {
            removed_rows += 1;
            tracing::warn!("[attachments] removed orphan row {} (file missing)", id);
        }
    }

    (removed_files, removed_rows)
}

/// 删除某个频道/DM 下的所有附件 blob 与元数据（频道删除时调用）。
///
/// 服务端不知道消息里最终引用了哪些附件，因此按“上传上下文”整体回收。
pub(crate) async fn purge_target(pool: &sqlx::SqlitePool, target: &str) {
    let ids: Vec<(String,)> = match sqlx::query_as("SELECT id FROM attachments WHERE target_id = ?")
        .bind(target)
        .fetch_all(pool)
        .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("attachment purge query failed ({}): {}", target, e);
            return;
        }
    };
    if ids.is_empty() {
        return;
    }
    if let Err(e) = sqlx::query("DELETE FROM attachments WHERE target_id = ?")
        .bind(target)
        .execute(pool)
        .await
    {
        tracing::warn!("attachment purge delete failed ({}): {}", target, e);
        return;
    }
    let dir = attachments_dir();
    for (id,) in ids {
        if !is_valid_attachment_id(&id) {
            continue;
        }
        if let Err(e) = tokio::fs::remove_file(dir.join(&id)).await
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!("attachment purge remove {} failed: {}", id, e);
        }
    }
}

/// 校验当前用户是否有权访问某个附件上下文（频道 / DM）。
///
/// - DM：`conv_id` 必须是当前用户参与的会话；
/// - 私密频道：必须是 `channel_members` 成员；
/// - 公开频道：所有登录用户可读；
/// - 频道不存在：404（附件应随频道删除被回收）。
async fn authorize_target(state: &AppState, uid: &str, target: &str) -> Result<(), ApiError> {
    if crate::dm::parse_conv_id(target).is_some() {
        if !crate::dm::is_member(target, uid) {
            return Err(ApiError::Forbidden("not a dm participant".into()));
        }
        return Ok(());
    }

    let meta: Option<(i64,)> = sqlx::query_as("SELECT is_private FROM channels WHERE id = ?")
        .bind(target)
        .fetch_optional(&state.pool)
        .await?;
    let Some((is_private,)) = meta else {
        return Err(ApiError::NotFound("channel".into()));
    };
    if is_private != 0 {
        let member: Option<(i64,)> =
            sqlx::query_as("SELECT 1 FROM channel_members WHERE channel_id = ? AND login_id = ?")
                .bind(target)
                .bind(uid)
                .fetch_optional(&state.pool)
                .await?;
        if member.is_none() {
            return Err(ApiError::Forbidden("not a channel member".into()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachment_id_accepts_urlsafe_alphabet() {
        assert!(is_valid_attachment_id("att_AbC012_-xyz"));
        assert!(is_valid_attachment_id("att_--------------------------"));
        assert!(is_valid_attachment_id("att__________________________"));
    }

    #[test]
    fn attachment_id_rejects_path_chars() {
        assert!(!is_valid_attachment_id(""));
        assert!(!is_valid_attachment_id("../etc/passwd"));
        assert!(!is_valid_attachment_id("att..\\x"));
        assert!(!is_valid_attachment_id("att/abc"));
        assert!(!is_valid_attachment_id(&"a".repeat(65)));
    }
}
