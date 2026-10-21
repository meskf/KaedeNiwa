//! 浏览器 DOM 事件桥接（wasm-only）。
//!
//! 拖拽 / 粘贴事件在 Dioxus 0.7 的 `DragData` / `ClipboardData` 里拿不到文件，
//! 必须直接绑 `web_sys` 事件。本模块提供幂等的 drop / dragover / dragleave 绑定。

#![cfg(target_arch = "wasm32")]

use crate::state::{AppState, UiState, UploadFile};
use dioxus::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use web_sys::{ClipboardEvent, DragEvent, Event, File};

/// 把拖拽 / 粘贴事件绑定到 `#composer-wrap`。
/// 幂等：重复调用不会重复绑定（用 DOM 元素属性标记）。
pub fn bind_drop_and_paste(ui: UiState, app: AppState) {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let Some(wrap) = doc.get_element_by_id("composer-wrap") else {
        // DOM 还没挂载，use_effect 会在挂载后再次触发
        return;
    };

    // 幂等检查
    let already = js_sys::Reflect::get(&wrap, &JsValue::from_str("__kaedeDndBound"))
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if already {
        return;
    }
    let _ = js_sys::Reflect::set(&wrap, &JsValue::from_str("__kaedeDndBound"), &JsValue::TRUE);

    // ===== dragover：阻止默认行为 + 高亮 =====
    {
        let wrap_c = wrap.clone();
        let over_closure = Closure::<dyn FnMut(Event)>::new(move |e: Event| {
            e.prevent_default();
            if let Some(el) = wrap_c.dyn_ref::<web_sys::Element>() {
                let _ = el.class_list().add_1("drag-over");
            }
        });
        let _ = wrap
            .add_event_listener_with_callback("dragover", over_closure.as_ref().unchecked_ref());
        over_closure.forget();
    }

    // ===== dragleave：取消高亮 =====
    {
        let wrap_c = wrap.clone();
        let leave_closure = Closure::<dyn FnMut(Event)>::new(move |e: Event| {
            e.prevent_default();
            if let Some(el) = wrap_c.dyn_ref::<web_sys::Element>() {
                let _ = el.class_list().remove_1("drag-over");
            }
        });
        let _ = wrap
            .add_event_listener_with_callback("dragleave", leave_closure.as_ref().unchecked_ref());
        leave_closure.forget();
    }

    // ===== drop：读文件 → 上传 =====
    {
        let wrap_c = wrap.clone();
        let drop_closure = Closure::<dyn FnMut(DragEvent)>::new(move |e: DragEvent| {
            e.prevent_default();
            if let Some(el) = wrap_c.dyn_ref::<web_sys::Element>() {
                let _ = el.class_list().remove_1("drag-over");
            }

            let Some(dt) = e.data_transfer() else {
                return;
            };
            let Some(files) = dt.files() else {
                return;
            };
            if files.length() == 0 {
                return;
            }
            let mut file_list = Vec::new();
            for i in 0..files.length() {
                if let Some(f) = files.get(i) {
                    file_list.push(f);
                }
            }
            if file_list.is_empty() {
                return;
            }

            // 在 Dioxus 的 spawn 里读 bytes（wasm 下 spawn 是 spawn_local，不要求 Send）
            dioxus::prelude::spawn(async move {
                let Some(ch) = ui.selected_channel.read().clone() else {
                    return;
                };
                let Some(sid) = app.current_server.read().clone() else {
                    return;
                };
                let mut uploads = Vec::new();
                for f in file_list {
                    if let Some(u) = file_to_upload(f).await {
                        uploads.push(u);
                    }
                }
                if !uploads.is_empty() {
                    crate::content::send_attachments_bytes(ui, app, ch, sid, uploads);
                }
            });
        });
        let _ =
            wrap.add_event_listener_with_callback("drop", drop_closure.as_ref().unchecked_ref());
        drop_closure.forget();
    }
}

/// 把 `web_sys::File` 读成 `UploadFile`（name + mime + bytes）。
async fn file_to_upload(file: File) -> Option<UploadFile> {
    let name = file.name();
    let raw_mime = file.type_();
    let mime = if raw_mime.is_empty() {
        "application/octet-stream".to_string()
    } else {
        raw_mime
    };
    let promise = file.array_buffer();
    let ab = wasm_bindgen_futures::JsFuture::from(promise).await.ok()?;
    let uint8 = js_sys::Uint8Array::new(&ab);
    let mut data = vec![0u8; uint8.length() as usize];
    uint8.copy_to(&mut data);
    Some(UploadFile { name, mime, data })
}

/// 把 paste 事件绑定到 `#composer-input`（textarea）。
///
/// 只处理剪贴板里的"文件"项（截图 / 复制的文件）；纯文本粘贴放行给浏览器原生处理。
/// 幂等：重复调用不会重复绑定（DOM 元素属性标记）。
pub fn bind_paste(ui: UiState, app: AppState) {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let Some(input) = doc.get_element_by_id("composer-input") else {
        return;
    };

    let already = js_sys::Reflect::get(&input, &JsValue::from_str("__kaedePasteBound"))
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if already {
        return;
    }
    let _ = js_sys::Reflect::set(&input, &JsValue::from_str("__kaedePasteBound"), &JsValue::TRUE);

    let closure = Closure::<dyn FnMut(ClipboardEvent)>::new(move |e: ClipboardEvent| {
        let Some(dt) = e.clipboard_data() else {
            return;
        };
        let items = dt.items();
        let mut file_list = Vec::new();
        for i in 0..items.length() {
            let Some(item) = items.get(i) else { continue };
            if item.kind() != "file" {
                continue;
            }
            if let Ok(Some(f)) = item.get_as_file() {
                file_list.push(f);
            }
        }
        if file_list.is_empty() {
            // 纯文本粘贴：放行给浏览器原生
            return;
        }
        e.prevent_default();

        dioxus::prelude::spawn(async move {
            let Some(ch) = ui.selected_channel.read().clone() else {
                return;
            };
            let Some(sid) = app.current_server.read().clone() else {
                return;
            };
            let mut uploads = Vec::new();
            for f in file_list {
                if let Some(u) = file_to_upload(f).await {
                    uploads.push(u);
                }
            }
            if !uploads.is_empty() {
                crate::content::send_attachments_bytes(ui, app, ch, sid, uploads);
            }
        });
    });

    let _ = input.add_event_listener_with_callback(
        "paste",
        closure.as_ref().unchecked_ref(),
    );
    closure.forget();
}
