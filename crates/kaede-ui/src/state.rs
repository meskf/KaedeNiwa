//! UI 状态层。

use crate::theme::Mode;
use dioxus::prelude::*;
use kaede_client::http::{DmConversation, Friend};
use kaede_client::{ClientState, RoomSession};
use kaede_protocol::{Attachment, Channel, Member, Reaction, ReplyRef};
use std::collections::HashMap;
use std::sync::Arc;

// ============================================================================
// 领域模型
// ============================================================================

#[derive(Clone, PartialEq, Default)]
pub struct UiMessage {
    pub id: String,
    pub channel_id: String,
    pub author: String,
    pub author_initial: String,
    pub day: String,
    pub time: String,
    pub ts: i64,
    pub text: String,
    pub attachments: Vec<Attachment>,
    pub reply_to: Option<ReplyRef>,
    pub reactions: Vec<Reaction>,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum AccountMode {
    #[default]
    Common,
    PerServer,
}

#[derive(Clone, PartialEq, Default, Debug)]
pub struct AccountInfo {
    pub login_id: String,
    pub password: String,
}

#[derive(Clone, PartialEq)]
pub struct UiServer {
    pub id: String,
    pub name: String,
    pub http_url: String,
    pub ws_url: String,
    pub state: ClientState,
    pub epoch: u64,
    pub account_mode: AccountMode,
    pub per_server_account: AccountInfo,
}

impl Default for UiServer {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            http_url: String::new(),
            ws_url: String::new(),
            state: ClientState::Idle,
            epoch: 0,
            account_mode: AccountMode::Common,
            per_server_account: AccountInfo::default(),
        }
    }
}

impl UiServer {
    #[allow(dead_code)]
    pub fn effective_account<'a>(&'a self, common: &'a AccountInfo) -> &'a AccountInfo {
        match self.account_mode {
            AccountMode::Common => common,
            AccountMode::PerServer => &self.per_server_account,
        }
    }
}

// ============================================================================
// AppState
// ============================================================================

#[derive(Clone, Copy)]
pub struct AppState {
    pub servers: Signal<HashMap<String, UiServer>>,
    pub current_server: Signal<Option<String>>,
    pub channels: Signal<HashMap<String, Vec<Channel>>>,
    pub messages: Signal<HashMap<String, Vec<UiMessage>>>,
    pub members: Signal<HashMap<String, Vec<Member>>>,
    pub presence: Signal<HashMap<String, String>>,
    pub friends: Signal<HashMap<String, Vec<Friend>>>,
    pub dm_conversations: Signal<HashMap<String, Vec<DmConversation>>>,
    pub dm_messages: Signal<HashMap<String, Vec<UiMessage>>>,
    pub room: Signal<Option<Arc<RoomSession>>>,
    /// P0-1 完整版：当前用户是否是 MLS group 的 host（op5）。
    /// host 收到 PeerJoined 会自动 host_add_peer(uid)。
    pub is_host: Signal<Option<bool>>,
    /// R3b: in-flight host_add_peer 去重。
    /// 防止同一 peer 的多个 PeerJoined 事件并发触发 add，导致 epoch 前进竞争。
    /// 内容 = 正在处理中的 uid 集合；task 完成/失败后移除（panic 除外，见 observer.rs 注释）。
    pub pending_adds: Signal<std::collections::HashSet<String>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            servers: Signal::new(HashMap::new()),
            current_server: Signal::new(None),
            channels: Signal::new(HashMap::new()),
            messages: Signal::new(HashMap::new()),
            members: Signal::new(HashMap::new()),
            presence: Signal::new(HashMap::new()),
            friends: Signal::new(HashMap::new()),
            dm_conversations: Signal::new(HashMap::new()),
            dm_messages: Signal::new(HashMap::new()),
            room: Signal::new(None),
            is_host: Signal::new(None),
            pending_adds: Signal::new(std::collections::HashSet::new()),
        }
    }

    pub fn channel_key(server_id: &str, channel_id: &str) -> String {
        format!("{}::{}", server_id, channel_id)
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// UiState
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    Login,
    Main,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MainTab {
    #[default]
    Server,
    Dm,
    Friends,
}

#[derive(Clone, Copy, PartialEq)]
pub struct UiState {
    pub theme_mode: Signal<Mode>,
    pub view: Signal<View>,
    pub selected_channel: Signal<Option<String>>,
    pub selected_message: Signal<Option<String>>,
    pub selected_dm_conv: Signal<Option<String>>,
    pub draft: Signal<String>,
    pub settings_open: Signal<bool>,

    /// 通用账户（默认用于所有服务器）。
    pub common_account: Signal<AccountInfo>,

    /// 服务器管理弹窗是否打开。
    pub server_modal_open: Signal<bool>,
    /// 弹窗里选中的服务器 id；`None` = 通用账户面板。
    pub server_modal_selected: Signal<Option<String>>,
    pub main_tab: Signal<MainTab>,
    pub members_open: Signal<bool>,
    pub create_channel_kind: Signal<Option<&'static str>>,
    pub join_channel_open: Signal<bool>,
    pub channel_menu: Signal<Option<String>>,
    pub channel_rename_input: Signal<Option<String>>,
    pub member_menu: Signal<Option<String>>,
    pub member_editing_title: Signal<Option<String>>,
    pub reply_target: Signal<Option<ReplyTarget>>,
    pub emoji_picker: Signal<Option<(String, i64)>>,
    pub toast: Signal<Option<(String, bool)>>,
    /// 画质档位选择器是否打开。
    pub video_profile_picker_open: Signal<bool>,
    /// 加入私密频道时预填的频道 ID（双击私密频道时设置）。
    pub join_channel_prefill_id: Signal<Option<String>>,
    /// 密码通过后是否立即连接语音（双击私密语音频道时为 true）。
    pub join_channel_then_voice: Signal<bool>,
    /// 成员选择器（好友添加 / 新建 DM）
    pub member_picker: Signal<Option<MemberPickerMode>>,
}

/// 成员选择器的使用场景。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberPickerMode {
    /// 从服务器成员中挑选发好友请求
    FriendRequest,
    /// 从服务器成员中挑选新建 DM 会话
    OpenDm,
}

