//! wasm32 实现：通过 window.LivekitClient 直接调用 LiveKit UMD。
//!
//! 不使用 wasm-bindgen 的 #[wasm_bindgen(js_namespace = ...)] 类绑定，
//! 而是用 Reflect 动态取值 —— 这样 LiveKit 升级到新 API 时不需要改 Rust 类型声明。

use crate::error::LiveKitWebError;
use js_sys::{Function, Object, Promise, Reflect};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

pub struct LiveKitWebSession {
    room: JsValue,
    #[allow(dead_code)]
    key_provider: Option<JsValue>,
}

fn global_lk() -> Result<JsValue, LiveKitWebError> {
    let g = js_sys::global();
    let lk = Reflect::get(&g, &JsValue::from_str("LivekitClient"))
        .map_err(|_| LiveKitWebError::SdkNotLoaded)?;
    if lk.is_undefined() || lk.is_null() {
        return Err(LiveKitWebError::SdkNotLoaded);
    }
    Ok(lk)
}

fn get_fn(obj: &JsValue, name: &str) -> Result<Function, LiveKitWebError> {
    Reflect::get(obj, &JsValue::from_str(name))
        .map_err(|e| LiveKitWebError::Js(format!("get {}: {:?}", name, e)))?
        .dyn_into::<Function>()
        .map_err(|_| LiveKitWebError::Js(format!("{} is not a function", name)))
}

fn ctor(lk: &JsValue, name: &str) -> Result<Function, LiveKitWebError> {
    Reflect::get(lk, &JsValue::from_str(name))
        .map_err(|_| LiveKitWebError::SdkNotLoaded)?
        .dyn_into::<Function>()
        .map_err(|_| LiveKitWebError::SdkNotLoaded)
}

async fn await_promise(v: JsValue) -> Result<JsValue, LiveKitWebError> {
    let p: Promise = v
        .dyn_into()
        .map_err(|_| LiveKitWebError::Js("expected Promise".into()))?;
    JsFuture::from(p)
        .await
        .map_err(|e| LiveKitWebError::Js(format!("promise rejected: {:?}", e)))
}

fn hex_key(key: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in key {
        use std::fmt::Write as _;
        let _ = write!(&mut s, "{b:02x}");
    }
    s
}

/// 通过宿主页面的 `window.__kaedeMakeE2EEWorker` 创建同源 Worker。
async fn make_e2ee_worker() -> Result<JsValue, LiveKitWebError> {
    let global = js_sys::global();
    let maker = Reflect::get(&global, &JsValue::from_str("__kaedeMakeE2EEWorker"))
        .map_err(|e| LiveKitWebError::Js(format!("get worker factory: {:?}", e)))?;
    if !maker.is_function() {
        return Err(LiveKitWebError::Js(
            "window.__kaedeMakeE2EEWorker missing; index.html not updated".into(),
        ));
    }
    let f: js_sys::Function = maker
        .dyn_into()
        .map_err(|_| LiveKitWebError::Js("worker factory is not a function".into()))?;
    let promise = f
        .call1(&global, &JsValue::from_str(crate::LIVEKIT_E2EE_WORKER_URL))
        .map_err(|e| LiveKitWebError::Js(format!("new e2ee worker: {:?}", e)))?;
    await_promise(promise).await
}

