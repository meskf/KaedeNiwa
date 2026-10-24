//! 客户端事件。所有事件携带 `server_id`，UI 层按 server 分发。

use kaede_protocol::UserId;

/// 客户端事件流。
#[derive(Debug, Clone)]
#[derive(serde::Serialize)]
pub enum ClientEvent {
    /// 收到群组应用消息（明文载荷，由 UI 决定如何展示）
    AppMessageReceived {
        from: String,
        channel_id: String,
        payload: Vec<u8>,
    },
    /// 状态变化。
    StateChanged {
        server_id: String,
        from: crate::ClientState,
        to: crate::ClientState,
    },

    /// 与服务器握手完成。
    HelloAck {
        server_id: String,
        server_name: String,
        protocol_version: u16,
    },

    /// 成功加入房间，携带当前所有 peer。
    Joined {
        server_id: String,
        peers: Vec<UserId>,
    },

    /// 新频道创建。
    ChannelCreated {
        server_id: String,
        channel: kaede_protocol::Channel,
    },

    /// 频道更新。
    ChannelUpdated {
        server_id: String,
        channel: kaede_protocol::Channel,
    },

    /// 频道删除。
    ChannelDeleted {
        server_id: String,
        channel_id: String,
    },

    /// 在线状态更新。
    PresenceUpdated {
        server_id: String,
        presence: Vec<kaede_protocol::Presence>,
    },

    /// 成员资料更新。
    MemberUpdated {
        server_id: String,
        member: kaede_protocol::Member,
    },

    /// 成员被踢。
    MemberRemoved { server_id: String, login_id: String },

    /// 好友关系发生变化（请求 / 接受 / 删除）。
    ///
    /// 无业务负载：UI 收到后重新拉取好友列表。
    FriendsChanged { server_id: String },

    /// 附件上传进度（已发送字节 / 总字节）。
    AttachmentProgress {
        server_id: String,
        name: String,
        sent: u64,
        total: u64,
    },

    /// 有 peer 加入。
    PeerJoined { server_id: String, uid: UserId },

    /// 对端正在输入（协议 v7）。
    ///
    /// 服务端不持久化、不离线补投；UI 应本地设置数秒超时自动消失。
    Typing {
        server_id: String,
        channel_id: String,
        from: UserId,
    },

    /// 有 peer 离开。
    PeerLeft { server_id: String, uid: UserId },

    /// 收到 MLS 应用消息（已解密的明文）。
    AppMessage {
        server_id: String,
        from: UserId,
        plaintext: Vec<u8>,
    },

    /// 成功加入一个私密频道（首次或迟到 Welcome 后）。
    ChannelJoined {
        server_id: String,
        channel_id: String,
    },

    /// 收到私密频道实时消息（已用频道 group key 解密）。
    ///
    /// 与 `AppMessage` 分开，避免把主群广播误当私密频道内容。
    ChannelMessage {
        server_id: String,
        channel_id: String,
        from: UserId,
        /// 服务器权威时间戳。
        ts: i64,
        plaintext: Vec<u8>,
    },

    /// 收到私密频道表情反应（已用频道 group key 解密）。
    ChannelReaction {
        server_id: String,
        channel_id: String,
        from: UserId,
        plaintext: Vec<u8>,
    },

    /// 收到媒体会话信令（媒体格式元数据 / 开始 / 停止）。
    ///
    /// 由 `MlsAppMessage` 解密后经 `payload::decode` 识别为
    /// [`crate::payload::MediaMetaPayload`] 时触发。
    /// UI 层应该把它存到 `peer_media_meta: HashMap<uid, MediaMetaPayload>`。
    MediaMetaReceived {
        server_id: String,
        from: UserId,
        meta: crate::payload::MediaMetaPayload,
    },

    /// 收到 DM 消息（B3 方案：per-conv AEAD 已解密）。
    DmReceived {
        server_id: String,

        conv_id: String,

        from: UserId,

        ts: i64,

        plaintext: Vec<u8>,
    },

    /// 主群 MLS epoch 变化（成员变更 / self update / 收到 Commit）。
    ///
    /// 语音通话中的客户端应重新建立媒体会话，让 LiveKit E2EE key
    /// 跟随新 epoch 轮换（否则通话中有人加入/离开后互相解不开）。
    MlsEpochChanged { server_id: String, epoch: u64 },
    /// 私密频道 MLS group epoch 变化（成员加/减）。
    ///
    /// 与 `MlsEpochChanged`（主群）平行。收到此事件时若正在该频道语音，
    /// 应触发媒体会话重连——旧 epoch 派生的 key 已失效。
    ChannelEpochChanged {
        server_id: String,
        channel_id: String,
        epoch: u64,
    },

    /// 服务器报告错误。
    ServerError {
        server_id: String,
        code: kaede_protocol::ErrorCode,
        message: String,
    },

    /// 连接关闭。
    Disconnected { server_id: String, reason: String },

    /// MLS 同步失败（广播失败 / commit 处理失败）。
    ///
    /// **P1-Critical C-1/C-2**：当本地 epoch 已前进但广播失败，或远端 commit
    /// 处理失败时，MLS 状态可能分裂。UI 层应提示用户手动恢复。
    ///
    /// - `recoverable == true`：自动离开+重进语音频道可能恢复
    /// - `recoverable == false`：需要重连整个 server 才能恢复
    MlsSyncError {
        server_id: String,
        user_message: String,
        recoverable: bool,
        detail: String,
    },

    /// 收到一条主群 Welcome，等待用户显式确认入群。
    ///
    /// **为什么需要确认**：服务器不再限制谁能发 Welcome（Signal 模型），
    /// 客户端通过用户确认决定是否信任。这样任何成员都能拉人，攻击者
    /// 伪造的 Welcome 会被用户拒绝。
    ///
    /// `is_rebuild == true` 表示当前已有一个主群，接受会**丢失旧群历史**
    /// （旧 epoch 密钥不再保留）。UI 必须显式说明这一点。
    PendingWelcome {
        server_id: String,
        from: UserId,
        is_rebuild: bool,
        /// 与 pending 列表配对的 stable id（= sender login_id），
        /// 避免 UI 用 UserId 做 key 时多个 welcome 互相覆盖。
        sender_login_id: String,
    },

    /// PendingWelcome 被用户接受或拒绝后触发，UI 应关闭对应对话框。
    PendingWelcomeResolved {
        server_id: String,
        sender_login_id: String,
        accepted: bool,
    },
}
