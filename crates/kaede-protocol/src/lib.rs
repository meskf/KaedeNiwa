//! KaedeNiwa 线协议类型。
//!
//! 服务器和客户端共享此 crate，编译期强制同步。
//!
//! # 编码
//!
//! - 格式：CBOR (ciborium)
//! - 顶层：internally tagged enum，tag 字段为 `t`
//! - 大小上限：`MAX_MESSAGE_BYTES` (16 MiB)
//!
//! # 方向
//!
//! 两个 enum 表达方向：
//! - [`ClientMessage`]：客户端 -> 服务器
//! - [`ServerMessage`]：服务器 -> 客户端
//!
//! 各自只接受自己的变体名。交叉发送会在反序列化时被拒绝。
//!
//! # 版本协商
//!
//! 连接建立后第一条消息必须是 [`ClientMessage::ClientHello`]。
//! 服务器检查 `protocol_version`，不匹配则回复
//! [`ServerMessage::Error`]（`code = UnsupportedVersion`）并断开。
//!
//! # 安全边界
//!
//! **服务器永不解析 MLS payload。** 所有 `Mls*` 变体的 `payload`
//! 对服务器是不透明字节流。`from` 字段由服务器注入，客户端不能伪造。

#![forbid(unsafe_code)]
use ts_rs::TS;

pub mod video;
pub use video::{
    ALL_VIDEO_PROFILES, DEFAULT_VIDEO_BITRATE, DEFAULT_VIDEO_PROFILE, VideoBitrate, VideoProfile,
};

use serde::de::{Deserializer, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::marker::PhantomData;

/// 限制序列元素数，防止 16 MiB 的“空数组”在解码时膨胀成数亿字节。
fn bounded_seq<'de, D, T>(deserializer: D, max: usize) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Bounded<T> {
        max: usize,
        _marker: PhantomData<T>,
    }
    impl<'de, T: Deserialize<'de>> Visitor<'de> for Bounded<T> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "a sequence with at most {} elements", self.max)
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut out = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(self.max));
            while let Some(item) = seq.next_element()? {
                if out.len() >= self.max {
                    return Err(serde::de::Error::custom("sequence too long"));
                }
                out.push(item);
            }
            Ok(out)
        }
    }
    deserializer.deserialize_seq(Bounded {
        max,
        _marker: PhantomData,
    })
}

fn bounded_packages<'de, D>(d: D) -> Result<Vec<Vec<u8>>, D::Error>
where
    D: Deserializer<'de>,
{
    bounded_seq(d, 256)
}

fn bounded_peers<'de, D>(d: D) -> Result<Vec<UserId>, D::Error>
where
    D: Deserializer<'de>,
{
    bounded_seq(d, 8192)
}

// ============================================================================
// 常量
// ============================================================================

/// 协议版本。
///
/// v3：新增 `ServerMessage::ChannelMessage`（私密频道实时消息）。
/// v4：DM/私密频道密文升级为 v2（AAD 绑定发送者/接收者/消息 id，0x05 前缀）。
/// v5：新增 `ServerMessage::FriendsChanged`；移除 SDP/ICE/SfuToken 遗留变体。
/// v6：新增私密频道表情反应（`ChannelReaction` 双向变体）。
/// v7：新增 `Typing` 双向变体（输入中指示）。
    /// v8：ChannelMessage 新增 id 字段（用于增量拉取）。
pub const PROTOCOL_VERSION: u16 = 8;

/// 服务器接受的最低协议版本。
///
/// v7：旧客户端（v6）收到 `Typing` 变体时 CBOR 反序列化会失败并断开，
/// 与其让旧客户端随机掉线，不如握手时明确拒绝并提示升级。
pub const MIN_SUPPORTED_VERSION: u16 = 7;

/// 单条线消息的硬上限。
pub const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

// ============================================================================
// 基础类型
// ============================================================================

/// 用户 ID。登录名（GitHub 风格：小写字母、数字、连字符）。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(transparent)]
pub struct UserId(pub String);

impl UserId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for UserId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for UserId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for UserId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<&String> for UserId {
    fn from(s: &String) -> Self {
        Self(s.clone())
    }
}

impl AsRef<str> for UserId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

// ============================================================================
// 客户端 -> 服务器
// ============================================================================

