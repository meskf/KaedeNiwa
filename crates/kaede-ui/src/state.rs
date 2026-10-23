//! UI 状态层。

use crate::theme::Mode;
use dioxus::prelude::*;
use kaede_client::http::{DmConversation, Friend};
use kaede_client::{ClientState, RoomSession};
use kaede_protocol::{Attachment, Channel, Member, Reaction, ReplyRef, UserId};
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
    /// @ 提及的 login_id（来自解密后的 payload）。
    pub mentions: Vec<String>,
}

/// 每个频道 / DM 会话的消息缓存上限。
///
/// 只约束实时消息与本地发送造成的增长（避免 DOM / 内存无限膨胀）；
/// 用户主动“加载更早”的分页不受限制——那是显式操作，且服务端保存了完整密文历史。
pub const MAX_CACHED_MESSAGES: usize = 1000;
/// 超过上限后一次性裁到的条数（避免每条消息都裁一次）。
pub const TRIM_CACHED_MESSAGES_TO: usize = 500;

/// 维持消息窗口：超出上限时从最旧的一端丢弃，保留最近的对话。
///
/// 所有实时追加 / 本地回显路径都必须调用它；历史分页反向前插不调用。
pub fn trim_cached_messages(list: &mut Vec<UiMessage>) {
    if list.len() > MAX_CACHED_MESSAGES {
        let drop_n = list.len() - TRIM_CACHED_MESSAGES_TO;
        list.drain(0..drop_n);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum AccountMode {
    #[default]
    Common,
    PerServer,
}

#[derive(Clone, PartialEq, Default)]
pub struct AccountInfo {
    pub login_id: String,
    pub password: zeroize::Zeroizing<String>,
}

impl std::fmt::Debug for AccountInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountInfo")
            .field("login_id", &self.login_id)
            .field("password", &"***")
            .finish()
    }
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

/// 未读状态：条数 + 其中 @ 自己的条数。
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct UnreadView {
    pub count: u32,
    pub mentions: u32,
}

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
    /// 已尝试加载过历史的频道（`server::channel`）。
    ///
    /// MLS 历史解密有“一次性”限制（同一密文二次解密会被当重放拒绝），
    /// 因此每个频道在一次会话内只拉取一次，避免空列表反复请求。
    pub loaded_channels: Signal<std::collections::HashSet<String>>,
    /// 自动重连是否正在进行（防重复任务）。
    pub reconnecting: Signal<bool>,
    /// R3b: in-flight host_add_peer 去重。
    /// 防止同一 peer 的多个 PeerJoined 事件并发触发 add，导致 epoch 前进竞争。
    /// 内容 = 正在处理中的 uid 集合；task 完成/失败后移除（panic 除外，见 observer.rs 注释）。
    pub pending_adds: Signal<std::collections::HashSet<String>>,
    /// 未读计数，键为 `server::channel_id` / `server::conv_id`。
    pub unread: Signal<HashMap<String, UnreadView>>,
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
            loaded_channels: Signal::new(std::collections::HashSet::new()),
            reconnecting: Signal::new(false),
            is_host: Signal::new(None),
            pending_adds: Signal::new(std::collections::HashSet::new()),
            unread: Signal::new(HashMap::new()),
        }
    }

    pub fn channel_key(server_id: &str, channel_id: &str) -> String {
        format!("{}::{}", server_id, channel_id)
    }

    /// 从服务器刷新未读计数（频道 + DM）。
    ///
    /// 服务器返回的计数不含 @ 信息；本地 mention 计数在刷新时清零，
    /// 之后由实时消息重新累加。
    pub async fn refresh_unread(mut self) {
        let Some(room) = self.room.read().clone() else {
            return;
        };
        let Some(sid) = self.current_server.read().clone() else {
            return;
        };
        let Ok(counts) = room.unread_counts().await else {
            return;
        };
        let prefix = format!("{}::", sid);
        let mut map = self.unread.write();
        map.retain(|k, _| !k.starts_with(&prefix));
        for (ch, count) in counts.channels {
            if count > 0 {
                map.insert(
                    Self::channel_key(&sid, &ch),
                    UnreadView { count, mentions: 0 },
                );
            }
        }
        for (conv, count) in counts.dms {
            if count > 0 {
                map.insert(
                    Self::channel_key(&sid, &conv),
                    UnreadView { count, mentions: 0 },
                );
            }
        }
    }

    /// 新消息到达时累加未读。
    pub fn bump_unread(mut self, server_id: &str, id: &str, mention_me: bool) {
        let key = Self::channel_key(server_id, id);
        let mut map = self.unread.write();
        let entry = map.entry(key).or_default();
        entry.count = entry.count.saturating_add(1);
        if mention_me {
            entry.mentions = entry.mentions.saturating_add(1);
        }
    }

    /// 打开频道/DM 时清零未读。
    pub fn clear_unread(mut self, server_id: &str, id: &str) {
        let key = Self::channel_key(server_id, id);
        self.unread.write().remove(&key);
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
pub enum LayoutMode {
    #[default]
    Desktop,
    #[allow(dead_code)] // M3 平板双栏启用
    Tablet,
    Mobile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MobileDetailKind {
    #[default]
    Channel,
    Dm,
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
    pub layout_mode: Signal<LayoutMode>,
    /// 移动端是否处于详情页（频道消息 / 私聊会话），None = 列表页。
    pub mobile_detail: Signal<Option<String>>,
    /// 详情页类型。mobile_detail 有值时一定伴随非 None。
    pub mobile_detail_kind: Signal<Option<MobileDetailKind>>,
    /// 移动端语音面板 Sheet 是否展开。
    pub voice_sheet_open: Signal<bool>,
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
    /// 频道成员列表弹窗：值为频道 id。
    pub channel_members_open: Signal<Option<String>>,
    /// 本地消息搜索弹窗。
    pub search_open: Signal<bool>,
    /// 附件上传进度：(文件名, 已发送百分比 0..=100)。
    pub upload_progress: Signal<Option<(String, u32)>>,
    /// Round 7：等待用户确认的主群 Welcome。按 Vec 顺序排队展示。
    pub pending_welcomes: Signal<Vec<PendingWelcomeUi>>,
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
            // 原生移动端默认走 MobileShell；Web 端在 app.rs 里按窗口宽度检测。
            layout_mode: Signal::new(if cfg!(any(target_os = "android", target_os = "ios")) {
                LayoutMode::Mobile
            } else {
                LayoutMode::default()
            }),
            mobile_detail: Signal::new(None),
            mobile_detail_kind: Signal::new(None),
            voice_sheet_open: Signal::new(false),
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
            channel_members_open: Signal::new(None),
            search_open: Signal::new(false),
            upload_progress: Signal::new(None),
            pending_welcomes: Signal::new(Vec::new()),
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

/// UI 层屏幕共享统计快照（与 kaede_media_session::ScreenStats 解耦，
/// 以便 wasm 目标也能持有该字段）。
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct ScreenStatsView {
    pub width: u32,
    pub height: u32,
    pub fps_actual: f64,
    pub fps_target: f64,
    pub bitrate_kbps: u32,
    pub bitrate_actual_kbps: u32,
}

#[derive(Clone, Copy)]
pub struct VoiceState {
    pub active_channel: Signal<Option<VoiceChannel>>,
    /// 当前语音会话的媒体 handle。`None` = 未连接。
    ///
    /// **只能在 `app.rs` 里通过 `use_context_provider` 持有**——不可放进
    /// 局部 Signal。否则组件重渲染会 drop handle 断流。
    ///
    /// wasm 下不存在该字段（`kaede-media-session` 是 native-only 依赖）。
    #[cfg(all(not(target_arch = "wasm32"), feature = "desktop-media"))]
    pub handle: Signal<Option<std::sync::Arc<kaede_media_session::NativeHandle>>>,
    /// C16: 持有 attach_audio 返回的 ConsumerHandle。
    /// 原实现直接 drop 会导致后台 task 泄漏，同时无法在离开频道/对方断开时主动停止。
    #[cfg(all(not(target_arch = "wasm32"), feature = "desktop-media"))]
    pub media_consumers: Signal<
        Vec<(
            kaede_media_session::TrackId,
            kaede_media_session::ConsumerHandle,
        )>,
    >,
    /// 当前选择的视频画质档位；屏幕共享与共享中重建都按此配置编码器。
    pub current_profile: Signal<kaede_protocol::VideoProfile>,
    /// 屏幕共享实时统计（开启时才有值）。
    pub screen_stats: Signal<Option<ScreenStatsView>>,
    /// 码率档位（低 / 标准 / 无损）。
    pub current_bitrate: Signal<kaede_protocol::VideoBitrate>,
    /// 主群 epoch 变化后，请求重建当前语音媒体会话（值为 channel_id）。
    pub reconnect_request: Signal<Option<String>>,
}

impl VoiceState {
    pub fn new() -> Self {
        Self {
            active_channel: Signal::new(None),
            #[cfg(all(not(target_arch = "wasm32"), feature = "desktop-media"))]
            handle: Signal::new(None),
            #[cfg(all(not(target_arch = "wasm32"), feature = "desktop-media"))]
            media_consumers: Signal::new(Vec::new()),
            current_profile: Signal::new(kaede_protocol::DEFAULT_VIDEO_PROFILE),
            current_bitrate: Signal::new(kaede_protocol::DEFAULT_VIDEO_BITRATE),
            screen_stats: Signal::new(None),
            reconnect_request: Signal::new(None),
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
                http_url: "http://localhost:18080".into(),
                ws_url: "ws://localhost:18080".into(),
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
            mentions: vec![],
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

#[cfg(target_arch = "wasm32")]
/// D1: 拖拽 / 粘贴桥接用的上传载荷（wasm-only）。
#[derive(Clone, Debug)]
pub struct UploadFile {
    pub name: String,
    pub mime: String,
    pub data: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(ts: i64) -> UiMessage {
        UiMessage {
            ts,
            text: format!("m{ts}"),
            ..Default::default()
        }
    }

    #[test]
    fn trims_oldest_when_over_cap() {
        let mut list: Vec<UiMessage> = (0..=MAX_CACHED_MESSAGES as i64).map(msg).collect();
        trim_cached_messages(&mut list);
        assert_eq!(list.len(), TRIM_CACHED_MESSAGES_TO);
        // 保留的是最近的对话：最旧一条被丢弃。
        assert_eq!(
            list.first().map(|m| m.ts),
            Some((MAX_CACHED_MESSAGES - TRIM_CACHED_MESSAGES_TO) as i64 + 1)
        );
    }

    #[test]
    fn keeps_window_at_or_below_cap() {
        let mut list: Vec<UiMessage> = (0..MAX_CACHED_MESSAGES as i64).map(msg).collect();
        trim_cached_messages(&mut list);
        assert_eq!(list.len(), MAX_CACHED_MESSAGES);
        assert_eq!(list.first().map(|m| m.ts), Some(0));
    }
}

/// Round 7：UI 层展示用的 pending 主群 Welcome 条目。
///
/// 相比 `kaede_client::PendingWelcomeInfo`，这里带 `server_id` 和
/// `from: UserId`，便于 UI 显示发送者昵称与区分不同 server 的请求。
#[derive(Clone, PartialEq, Debug)]
pub struct PendingWelcomeUi {
    pub server_id: String,
    pub sender_login_id: String,
    pub from: UserId,
    pub is_rebuild: bool,
}
