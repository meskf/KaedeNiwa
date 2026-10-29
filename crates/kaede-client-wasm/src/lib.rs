use js_sys::Function;
use kaede_client::room::RoomSession;
use kaede_client::{ClientEvent, ClientObserver};
use std::sync::{Arc, Mutex};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}

/// Init WebCrypto keystore (DEK).
/// Idempotent: internal IndexedDB KEK protects the DEK across reloads.
#[wasm_bindgen(js_name = initKeystore)]
pub async fn init_keystore() -> Result<(), JsValue> {
    kaede_client::keystore_web::init_web_keystore()
        .await
        .map_err(|e| JsValue::from_str(&format!("keystore init failed: {}", e)))
}

struct WasmObserver {
    callback: Arc<Mutex<Option<Function>>>,
    queue: Arc<Mutex<Vec<ClientEvent>>>,
}

impl ClientObserver for WasmObserver {
    fn on_event(&self, event: ClientEvent) {
        // 【死锁修复】不能在持有 callback 锁时调 JS。
        // 否则 JS 回调里调 acceptPendingWelcome → emit PendingWelcomeResolved
        // → 再次进入 on_event → 递归获取 callback 锁 → wasm panic。
        let cb_opt = {
            let cb_borrow = self.callback.lock().unwrap();
            cb_borrow.clone()
        };
        if let Some(cb) = cb_opt {
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
        // 先 drain 到本地 Vec，释放 queue 锁，再 clone callback，最后调 JS。
        let queued: Vec<ClientEvent> = {
            let mut queue = self.observer.queue.lock().unwrap();
            if queue.is_empty() { return; }
            queue.drain(..).collect()
        };
        let cb_opt = {
            let cb_borrow = self.observer.callback.lock().unwrap();
            cb_borrow.clone()
        };
        if let Some(cb) = cb_opt {
            for ev in queued {
                if let Ok(json) = serde_json::to_string(&ev) {
                    let js_val = JsValue::from_str(&json);
                    let _ = cb.call1(&JsValue::NULL, &js_val);
                }
            }
        }
    }

    pub async fn login(
        &self,
        http_url: String,
        ws_url: String,
        login_id: String,
        token: String,
    ) -> Result<(), JsValue> {
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
            &http_url,
            &ws_url,
            &login_id,
            &token,
            storage,
            Some(obs),
        )
        .await
        .map_err(|e| JsValue::from_str(&format!("Connect error: {}", e)))?;

        let room = Arc::new(room);
        room.spawn_event_loop();
        // P0-2：启动心跳（每 20s Ping，30s 无 Pong 断开）
        // P0-2: room.connection() 返回 &Arc，spawn_heartbeat 要 owned，先 clone
        room.connection().clone().spawn_heartbeat();
        room.join_room()
            .await
            .map_err(|e| JsValue::from_str(&format!("Join error: {}", e)))?;

        // become_host / become_guest 的决策已上移到 RoomSession::join_room，
        // 这里不再重复调用（避免重复发布 KeyPackage）。

        *self.room.lock().unwrap() = Some(room);
        Ok(())
    }

