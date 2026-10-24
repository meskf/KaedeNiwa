//! MLS Application Message 的明文 payload 结构。

use kaede_protocol::{Attachment, ReplyRef};
use serde::{Deserialize, Serialize};

/// MLS 消息的明文 payload（普通消息）。
///
/// `c` 字段：
/// - 频道消息：`"general"` 等
/// - DM 消息：`"dm:alice:bob"`（服务端会验证成员）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagePayload {
    pub c: String,
    pub u: String,
    pub t: String,
    /// C18: 发送端本地时间（毫秒）。旧客户端不传时为 0，接收端回退到 now_ms()
    #[serde(default)]
    pub ts: i64,
    #[serde(default)]
    pub a: Vec<Attachment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r: Option<ReplyRef>,
    /// @ 提及的 login_id 列表（E2EE 内部字段，服务端不可见）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub m: Vec<String>,
}

impl MessagePayload {
    pub fn new(
        channel_id: impl Into<String>,
        uid: impl Into<String>,
        text: impl Into<String>,
        attachments: Vec<Attachment>,
        reply_to: Option<ReplyRef>,
    ) -> Self {
        Self {
            c: channel_id.into(),
            u: uid.into(),
            t: text.into(),
            ts: crate::room::now_ms(), // C18: 自动打上当前发送时间
            a: attachments,
            r: reply_to,
            m: Vec::new(),
        }
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    pub fn parse(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice(bytes).ok()
    }
}

/// 表情回复 payload（独立 MLS 消息）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReactionPayload {
    pub c: String,
    pub u: String,
    /// 目标消息的 ts。
    pub target_ts: i64,
    pub emoji: String,
    /// `"add"` | `"remove"`。
    pub action: String,
}

impl ReactionPayload {
    pub fn new(
        channel_id: impl Into<String>,
        uid: impl Into<String>,
        target_ts: i64,
        emoji: impl Into<String>,
        action: impl Into<String>,
    ) -> Self {
        Self {
            c: channel_id.into(),
            u: uid.into(),
            target_ts,
            emoji: emoji.into(),
            action: action.into(),
        }
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    pub fn parse(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice(bytes).ok()
    }
}

/// 消息编辑 payload（E2EE 应用层操作）。
///
/// 编辑不走新的协议消息，而是作为加密 envelope 内的独立 payload 类型
/// （与 [`ReactionPayload`] 同模式），服务端只存/转密文，零改动。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditPayload {
    /// 固定 `"edit"`。人类可读的 tag，也用于防御性校验。
    pub kind: String,
    pub c: String,
    pub u: String,
    /// 目标消息的 ts。
    pub target_ts: i64,
    /// 新文本。
    pub t: String,
}

impl EditPayload {
    pub fn new(
        channel_id: impl Into<String>,
        uid: impl Into<String>,
        target_ts: i64,
        new_text: impl Into<String>,
    ) -> Self {
        Self {
            kind: "edit".into(),
            c: channel_id.into(),
            u: uid.into(),
            target_ts,
            t: new_text.into(),
        }
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    pub fn parse(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice(bytes).ok()
    }
}

/// 消息删除 payload（E2EE 应用层操作）。
///
/// 与 [`EditPayload`] 同模式：加密 envelope 内的独立 payload 类型，
/// 服务端零改动。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeletePayload {
    /// 固定 `"delete"`。
    pub kind: String,
    pub c: String,
    pub u: String,
    /// 目标消息的 ts。
    pub target_ts: i64,
}

impl DeletePayload {
    pub fn new(channel_id: impl Into<String>, uid: impl Into<String>, target_ts: i64) -> Self {
        Self {
            kind: "delete".into(),
            c: channel_id.into(),
            u: uid.into(),
            target_ts,
        }
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    pub fn parse(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice(bytes).ok()
    }
}

/// 媒体会话信令 payload。
///
/// 用 MLS Application Message 广播媒体格式元数据，让接收端
/// 知道怎么解码 SFrame 帧（采样率 / 声道 / 编码格式）。
///
/// **不修改 SFrame 帧格式**——只做带内信令。SFrame 帧仍然只有
/// `key_id + counter + ct || tag`。
///
/// 字段名特意避开 `c` / `u` / `t`（MessagePayload 用），
/// 让 serde 反序列化时自动区分 payload 类型。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaMetaPayload {
    /// 固定 `"media_meta"`。人类可读的 tag，也用于防御性校验。
    pub kind: String,

    /// 频道 ID。
    pub channel_id: String,

    /// 发送者 uid。
    pub uid: String,

    /// SFrame purpose（和 `kaede_mls::SframePurpose::as_str()` 一致）。
    /// 例：`"voice-audio"` / `"screen-video"` / `"screen-audio"`。
    pub purpose: String,

    /// 编码格式。`"pcm-s16le"` / `"h264-annexb"` 等。
    pub codec: String,

    /// SFrame key_id（一般 = MLS epoch）。
    pub key_id: u32,

