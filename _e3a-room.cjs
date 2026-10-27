const fs = require('fs')
const p = 'crates/kaede-client/src/room.rs'
let s = fs.readFileSync(p, 'utf8')
const anchor = `    /// 创建频道（需 op3+）。`
const insert = `    /// 分类列表。
    pub async fn list_categories(&self) -> Result<Vec<kaede_protocol::Category>, ClientError> {
        self.http.list_categories().await
    }

    /// 创建分类（需 op3+）。
    pub async fn create_category(&self, name: &str, position: Option<i64>) -> Result<kaede_protocol::Category, ClientError> {
        self.http.create_category(name, position).await
    }

    /// 更新分类（需 op3+）。
    pub async fn update_category(&self, id: &str, name: Option<&str>, position: Option<i64>) -> Result<kaede_protocol::Category, ClientError> {
        self.http.update_category(id, name, position).await
    }

    /// 删除分类（需 op3+）。
    pub async fn delete_category(&self, id: &str) -> Result<(), ClientError> {
        self.http.delete_category(id).await
    }

` + anchor
if (!s.includes(anchor)) { console.error('FAIL room.rs anchor'); process.exit(1) }
s = s.replace(anchor, insert)
fs.writeFileSync(p, s)
console.log('OK room.rs')