    /// 主群 host 主动把新登录的 peer 加进主群（含 Add + Welcome + Commit）。
    /// 幂等：peer 已在群时是 no-op。
    #[wasm_bindgen(js_name = hostAddPeer)]
    pub async fn host_add_peer(&self, peer: String) -> Result<(), JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        let uid = kaede_protocol::UserId::from(peer);
        room.host_add_peer(&uid)
            .await
            .map_err(|e| JsValue::from_str(&format!("hostAddPeer: {}", e)))
    }

    /// 主群是否处于"我是第一个成员（host）"状态。
    /// true = 收到 PeerJoined 时应该调 hostAddPeer 拉人。
    #[wasm_bindgen(js_name = isMlsHost)]
    pub fn is_mls_host(&self) -> bool {
        let room_opt = self.room.lock().unwrap().clone();
        if let Some(room) = room_opt {
            let conn = room.connection();
            conn.mls_is_ready() && conn.mls_member_uids().len() == 1
        } else {
            false
        }
    }

    /// 用户确认接受来自 sender 的主群 Welcome（Signal 模型的 UI 回调）。
    #[wasm_bindgen(js_name = acceptPendingWelcome)]
    pub fn accept_pending_welcome(&self, sender_login_id: String) -> Result<(), JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        room.connection()
            .accept_pending_welcome(&sender_login_id)
            .map_err(|e| JsValue::from_str(&format!("acceptPendingWelcome: {}", e)))
    }

    /// 用户拒绝来自 sender 的主群 Welcome。
    #[wasm_bindgen(js_name = rejectPendingWelcome)]
    pub fn reject_pending_welcome(&self, sender_login_id: String) -> Result<(), JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        room.connection().reject_pending_welcome(&sender_login_id);
        Ok(())
    }

    /// 列出当前所有 pending 主群 Welcome（UI 渲染对话框用）。
    #[wasm_bindgen(js_name = listPendingWelcomes)]
    pub fn list_pending_welcomes(&self) -> Result<JsValue, JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        let list = room.connection().pending_welcome_info();
        let json: Vec<serde_json::Value> = list.iter().map(|p| serde_json::json!({
            "sender_login_id": p.sender_login_id,
            "is_rebuild": p.is_rebuild,
            "age_secs": p.age_secs,
        })).collect();
        Ok(JsValue::from_str(&serde_json::to_string(&json).unwrap_or_else(|_| "[]".into())))
    }

    /// P0-1 测试钩子：模拟网络错误断开 WS，触发 JS 层自动重连。
    #[wasm_bindgen(js_name = forceDisconnect)]
    pub async fn force_disconnect(&self) -> Result<(), JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        room.connection()
            .force_close_for_reconnect()
            .await
            .map_err(|e| JsValue::from_str(&format!("force_disconnect error: {}", e)))
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
        // 完整 welcome 流：提交 KP → 等待 Welcome → 加入共享 group
        // （旧的 join_channel_only 不提交 KP，无法触发欢迎流程，导致 voice 双人各自 solo group）
        room.join_channel(&channel_id, "")
            .await
            .map_err(|e| JsValue::from_str(&format!("Join channel error: {}", e)))
    }

    pub async fn send_message(&self, channel_id: String, text: String) -> Result<(), JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        room.send_text_to(&channel_id, &text)
            .await
            .map_err(|e| JsValue::from_str(&format!("Send error: {}", e)))
    }

    // 把新成员拉进当前频道
    pub async fn add_member_to_channel(
        &self,
        channel_id: String,
        uid: String,
    ) -> Result<(), JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        room.channel_add_peer(&channel_id, &kaede_protocol::UserId::new(uid))
            .await
            .map_err(|e| JsValue::from_str(&format!("Add member error: {}", e)))
    }

    // 处理频道的 Welcome / Commit
    pub async fn process_channel_sync(
        &self,
        channel_id: String,
        payload: Vec<u8>,
    ) -> Result<(), JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        let _ = room
            .connection()
            .channel_process_commit(&channel_id, &payload)
            .await
            .map_err(|e| JsValue::from_str(&format!("Process sync error: {}", e)))?;
        Ok(())
    }

    // 拉取指定频道的历史
    // === 附件 ===

    /// 上传附件到 target（频道 id 或 DM conv_id）。
    #[wasm_bindgen(js_name = uploadAttachment)]
    pub async fn upload_attachment(
        &self,
        target_id: String,
        name: String,
        mime: Option<String>,
        bytes: Vec<u8>,
    ) -> Result<JsValue, JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        let att = room
            .upload_attachment(&target_id, &name, mime.as_deref(), bytes)
            .await
            .map_err(|e| JsValue::from_str(&format!("upload attachment: {e}")))?;
        Ok(JsValue::from_str(&serde_json::to_string(&att).unwrap()))
    }

    /// 下载并解密附件。返回明文字节（Uint8Array）。
    #[wasm_bindgen(js_name = downloadAttachment)]
    pub async fn download_attachment(
        &self,
        target_id: String,
        id: String,
        expected_sha256: Option<String>,
    ) -> Result<Vec<u8>, JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        room.download_attachment(&target_id, &id, expected_sha256.as_deref())
            .await
            .map_err(|e| JsValue::from_str(&format!("download attachment: {e}")))
    }

    /// 发送带附件的频道消息。
    #[wasm_bindgen(js_name = sendMessageWithAttachments)]
    pub async fn send_message_with_attachments(
        &self,
        channel_id: String,
        text: String,
        attachments_json: String,
    ) -> Result<(), JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        let atts: Vec<kaede_protocol::Attachment> = serde_json::from_str(&attachments_json)
            .map_err(|e| JsValue::from_str(&format!("parse attachments: {e}")))?;
        room.send_text_with_attachments(&channel_id, &text, &atts)
            .await
            .map_err(|e| JsValue::from_str(&format!("send with attachments: {e}")))
    }

    /// 发送带附件的 DM。
    #[wasm_bindgen(js_name = sendDmWithAttachments)]
    pub async fn send_dm_with_attachments(
        &self,
        conv_id: String,
        text: String,
        attachments_json: String,
    ) -> Result<(), JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        let atts: Vec<kaede_protocol::Attachment> = serde_json::from_str(&attachments_json)
            .map_err(|e| JsValue::from_str(&format!("parse attachments: {e}")))?;
        room.send_dm_with_attachments(&conv_id, &text, &atts)
            .await
            .map_err(|e| JsValue::from_str(&format!("send dm with attachments: {e}")))
    }

    // === 屏幕共享 / 语音 ===

    /// 获取 LiveKit token + E2EE seed
    pub async fn begin_media_session(&self, channel_id: String) -> Result<JsValue, JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;

        let (token, seed_opt) = room
            .begin_media_session(&channel_id)
            .await
            .map_err(|e| JsValue::from_str(&format!("Begin media session: {}", e)))?;

        let e2ee_seed = seed_opt.map(|s| {
            // Zeroizing<[u8;32]> → base64url
            base64::Engine::encode(
                &base64::engine::general_purpose::URL_SAFE_NO_PAD,
                s.as_ref(),
            )
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
        room.leave_media_room(&channel_id)
            .await
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
            conn.channel_export_key(&channel_id).ok().map(|k| {
                base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, k)
            })
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
        let msgs = room
            .load_history(&channel_id, limit)
            .await
            .map_err(|e| JsValue::from_str(&format!("History error: {}", e)))?;
        let json_msgs: Vec<serde_json::Value> = msgs
            .into_iter()
            .map(|m| {
                serde_json::json!({
                    "sender": m.sender,
                    "ts": m.ts,
                    "plaintext": m.plaintext,
                })
            })
            .collect();
        Ok(JsValue::from_str(
            &serde_json::to_string(&json_msgs).unwrap(),
        ))
    }

    // ============ DM wasm 桥 ============
    // 对应的 RoomSession 方法已存在，这里只做 JSON 序列化 + JsValue 包装

    #[wasm_bindgen(js_name = listDmConversations)]
    pub async fn list_dm_conversations(&self) -> Result<JsValue, JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        let convs = room
            .list_dm_conversations()
            .await
            .map_err(|e| JsValue::from_str(&format!("list dm conversations: {}", e)))?;
        let json = serde_json::to_string(&convs)
            .map_err(|e| JsValue::from_str(&format!("serialize: {}", e)))?;
        Ok(JsValue::from_str(&json))
    }

    #[wasm_bindgen(js_name = openDm)]
    pub async fn open_dm(&self, peer: String) -> Result<JsValue, JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        let conv = room
            .open_dm(&peer)
            .await
            .map_err(|e| JsValue::from_str(&format!("open dm: {}", e)))?;
        let json = serde_json::to_string(&conv)
            .map_err(|e| JsValue::from_str(&format!("serialize: {}", e)))?;
        Ok(JsValue::from_str(&json))
    }

    #[wasm_bindgen(js_name = dmLoadHistory)]
    pub async fn dm_load_history(&self, conv_id: String, limit: usize) -> Result<JsValue, JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        let history = room
            .dm_load_history(&conv_id, None, limit)
            .await
            .map_err(|e| JsValue::from_str(&format!("dm history: {}", e)))?;
        let json = serde_json::to_string(&history)
            .map_err(|e| JsValue::from_str(&format!("serialize: {}", e)))?;
        Ok(JsValue::from_str(&json))
    }

    #[wasm_bindgen(js_name = sendDm)]
    pub async fn send_dm(&self, conv_id: String, text: String) -> Result<(), JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        room.send_dm(&conv_id, &text)
            .await
            .map_err(|e| JsValue::from_str(&format!("send dm: {}", e)))
    }

    #[wasm_bindgen(js_name = markDmRead)]
    pub async fn mark_dm_read(&self, conv_id: String, ts: i64) -> Result<(), JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        room.mark_dm_read(&conv_id, ts)
            .await
            .map_err(|e| JsValue::from_str(&format!("mark dm read: {}", e)))
    }

    // ============ Channel list wasm 桥 ============

    #[wasm_bindgen(js_name = joinChannelWithPassword)]
    pub async fn join_channel_with_password(
        &self,
        channel_id: String,
        password: String,
    ) -> Result<(), JsValue> {
        let room = {
            let guard = self.room.lock().unwrap();
            guard.as_ref().cloned()
        };
        let Some(room) = room else {
            return Err(JsValue::from_str("not logged in"));
        };
        room.join_channel(&channel_id, &password)
            .await
            .map_err(|e| JsValue::from_str(&format!("join channel: {}", e)))
    }

    #[wasm_bindgen(js_name = sendTyping)]
    pub async fn send_typing(&self, channel_id: String) -> Result<(), JsValue> {
        let room = {
            let guard = self.room.lock().unwrap();
            guard.as_ref().cloned()
        };
        let Some(room) = room else {
            return Err(JsValue::from_str("not logged in"));
        };
        room.send_typing(&channel_id)
            .await
            .map_err(|e| JsValue::from_str(&format!("typing error: {}", e)))
    }

    #[wasm_bindgen(js_name = sendReaction)]
    pub async fn send_reaction(
        &self,
        channel_id: String,
        target_ts: i64,
        emoji: String,
        action: String,
    ) -> Result<(), JsValue> {
        let room = {
            let guard = self.room.lock().unwrap();
            guard.as_ref().cloned()
        };
        let Some(room) = room else {
            return Err(JsValue::from_str("not logged in"));
        };
        room.send_reaction(&channel_id, target_ts, &emoji, &action)
            .await
            .map_err(|e| JsValue::from_str(&format!("reaction error: {}", e)))
    }

    #[wasm_bindgen(js_name = setPresence)]
    pub async fn set_presence(&self, status: String) -> Result<(), JsValue> {
        let room = {
            let guard = self.room.lock().unwrap();
            guard.as_ref().cloned()
        };
        let Some(room) = room else {
            return Err(JsValue::from_str("not logged in"));
        };
        room.set_presence(&status)
            .await
            .map_err(|e| JsValue::from_str(&format!("presence error: {}", e)))
    }

    
    #[wasm_bindgen(js_name = listNodes)]
    pub async fn list_nodes(&self) -> Result<JsValue, JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        let nodes = room.list_nodes().await.map_err(|e| JsValue::from_str(&format!("list nodes: {}", e)))?;
        let arr = js_sys::Array::new();
        for n in nodes {
            arr.push(&JsValue::from_str(&serde_json::to_string(&n).unwrap_or_default()));
        }
        Ok(arr.into())
    }

    #[wasm_bindgen(js_name = createNode)]
    pub async fn create_node(&self, name: String, kind: String, parent_id: Option<String>, is_private: bool, password: Option<String>) -> Result<JsValue, JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        let node = room.create_node(&name, &kind, parent_id.as_deref(), is_private, password.as_deref()).await.map_err(|e| JsValue::from_str(&format!("create node: {}", e)))?;
        Ok(JsValue::from_str(&serde_json::to_string(&node).unwrap_or_default()))
    }

    #[wasm_bindgen(js_name = updateNode)]
    pub async fn update_node(&self, id: String, name: Option<String>, parent_id: Option<String>, position: Option<i64>) -> Result<(), JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        room.update_node(&id, name.as_deref(), parent_id.as_deref(), position).await.map_err(|e| JsValue::from_str(&format!("update node: {}", e)))
    }

    #[wasm_bindgen(js_name = deleteNode)]
    pub async fn delete_node(&self, id: String) -> Result<(), JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        room.delete_node(&id).await.map_err(|e| JsValue::from_str(&format!("delete node: {}", e)))
    }

    #[wasm_bindgen(js_name = listChannels)]
    pub async fn list_channels(&self) -> Result<JsValue, JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        let channels = room
            .list_channels()
            .await
            .map_err(|e| JsValue::from_str(&format!("list channels: {}", e)))?;
        let json = serde_json::to_string(&channels)
            .map_err(|e| JsValue::from_str(&format!("serialize: {}", e)))?;
        Ok(JsValue::from_str(&json))
    }

    // ============ Channel CRUD wasm 桥 ============

    #[wasm_bindgen(js_name = createChannel)]
    pub async fn create_channel_js(
        &self,
        name: String,
        kind: String,
        category_id: Option<String>,
        is_private: Option<bool>,
        password: Option<String>,
    ) -> Result<JsValue, JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        let ch = room
            .create_channel_with(
                &name,
                &kind,
                is_private.unwrap_or(false),
                password.as_deref(),
                None,
                category_id.as_deref(),
            )
            .await
            .map_err(|e| JsValue::from_str(&format!("create channel: {}", e)))?;
        let json = serde_json::to_string(&ch)
            .map_err(|e| JsValue::from_str(&format!("serialize: {}", e)))?;
        Ok(JsValue::from_str(&json))
    }

    #[wasm_bindgen(js_name = updateChannel)]
    pub async fn update_channel_js(
        &self,
        id: String,
        name: Option<String>,
        category_id: Option<String>,
    ) -> Result<JsValue, JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        let patch = kaede_client::http::UpdateChannelReq {
            name,
            position: None,
            category_id,
        };
        let ch = room.update_channel(&id, patch).await
            .map_err(|e| JsValue::from_str(&format!("update channel: {}", e)))?;
        let json = serde_json::to_string(&ch)
            .map_err(|e| JsValue::from_str(&format!("serialize: {}", e)))?;
        Ok(JsValue::from_str(&json))
    }

    #[wasm_bindgen(js_name = deleteChannel)]
    pub async fn delete_channel_js(&self, id: String) -> Result<(), JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        room.delete_channel(&id).await
            .map_err(|e| JsValue::from_str(&format!("delete channel: {}", e)))
    }

    // ============ Category wasm 桥 ============

    #[wasm_bindgen(js_name = listCategories)]
    pub async fn list_categories(&self) -> Result<JsValue, JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        let cats = room.list_categories().await
            .map_err(|e| JsValue::from_str(&format!("list categories: {}", e)))?;
        let json = serde_json::to_string(&cats)
            .map_err(|e| JsValue::from_str(&format!("serialize: {}", e)))?;
        Ok(JsValue::from_str(&json))
    }

    #[wasm_bindgen(js_name = createCategory)]
    pub async fn create_category(&self, name: String, position: Option<i32>) -> Result<JsValue, JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        let cat = room.create_category(&name, position.map(|p| p as i64)).await
            .map_err(|e| JsValue::from_str(&format!("create category: {}", e)))?;
        let json = serde_json::to_string(&cat)
            .map_err(|e| JsValue::from_str(&format!("serialize: {}", e)))?;
        Ok(JsValue::from_str(&json))
    }

    #[wasm_bindgen(js_name = updateCategory)]
    pub async fn update_category(&self, id: String, name: Option<String>, position: Option<i32>) -> Result<JsValue, JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        let cat = room.update_category(&id, name.as_deref(), position.map(|p| p as i64)).await
            .map_err(|e| JsValue::from_str(&format!("update category: {}", e)))?;
        let json = serde_json::to_string(&cat)
            .map_err(|e| JsValue::from_str(&format!("serialize: {}", e)))?;
        Ok(JsValue::from_str(&json))
    }

    #[wasm_bindgen(js_name = deleteCategory)]
    pub async fn delete_category(&self, id: String) -> Result<(), JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        room.delete_category(&id).await
            .map_err(|e| JsValue::from_str(&format!("delete category: {}", e)))
    }

    // ============ Friend wasm 桥 ============
    // 对应的 RoomSession 方法在 room.rs:2399-2414 已实现

    #[wasm_bindgen(js_name = listFriends)]
    pub async fn list_friends(&self) -> Result<JsValue, JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        let friends = room
            .list_friends()
            .await
            .map_err(|e| JsValue::from_str(&format!("list friends: {}", e)))?;
        let json = serde_json::to_string(&friends)
            .map_err(|e| JsValue::from_str(&format!("serialize: {}", e)))?;
        Ok(JsValue::from_str(&json))
    }

    #[wasm_bindgen(js_name = sendFriendRequest)]
    pub async fn send_friend_request(&self, login_id: String) -> Result<(), JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        room.send_friend_request(&login_id)
            .await
            .map_err(|e| JsValue::from_str(&format!("send friend request: {}", e)))
    }

    #[wasm_bindgen(js_name = acceptFriend)]
    pub async fn accept_friend(&self, from_login_id: String) -> Result<(), JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        room.accept_friend(&from_login_id)
            .await
            .map_err(|e| JsValue::from_str(&format!("accept friend: {}", e)))
    }

    #[wasm_bindgen(js_name = removeFriend)]
    pub async fn remove_friend(&self, login_id: String) -> Result<(), JsValue> {
        let room_opt = self.room.lock().unwrap().clone();
        let room = room_opt.ok_or_else(|| JsValue::from_str("not logged in"))?;
        room.remove_friend(&login_id)
            .await
            .map_err(|e| JsValue::from_str(&format!("remove friend: {}", e)))
    }
}