    /// SFrame sender_id（counter 高 32 位）。
    pub sender_id: u32,

    /// 音频采样率。视频 / 非音频为 `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_rate: Option<u32>,

    /// 音频声道数。视频 / 非音频为 `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channels: Option<u16>,

    /// `"start"` 或 `"stop"`。
    pub action: String,
}

impl MediaMetaPayload {
    /// 构造一条「开始」媒体会话通知。
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        channel_id: impl Into<String>,
        uid: impl Into<String>,
        purpose: impl Into<String>,
        codec: impl Into<String>,
        key_id: u32,
        sender_id: u32,
        sample_rate: Option<u32>,
        channels: Option<u16>,
    ) -> Self {
        Self {
            kind: "media_meta".into(),
            channel_id: channel_id.into(),
            uid: uid.into(),
            purpose: purpose.into(),
            codec: codec.into(),
            key_id,
            sender_id,
            sample_rate,
            channels,
            action: "start".into(),
        }
    }

    /// 构造一条「停止」媒体会话通知。
    pub fn stop(
        channel_id: impl Into<String>,
        uid: impl Into<String>,
        purpose: impl Into<String>,
    ) -> Self {
        Self {
            kind: "media_meta".into(),
            channel_id: channel_id.into(),
            uid: uid.into(),
            purpose: purpose.into(),
            codec: String::new(),
            key_id: 0,
            sender_id: 0,
            sample_rate: None,
            channels: None,
            action: "stop".into(),
        }
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    pub fn parse(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice(bytes).ok()
    }
}

/// 尝试解析 MLS 解密后的 payload。
#[derive(Debug, Clone)]
pub enum DecodedPayload {
    Message(MessagePayload),
    Reaction(ReactionPayload),
    MediaMeta(MediaMetaPayload),
    Edit(EditPayload),
    Delete(DeletePayload),
    /// 无法识别（旧版消息或纯文本）。
    Raw(Vec<u8>),
}

