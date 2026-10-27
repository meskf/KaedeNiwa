const fs = require('fs')
const p = 'crates/kaede-client/src/http.rs'
let s = fs.readFileSync(p, 'utf8')
const log = []
function rep(from, to, name) {
  if (!s.includes(from)) { log.push('MISS: ' + name); return }
  s = s.replace(from, to)
  log.push('OK: ' + name)
}

// 在 list_channels 后面插 list_categories 系列
const anchor = `    pub async fn create_channel(
        &self,
        name: &str,
        kind: &str,
    ) -> Result<kaede_protocol::Channel, ClientError> {`
const insert = `    /// 分类列表（GET /api/categories）
    pub async fn list_categories(&self) -> Result<Vec<kaede_protocol::Category>, ClientError> {
        #[derive(Deserialize)]
        struct Resp { categories: Vec<kaede_protocol::Category> }
        let resp = self
            .http
            .get(format!("{}/api/categories", self.base_url))
            .bearer_auth(self.bearer()?)
            .send()
            .await
            .map_err(map_reqwest_err)?;
        let body = parse_json::<Resp>(resp).await?;
        Ok(body.categories)
    }

    /// 新建分类（需 op3+）。
    pub async fn create_category(
        &self,
        name: &str,
        position: Option<i64>,
    ) -> Result<kaede_protocol::Category, ClientError> {
        #[derive(Serialize)]
        struct Req<'a> { name: &'a str, #[serde(skip_serializing_if = "Option::is_none")] position: Option<i64> }
        let resp = self
            .http
            .post(format!("{}/api/categories", self.base_url))
            .bearer_auth(self.bearer()?)
            .json(&Req { name, position })
            .send()
            .await
            .map_err(map_reqwest_err)?;
        parse_json::<kaede_protocol::Category>(resp).await
    }

    /// 更新分类（需 op3+）。
    pub async fn update_category(
        &self,
        id: &str,
        name: Option<&str>,
        position: Option<i64>,
    ) -> Result<kaede_protocol::Category, ClientError> {
        #[derive(Serialize)]
        struct Req<'a> {
            #[serde(skip_serializing_if = "Option::is_none")] name: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")] position: Option<i64>,
        }
        let resp = self
            .http
            .patch(format!("{}/api/categories/{}", self.base_url, id))
            .bearer_auth(self.bearer()?)
            .json(&Req { name, position })
            .send()
            .await
            .map_err(map_reqwest_err)?;
        parse_json::<kaede_protocol::Category>(resp).await
    }

    /// 删除分类（需 op3+）。分类下的频道会变为未分类。
    pub async fn delete_category(&self, id: &str) -> Result<(), ClientError> {
        let resp = self
            .http
            .delete(format!("{}/api/categories/{}", self.base_url, id))
            .bearer_auth(self.bearer()?)
            .send()
            .await
            .map_err(map_reqwest_err)?;
        if resp.status().is_success() { Ok(()) } else { Err(parse_api_error(resp).await) }
    }

` + anchor
rep(anchor, insert, 'http.rs list_categories')

fs.writeFileSync(p, s)
log.forEach(l => console.log(l))