impl UiState {
    pub fn new() -> Self {
        Self {
            theme_mode: Signal::new(Mode::default()),
            view: Signal::new(View::default()),
            selected_channel: Signal::new(None),
            selected_message: Signal::new(None),
            selected_dm_conv: Signal::new(None),
            draft: Signal::new(String::new()),
            settings_open: Signal::new(false),
            common_account: Signal::new(AccountInfo::default()),
            server_modal_open: Signal::new(false),
            server_modal_selected: Signal::new(None),
            main_tab: Signal::new(MainTab::default()),
            members_open: Signal::new(true),
            create_channel_kind: Signal::new(None),
            join_channel_open: Signal::new(false),
            channel_menu: Signal::new(None),
            channel_rename_input: Signal::new(None),
            member_menu: Signal::new(None),
            member_editing_title: Signal::new(None),
            reply_target: Signal::new(None),
            emoji_picker: Signal::new(None),
            toast: Signal::new(None),
            video_profile_picker_open: Signal::new(false),
            join_channel_prefill_id: Signal::new(None),
            join_channel_then_voice: Signal::new(false),
            member_picker: Signal::new(None),
        }
    }

    pub fn toggle_theme(&mut self) {
        let next = match *self.theme_mode.read() {
            Mode::Light => Mode::Dark,
            Mode::Dark => Mode::Light,
        };
        self.theme_mode.set(next);
    }
}

impl Default for UiState {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Voice
// ============================================================================

#[derive(Clone, Copy)]
pub struct VoiceState {
    pub active_channel: Signal<Option<VoiceChannel>>,
    /// 当前语音会话的媒体 handle。`None` = 未连接。
    ///
    /// **只能在 `app.rs` 里通过 `use_context_provider` 持有**——不可放进
    /// 局部 Signal。否则组件重渲染会 drop handle 断流。
    ///
    /// wasm 下不存在该字段（`kaede-media-session` 是 native-only 依赖）。
    #[cfg(not(target_arch = "wasm32"))]
    pub handle: Signal<Option<std::sync::Arc<kaede_media_session::NativeHandle>>>,
    /// 当前选择的视频画质档位（Phase 3a：UI 状态，未接入 media 层）。
    pub current_profile: Signal<kaede_protocol::VideoProfile>,
    /// 屏幕共享实时统计（开启时才有值）。
    pub screen_stats: Signal<Option<kaede_media_session::ScreenStats>>,
    /// 码率档位（低 / 标准 / 无损）。
    pub current_bitrate: Signal<kaede_protocol::VideoBitrate>,
}

impl VoiceState {
    pub fn new() -> Self {
        Self {
            active_channel: Signal::new(None),
            #[cfg(not(target_arch = "wasm32"))]
            handle: Signal::new(None),
            current_profile: Signal::new(kaede_protocol::DEFAULT_VIDEO_PROFILE),
            current_bitrate: Signal::new(kaede_protocol::DEFAULT_VIDEO_BITRATE),
            screen_stats: Signal::new(None),
        }
    }
}

impl Default for VoiceState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, PartialEq)]
pub struct VoiceChannel {
    pub id: String,
    pub name: String,
    pub members: Vec<VoiceMember>,
}