impl LiveKitWebSession {
    pub async fn new(e2ee_key: Option<[u8; 32]>) -> Result<Self, LiveKitWebError> {
        let lk = global_lk()?;
        let room_ctor = ctor(&lk, "Room")?;
        let opts = JsValue::from(Object::new());
        let mut key_provider = None;

        if let Some(key) = e2ee_key {
            let kp_ctor = ctor(&lk, "ExternalE2EEKeyProvider")?;
            let kp = kp_ctor
                .call0(&JsValue::UNDEFINED)
                .map_err(|e| LiveKitWebError::Js(format!("new keyProvider: {:?}", e)))?;

            // 关键：`livekit-client` 的 setKey(string) 走 PBKDF2，
            // setKey(ArrayBuffer) 走 HKDF；native 侧固定 PBKDF2。
            // 传 hex 文本让两端用同一 password 字节（ASCII hex）。
            let set_key = get_fn(&kp, "setKey")?;
            let key_hex = hex_key(&key);
            await_promise(
                set_key
                    .call1(&kp, &JsValue::from_str(&key_hex))
                    .map_err(|e| LiveKitWebError::Js(format!("setKey call: {:?}", e)))?,
            )
            .await?;

            // 2.14.0 的 RoomOptions 字段是 `e2ee`（不是 `encryption`），
            // 且 E2EEOptions 必须同时提供 worker。
            let worker = make_e2ee_worker().await?;
            let enc = JsValue::from(Object::new());
            Reflect::set(&enc, &JsValue::from_str("keyProvider"), &kp)
                .map_err(|e| LiveKitWebError::Js(format!("e2ee.keyProvider: {:?}", e)))?;
            Reflect::set(&enc, &JsValue::from_str("worker"), &worker)
                .map_err(|e| LiveKitWebError::Js(format!("e2ee.worker: {:?}", e)))?;
            Reflect::set(&opts, &JsValue::from_str("e2ee"), &enc)
                .map_err(|e| LiveKitWebError::Js(format!("opts.e2ee: {:?}", e)))?;

            key_provider = Some(kp);
        }

        let room = room_ctor
            .call1(&JsValue::UNDEFINED, &opts)
            .map_err(|e| LiveKitWebError::Js(format!("new Room: {:?}", e)))?;

        tracing::info!(
            "[livekit-web] session created (e2ee={})",
            key_provider.is_some()
        );
        Ok(Self { room, key_provider })
    }

    pub async fn connect(&self, url: &str, token: &str) -> Result<(), LiveKitWebError> {
        let connect = get_fn(&self.room, "connect")?;
        await_promise(
            connect
                .call2(
                    &self.room,
                    &JsValue::from_str(url),
                    &JsValue::from_str(token),
                )
                .map_err(|e| LiveKitWebError::Js(format!("connect: {:?}", e)))?,
        )
        .await?;
        tracing::info!("[livekit-web] connected to {}", url);
        Ok(())
    }

    pub async fn set_screen_share(
        &self,
        enabled: bool,
        profile: kaede_protocol::VideoProfile,
    ) -> Result<(), LiveKitWebError> {
        let lp = Reflect::get(&self.room, &JsValue::from_str("localParticipant"))
            .map_err(|e| LiveKitWebError::Js(format!("localParticipant: {:?}", e)))?;
        let set_ss = get_fn(&lp, "setScreenShareEnabled")?;

        let video_opts = JsValue::from(Object::new());
        Reflect::set(
            &video_opts,
            &JsValue::from_str("displaySurface"),
            &JsValue::from_str("monitor"),
        )
        .map_err(|e| LiveKitWebError::Js(format!("video.displaySurface: {:?}", e)))?;

        // LiveKit 会把 resolution 转成 getDisplayMedia 的 width/height/frameRate 约束。
        let resolution = JsValue::from(Object::new());
        Reflect::set(
            &resolution,
            &JsValue::from_str("width"),
            &JsValue::from_f64(profile.width as f64),
        )
        .map_err(|e| LiveKitWebError::Js(format!("resolution.width: {:?}", e)))?;
        Reflect::set(
            &resolution,
            &JsValue::from_str("height"),
            &JsValue::from_f64(profile.height as f64),
        )
        .map_err(|e| LiveKitWebError::Js(format!("resolution.height: {:?}", e)))?;
        Reflect::set(
            &resolution,
            &JsValue::from_str("frameRate"),
            &JsValue::from_f64(profile.fps as f64),
        )
        .map_err(|e| LiveKitWebError::Js(format!("resolution.frameRate: {:?}", e)))?;
        Reflect::set(&video_opts, &JsValue::from_str("resolution"), &resolution)
            .map_err(|e| LiveKitWebError::Js(format!("video.resolution: {:?}", e)))?;

        let opts = JsValue::from(Object::new());
        Reflect::set(&opts, &JsValue::from_str("video"), &video_opts)
            .map_err(|e| LiveKitWebError::Js(format!("opts.video: {:?}", e)))?;
        Reflect::set(&opts, &JsValue::from_str("audio"), &JsValue::FALSE)
            .map_err(|e| LiveKitWebError::Js(format!("opts.audio: {:?}", e)))?;

        await_promise(
            set_ss
                .call2(&lp, &JsValue::from_bool(enabled), &opts)
                .map_err(|e| LiveKitWebError::Js(format!("setScreenShareEnabled: {:?}", e)))?,
        )
        .await?;
        tracing::info!(
            "[livekit-web] screen share -> {} ({}x{}@{})",
            enabled,
            profile.width,
            profile.height,
            profile.fps
        );
        Ok(())
    }