/// 客户端发给服务器的消息。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ClientMessage {
    // ---- 握手 ----
    /// 连接建立后必须首先发送。
    ClientHello {
        protocol_version: u16,
        /// 客户端标识：`"kaede-web"` / `"kaede-desktop"` / `"kaede-mobile"`。
        client_name: String,
        /// 客户端版本（semver）。
        client_version: String,
        /// HTTP 登录后拿到的 session token。
        ///
        /// **必填**。服务器在版本协商通过后立即校验；缺失或无效都会收到
        /// `Unauthenticated` 并断开，防止未认证连接占满 WS 连接池。
        #[serde(default)]
        session_token: Option<String>,
    },

    // ---- 房间 ----
    /// 加入服务器的房间。一个 server 进程 = 一个房间。
    Join,

    /// 离开房间。服务器广播 PeerLeft。
    Leave,

    /// B2：频道级 MLS 同步（Welcome 0x04 / Commit 0x05）。
    /// 定向发给频道成员；服务端校验发送者必须是 channel_id 的成员。
    ChannelSync {
        to: UserId,
        channel_id: String,
        #[serde(with = "serde_bytes")]
        #[ts(type = "number[]")]
        payload: Vec<u8>,
    },

    /// 私密频道表情反应。
    ///
    /// `ciphertext` = `0x03 || dm_cipher::seal(channel_key, channel_id, from, "", payload)`。
    /// 服务端只校验成员关系与前缀，不解析内容；实时 fanout 给频道成员。
    ChannelReaction {
        channel_id: String,
        #[serde(with = "serde_bytes")]
        #[ts(type = "number[]")]
        ciphertext: Vec<u8>,
    },

    /// 频道级 MLS Commit 广播（0x05）。
    ///
    /// 由频道 group 的**权威成员**（最早加入者）在 Add 新成员后发送。
    /// 服务端校验发送者属于该频道，然后仅转发给该频道的其他成员。
    /// 客户端收到后调用 `channel_process_commit` 使 epoch 对齐。
    ChannelCommit {
        channel_id: String,
        /// 不投递该用户（通常是刚通过 Welcome 入组的新成员：
        /// 其 Welcome 已经包含该 Commit 的效果，重复处理会报 WrongEpoch）。
        #[serde(default)]
        exclude: Option<UserId>,
        /// 该 Commit 应用后的 epoch。authority 已知本地状态，服务端据此落库以便补发。
        /// 0 = 未提供（老客户端/测试），服务端跳过落库但仍广播。
        #[serde(default)]
        epoch: u64,
        #[serde(with = "serde_bytes")]
        #[ts(type = "number[]")]
        payload: Vec<u8>,
    },

    // ---- MLS 目录 ----
    /// 上传自己的 KeyPackage 队列。服务器排队，Add 时按需取出。
    /// 一个 KeyPackage 只能用一次，用完服务器删除。
    PublishKeyPackages {
        #[serde(deserialize_with = "bounded_packages")]
        packages: Vec<Vec<u8>>,
    },

    /// 请求某个用户的 KeyPackage。服务器返回
    /// [`ServerMessage::KeyPackageFor`] 或 `TargetNotFound` / `KeyPackageNotFound`。
    FetchKeyPackage {
        target: UserId,
    },

    // ---- MLS 消息 ----
    /// 定向 Welcome。发给刚被 Add 的新成员。
    MlsWelcome {
        to: UserId,
        #[serde(with = "serde_bytes")]
        #[ts(type = "number[]")]
        payload: Vec<u8>,
    },

    /// 广播 Commit。服务器转发给所有已 join 的其他 session。
    MlsCommit {
        /// Commit 产生后的新 epoch。服务器用它实现离线成员 epoch 追赶。
        #[serde(default)]
        epoch: u64,
        #[serde(with = "serde_bytes")]
        #[ts(type = "number[]")]
        payload: Vec<u8>,
    },

    /// 客户端上报自己当前的主群 epoch。
    ///
    /// 服务器据此补投 `epoch > 上报值` 的最近 Commit（离线期间错过的）。
    ReportEpoch {
        epoch: u64,
        /// Some(channel_id) = 频道 backfill；None = 主群。
        #[serde(default)]
        channel_id: Option<String>,
    },

    /// 广播 Proposal。
    MlsProposal {
        #[serde(with = "serde_bytes")]
        #[ts(type = "number[]")]
        payload: Vec<u8>,
    },

    /// 广播 Application Message（聊天 / 数据）。
    MlsAppMessage {
        channel_id: String,
        #[serde(with = "serde_bytes")]
        #[ts(type = "number[]")]
        payload: Vec<u8>,
    },

    // ---- 在线状态 ----
    /// 设置自己的在线状态。
    SetPresence {
        /// `"online"` | `"idle"` | `"dnd"`。
        status: String,
    },

    /// 输入中指示（v7）。
    ///
    /// `channel_id` 可以是公开频道、私密频道或 DM 会话 id（`dm:a:b`）。
    /// 纯元数据、不落盘、不补投；服务端按 uid 限流后定向扇出：
    /// - DM → 仅对端
    /// - 私密频道 → 在线成员（除发送者）
    /// - 公开频道 → 全房间广播（除发送者，可走丢弃语义的通道）
    Typing {
        channel_id: String,
    },

    // ---- 保活 ----
    Ping {
        nonce: u64,
    },
}

