//! 附件渲染组件。
//!
//! 附件下载接口需要 Bearer token，浏览器 `<img src>` / `<a href>` 不会带
//! Authorization；客户端上传的附件还是 AES-GCM 密文（`0x04` 包装）。
//! 因此这里必须走 `RoomSession::download_attachment`：认证下载 + 解密，
//! 再转成 data URL 渲染。
//!
//! **不直接把服务器 URL 塞进 img/a**，否则 100% 401 或显示密文。

use std::sync::Arc;

use dioxus::html::FileData;
use dioxus::prelude::*;
use kaede_client::RoomSession;
use kaede_protocol::Attachment;

use crate::state::AppState;

/// 单个附件上限（与 server `MAX_ATTACHMENT_BYTES` 一致）。
const MAX_ATTACHMENT_BYTES: u64 = 32 * 1024 * 1024;

/// 读取本地文件 → 加密上传 → 返回附件元数据。
pub async fn upload_file(
    room: Arc<RoomSession>,
    target_id: &str,
    file: FileData,
) -> Result<Attachment, String> {
    if file.size() > MAX_ATTACHMENT_BYTES {
        return Err(format!(
            "文件过大（最大 {} MiB）",
            MAX_ATTACHMENT_BYTES / 1024 / 1024
        ));
    }
    let name = file.name();
    let mime = file.content_type();
    let bytes = file
        .read_bytes()
        .await
        .map_err(|e| format!("读取文件失败: {}", e))?
        .to_vec();

    room.upload_attachment(target_id, &name, mime.as_deref(), bytes)
        .await
        .map_err(|e| e.to_string())
}

/// 只有这些 MIME 才允许作为图片渲染（避免 SVG / HTML 执行脚本）。
fn is_safe_image_mime(mime: &str) -> bool {
    matches!(
        mime,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp"
    )
}

/// 超过该大小的附件不做 data URL 内联，避免 base64 后内存翻倍。
const INLINE_PREVIEW_MAX: u64 = 8 * 1024 * 1024;

fn should_inline_preview(att: &Attachment) -> bool {
    is_safe_image_mime(&att.mime) && att.size <= INLINE_PREVIEW_MAX
}

#[component]
pub fn AttachmentView(att: Attachment, target_id: String) -> Element {
    let app = use_context::<AppState>();
    let mut data_url = use_signal(|| Option::<String>::None);
    let mut error = use_signal(|| Option::<String>::None);
    let att_for_render = att.clone();

    // 仅小图片自动下载 + 内联预览；其它附件按需下载，避免 32MiB -> 43MiB 字符串。
    {
        let att = att.clone();
        let target_id = target_id.clone();
        use_effect(move || {
            if !should_inline_preview(&att) {
                return;
            }
            let att = att.clone();
            let target_id = target_id.clone();
            let room = app.room.read().clone();
            let Some(room) = room else {
                error.set(Some("未连接".into()));
                return;
            };
            spawn(async move {
                match room.download_attachment(&target_id, &att.id).await {
                    Ok(bytes) => {
                        use base64::Engine;
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                        data_url.set(Some(format!("data:{};base64,{}", att.mime, b64)));
                    }
                    Err(e) => {
                        tracing::warn!("attachment download failed ({}): {}", att.id, e);
                        error.set(Some("附件加载失败".into()));
                    }
                }
            });
        });
    }

    let url = data_url.read().clone();
    let err = error.read().clone();

    if let Some(err) = err {
        return rsx! {
            span { style: "font-size: 12px; color: var(--fg-faint);", "{err}" }
        };
    }

    if should_inline_preview(&att_for_render) {
        match url {
            Some(url) => rsx! {
                img {
                    src: "{url}",
                    alt: "{att_for_render.name}",
                    style: "max-width: 400px; max-height: 300px; border-radius: 8px; object-fit: contain;",
                }
            },
            None => rsx! {
                span { style: "font-size: 12px; color: var(--fg-faint);", "图片加载中..." }
            },
        }
    } else {
        rsx! {
            button {
                style: "display: inline-flex; align-items: center; gap: 6px; padding: 6px 10px; border-radius: 6px; background: rgba(0,0,0,0.06); color: var(--fg); font-size: 13px; border: none; cursor: pointer; max-width: 320px; overflow: hidden; text-overflow: ellipsis;",
                onclick: {
                    let att = att_for_render.clone();
                    let target_id = target_id.clone();
                    let app = app;
                    move |_| {
                        let att = att.clone();
                        let target_id = target_id.clone();
                        let room = app.room.read().clone();
                        let Some(room) = room else { return; };
                        spawn(async move {
                            match room.download_attachment(&target_id, &att.id).await {
                                Ok(bytes) => {
                                    use base64::Engine;
                                    let b64 =
                                        base64::engine::general_purpose::STANDARD.encode(&bytes);
                                    let mime = if is_safe_image_mime(&att.mime) {
                                        att.mime.clone()
                                    } else {
                                        "application/octet-stream".to_string()
                                    };
                                    let url = format!("data:{};base64,{}", mime, b64);
                                    // {:?} 确保文件名在 JS 字符串里正确转义。
                                    let script = format!(
                                        "(function(){{var a=document.createElement('a');a.href={:?};a.download={:?};document.body.appendChild(a);a.click();a.remove();}})()",
                                        url, att.name
                                    );
                                    let _ = dioxus::document::eval(&script);
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        "attachment download failed ({}): {}",
                                        att.id,
                                        e
                                    );
                                }
                            }
                        });
                    }
                },
                "📎 {att_for_render.name}"
            }
        }
    }
}
