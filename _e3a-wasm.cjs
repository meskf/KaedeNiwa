const fs = require('fs')
const p = 'crates/kaede-client-wasm/src/lib.rs'
let s = fs.readFileSync(p, 'utf8')
const anchor = `    // ============ Friend wasm 桥 ============`
const insert = `    // ============ Category wasm 桥 ============

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

` + anchor
if (!s.includes(anchor)) { console.error('FAIL wasm anchor'); process.exit(1) }
s = s.replace(anchor, insert)
fs.writeFileSync(p, s)
console.log('OK wasm lib.rs')
