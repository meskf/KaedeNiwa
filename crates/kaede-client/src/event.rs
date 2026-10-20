//! 客户端事件。所有事件携带 `server_id`，UI 层按 server 分发。

use kaede_protocol::UserId;

/// 客户端事件流。
#[derive(Debug, Clone)]
pub enum ClientEvent {
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

    /// 有 peer 加入。
    PeerJoined { server_id: String, uid: UserId },

    /// 有 peer 离开。
    PeerLeft { server_id: String, uid: UserId },

    /// 收到 MLS 应用消息（已解密的明文）。
    AppMessage {
        server_id: String,
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

    /// 收到 WebRTC 信令。
    SdpOffer {
        server_id: String,
        from: UserId,
        sdp: String,
    },
    SdpAnswer {
        server_id: String,
        from: UserId,
        sdp: String,
    },
    IceCandidate {
        server_id: String,
        from: UserId,
        candidate: String,
    },

    /// 收到 SFU token。
    SfuToken {
        server_id: String,
        token: String,
        ws_url: String,
        room: String,
        identity: String,
    },

    /// 收到 DM 消息（B3 方案：per-conv AEAD 已解密）。
    DmReceived {
        server_id: String,

        conv_id: String,

        from: UserId,

        ts: i64,

        plaintext: Vec<u8>,
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
}