// ============================================================================
// 领域类型
// ============================================================================

/// 频道。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Channel {
    pub id: String,
    pub name: String,
    /// `"text"` | `"voice"`。
    pub kind: String,
    /// 排序位置（越大越靠后）。
    pub position: i64,
    pub created_at: i64,
    /// 私密频道：仅 channel_members 成员可见/可发。
    #[serde(default)]
    pub is_private: bool,
    /// 哈希强度档位（"19" / "64" / "256"）。仅 UI 展示，verify 走 PHC 串真实参数。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash_tier: Option<String>,
    /// 所属分类（categories.id）。None = 未分类。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category_id: Option<String>,
}

/// 频道分类（Discord 风格的可折叠分组）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Category {
    pub id: String,
    pub name: String,
    /// 排序位置（越小越前）。
    pub position: i64,
    pub created_at: i64,
}

/// 在线状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Presence {
    pub login_id: String,
    /// `"online"` | `"idle"` | `"dnd"` | `"offline"`。
    pub status: String,
}

/// 引用消息的引用信息。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ReplyRef {
    /// 被引用消息的 ts（用 ts 当 id）。
    pub ts: i64,
    pub author: String,
    /// 摘要（截断到 80 字符）。
    pub preview: String,
}

/// 表情回复。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Reaction {
    pub emoji: String,
    pub users: Vec<String>,
}

/// 附件元数据。
///
/// 附件 blob 由客户端加密后上传（`0x04` 包装），服务端只存密文。
/// 消息里的这个结构位于 E2EE payload 内，因此 `sha256` 字段本身不可被服务端篡改；
/// 接收方下载后必须校验密文哈希，防止服务端替换/降级附件内容。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Attachment {
    /// 服务器分配的 id。
    pub id: String,
    /// 原始文件名。
    pub name: String,
    /// MIME 类型。
    pub mime: String,
    /// 字节数。
    pub size: u64,
    /// 下载 URL（相对或绝对）。
    pub url: String,
    /// 图片缩略图附件 id（可选，历史字段名沿用 `url` 语义）。
    #[serde(default)]
    pub thumbnail_url: Option<String>,
    /// 附件密文（`0x04 || nonce || ct || tag`）的 SHA-256，小写 hex。
    ///
    /// E2EE payload 内字段，由发送方在加密上传后填写；服务端上传响应不填。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// 缩略图密文的 SHA-256，小写 hex。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail_sha256: Option<String>,
}

/// 成员。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Member {
    pub login_id: String,
    pub nickname: String,
    /// `"op1"` .. `"op5"`。
    pub op: String,
    pub bio: String,
    pub title: String,
    pub avatar_url: String,
    pub joined_at: i64,
}

// ============================================================================
// 服务器 -> 客户端
// ============================================================================

