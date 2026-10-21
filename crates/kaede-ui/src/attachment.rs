//! 附件渲染组件。
//!
//! 附件下载接口需要 Bearer token，浏览器 `<img src>` / `<a href>` 不会带
//! Authorization；客户端上传的附件还是 AES-GCM 密文（`0x04` 包装）。
//! 因此这里必须走 `RoomSession::download_attachment`：认证下载 + 解密，
//! 再转成 data URL 渲染。
//!
//! 图片缩略图：上传时客户端生成小图，用同一附件 key 加密后作为第二个附件上传，
//! 消息里的 `thumbnail_url` 存缩略图附件 id。服务端仍然只存密文。

use std::sync::Arc;

use dioxus::html::FileData;
use dioxus::prelude::*;
use kaede_client::RoomSession;
use kaede_protocol::Attachment;

use crate::state::AppState;

/// 单个附件上限（与 server `MAX_ATTACHMENT_BYTES` 一致）。
const MAX_ATTACHMENT_BYTES: u64 = 32 * 1024 * 1024;

/// 超过该大小的图片才生成缩略图。
const THUMB_TRIGGER_BYTES: usize = 256 * 1024;

/// 缩略图最长边。
const THUMB_MAX_DIM: u32 = 512;

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

    let wants_thumb =
        mime.as_deref().is_some_and(is_safe_image_mime) && bytes.len() > THUMB_TRIGGER_BYTES;

    #[cfg(not(target_arch = "wasm32"))]
    let thumb = if wants_thumb {
        make_thumbnail(&bytes)
    } else {
        None
    };
    #[cfg(target_arch = "wasm32")]
    let thumb = if wants_thumb {
        make_thumbnail(&bytes).await
    } else {
        None
    };

    let mut att = room
        .upload_attachment(target_id, &name, mime.as_deref(), bytes)
        .await
        .map_err(|e| e.to_string())?;

    if let Some(thumb_bytes) = thumb {
        let thumb_name = format!("{}_thumb.jpg", name);
        match room
            .upload_attachment(target_id, &thumb_name, Some("image/jpeg"), thumb_bytes)
            .await
        {
            Ok(t) => att.thumbnail_url = Some(t.id),
            Err(e) => tracing::warn!("thumbnail upload failed: {}", e),
        }
    }

    Ok(att)
}

#[cfg(not(target_arch = "wasm32"))]
fn make_thumbnail(bytes: &[u8]) -> Option<Vec<u8>> {
    let img = image::load_from_memory(bytes).ok()?;
    if img.width() <= THUMB_MAX_DIM && img.height() <= THUMB_MAX_DIM {
        return None;
    }
    let rgb = img.thumbnail(THUMB_MAX_DIM, THUMB_MAX_DIM).to_rgb8();
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(rgb)
        .write_to(&mut out, image::ImageFormat::Jpeg)
        .ok()?;
    Some(out.into_inner())
}