pub fn decode(bytes: &[u8]) -> DecodedPayload {
    // 顺序安全（关键）：Edit/Delete 的 JSON 也含 c/u/t 字段，会被
    // MessagePayload 的 serde 解析接受（多余字段被忽略）。所以必须先试
    // Edit / Delete，再试 Message。
    //
    // 其余类型靠字段名天然区分：
    // - MessagePayload 必填 c / u / t
    // - ReactionPayload 必填 c / u / target_ts / emoji / action
    // - MediaMetaPayload 必填 kind / channel_id / uid / purpose / codec / action

    if let Some(e) = EditPayload::parse(bytes)
        && e.kind == "edit"
        && e.target_ts > 0
        && !e.c.is_empty()
        && !e.u.is_empty()
        && !e.t.is_empty()
    {
        return DecodedPayload::Edit(e);
    }
    if let Some(d) = DeletePayload::parse(bytes)
        && d.kind == "delete"
        && d.target_ts > 0
        && !d.c.is_empty()
        && !d.u.is_empty()
    {
        return DecodedPayload::Delete(d);
    }
    // 先试 MessagePayload（更常见）
    if let Some(p) = MessagePayload::parse(bytes)
        && !p.c.is_empty()
        && !p.u.is_empty()
    {
        return DecodedPayload::Message(p);
    }
    if let Some(r) = ReactionPayload::parse(bytes)
        && !r.c.is_empty()
        && !r.u.is_empty()
    {
        return DecodedPayload::Reaction(r);
    }
    if let Some(m) = MediaMetaPayload::parse(bytes) {
        // 防御性校验：kind 必须是 "media_meta"，
        // 且 channel_id / uid 非空。
        if m.kind == "media_meta" && !m.channel_id.is_empty() && !m.uid.is_empty() {
            return DecodedPayload::MediaMeta(m);
        }
    }
    DecodedPayload::Raw(bytes.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_roundtrip() {
        let p = MessagePayload::new("general", "alice", "hi", Vec::new(), None);
        let b = p.to_bytes().unwrap();
        match decode(&b) {
            DecodedPayload::Message(q) => assert_eq!(q.t, "hi"),
            other => panic!("expected Message, got {:?}", other),
        }
    }

    #[test]
    fn media_meta_roundtrip_start() {
        let m = MediaMetaPayload::start(
            "voice",
            "alice",
            "voice-audio",
            "pcm-s16le",
            42,
            0xABCD1234,
            Some(48000),
            Some(2),
        );
        let b = m.to_bytes().unwrap();
        match decode(&b) {
            DecodedPayload::MediaMeta(q) => {
                assert_eq!(q.kind, "media_meta");
                assert_eq!(q.channel_id, "voice");
                assert_eq!(q.uid, "alice");
                assert_eq!(q.purpose, "voice-audio");
                assert_eq!(q.codec, "pcm-s16le");
                assert_eq!(q.key_id, 42);
                assert_eq!(q.sender_id, 0xABCD1234);
                assert_eq!(q.sample_rate, Some(48000));
                assert_eq!(q.channels, Some(2));
                assert_eq!(q.action, "start");
            }
            other => panic!("expected MediaMeta, got {:?}", other),
        }
    }

    #[test]
    fn media_meta_roundtrip_stop() {
        let m = MediaMetaPayload::stop("voice", "alice", "screen-video");
        let b = m.to_bytes().unwrap();
        match decode(&b) {
            DecodedPayload::MediaMeta(q) => {
                assert_eq!(q.action, "stop");
                assert_eq!(q.purpose, "screen-video");
                assert!(q.sample_rate.is_none());
                assert!(q.channels.is_none());
            }
            other => panic!("expected MediaMeta, got {:?}", other),
        }
    }

    #[test]
    fn media_meta_not_confused_with_message() {
        // MediaMeta 的 JSON 字段里没有 c / u / t，所以不会被识别为 Message
        let m = MediaMetaPayload::start(
            "voice",
            "alice",
            "voice-audio",
            "pcm-s16le",
            1,
            2,
            Some(48000),
            Some(1),
        );
        let b = m.to_bytes().unwrap();
        let as_msg = MessagePayload::parse(&b);
        assert!(as_msg.is_none() || as_msg.unwrap().c.is_empty());
    }

    #[test]
    fn unknown_payload_falls_back_to_raw() {
        let d = decode(b"{\"weird\": true}");
        assert!(matches!(d, DecodedPayload::Raw(_)));
    }

    #[test]
    fn edit_roundtrip() {
        let p = EditPayload::new("general", "alice", 12345, "edited text");
        assert_eq!(p.kind, "edit");
        let b = p.to_bytes().unwrap();
        match decode(&b) {
            DecodedPayload::Edit(q) => {
                assert_eq!(q.c, "general");
                assert_eq!(q.u, "alice");
                assert_eq!(q.target_ts, 12345);
                assert_eq!(q.t, "edited text");
            }
            other => panic!("expected Edit, got {:?}", other),
        }
    }

    #[test]
    fn delete_roundtrip() {
        let p = DeletePayload::new("general", "alice", 12345);
        assert_eq!(p.kind, "delete");
        let b = p.to_bytes().unwrap();
        match decode(&b) {
            DecodedPayload::Delete(q) => {
                assert_eq!(q.c, "general");
                assert_eq!(q.u, "alice");
                assert_eq!(q.target_ts, 12345);
            }
            other => panic!("expected Delete, got {:?}", other),
        }
    }

    #[test]
    fn edit_bytes_never_decoded_as_message() {
        // Edit 的 JSON 含 c/u/t，MessagePayload 的 serde 解析会接受它
        // （多余字段被忽略）。decode 必须先识别 Edit。
        let b = EditPayload::new("general", "alice", 12345, "new")
            .to_bytes()
            .unwrap();
        assert!(MessagePayload::parse(&b).is_some()); // 证明确实会被误收
        assert!(matches!(decode(&b), DecodedPayload::Edit(_)));
    }

    #[test]
    fn delete_bytes_never_decoded_as_message() {
        // Delete 的 JSON 含 c/u，但缺 t——MessagePayload 不会接受。
        // 仍验证 decode 落到 Delete 而非 Raw。
        let b = DeletePayload::new("general", "alice", 12345)
            .to_bytes()
            .unwrap();
        assert!(matches!(decode(&b), DecodedPayload::Delete(_)));
    }

    #[test]
    fn message_bytes_never_decoded_as_edit_or_delete() {
        // 普通 message 没有 kind / target_ts，绝不能被 decode 成 Edit/Delete。
        let b = MessagePayload::new("general", "alice", "hi", Vec::new(), None)
            .to_bytes()
            .unwrap();
        match decode(&b) {
            DecodedPayload::Message(q) => assert_eq!(q.t, "hi"),
            other => panic!("expected Message, got {:?}", other),
        }
    }

    #[test]
    fn edit_rejects_bad_kind_and_ts() {
        // kind 不对：不能当 Edit（含 c/u/t，会按 Message 处理）。
        let raw = br#"{"kind":"not-edit","c":"general","u":"alice","target_ts":1,"t":"x"}"#;
        assert!(matches!(decode(raw), DecodedPayload::Message(_)));

        // target_ts <= 0：无效 Edit。
        let mut p = EditPayload::new("general", "alice", 0, "x");
        let b = p.to_bytes().unwrap();
        assert!(!matches!(decode(&b), DecodedPayload::Edit(_)));
        p.target_ts = -5;
        let b = p.to_bytes().unwrap();
        assert!(!matches!(decode(&b), DecodedPayload::Edit(_)));
    }

    #[test]
    fn delete_rejects_bad_kind_and_ts() {
        let raw = br#"{"kind":"not-delete","c":"general","u":"alice","target_ts":1}"#;
        assert!(matches!(decode(raw), DecodedPayload::Raw(_)));

        let b = DeletePayload::new("general", "alice", 0)
            .to_bytes()
            .unwrap();
        assert!(!matches!(decode(&b), DecodedPayload::Delete(_)));
    }
}