/// 服务器发给客户端的消息。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ServerMessage {
    // ---- 握手 ----
    /// 对 [`ClientMessage::ClientHello`] 的回应。
    ServerHello {
        protocol_version: u16,
        server_id: String,
        server_name: String,
    },

    // ---- 频道 / 成员变更广播 ----
    /// 新频道创建。
    ChannelCreated {
        channel: Channel,
    },

    /// 频道更新（改名 / 排序）。
    ChannelUpdated {
        channel: Channel,
    },

    /// 频道删除。
    ChannelDeleted {
        id: String,
    },

    /// 成员资料更新（昵称 / op / bio / ...）。
    MemberUpdated {
        member: Member,
    },

    /// B2：某用户通过密码加入了私密频道，等待已有 group 成员把他加进 group。
    /// 仅广播给该频道的已有成员（channel_members 里除申请人外的所有人）。
    ChannelMemberJoined {
        channel_id: String,
        requester: UserId,
        /// 申请者的 MLS KeyPackage（base64 URL-safe 编码）
        key_package: String,
    },

    /// B2：已加入的成员收到 group 同步包（Welcome / Commit），用于更新本地 channel group。
    ChannelSync {
        channel_id: String,
        /// Commit 应用后的 epoch（仅 0x05 有意义）。
        /// 客户端据此检测 gap 并请求补发；None = 兼容老服务端/enqueue 路径。
        #[serde(default)]
        epoch: Option<u64>,
        /// 首字节 0x04 = Welcome, 0x05 = Commit
        #[serde(with = "serde_bytes")]
        #[ts(type = "number[]")]
        payload: Vec<u8>,
    },

    /// 某成员离开了私密频道。
    ///
    /// 只发给频道 group 的权威成员：它负责从 channel group 中移除该 leaf
    /// 并广播 Commit，使频道密钥轮换（离开者无法再解密后续消息）。
    ChannelMemberLeft {
        channel_id: String,
        member: UserId,
    },

    /// 当前用户被移出私密频道（或频道已不可见）。
    ///
    /// 客户端应删除本地 channel group，并把频道从列表移除。
    ChannelRemoved {
        channel_id: String,
    },

    /// 私密频道实时消息（v3）。
    ///
    /// 客户端把 `0x03 || AES-GCM 密文` 写入 `POST /api/messages` 后，
    /// 服务器用本消息实时推送除发送者外的频道成员。离线成员靠历史补齐。
    ChannelMessage {
        /// 消息在服务端的自增 id（v8）。
        /// 用于客户端增量拉取；pending 旧格式投递时置 0。
        #[serde(default)]
        id: i64,
        channel_id: String,
        from: UserId,
        /// 服务器权威时间戳。
        ts: i64,
        /// 与历史中完全相同的 `0x03` 封装密文。
        #[serde(with = "serde_bytes")]
        #[ts(type = "number[]")]
        ciphertext: Vec<u8>,
    },

    /// 私密频道表情反应（v6）。
    ///
    /// 与 `ChannelMessage` 一样由服务端定向 fanout 给频道成员；
    /// `ciphertext` 是 `0x03 || AES-GCM 密文`，客户端用频道 group key 解。
    ChannelReaction {
        channel_id: String,
        from: UserId,
        #[serde(with = "serde_bytes")]
        #[ts(type = "number[]")]
        ciphertext: Vec<u8>,
    },

    /// 在线状态更新（单个或多个）。
    PresenceUpdated {
        presence: Vec<Presence>,
    },

    /// 输入中指示（v7）。
    ///
    /// 服务端不持久化、不离线补投；客户端 UI 应本地设置数秒超时自动消失。
    Typing {
        channel_id: String,
        uid: UserId,
    },

    /// 成员被踢。
    MemberRemoved {
        login_id: String,
    },

    /// 好友关系发生变化（请求 / 接受 / 删除）。
    ///
    /// 无业务负载：客户端收到后重新拉取 `GET /api/friends`。
    /// 离线不补投，登录后由 HTTP 列表兜底。
    FriendsChanged,

    // ---- 房间 ----
    /// 对 [`ClientMessage::Join`] 的回应。携带当前所有其他成员。
    Joined {
        #[serde(deserialize_with = "bounded_peers")]
        peers: Vec<UserId>,
    },

    /// 有新成员加入。
    PeerJoined {
        uid: UserId,
    },

    /// 有成员离开。
    PeerLeft {
        uid: UserId,
    },

    // ---- MLS 目录回应 ----
    /// [`ClientMessage::FetchKeyPackage`] 的回应。
    KeyPackageFor {
        target: UserId,
        #[serde(with = "serde_bytes")]
        #[ts(type = "number[]")]
        package: Vec<u8>,
    },

    // ---- MLS 消息转发 ----
    MlsWelcome {
        from: UserId,
        #[serde(with = "serde_bytes")]
        #[ts(type = "number[]")]
        payload: Vec<u8>,
    },

    MlsCommit {
        from: UserId,
        #[serde(default)]
        epoch: u64,
        /// Some(channel_id) = 频道 Commit 补发；None = 主群。
        #[serde(default)]
        channel_id: Option<String>,
        #[serde(with = "serde_bytes")]
        #[ts(type = "number[]")]
        payload: Vec<u8>,
    },

    MlsProposal {
        from: UserId,
        #[serde(with = "serde_bytes")]
        #[ts(type = "number[]")]
        payload: Vec<u8>,
    },

    MlsAppMessage {
        from: UserId,
        channel_id: String,
        #[serde(with = "serde_bytes")]
        #[ts(type = "number[]")]
        payload: Vec<u8>,
    },

    // ---- 保活 ----
    /// DM 消息（B3 方案：per-conv AEAD 密文）。
    ///
    /// - 加密走 dm_cipher::seal，key 从 MLS exporter 派生（purpose = Dm）
    /// - 不走 MLS Application Message（group-level 多播）
    /// - 只投递给 conv 的对方（server 按 conv_id 解析两个 uid）
    /// - sender 靠本地回显，server 不推回 sender
    DmReceived {
        conv_id: String,
        from: UserId,
        ts: i64,
        #[serde(with = "serde_bytes")]
        #[ts(type = "number[]")]
        ciphertext: Vec<u8>,
    },

    Pong {
        nonce: u64,
    },

    // ---- 错误 ----
    Error {
        code: ErrorCode,
        message: String,
    },
}

