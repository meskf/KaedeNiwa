//! HTTP API 客户端。
//!
//! 对应 `kaede-server` 的 HTTP 端点：
//!
//! | 方法 | 路径 | 用途 |
//! |---|---|---|
//! | POST | `/api/register` | 注册 |
//! | POST | `/api/login` | 登录 |
//! | GET  | `/api/me` | 取当前账户 |
//! | POST | `/api/keypackages` | 上传 KeyPackage 队列 |
//! | GET  | `/api/keypackages/{uid}` | 拉取并消费一个 KeyPackage |

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use kaede_protocol::UserId;
use serde::{Deserialize, Serialize};

use crate::error::ClientError;

/// HTTP 客户端。
///
/// 每个 server 一个实例。
#[derive(Clone)]
pub struct HttpApi {
    base_url: String,
    http: reqwest::Client,
    /// 登录后由 [`HttpApi::set_token`] 设置。
    token: std::sync::Arc<std::sync::Mutex<Option<String>>>,
}

// ---------------------------------------------------------------------------
// 请求 / 响应结构
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct RegisterReq {
    pub login_id: String,
    pub password: String,
    #[serde(default)]
    pub nickname: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LoginReq {
    pub login_id: String,
    pub password: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthResp {
    pub login_id: String,
    pub nickname: String,
    pub session_token: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MeResp {
    pub login_id: String,
    pub nickname: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublishKeyPackagesReq {
    /// base64(URL-safe, no pad) 的 KeyPackage 列表。
    pub packages: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FetchKeyPackageResp {
    pub target: String,
    /// base64(URL-safe, no pad)
    pub package: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PostMessageReq {
    pub channel_id: String,
    pub ts: i64,
    pub sender: String,
    pub ciphertext: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MessageRecord {
    pub ts: i64,
    pub sender: String,
    pub ciphertext: String,
}

#[derive(Debug, Clone, Deserialize)]
struct MessagesResp {
    messages: Vec<MessageRecord>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateChannelReq {
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub is_private: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash_tier: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JoinChannelReq {
    pub password: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_package: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct UpdateChannelReq {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct UpdateMemberReq {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bio: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub op: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ChannelsResp {
    channels: Vec<kaede_protocol::Channel>,
}

#[derive(Debug, Clone, Deserialize)]
struct MembersResp {
    members: Vec<kaede_protocol::Member>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DmConversation {
    pub conv_id: String,
    pub peer_login_id: String,
    pub peer_nickname: String,
}

#[derive(Debug, Clone, Deserialize)]
struct DmConversationsResp {
    conversations: Vec<DmConversation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Friend {
    pub login_id: String,
    pub nickname: String,
    /// `"outgoing"` | `"incoming"` | `"friend"`
    pub direction: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct FriendsResp {
    friends: Vec<Friend>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LivekitToken {
    pub token: String,
    pub ws_url: String,
    pub room: String,
    pub identity: String,
    /// P0-2: 房间级 e2ee seed（base64 URL-safe no-pad）。
    /// 老 server 不返回时为 None → `begin_media_session` 会报错要求升级。
    #[serde(default)]
    pub e2ee_seed: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ErrorResp {
    error: String,
    message: String,
}

// ---------------------------------------------------------------------------
// 实现
// ---------------------------------------------------------------------------

impl HttpApi {
    /// 新建。`base_url` 形如 `http://localhost:8080`。
    pub fn new(base_url: impl Into<String>) -> Result<Self, ClientError> {
        #[allow(unused_mut)]
        let mut builder = reqwest::Client::builder()
            .user_agent(concat!("kaede-client/", env!("CARGO_PKG_VERSION")));
        #[cfg(not(target_arch = "wasm32"))]
        {
            builder = builder
                .timeout(std::time::Duration::from_secs(30))
                .connect_timeout(std::time::Duration::from_secs(10));
        }
        let http = builder
            .build()
            .map_err(|e| ClientError::Transport(format!("build http client: {}", e)))?;
        Ok(Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            http,
            token: std::sync::Arc::new(std::sync::Mutex::new(None)),
        })
    }

    /// 设置 session token（登录/注册成功后调用）。
    pub fn set_token(&self, token: impl Into<String>) {
        *self.token.lock().unwrap() = Some(token.into());
    }

    /// 清除 token。
    pub fn clear_token(&self) {
        *self.token.lock().unwrap() = None;
    }

    /// 当前 token 副本。
    pub fn token(&self) -> Option<String> {
        self.token.lock().unwrap().clone()
    }

    // -----------------------------------------------------------------
    // 端点
    // -----------------------------------------------------------------

    /// 注册。
    pub async fn register(&self, req: RegisterReq) -> Result<AuthResp, ClientError> {
        let resp = self
            .http
            .post(format!("{}/api/register", self.base_url))
            .json(&req)
            .send()
            .await
            .map_err(map_reqwest_err)?;
        let auth = parse_json::<AuthResp>(resp).await?;
        self.set_token(auth.session_token.clone());
        Ok(auth)
    }

    /// 登录。
    pub async fn login(&self, req: LoginReq) -> Result<AuthResp, ClientError> {
        let resp = self
            .http
            .post(format!("{}/api/login", self.base_url))
            .json(&req)
            .send()
            .await
            .map_err(map_reqwest_err)?;
        let auth = parse_json::<AuthResp>(resp).await?;
        self.set_token(auth.session_token.clone());
        Ok(auth)
    }

    /// 当前账户。
    pub async fn me(&self) -> Result<MeResp, ClientError> {
        let resp = self
            .http
            .get(format!("{}/api/me", self.base_url))
            .bearer_auth(self.bearer()?)
            .send()
            .await
            .map_err(map_reqwest_err)?;
        parse_json::<MeResp>(resp).await
    }

    /// 上传一批 KeyPackage。
    pub async fn publish_key_packages(&self, packages: Vec<Vec<u8>>) -> Result<(), ClientError> {
        let req = PublishKeyPackagesReq {
            packages: packages.iter().map(|p| URL_SAFE_NO_PAD.encode(p)).collect(),
        };
        let resp = self
            .http
            .post(format!("{}/api/keypackages", self.base_url))
            .bearer_auth(self.bearer()?)
            .json(&req)
            .send()
            .await
            .map_err(map_reqwest_err)?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(parse_api_error(resp).await)
        }
    }

    /// 拉取并消费目标用户的一个 KeyPackage。
    pub async fn fetch_key_package(&self, target: &UserId) -> Result<Vec<u8>, ClientError> {
        let resp = self
            .http
            .get(format!(
                "{}/api/keypackages/{}",
                self.base_url,
                target.as_str()
            ))
            .bearer_auth(self.bearer()?)
            .send()
            .await
            .map_err(map_reqwest_err)?;
        let body = parse_json::<FetchKeyPackageResp>(resp).await?;
        // P1-AJ：校验 server 返回的 target，防恶意/故障 server 返回别人的 KP
        if body.target != target.as_str() {
            return Err(ClientError::Protocol(format!(
                "key package target mismatch: requested {}, got {}",
                target, body.target
            )));
        }
        let kp_bytes = URL_SAFE_NO_PAD
            .decode(&body.package)
            .map_err(|e| ClientError::Protocol(format!("bad kp base64: {}", e)))?;
        // H8: 校验 KP 内部 credential 与 target 一致。
        // server 侧 C8 已校验，但恶意/故障 server 仍可能返回错误 KP；
        // 客户端必须独立验证，否则 group state 会被污染。
        let cred = kaede_mls::extract_credential_from_kp(&kp_bytes)
            .map_err(|e| ClientError::Protocol(format!("invalid kp credential: {}", e)))?;
        if cred != target.as_str() {
            return Err(ClientError::Protocol(format!(
                "kp credential mismatch: expected {}, got {}",
                target, cred
            )));
        }
        Ok(kp_bytes)
    }

    /// 保存一条消息（服务端只存密文，不解密）。
    pub async fn post_message(
        &self,
        channel_id: &str,
        ts: i64,
        sender: &str,
        ciphertext: &[u8],
    ) -> Result<(), ClientError> {
        let req = PostMessageReq {
            channel_id: channel_id.into(),
            ts,
            sender: sender.into(),
            ciphertext: URL_SAFE_NO_PAD.encode(ciphertext),
        };
        let resp = self
            .http
            .post(format!("{}/api/messages", self.base_url))
            .bearer_auth(self.bearer()?)
            .json(&req)
            .send()
            .await
            .map_err(map_reqwest_err)?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(parse_api_error(resp).await)
        }
    }

    /// 拉取历史消息。返回的密文由调用方 MLS 解密。
    pub async fn load_messages(
        &self,
        channel_id: &str,
        before_ts: Option<i64>,
        limit: usize,
    ) -> Result<Vec<MessageRecord>, ClientError> {
        let mut url = format!(
            "{}/api/messages/{}?limit={}",
            self.base_url, channel_id, limit
        );
        if let Some(b) = before_ts {
            url.push_str(&format!("&before_ts={}", b));
        }
        let resp = self
            .http
            .get(&url)
            .bearer_auth(self.bearer()?)
            .send()
            .await
            .map_err(map_reqwest_err)?;
        let body = parse_json::<MessagesResp>(resp).await?;
        Ok(body.messages)
    }

    // -----------------------------------------------------------------
    // 频道
    // -----------------------------------------------------------------

    pub async fn list_channels(&self) -> Result<Vec<kaede_protocol::Channel>, ClientError> {
        let resp = self
            .http
            .get(format!("{}/api/channels", self.base_url))
            .bearer_auth(self.bearer()?)
            .send()
            .await
            .map_err(map_reqwest_err)?;
        let body = parse_json::<ChannelsResp>(resp).await?;
        Ok(body.channels)
    }

    pub async fn create_channel(
        &self,
        name: &str,
        kind: &str,
    ) -> Result<kaede_protocol::Channel, ClientError> {
        self.create_channel_with(name, kind, false, None, None)
            .await
    }

    /// 创建频道（扩展版：支持私密 + 密码）。
    pub async fn create_channel_with(
        &self,
        name: &str,
        kind: &str,
        is_private: bool,
        password: Option<&str>,
        hash_tier: Option<&str>,
    ) -> Result<kaede_protocol::Channel, ClientError> {
        let resp = self
            .http
            .post(format!("{}/api/channels", self.base_url))
            .bearer_auth(self.bearer()?)
            .json(&CreateChannelReq {
                name: name.into(),
                kind: kind.into(),
                is_private,
                password: password.map(|s| s.to_string()),
                hash_tier: hash_tier.map(|s| s.to_string()),
            })
            .send()
            .await
            .map_err(map_reqwest_err)?;
        parse_json::<kaede_protocol::Channel>(resp).await
    }

    /// 加入私密频道（校验密码）。
    pub async fn join_channel(
        &self,
        channel_id: &str,
        password: &str,
        key_package: Option<&[u8]>,
    ) -> Result<(), ClientError> {
        let resp = self
            .http
            .post(format!(
                "{}/api/channels/{}/join",
                self.base_url, channel_id
            ))
            .bearer_auth(self.bearer()?)
            .json(&JoinChannelReq {
                password: password.into(),
                key_package: key_package.map(|b| URL_SAFE_NO_PAD.encode(b)),
            })
            .send()
            .await
            .map_err(map_reqwest_err)?;
        if resp.status().is_success() {
            Ok(())
        } else {
            // P1：返回结构化 Http 错误，让上层能根据 status 分派
            // （409 = 单人频道 group 丢失需重建；401 = 密码错）
            let status = resp.status().as_u16();
            let message = format!("join_channel: {}", resp.status());
            Err(ClientError::Http { status, message })
        }
    }

    /// 离开频道。
    pub async fn leave_channel(&self, channel_id: &str) -> Result<(), ClientError> {
        let resp = self
            .http
            .delete(format!(
                "{}/api/channels/{}/leave",
                self.base_url, channel_id
            ))
            .bearer_auth(self.bearer()?)
            .send()
            .await
            .map_err(map_reqwest_err)?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(ClientError::Protocol(format!(
                "leave_channel failed: HTTP {}",
                resp.status()
            )))
        }
    }

    pub async fn update_channel(
        &self,
        id: &str,
        patch: UpdateChannelReq,
    ) -> Result<kaede_protocol::Channel, ClientError> {
        let resp = self
            .http
            .patch(format!("{}/api/channels/{}", self.base_url, id))
            .bearer_auth(self.bearer()?)
            .json(&patch)
            .send()
            .await
            .map_err(map_reqwest_err)?;
        parse_json::<kaede_protocol::Channel>(resp).await
    }

    pub async fn delete_channel(&self, id: &str) -> Result<(), ClientError> {
        let resp = self
            .http
            .delete(format!("{}/api/channels/{}", self.base_url, id))
            .bearer_auth(self.bearer()?)
            .send()
            .await
            .map_err(map_reqwest_err)?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(parse_api_error(resp).await)
        }
    }

    // -----------------------------------------------------------------
    // 成员
    // -----------------------------------------------------------------

    pub async fn list_members(&self) -> Result<Vec<kaede_protocol::Member>, ClientError> {
        let resp = self
            .http
            .get(format!("{}/api/members", self.base_url))
            .bearer_auth(self.bearer()?)
            .send()
            .await
            .map_err(map_reqwest_err)?;
        let body = parse_json::<MembersResp>(resp).await?;
        Ok(body.members)
    }

    pub async fn update_member(
        &self,
        login_id: &str,
        patch: UpdateMemberReq,
    ) -> Result<kaede_protocol::Member, ClientError> {
        let resp = self
            .http
            .patch(format!("{}/api/members/{}", self.base_url, login_id))
            .bearer_auth(self.bearer()?)
            .json(&patch)
            .send()
            .await
            .map_err(map_reqwest_err)?;
        parse_json::<kaede_protocol::Member>(resp).await
    }

    pub async fn remove_member(&self, login_id: &str) -> Result<(), ClientError> {
        let resp = self
            .http
            .delete(format!("{}/api/members/{}", self.base_url, login_id))
            .bearer_auth(self.bearer()?)
            .send()
            .await
            .map_err(map_reqwest_err)?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(parse_api_error(resp).await)
        }
    }

    // -----------------------------------------------------------------
    // 附件
    // -----------------------------------------------------------------

    /// 上传附件。返回服务器分配元数据。
    pub async fn upload_attachment(
        &self,
        name: &str,
        mime: Option<&str>,
        bytes: Vec<u8>,
    ) -> Result<kaede_protocol::Attachment, ClientError> {
        let mut url = format!("{}/api/attachments?name={}", self.base_url, urlencode(name));
        if let Some(m) = mime {
            url.push_str(&format!("&mime={}", urlencode(m)));
        }
        let resp = self
            .http
            .post(&url)
            .bearer_auth(self.bearer()?)
            .header("content-type", "application/octet-stream")
            .body(bytes)
            .send()
            .await
            .map_err(map_reqwest_err)?;

        #[derive(Deserialize)]
        struct UploadResp {
            attachment: kaede_protocol::Attachment,
        }
        let body = parse_json::<UploadResp>(resp).await?;
        Ok(body.attachment)
    }

    /// 下载附件。
    pub async fn download_attachment(&self, id: &str) -> Result<Vec<u8>, ClientError> {
        let resp = self
            .http
            .get(format!("{}/api/attachments/{}", self.base_url, id))
            .bearer_auth(self.bearer()?)
            .send()
            .await
            .map_err(map_reqwest_err)?;
        if !resp.status().is_success() {
            return Err(parse_api_error(resp).await);
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ClientError::Transport(format!("read body: {}", e)))?;
        Ok(bytes.to_vec())
    }

    // -----------------------------------------------------------------
    // 好友
    // -----------------------------------------------------------------

    pub async fn list_friends(&self) -> Result<Vec<Friend>, ClientError> {
        let resp = self
            .http
            .get(format!("{}/api/friends", self.base_url))
            .bearer_auth(self.bearer()?)
            .send()
            .await
            .map_err(map_reqwest_err)?;
        let body = parse_json::<FriendsResp>(resp).await?;
        Ok(body.friends)
    }

    pub async fn send_friend_request(&self, login_id: &str) -> Result<(), ClientError> {
        let resp = self
            .http
            .post(format!("{}/api/friends/{}", self.base_url, login_id))
            .bearer_auth(self.bearer()?)
            .send()
            .await
            .map_err(map_reqwest_err)?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(parse_api_error(resp).await)
        }
    }

    pub async fn accept_friend(&self, from_login_id: &str) -> Result<(), ClientError> {
        let resp = self
            .http
            .post(format!(
                "{}/api/friends/{}/accept",
                self.base_url, from_login_id
            ))
            .bearer_auth(self.bearer()?)
            .send()
            .await
            .map_err(map_reqwest_err)?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(parse_api_error(resp).await)
        }
    }

    pub async fn remove_friend(&self, login_id: &str) -> Result<(), ClientError> {
        let resp = self
            .http
            .delete(format!("{}/api/friends/{}", self.base_url, login_id))
            .bearer_auth(self.bearer()?)
            .send()
            .await
            .map_err(map_reqwest_err)?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(parse_api_error(resp).await)
        }
    }

    // -----------------------------------------------------------------
    // DM
    // -----------------------------------------------------------------

    pub async fn list_dm_conversations(&self) -> Result<Vec<DmConversation>, ClientError> {
        let resp = self
            .http
            .get(format!("{}/api/dm", self.base_url))
            .bearer_auth(self.bearer()?)
            .send()
            .await
            .map_err(map_reqwest_err)?;
        let body = parse_json::<DmConversationsResp>(resp).await?;
        Ok(body.conversations)
    }

    pub async fn open_dm(&self, peer: &str) -> Result<DmConversation, ClientError> {
        let resp = self
            .http
            .post(format!("{}/api/dm/{}", self.base_url, peer))
            .bearer_auth(self.bearer()?)
            .send()
            .await
            .map_err(map_reqwest_err)?;
        parse_json::<DmConversation>(resp).await
    }

    pub async fn dm_messages(
        &self,
        conv_id: &str,
        before_ts: Option<i64>,
        limit: usize,
    ) -> Result<Vec<MessageRecord>, ClientError> {
        let mut url = format!(
            "{}/api/dm/{}/messages?limit={}",
            self.base_url, conv_id, limit
        );
        if let Some(b) = before_ts {
            url.push_str(&format!("&before_ts={}", b));
        }
        let resp = self
            .http
            .get(&url)
            .bearer_auth(self.bearer()?)
            .send()
            .await
            .map_err(map_reqwest_err)?;
        let body = parse_json::<MessagesResp>(resp).await?;
        Ok(body.messages)
    }

    pub async fn dm_post_message(
        &self,
        conv_id: &str,
        ts: i64,
        sender: &str,
        ciphertext: &[u8],
    ) -> Result<(), ClientError> {
        let resp = self
            .http
            .post(format!("{}/api/dm/{}/messages", self.base_url, conv_id))
            .bearer_auth(self.bearer()?)
            .json(&serde_json::json!({
                "ts": ts,
                "sender": sender,
                "ciphertext": URL_SAFE_NO_PAD.encode(ciphertext),
            }))
            .send()
            .await
            .map_err(map_reqwest_err)?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(parse_api_error(resp).await)
        }
    }

    // -----------------------------------------------------------------
    // LiveKit
    // -----------------------------------------------------------------

    pub async fn get_livekit_token(&self, channel_id: &str) -> Result<LivekitToken, ClientError> {
        let resp = self
            .http
            .post(format!("{}/api/livekit/token", self.base_url))
            .bearer_auth(self.bearer()?)
            .json(&serde_json::json!({ "channel_id": channel_id }))
            .send()
            .await
            .map_err(map_reqwest_err)?;
        parse_json::<LivekitToken>(resp).await
    }

    /// 通知服务器离开媒体房间（用于 seed 生命周期管理）。
    pub async fn leave_livekit_room(&self, channel_id: &str) -> Result<(), ClientError> {
        let resp = self
            .http
            .post(format!("{}/api/livekit/leave", self.base_url))
            .bearer_auth(self.bearer()?)
            .json(&serde_json::json!({ "channel_id": channel_id }))
            .send()
            .await
            .map_err(map_reqwest_err)?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(parse_api_error(resp).await)
        }
    }

    // -----------------------------------------------------------------
    // E2EE
    // -----------------------------------------------------------------

    /// 获取 LiveKit E2EE 共享密钥（32 字节 AES-256）。
    ///
    /// **DEPRECATED (P0-Z)**：server 端 `/api/livekit/e2ee-key` 已返回 **410 Gone**。
    /// 原因：server 端 key 派生是确定性的，配合 LiveKit SDK ratchet 重启归零会造成
    /// **nonce 复用**（密码学灾难）。
    ///
    /// 请改用 `RoomSession::begin_media_session`。
    #[deprecated(
        since = "0.1.0",
        note = "server endpoint returns 410 Gone; use RoomSession::begin_media_session instead"
    )]
    pub async fn get_e2ee_key(&self, channel_id: &str) -> Result<[u8; 32], ClientError> {
        #[derive(Deserialize)]
        struct E2eeKeyResp {
            key: String,
            #[allow(dead_code)]
            algorithm: String,
            #[allow(dead_code)]
            key_size: u32,
        }

        let resp = self
            .http
            .post(format!("{}/api/livekit/e2ee-key", self.base_url))
            .bearer_auth(self.bearer()?)
            .json(&serde_json::json!({ "channel_id": channel_id }))
            .send()
            .await
            .map_err(map_reqwest_err)?;

        let body = parse_json::<E2eeKeyResp>(resp).await?;
        let bytes = URL_SAFE_NO_PAD
            .decode(&body.key)
            .map_err(|e| ClientError::Protocol(format!("bad e2ee key base64: {}", e)))?;

        if bytes.len() != 32 {
            return Err(ClientError::Protocol(format!(
                "e2ee key length {} != 32",
                bytes.len()
            )));
        }

        let mut key = [0u8; 32];
        key.copy_from_slice(&bytes);
        Ok(key)
    }

    // -----------------------------------------------------------------
    // 内部
    // -----------------------------------------------------------------

    fn bearer(&self) -> Result<String, ClientError> {
        self.token
            .lock()
            .unwrap()
            .clone()
            .ok_or(ClientError::NotAuthenticated)
    }
}

fn map_reqwest_err(e: reqwest::Error) -> ClientError {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if e.is_timeout() {
            return ClientError::Timeout;
        }
        if e.is_connect() {
            return ClientError::Connection(format!("http: {}", e));
        }
        if e.is_decode() {
            return ClientError::Protocol(format!("decode: {}", e));
        }
    }
    ClientError::Transport(format!("http: {}", e))
}

async fn parse_json<T: serde::de::DeserializeOwned>(
    resp: reqwest::Response,
) -> Result<T, ClientError> {
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| ClientError::Transport(format!("read body: {}", e)))?;

    if !status.is_success() {
        return Err(decode_error(status.as_u16(), &body));
    }

    serde_json::from_str(&body).map_err(|e| {
        let preview: String = body.chars().take(200).collect();
        let suffix = if body.chars().count() > 200 {
            "..."
        } else {
            ""
        };
        ClientError::Protocol(format!("decode {}: {}{}", e, preview, suffix))
    })
}

async fn parse_api_error(resp: reqwest::Response) -> ClientError {
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    decode_error(status, &body)
}

fn decode_error(status: u16, body: &str) -> ClientError {
    let message = serde_json::from_str::<ErrorResp>(body)
        .map(|e| format!("{}: {}", e.error, e.message))
        .unwrap_or_else(|_| body.to_string());
    ClientError::Http { status, message }
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;

    #[test]
    fn new_normalizes_url() {
        let api = HttpApi::new("http://localhost:8080/").unwrap();
        assert_eq!(api.base_url, "http://localhost:8080");
        let api = HttpApi::new("http://localhost:8080").unwrap();
        assert_eq!(api.base_url, "http://localhost:8080");
    }

    #[test]
    fn token_lifecycle() {
        let api = HttpApi::new("http://localhost:8080").unwrap();
        assert!(api.token().is_none());
        assert!(api.bearer().is_err());

        api.set_token("abc");
        assert_eq!(api.token().as_deref(), Some("abc"));
        assert_eq!(api.bearer().unwrap(), "abc");

        api.clear_token();
        assert!(api.token().is_none());
    }

    #[test]
    fn decode_error_handles_json() {
        let e = decode_error(400, r#"{"error":"bad_request","message":"oops"}"#);
        match e {
            ClientError::Http { status, message } => {
                assert_eq!(status, 400);
                assert!(message.contains("bad_request"));
            }
            _ => panic!("expected Transport error"),
        }
    }

    #[test]
    fn decode_error_falls_back_to_raw() {
        let e = decode_error(500, "server exploded");
        match e {
            ClientError::Http { status, message } => {
                assert_eq!(status, 500);
                assert!(message.contains("server exploded"));
            }
            _ => panic!("expected Transport error"),
        }
    }
}

/// 极简 URL 编码（仅编码 query 里必须转义的字符）。
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}
