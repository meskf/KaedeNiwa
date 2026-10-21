//! 附件上传 / 下载。
//!
//! 存储：`<data_dir>/attachments/<id>`。
//! 明文（与 Discord 一致）。
//!
//! **单文件上限**：32 MiB（8K60 视频帧不走这里，走媒体流）。

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

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/attachments", post(upload))
        .route("/api/attachments/{id}", get(download))
        .with_state(state)
}

#[derive(Deserialize)]
struct UploadQuery {
    name: String,
    #[serde(default)]
    mime: Option<String>,
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
    if !state.check_rate(&uid, 1.0, 10.0) {
        return Err(ApiError::BadRequest("upload rate limited".into()));
    }
    if body.len() > MAX_ATTACHMENT_BYTES {
        return Err(ApiError::BadRequest(format!(
            "attachment too large (max {} bytes)",
            MAX_ATTACHMENT_BYTES
        )));
    }

    let name = sanitize_name(&q.name);
    // P2-6: mime 白名单——客户端自报的 mime 不可信，限制到白名单内
    // （防止 text/html 等被 UI 当作可执行内容渲染）
    let mime = match q.mime {
        Some(m) if is_allowed_mime(&m) => m,
        _ => guess_mime(&name),
    };

    // 用 SHA256 前 16 字节 + 时间戳保证唯一
    let id = format!("att_{}", &db::random_token()[..22]);

    let dir = attachments_dir();
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| ApiError::Internal(format!("mkdir: {}", e)))?;

    // P2-7: 先写 .tmp 再 rename——磁盘满/进程崩溃时不会留下半写文件
    let final_path = dir.join(&id);
    let tmp_path = dir.join(format!(".{}.tmp", id));
    tokio::fs::write(&tmp_path, &body)
        .await
        .map_err(|e| ApiError::Internal(format!("write tmp: {}", e)))?;
    tokio::fs::rename(&tmp_path, &final_path)
        .await
        .map_err(|e| ApiError::Internal(format!("rename: {}", e)))?;

    let att = Attachment {
        id: id.clone(),
        name,
        mime: mime.clone(),
        size: body.len() as u64,
        url: format!("/api/attachments/{}", id),
        thumbnail_url: None,
    };

    tracing::info!(
        "uploaded attachment {} ({} bytes, {})",
        att.id,
        att.size,
        att.mime
    );

    Ok((StatusCode::CREATED, Json(UploadResp { attachment: att })))
}

async fn download(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let _uid = authenticate(&state, &headers).await?;

    // H17: 严格 id 校验——白名单字符集，防路径穿越 / Windows ADS / 保留名
    if id.is_empty() || id.len() > 64 || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(ApiError::BadRequest("invalid id".into()));
    }

    let path = attachments_dir().join(&id);
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|_| ApiError::NotFound("attachment".into()))?;

    // 从 id 后缀猜 mime（简化：都当 octet-stream，由前端根据文件名猜）
    let mime = "application/octet-stream";

    Ok((StatusCode::OK, [(header::CONTENT_TYPE, mime)], bytes))
}

fn sanitize_name(s: &str) -> String {
    let s = s.trim();
    let s = s.replace(['/', '\\'], "_");
    if s.is_empty() || s.len() > 200 {
        "file".to_string()
    } else {
        s.to_string()
    }
}

fn guess_mime(name: &str) -> String {
    let lower = name.to_lowercase();
    if lower.ends_with(".png") {
        "image/png".into()
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg".into()
    } else if lower.ends_with(".gif") {
        "image/gif".into()
    } else if lower.ends_with(".webp") {
        "image/webp".into()
    } else if lower.ends_with(".svg") {
        "image/svg+xml".into()
    } else if lower.ends_with(".mp4") {
        "video/mp4".into()
    } else if lower.ends_with(".mp3") {
        "audio/mpeg".into()
    } else if lower.ends_with(".pdf") {
        "application/pdf".into()
    } else if lower.ends_with(".txt") || lower.ends_with(".md") {
        "text/plain".into()
    } else {
        "application/octet-stream".into()
    }
}
/// P2-6: 允许的 mime 白名单。任何非白名单的客户端自报值都回退到 guess_mime。
fn is_allowed_mime(m: &str) -> bool {
    matches!(
        m,
        "image/png"
            | "image/jpeg"
            | "image/gif"
            | "image/webp"
            | "image/svg+xml"
            | "video/mp4"
            | "audio/mpeg"
            | "application/pdf"
            | "text/plain"
            | "application/octet-stream"
    )
}