#[cfg(target_arch = "wasm32")]
async fn make_thumbnail(bytes: &[u8]) -> Option<Vec<u8>> {
    use wasm_bindgen::{JsCast, JsValue};

    let win = web_sys::window()?;
    let array = js_sys::Uint8Array::from(bytes);
    let parts = js_sys::Array::new();
    parts.push(&array.buffer());
    let blob = web_sys::Blob::new_with_u8_array_sequence(&JsValue::from(parts)).ok()?;
    let promise = win.create_image_bitmap_with_blob(&blob).ok()?;
    let bitmap: web_sys::ImageBitmap = wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .ok()?
        .dyn_into()
        .ok()?;

    let (bw, bh) = (bitmap.width(), bitmap.height());
    if bw <= THUMB_MAX_DIM && bh <= THUMB_MAX_DIM {
        return None;
    }
    let scale = THUMB_MAX_DIM as f64 / bw.max(bh) as f64;
    let w = ((bw as f64 * scale).round() as u32).max(1);
    let h = ((bh as f64 * scale).round() as u32).max(1);

    let doc = win.document()?;
    let canvas: web_sys::HtmlCanvasElement = doc.create_element("canvas").ok()?.dyn_into().ok()?;
    canvas.set_width(w);
    canvas.set_height(h);
    let ctx: web_sys::CanvasRenderingContext2d = canvas.get_context("2d").ok()??.dyn_into().ok()?;
    ctx.draw_image_with_image_bitmap_and_dw_and_dh(&bitmap, 0.0, 0.0, w as f64, h as f64)
        .ok()?;
    let url = canvas
        .to_data_url_with_type_and_encoder_options("image/jpeg", &JsValue::from_f64(0.7))
        .ok()?;
    let (_head, b64) = url.split_once(',')?;
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(b64).ok()
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

/// 下载原附件并触发浏览器保存。
fn download_and_save(app: AppState, target_id: String, att: Attachment) {
    let room = app.room.read().clone();
    let Some(room) = room else { return };
    spawn(async move {
        match room.download_attachment(&target_id, &att.id).await {
            Ok(bytes) => {
                use base64::Engine;
                let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                let mime = if is_safe_image_mime(&att.mime) {
                    att.mime.clone()
                } else {
                    "application/octet-stream".to_string()
                };
                let url = format!("data:{};base64,{}", mime, b64);
                let script = format!(
                    "(function(){{var a=document.createElement('a');a.href={:?};a.download={:?};document.body.appendChild(a);a.click();a.remove();}})()",
                    url, att.name
                );
                let _ = dioxus::document::eval(&script);
            }
            Err(e) => {
                tracing::warn!("attachment download failed ({}): {}", att.id, e);
            }
        }
    });
}

#[component]
pub fn AttachmentView(att: Attachment, target_id: String) -> Element {
    let app = use_context::<AppState>();
    let mut data_url = use_signal(|| Option::<String>::None);
    let mut error = use_signal(|| Option::<String>::None);
    let att_for_render = att.clone();

    let is_image = is_safe_image_mime(&att.mime);
    // 优先下载缩略图；没有缩略图的小图继续内联原图。
    let preview: Option<(String, String)> = if is_image {
        if let Some(thumb_id) = att.thumbnail_url.clone() {
            Some((thumb_id, "image/jpeg".to_string()))
        } else if should_inline_preview(&att) {
            Some((att.id.clone(), att.mime.clone()))
        } else {
            None
        }
    } else {
        None
    };

    {
        let preview = preview.clone();
        let target_id = target_id.clone();
        use_effect(move || {
            let Some((preview_id, preview_mime)) = preview.clone() else {
                return;
            };
            let target_id = target_id.clone();
            let room = app.room.read().clone();
            let Some(room) = room else {
                error.set(Some("未连接".into()));
                return;
            };
            spawn(async move {
                match room.download_attachment(&target_id, &preview_id).await {
                    Ok(bytes) => {
                        // 不能信任发送方声明的 size：以实际字节数为准。
                        if bytes.len() as u64 > INLINE_PREVIEW_MAX {
                            return;
                        }
                        use base64::Engine;
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                        data_url.set(Some(format!("data:{};base64,{}", preview_mime, b64)));
                    }
                    Err(e) => {
                        tracing::warn!("attachment download failed ({}): {}", preview_id, e);
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

    if let Some(preview) = preview {
        return match url {
            Some(url) => rsx! {
                img {
                    src: "{url}",
                    alt: "{att_for_render.name}",
                    title: "点击下载原图",
                    style: "max-width: 400px; max-height: 300px; border-radius: 8px; object-fit: contain; cursor: pointer;",
                    onclick: {
                        let att = att_for_render.clone();
                        let target_id = target_id.clone();
                        move |_| {
                            download_and_save(app, target_id.clone(), att.clone());
                        }
                    },
                }
            },
            None => rsx! {
                span { style: "font-size: 12px; color: var(--fg-faint);",
                    if preview.1 == "image/jpeg" { "缩略图加载中..." } else { "图片加载中..." }
                }
            },
        };
    }

    rsx! {
        button {
            style: "display: inline-flex; align-items: center; gap: 6px; padding: 6px 10px; border-radius: 6px; background: rgba(0,0,0,0.06); color: var(--fg); font-size: 13px; border: none; cursor: pointer; max-width: 320px; overflow: hidden; text-overflow: ellipsis;",
            onclick: {
                let att = att_for_render.clone();
                let target_id = target_id.clone();
                move |_| {
                    download_and_save(app, target_id.clone(), att.clone());
                }
            },
            "📎 {att_for_render.name}"
        }
    }
}

#[cfg(target_arch = "wasm32")]
/// 从原始字节上传附件（供 wasm 拖拽 / 粘贴桥接复用）。
/// 与 upload_file 逻辑一致：上传原文件 → 可选生成缩略图 → 上传缩略图。
pub async fn upload_file_from_bytes(
    room: std::sync::Arc<kaede_client::RoomSession>,
    target_id: &str,
    name: &str,
    mime: Option<&str>,
    bytes: Vec<u8>,
) -> Result<kaede_protocol::Attachment, kaede_client::ClientError> {
    let is_image = mime.is_some_and(is_safe_image_mime) && bytes.len() > THUMB_TRIGGER_BYTES;
    #[cfg(target_arch = "wasm32")]
    let thumb_bytes = if is_image {
        make_thumbnail(&bytes).await
    } else {
        None
    };
    #[cfg(not(target_arch = "wasm32"))]
    let thumb_bytes = if is_image { make_thumbnail(&bytes) } else { None };
    let mut att = room.upload_attachment(target_id, name, mime, bytes).await?;
    if let Some(thumb_bytes) = thumb_bytes {
        let thumb_name = format!("{}_thumb.jpg", name);
        match room
            .upload_attachment(target_id, &thumb_name, Some("image/jpeg"), thumb_bytes)
            .await
        {
            Ok(t) => att.thumbnail_url = Some(t.id),
            Err(e) => tracing::warn!("thumbnail upload failed: {}", e),
        }
    }
    Ok(att)
}

