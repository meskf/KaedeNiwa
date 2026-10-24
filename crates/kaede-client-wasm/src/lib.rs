use wasm_bindgen::prelude::*;
use js_sys::Function;
use std::sync::{Arc, Mutex};
use kaede_client::{ClientObserver, ClientEvent};
use kaede_client::room::RoomSession;

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}

struct WasmObserver {
    callback: Arc<Mutex<Option<Function>>>,
    queue: Arc<Mutex<Vec<ClientEvent>>>,
}

impl ClientObserver for WasmObserver {
    fn on_event(&self, event: ClientEvent) {
        let cb_borrow = self.callback.lock().unwrap();
        if let Some(cb) = cb_borrow.as_ref() {
            if let Ok(json) = serde_json::to_string(&event) {
                let js_val = JsValue::from_str(&json);
                let _ = cb.call1(&JsValue::NULL, &js_val);
            }
        } else {
            self.queue.lock().unwrap().push(event);
        }
    }
}

#[wasm_bindgen]
pub struct JsClient {
    room: Arc<Mutex<Option<Arc<RoomSession>>>>,
    observer: Arc<WasmObserver>,
}

#[wasm_bindgen]
impl JsClient {
    #[wasm_bindgen(constructor)]
    pub fn new() -> JsClient {
        JsClient {
            room: Arc::new(Mutex::new(None)),
            observer: Arc::new(WasmObserver {
                callback: Arc::new(Mutex::new(None)),
                queue: Arc::new(Mutex::new(Vec::new())),
            }),
        }
    }

    #[wasm_bindgen(js_name = setOnEvent)]
    pub fn set_on_event(&self, callback: Function) {
        *self.observer.callback.lock().unwrap() = Some(callback);
        let mut queue = self.observer.queue.lock().unwrap();
        if !queue.is_empty() {
            if let Some(cb) = self.observer.callback.lock().unwrap().as_ref() {
                for ev in queue.drain(..) {
                    if let Ok(json) = serde_json::to_string(&ev) {
                        let js_val = JsValue::from_str(&json);
                        let _ = cb.call1(&JsValue::NULL, &js_val);
                    }
                }
            }
        }
    }

    pub async fn login(&self, http_url: String, ws_url: String, login_id: String, token: String) -> Result<(), JsValue> {
        let obs: Arc<dyn ClientObserver> = self.observer.clone();
        
        // 打开本地 IndexedDB 存储（WASM 下 open_default_store 会忽略路径，用固定 IDB）
        let storage = match kaede_client::room::open_default_store("kaede").await {
            Ok(s) => {
                web_sys::console::log_1(&"[login] IndexedDB 已打开，MLS 快照会持久化".into());
                Some(s)
            }
            Err(e) => {
                web_sys::console::warn_1(&format!("[login] open_default_store 失败: {}", e).into());
                None
            }
        };
        
        let room = RoomSession::connect_with_token_and_storage(
            &http_url, &ws_url, &login_id, &token, storage, Some(obs)
        ).await.map_err(|e| JsValue::from_str(&format!("Connect error: {}", e)))?;

        let room = Arc::new(room);
        room.spawn_event_loop();
        // P0-2：启动心跳（每 20s Ping，30s 无 Pong 断开）
        // P0-2: room.connection() 返回 &Arc，spawn_heartbeat 要 owned，先 clone
        room.connection().clone().spawn_heartbeat();
        room.join_room().await.map_err(|e| JsValue::from_str(&format!("Join error: {}", e)))?;
        
        // 【核心】让新用户自动上传 KeyPackage，供别人拉进频道群组使用
        if let Err(e) = room.become_guest().await {
            tracing::warn!("become_guest failed: {}", e);
        }
        
        *self.room.lock().unwrap() = Some(room);
        Ok(())
    }