// ============================================================================
// 错误码
// ============================================================================

/// 结构化错误码。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// 协议版本不兼容。
    UnsupportedVersion,
    /// 未认证。
    Unauthenticated,
    /// 未 join 房间。
    NotJoined,
    /// 目标用户不存在或不在线。
    TargetNotFound,
    /// 目标用户没有可用的 KeyPackage。
    KeyPackageNotFound,
    /// 消息过大。
    MessageTooLarge,
    /// 消息格式非法。
    MalformedMessage,
    /// 触发限流。
    RateLimited,
    /// 被管理员踢出。客户端收到后应关闭连接。
    Kicked,
    /// 内部错误。
    InternalError,
}

impl ErrorCode {
    /// 是否为连接级致命错误（收到后应断开）。
    pub fn is_fatal(self) -> bool {
        matches!(
            self,
            Self::UnsupportedVersion
                | Self::MalformedMessage
                | Self::MessageTooLarge
                | Self::Kicked
                | Self::Unauthenticated
        )
    }
}

// ============================================================================
// 路由
// ============================================================================

/// 服务器对客户端消息的转发决策。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForwardTarget<'a> {
    /// 定向给某个用户。
    Direct(&'a UserId),
    /// 广播给所有已 join 的其他 session。
    Broadcast,
}

impl ClientMessage {
    /// 服务器判断该消息是否需要转发。
    ///
    /// 返回 `None` 表示服务器自己处理（握手、房间、目录、ping）。
    /// 服务器不需要 match 每个变体，用此方法即可实现转发。
    pub fn forward_target(&self) -> Option<ForwardTarget<'_>> {
        match self {
            // 点对点
            Self::MlsWelcome { to, .. } | Self::ChannelSync { to, .. } => {
                Some(ForwardTarget::Direct(to))
            }

            // 广播
            Self::MlsCommit { .. } | Self::MlsProposal { .. } | Self::MlsAppMessage { .. } => {
                Some(ForwardTarget::Broadcast)
            }

            // 服务器自己处理（需要查 DB 补投）
            Self::ReportEpoch { .. } => None,

            // 服务器自己处理
            Self::ClientHello { .. }
            | Self::Join
            | Self::Leave
            | Self::PublishKeyPackages { .. }
            | Self::FetchKeyPackage { .. }
            | Self::ChannelCommit { .. }
            | Self::ChannelReaction { .. }
            | Self::SetPresence { .. }
            // Typing：服务端自己处理（限流 + 定向扇出），不走通用转发。
            | Self::Typing { .. }
            | Self::Ping { .. } => None,
        }
    }
}