    pub async fn set_microphone(&self, enabled: bool) -> Result<(), LiveKitWebError> {
        let lp = Reflect::get(&self.room, &JsValue::from_str("localParticipant"))
            .map_err(|e| LiveKitWebError::Js(format!("localParticipant: {:?}", e)))?;
        let f = get_fn(&lp, "setMicrophoneEnabled")?;
        await_promise(
            f.call1(&lp, &JsValue::from_bool(enabled))
                .map_err(|e| LiveKitWebError::Js(format!("setMicrophoneEnabled: {:?}", e)))?,
        )
        .await?;
        tracing::info!("[livekit-web] microphone -> {}", enabled);
        Ok(())
    }

    /// 监听 LiveKit 的 trackSubscribed 事件：远端视频到达时创建 <video> 元素，
    /// 调 track.attach(video) 绑定，append 到指定 container（不存在则 body）。
    ///
    /// 使用 Closure::forget 让回调活到页面关闭——session 级别监听器，泄漏可接受。
    pub fn attach_remote_videos_to(&self, container_id: &str) -> Result<(), LiveKitWebError> {
        let on = get_fn(&self.room, "on")?;
        let cid = container_id.to_string();
        let closure = wasm_bindgen::closure::Closure::<dyn FnMut(JsValue, JsValue)>::new(
            move |track: JsValue, _participant: JsValue| {
                let kind = Reflect::get(&track, &JsValue::from_str("kind"))
                    .ok()
                    .and_then(|v| v.as_string())
                    .unwrap_or_default();
                if kind != "video" {
                    return;
                }

                let Some(window) = web_sys::window() else {
                    return;
                };
                let Some(document) = window.document() else {
                    return;
                };

                let Ok(video) = document.create_element("video") else {
                    return;
                };
                let _ = video.set_attribute("autoplay", "");
                let _ = video.set_attribute("playsinline", "");
                let _ = video.set_attribute(
                    "style",
                    "width: 100%; max-width: 480px; border-radius: 4px; background:#000;",
                );

                if let Ok(attach) = Reflect::get(&track, &JsValue::from_str("attach"))
                    && let Ok(f) = attach.dyn_into::<Function>()
                {
                    let _ = f.call1(&track, &video);
                }

                let container: web_sys::Node = document
                    .get_element_by_id(&cid)
                    .map(|e| e.into())
                    .or_else(|| document.body().map(|b| b.into()))
                    .expect("container fallback");
                let _ = container.append_child(&video);
                tracing::info!("[livekit-web] remote video attached");
            },
        );
        on.call2(
            &self.room,
            &JsValue::from_str("trackSubscribed"),
            closure.as_ref().unchecked_ref(),
        )
        .map_err(|e| LiveKitWebError::Js(format!("room.on(trackSubscribed): {:?}", e)))?;
        closure.forget();
        Ok(())
    }

    pub async fn disconnect(&self) -> Result<(), LiveKitWebError> {
        let f = get_fn(&self.room, "disconnect")?;
        await_promise(
            f.call0(&self.room)
                .map_err(|e| LiveKitWebError::Js(format!("disconnect: {:?}", e)))?,
        )
        .await?;
        Ok(())
    }
}

impl Drop for LiveKitWebSession {
    fn drop(&mut self) {
        if let Ok(f) = get_fn(&self.room, "disconnect") {
            let _ = f.call0(&self.room);
        }
    }
}
