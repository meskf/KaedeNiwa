const fs = require('fs')
const log = []
function patchFile(path, fn) {
  try {
    let s = fs.readFileSync(path, 'utf8')
    s = fn(s)
    fs.writeFileSync(path, s)
  } catch (e) {
    log.push('FAIL ' + path + ': ' + e.message)
    throw e
  }
}
function rep(s, from, to, name) {
  if (!s.includes(from)) { log.push('MISS: ' + name); return s }
  log.push('OK: ' + name)
  return s.replace(from, to)
}

// ===== 1. http.rs =====
patchFile('crates/kaede-client/src/http.rs', s => {
  // CreateChannelReq 加 category_id
  s = rep(s,
`pub struct CreateChannelReq {
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub is_private: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash_tier: Option<String>,
}`,
`pub struct CreateChannelReq {
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub is_private: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash_tier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category_id: Option<String>,
}`,
  'http CreateChannelReq.category_id')

  // UpdateChannelReq 加 category_id
  s = rep(s,
`pub struct UpdateChannelReq {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<i64>,
}`,
`pub struct UpdateChannelReq {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_id: Option<String>,
}`,
  'http UpdateChannelReq.category_id')

  // create_channel 简版 delegate
  s = rep(s,
`        self.create_channel_with(name, kind, false, None, None)
            .await`,
`        self.create_channel_with(name, kind, false, None, None, None)
            .await`,
  'http create_channel delegate')

  // create_channel_with 签名
  s = rep(s,
`    pub async fn create_channel_with(
        &self,
        name: &str,
        kind: &str,
        is_private: bool,
        password: Option<&str>,
        hash_tier: Option<&str>,
    ) -> Result<kaede_protocol::Channel, ClientError> {`,
`    pub async fn create_channel_with(
        &self,
        name: &str,
        kind: &str,
        is_private: bool,
        password: Option<&str>,
        hash_tier: Option<&str>,
        category_id: Option<&str>,
    ) -> Result<kaede_protocol::Channel, ClientError> {`,
  'http create_channel_with sig')

  // 构造体加 category_id
  s = rep(s,
`            .json(&CreateChannelReq {
                name: name.into(),
                kind: kind.into(),
                is_private,
                password: password.map(|s| s.to_string()),
                hash_tier: hash_tier.map(|s| s.to_string()),
            })`,
`            .json(&CreateChannelReq {
                name: name.into(),
                kind: kind.into(),
                is_private,
                password: password.map(|s| s.to_string()),
                hash_tier: hash_tier.map(|s| s.to_string()),
                category_id: category_id.map(|s| s.to_string()),
            })`,
  'http CreateChannelReq construct')

  return s
})

// ===== 2. room.rs =====
patchFile('crates/kaede-client/src/room.rs', s => {
  s = rep(s,
`    pub async fn create_channel_with(
        &self,
        name: &str,
        kind: &str,
        is_private: bool,
        password: Option<&str>,
        hash_tier: Option<&str>,
    ) -> Result<kaede_protocol::Channel, ClientError> {
        let ch = self
            .http
            .create_channel_with(name, kind, is_private, password, hash_tier)
            .await?;`,
`    pub async fn create_channel_with(
        &self,
        name: &str,
        kind: &str,
        is_private: bool,
        password: Option<&str>,
        hash_tier: Option<&str>,
        category_id: Option<&str>,
    ) -> Result<kaede_protocol::Channel, ClientError> {
        let ch = self
            .http
            .create_channel_with(name, kind, is_private, password, hash_tier, category_id)
            .await?;`,
  'room create_channel_with')
  return s
})

// ===== 3. wasm lib.rs: 3 个桥 =====
patchFile('crates/kaede-client-wasm/src/lib.rs', s => {
  const anchor = `    // ============ Category wasm 桥 ============`
  const insert = `    // ============ Channel CRUD wasm 桥 ============

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

` + anchor
  if (!s.includes(anchor)) throw new Error('wasm anchor missing')
  s = s.replace(anchor, insert)
  log.push('OK: wasm channel CRUD bridges')
  return s
})

log.forEach(l => console.log(l))
console.log('DONE')