    // 确保频道拥有独立的 MLS 群组
    /// 确保频道 group 存在。返回 true 表示本次新建（即当前用户是 authority 候选）。
    pub fn ensure_channel(&self, channel_id: String) -> Result<bool, JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        let conn = room.connection();
        if conn.channel_has_group(&channel_id) {
            return Ok(false);
        }
        conn.channel_ensure_group(&channel_id)
            .map_err(|e| JsValue::from_str(&format!("Ensure group error: {}", e)))?;
        Ok(true)
    }

    // 发送加密消息到指定频道
    pub async fn join_channel(&self, channel_id: String) -> Result<(), JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        room.join_channel_only(&channel_id).await
            .map_err(|e| JsValue::from_str(&format!("Join channel error: {}", e)))
    }

    pub async fn send_message(&self, channel_id: String, text: String) -> Result<(), JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        room.send_text_to(&channel_id, &text).await
            .map_err(|e| JsValue::from_str(&format!("Send error: {}", e)))
    }

    // 把新成员拉进当前频道
    pub async fn add_member_to_channel(&self, channel_id: String, uid: String) -> Result<(), JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        room.channel_add_peer(&channel_id, &kaede_protocol::UserId::new(uid)).await
            .map_err(|e| JsValue::from_str(&format!("Add member error: {}", e)))
    }

    // 处理频道的 Welcome / Commit
    pub async fn process_channel_sync(&self, channel_id: String, payload: Vec<u8>) -> Result<(), JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        let _ = room.connection().channel_process_commit(&channel_id, &payload).await
            .map_err(|e| JsValue::from_str(&format!("Process sync error: {}", e)))?;
        Ok(())
    }

    // 拉取指定频道的历史
    // === 屏幕共享 / 语音 ===

    /// 获取 LiveKit token + E2EE seed
    pub async fn begin_media_session(&self, channel_id: String) -> Result<JsValue, JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        
        let (token, seed_opt) = room.begin_media_session(&channel_id).await
            .map_err(|e| JsValue::from_str(&format!("Begin media session: {}", e)))?;
        
        let e2ee_seed = seed_opt.map(|s| {
            // Zeroizing<[u8;32]> → base64url
            base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, s.as_ref())
        });
        
        let result = serde_json::json!({
            "token": token.token,
            "ws_url": token.ws_url,
            "e2ee_seed": e2ee_seed,
        });
        
        Ok(JsValue::from_str(&serde_json::to_string(&result).unwrap()))
    }

    /// 离开媒体房间
    pub async fn leave_media_room(&self, channel_id: String) -> Result<(), JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        room.leave_media_room(&channel_id).await
            .map_err(|e| JsValue::from_str(&format!("Leave media: {}", e)))
    }

    /// Debug: 打印频道群组状态（DEV 用）
    pub fn debug_channel_state(&self, channel_id: String) -> Result<JsValue, JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        let conn = room.connection();

        let has_group = conn.channel_has_group(&channel_id);
        let epoch = conn.channel_epoch(&channel_id);
        let key_b64 = if has_group {
            conn.channel_export_key(&channel_id).ok()
                .map(|k| base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, k))
        } else {
            None
        };

        // 也打印主群状态，便于对比
        let main_epoch = conn.mls_epoch();
        let main_ready = conn.mls_is_ready();

        let result = serde_json::json!({
            "has_group": has_group,
            "epoch": epoch,
            "key_b64": key_b64,
            "main_epoch": main_epoch,
            "main_ready": main_ready,
        });

        Ok(JsValue::from_str(&serde_json::to_string(&result).unwrap()))
    }

    pub async fn load_history(&self, channel_id: String, limit: usize) -> Result<JsValue, JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        let msgs = room.load_history(&channel_id, limit).await
            .map_err(|e| JsValue::from_str(&format!("History error: {}", e)))?;
        let json_msgs: Vec<serde_json::Value> = msgs.into_iter().map(|m| {
            serde_json::json!({
                "sender": m.sender,
                "ts": m.ts,
                "plaintext": m.plaintext,
            })
        }).collect();
        Ok(JsValue::from_str(&serde_json::to_string(&json_msgs).unwrap()))
    }
}