#[derive(Clone, PartialEq)]
pub struct VoiceMember {
    pub uid: String,
    pub name: String,
    pub initial: String,
    pub speaking: bool,
}

#[derive(Clone, Copy, PartialEq)]
pub struct VoiceControls {
    pub mic_on: Signal<bool>,
    pub speaker_on: Signal<bool>,
    pub screen_on: Signal<bool>,
}

impl VoiceControls {
    pub fn new() -> Self {
        Self {
            mic_on: Signal::new(true),
            speaker_on: Signal::new(true),
            screen_on: Signal::new(false),
        }
    }
}

impl Default for VoiceControls {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// 演示数据
// ============================================================================

impl AppState {
    #[allow(dead_code)]
    pub fn seed_demo(mut self) {
        use kaede_protocol::{Channel, Member};

        let sid = "local".to_string();
        self.servers.write().insert(
            sid.clone(),
            UiServer {
                id: sid.clone(),
                name: "本地服务器".into(),
                http_url: "http://localhost:8080".into(),
                ws_url: "ws://localhost:8080".into(),
                state: kaede_client::ClientState::MlsReady,
                epoch: 1,
                account_mode: AccountMode::Common,
                per_server_account: AccountInfo::default(),
            },
        );
        self.current_server.set(Some(sid.clone()));

        self.channels.write().insert(
            sid.clone(),
            vec![
                Channel {
                    id: "general".into(),
                    name: "general".into(),
                    kind: "text".into(),
                    position: 0,
                    created_at: 0,
                    is_private: false,
                    hash_tier: None,
                },
                Channel {
                    id: "chat".into(),
                    name: "闲聊".into(),
                    kind: "text".into(),
                    position: 1,
                    created_at: 0,
                    is_private: false,
                    hash_tier: None,
                },
                Channel {
                    id: "proj".into(),
                    name: "项目讨论".into(),
                    kind: "text".into(),
                    position: 2,
                    created_at: 0,
                    is_private: false,
                    hash_tier: None,
                },
                Channel {
                    id: "lobby".into(),
                    name: "大厅".into(),
                    kind: "voice".into(),
                    position: 3,
                    created_at: 0,
                    is_private: false,
                    hash_tier: None,
                },
            ],
        );

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        let mk = |i: i64, author: &str, text: &str, time: &str| UiMessage {
            id: format!("demo-m{}", i),
            channel_id: "general".into(),
            author: author.into(),
            author_initial: author
                .chars()
                .next()
                .map(|c| c.to_string())
                .unwrap_or_default(),
            day: "今天".into(),
            time: time.into(),
            ts: now + i,
            text: text.into(),
            attachments: vec![],
            reply_to: None,
            reactions: vec![],
        };

        let key = AppState::channel_key(&sid, "general");
        self.messages.write().insert(
            key,
            vec![
                mk(1, "Alice", "UI 重做版跑起来了。", "10:00"),
                mk(
                    2,
                    "Alice",
                    "服务器栏、频道栏、消息列表都空着，先塞点演示数据。",
                    "10:01",
                ),
                mk(3, "Bob", "看着挺干净的。", "10:05"),
                mk(
                    4,
                    "Alice",
                    "字体用了 Times New Roman + 仿宋 + 楷体。",
                    "10:06",
                ),
                mk(5, "Bob", "有点像书，还行。", "10:07"),
            ],
        );

        self.members.write().insert(
            sid.clone(),
            vec![
                Member {
                    login_id: "alice".into(),
                    nickname: "Alice".into(),
                    op: "op5".into(),
                    bio: String::new(),
                    title: String::new(),
                    avatar_url: String::new(),
                    joined_at: 0,
                },
                Member {
                    login_id: "bob".into(),
                    nickname: "Bob".into(),
                    op: "op1".into(),
                    bio: String::new(),
                    title: String::new(),
                    avatar_url: String::new(),
                    joined_at: 0,
                },
            ],
        );

        self.presence
            .write()
            .insert("alice".into(), "online".into());
        self.presence.write().insert("bob".into(), "online".into());
    }
}
/// 当前正在回复的目标消息（用于输入框上方的预览）。
#[derive(Clone, PartialEq)]
pub struct ReplyTarget {
    pub author: String,
    pub preview: String,
    pub ts: i64,
}