// ============================================================================
// 编解码
// ============================================================================

/// 线消息的编解码接口。
pub trait WireMessage: Serialize + for<'de> Deserialize<'de> + Sized {
    /// 编码为 CBOR。
    fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        let mut buf = Vec::new();
        ciborium::into_writer(self, &mut buf).map_err(|e| ProtocolError::Encode(e.to_string()))?;
        if buf.len() > MAX_MESSAGE_BYTES {
            return Err(ProtocolError::MessageTooLarge(buf.len()));
        }
        Ok(buf)
    }

    /// 从 CBOR 解码。
    fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        if bytes.len() > MAX_MESSAGE_BYTES {
            return Err(ProtocolError::MessageTooLarge(bytes.len()));
        }
        ciborium::from_reader(bytes).map_err(|e| ProtocolError::Decode(e.to_string()))
    }
}

impl WireMessage for ClientMessage {}
impl WireMessage for ServerMessage {}

// ============================================================================
// 错误
// ============================================================================

/// 协议层错误（编解码相关）。
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("encode error: {0}")]
    Encode(String),
    #[error("decode error: {0}")]
    Decode(String),
    #[error("message too large: {0} bytes")]
    MessageTooLarge(usize),
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip_client(msg: ClientMessage) {
        let bytes = msg.encode().expect("encode");
        let back = ClientMessage::decode(&bytes).expect("decode");
        assert_eq!(msg, back);
    }

    #[test]
    fn bounded_packages_rejects_oversized_sequence() {
        // 16 MiB 的“空数组”会在解码时膨胀；限制必须在 serde visitor 层生效。
        let msg = ClientMessage::PublishKeyPackages {
            packages: vec![Vec::new(); 300],
        };
        let bytes = msg.encode().expect("encode");
        assert!(ClientMessage::decode(&bytes).is_err());
    }

    fn round_trip_server(msg: ServerMessage) {
        let bytes = msg.encode().expect("encode");
        let back = ServerMessage::decode(&bytes).expect("decode");
        assert_eq!(msg, back);
    }

    #[test]
    fn client_hello_round_trip() {
        round_trip_client(ClientMessage::ClientHello {
            protocol_version: PROTOCOL_VERSION,
            client_name: "kaede-web".into(),
            client_version: "0.1.0".into(),
            session_token: None,
        });
        round_trip_client(ClientMessage::ClientHello {
            protocol_version: PROTOCOL_VERSION,
            client_name: "kaede-desktop".into(),
            client_version: "0.2.0".into(),
            session_token: Some("tok".into()),
        });
    }

    #[test]
    fn join_round_trip() {
        round_trip_client(ClientMessage::Join);
    }

    #[test]
    fn mls_app_message_round_trip() {
        round_trip_client(ClientMessage::MlsAppMessage {
            channel_id: "general".into(),
            payload: vec![0xde, 0xad, 0xbe, 0xef],
        });
    }

    #[test]
    fn channel_commit_round_trip() {
        round_trip_client(ClientMessage::ChannelCommit {
            channel_id: "ch_abc".into(),
            exclude: Some(UserId::new("bob")),
            epoch: 1,
            payload: vec![0x05, 1, 2, 3],
        });
    }

    #[test]
    fn server_error_round_trip() {
        round_trip_server(ServerMessage::Error {
            code: ErrorCode::UnsupportedVersion,
            message: "server requires v2".into(),
        });
    }

    #[test]
    fn mls_welcome_has_from() {
        round_trip_server(ServerMessage::MlsWelcome {
            from: UserId::new("bob"),
            payload: vec![1, 2, 3],
        });
    }

    #[test]
    fn channel_message_round_trip() {
        round_trip_server(ServerMessage::ChannelMessage {
            channel_id: "ch_secret".into(),
            from: UserId::new("alice"),
            ts: 123456789,
            ciphertext: vec![0x03, 9, 8, 7],
        });
    }

    #[test]
    fn channel_reaction_round_trip() {
        round_trip_client(ClientMessage::ChannelReaction {
            channel_id: "ch_secret".into(),
            ciphertext: vec![0x03, 1, 2, 3],
        });
        round_trip_server(ServerMessage::ChannelReaction {
            channel_id: "ch_secret".into(),
            from: UserId::new("alice"),
            ciphertext: vec![0x03, 4, 5, 6],
        });
    }

    #[test]
    fn friends_changed_round_trip() {
        round_trip_server(ServerMessage::FriendsChanged);
    }

    #[test]
    fn empty_payload_ok() {
        round_trip_client(ClientMessage::MlsCommit {
            epoch: 7,
            payload: vec![],
        });
    }

    #[test]
    fn unknown_tag_rejected() {
        #[derive(Serialize)]
        struct Unknown {
            t: String,
        }
        let unknown = Unknown {
            t: "xyz_not_a_message".into(),
        };
        let mut buf = Vec::new();
        ciborium::into_writer(&unknown, &mut buf).unwrap();

        assert!(ClientMessage::decode(&buf).is_err());
    }

    #[test]
    fn oversize_rejected_on_encode() {
        let big = ClientMessage::MlsAppMessage {
            channel_id: "general".into(),
            payload: vec![0; MAX_MESSAGE_BYTES + 1],
        };
        assert!(matches!(
            big.encode(),
            Err(ProtocolError::MessageTooLarge(_))
        ));
    }

    #[test]
    fn oversize_rejected_on_decode() {
        let bytes = vec![0u8; MAX_MESSAGE_BYTES + 1];
        assert!(matches!(
            ClientMessage::decode(&bytes),
            Err(ProtocolError::MessageTooLarge(_))
        ));
    }

    #[test]
    fn forward_target_classification() {
        let direct = ClientMessage::MlsWelcome {
            to: UserId::new("alice"),
            payload: vec![],
        };
        assert!(matches!(
            direct.forward_target(),
            Some(ForwardTarget::Direct(_))
        ));

        let commit = ClientMessage::MlsCommit {
            epoch: 1,
            payload: vec![],
        };
        assert!(matches!(
            commit.forward_target(),
            Some(ForwardTarget::Broadcast)
        ));

        let hello = ClientMessage::ClientHello {
            protocol_version: 2,
            client_name: "x".into(),
            client_version: "0".into(),
            session_token: None,
        };
        assert!(hello.forward_target().is_none());
    }

    #[test]
    fn error_code_fatality() {
        assert!(ErrorCode::UnsupportedVersion.is_fatal());
        assert!(ErrorCode::MalformedMessage.is_fatal());
        assert!(!ErrorCode::TargetNotFound.is_fatal());
        assert!(!ErrorCode::RateLimited.is_fatal());
    }

    #[test]
    fn cbor_is_compact() {
        // 空 MlsAppMessage 应该 < 60 字节。
        // 实测当前 47 字节：CBOR external tag + map{channel_id, payload}。
        // 对比 JSON 同结构约 80+ 字节，仍有明显优势。
        let msg = ClientMessage::MlsAppMessage {
            channel_id: "general".into(),
            payload: vec![],
        };
        let bytes = msg.encode().unwrap();
        assert!(
            bytes.len() < 60,
            "empty MlsAppMessage too large: {} bytes",
            bytes.len()
        );
    }
}
// ============================================================================
// TypeScript 导出（feature = "ts-export"）
// ============================================================================

#[cfg(feature = "ts-export")]
pub fn export_typescript(dir: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(dir)?;
    UserId::export_all_to(dir)?;
    Channel::export_all_to(dir)?;
    Member::export_all_to(dir)?;
    Presence::export_all_to(dir)?;
    ReplyRef::export_all_to(dir)?;
    Reaction::export_all_to(dir)?;
    Attachment::export_all_to(dir)?;
    ClientMessage::export_all_to(dir)?;
    ServerMessage::export_all_to(dir)?;
    ErrorCode::export_all_to(dir)?;
    Ok(())
}

#[cfg(feature = "ts-export")]
#[test]
fn export_ts() {
    let dir = std::path::Path::new("../../apps/web/src/protocol");
    export_typescript(dir).expect("ts-rs export failed");
    eprintln!("TS types written to {:?}", dir);
}
