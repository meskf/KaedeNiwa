//! 左栏：文件夹式分组 + 频道列表 + 语音面板 + 用户卡片。

#![allow(clippy::collapsible_if)]

use crate::i18n::use_i18n;
use dioxus::prelude::*;
use std::collections::{BTreeMap, HashSet};

use crate::icons::{IconHash, IconLock, IconMic, IconMonitor, IconPlus, IconVolume2, IconX};
use crate::state::{
    AppState, MainTab, UiState, VoiceChannel, VoiceControls, VoiceMember, VoiceState,
};

#[derive(Clone, PartialEq)]
struct Category {
    id: String,
    name: String,
    channels: Vec<kaede_protocol::Channel>,
}

fn parse_category(name: &str) -> (&str, &str) {
    if let Some((cat, rest)) = name.split_once('/') {
        let cat = cat.trim();
        let rest = rest.trim();
        if !cat.is_empty() && !rest.is_empty() {
            return (cat, rest);
        }
    }
    ("未分类", name)
}

fn build_categories(channels: &[kaede_protocol::Channel]) -> Vec<Category> {
    if channels.is_empty() {
        return Vec::new();
    }
    let mut groups: BTreeMap<String, Vec<kaede_protocol::Channel>> = BTreeMap::new();
    for ch in channels {
        let (cat, _) = parse_category(&ch.name);
        groups.entry(cat.to_string()).or_default().push(ch.clone());
    }
    let mut out: Vec<Category> = groups
        .into_iter()
        .map(|(name, mut chs)| {
            chs.sort_by_key(|c| c.position);
            Category {
                id: name.clone(),
                name,
                channels: chs,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        let ka = if a.name == "未分类" { 1 } else { 0 };
        let kb = if b.name == "未分类" { 1 } else { 0 };
        ka.cmp(&kb).then_with(|| a.name.cmp(&b.name))
    });
    out
}

fn display_name(ch: &kaede_protocol::Channel) -> String {
    parse_category(&ch.name).1.to_string()
}

/// 左栏外壳。
///
/// # 结构
///
/// ```text
/// ┌──────────────┐
/// │ 频道列表区   │ ← 随 MainTab 变化（Server/Dm/Friends）
/// │  (flex: 1)   │    Server  → ChannelList
/// │              │    Dm      → 占位
/// │              │    Friends → 占位
/// ├──────────────┤
/// │ VoicePanel   │ ← 恒定渲染。语音会话状态由全局 VoiceState 持有，
/// │              │    切 Tab 不中断语音连接。
/// ├──────────────┤
/// │ UserPanel    │ ← 恒定渲染。当前登录用户。
/// └──────────────┘
/// ```
///
/// # 关键约束
///
/// - **`VoicePanel` 必须在 `match tab` 外面**。语音会话不应随 Tab 切换停止。
/// - **未来接媒体时，`Arc<dyn MediaSessionHandle>` 必须放在全局 context**（和
///   `VoiceState` 同级，在 `app.rs` 的 `use_context_provider`），绝不能放在
///   `Sidebar` 里的局部 `Signal`——否则 Sidebar 重渲染时会 drop handle，断流。
#[component]
pub fn Sidebar() -> Element {
    let mut ui = use_context::<UiState>();
    let tab = *ui.main_tab.read();

    rsx! {
        div {
            style: "width: 220px; flex-shrink: 0; display: flex; flex-direction: column; min-height: 0;",
            div {
                style: "padding: 8px 8px 4px 8px;",
                button {
                    class: "btn-secondary",
                    style: "width: 100%; display: flex; align-items: center; gap: 6px; justify-content: center; font-size: 12px;",
                    onclick: move |_| ui.search_open.set(true),
                    "搜索本地消息"
                }
            }
            div {
                style: "flex: 1; min-height: 0; overflow-y: auto; padding: 0 8px;",
                match tab {
                    MainTab::Server => rsx! { ChannelList {} },
                    MainTab::Dm => rsx! { crate::dm_view::DmSidebar {} },
                    MainTab::Friends => rsx! { crate::friends_view::FriendsSidebar {} },
                }
            }
            // 语音面板：与 MainTab 无关，永远在底部
            VoicePanel {}
            // 用户卡片：与 MainTab 无关
            UserPanel {}
        }
    }
}

#[component]
fn SidebarPlaceholder(text: String) -> Element {
    rsx! {
        div {
            style: "padding: 24px 12px; color: var(--fg-faint); font-size: 13px; text-align: center;",
            "{text}"
        }
    }
}

#[component]
fn ChannelList() -> Element {
    let app = use_context::<AppState>();
    let mut ui = use_context::<UiState>();
    let collapsed = use_signal(HashSet::<String>::new);

    let categories = use_memo(move || {
        let sid = app.current_server.read().clone();
        let chans = app.channels.read();
        let list = sid
            .as_ref()
            .and_then(|s| chans.get(s).cloned())
            .unwrap_or_default();
        build_categories(&list)
    });

    rsx! {
                if categories.read().is_empty() {
            div {
                style: "padding: 24px 16px; text-align: center; display: flex; flex-direction: column; align-items: center; gap: 12px;",
                span { style: "color: var(--fg-faint); font-size: 13px;", "暂无频道" }
                div { style: "display: flex; gap: 8px;",
                    button {
                        class: "btn-secondary",
                        style: "display: flex; align-items: center; gap: 6px;",
                        onclick: move |_| ui.create_channel_kind.set(Some("text")),
                        IconPlus { size: 14 }
                        "文字频道"
                    }
                    button {
                        class: "btn-secondary",
                        style: "display: flex; align-items: center; gap: 6px;",
                        onclick: move |_| ui.create_channel_kind.set(Some("voice")),
                        IconVolume2 { size: 14 }
                        "语音频道"
                    }
                }
            }
        }
        for cat in categories.read().iter() {
            CategoryGroup {
                key: "{cat.id}",
                cat: cat.clone(),
                collapsed: collapsed,
            }
        }
        div { style: "padding: 8px 12px;",
            button {
                class: "btn-secondary",
                style: "width: 100%; display: flex; align-items: center; gap: 6px; justify-content: center;",
                onclick: move |_| ui.join_channel_open.set(true),
                "加入私密频道"
            }
        }
    }
}

#[component]
fn CategoryGroup(cat: Category, collapsed: Signal<HashSet<String>>) -> Element {
    let mut ui = use_context::<UiState>();
    let mut collapsed = collapsed;
    let is_collapsed = collapsed.read().contains(&cat.id);
    let arrow = if is_collapsed { ">" } else { "v" };
    let cat_id_click = cat.id.clone();
    let cat_name = cat.name.clone();
    let chs = cat.channels.clone();

    rsx! {
        div {
            style: "display: flex; align-items: center; justify-content: space-between; padding: 16px 12px 8px 12px;",
            button {
                onclick: move |_| {
                    let mut c = collapsed.write();
                    if !c.remove(&cat_id_click) {
                        c.insert(cat_id_click.clone());
                    }
                },
                style: "flex: 1; display: flex; align-items: center; gap: 8px; color: var(--fg-faint); font-size: 12px; letter-spacing: 0.8px; text-transform: uppercase; user-select: none; text-align: left;",
                span { style: "font-size: 10px; width: 10px;", "{arrow}" }
                span { "{cat_name}" }
            }
            div { style: "display: flex; gap: 2px;",
                button {
                    style: "color: var(--fg-muted); padding: 2px;",
                    title: "新建文字频道",
                    onclick: move |_| ui.create_channel_kind.set(Some("text")),
                    IconPlus { size: 12 }
                }
                button {
                    style: "color: var(--fg-muted); padding: 2px;",
                    title: "新建语音频道",
                    onclick: move |_| ui.create_channel_kind.set(Some("voice")),
                    IconVolume2 { size: 12 }
                }
            }
        }
        if !is_collapsed {
            for (i, ch) in chs.iter().enumerate() {
                ChannelRow {
                    key: "{ch.id}-{i}",
                    channel: ch.clone(),
                    display: display_name(ch),
                }
            }
        }
    }
}

#[component]
fn ChannelRow(channel: kaede_protocol::Channel, display: String) -> Element {
    let mut ui = use_context::<UiState>();
    let mut voice = use_context::<VoiceState>();
    let app = use_context::<AppState>();
    #[allow(unused_variables)]
    let controls = use_context::<VoiceControls>();

    let ch_id = channel.id.clone();
    let is_voice = channel.kind == "voice";
    let is_private = channel.is_private;

    let is_selected = use_memo({
        let ch_id = ch_id.clone();
        move || {
            if is_voice {
                voice.active_channel.read().as_ref().map(|v| v.id.as_str()) == Some(ch_id.as_str())
            } else {
                ui.selected_channel.read().as_deref() == Some(ch_id.as_str())
            }
        }
    });

    let ch_id_click = ch_id.clone();
    let ch_id_menu = ch_id.clone();
    let ch_name_click = display.clone();
    let (unread_count, mention_count) = {
        let sid = app.current_server.read().clone();
        sid.and_then(|sid| {
            app.unread
                .read()
                .get(&AppState::channel_key(&sid, &ch_id))
                .map(|u| (u.count, u.mentions))
        })
        .unwrap_or((0, 0))
    };

    rsx! {
        button {
            class: if is_selected() { "channel-row selected" } else { "channel-row" },
            ondoubleclick: move |_| {
                if is_voice {
                    // 已在某个语音频道里：忽略（先离开再进新的）
                    #[cfg(all(not(target_arch = "wasm32"), feature = "desktop-media"))]
                    {
                        if voice.handle.read().is_some() {
                            return;
                        }
                    }
                    // 私密频道只对已加入成员可见（服务端 list_channels 已过滤），
                    // 能看到就说明已通过密码校验，直接连接即可，不再重复弹密码框。
                    let ch_id_v = ch_id_click.clone();
                    let ch_name_v = ch_name_click.clone();
                    voice.active_channel.set(Some(VoiceChannel {
                        id: ch_id_v.clone(),
                        name: ch_name_v,
                        members: vec![VoiceMember { uid: "me".into(), name: "我".into(), initial: "我".into(), speaking: false }],
                    }));
                    #[cfg(all(not(target_arch = "wasm32"), feature = "desktop-media"))]
                    {
                        spawn(async move {
                            if let Err(e) = connect_voice(app, voice, controls, ch_id_v).await {
                                tracing::warn!("connect_voice failed: {}", e);
                                ui.toast.set(Some((format!("语音连接失败: {}", e), true)));
                                voice.active_channel.set(None);
                            }
                        });
                    }
                    #[cfg(target_arch = "wasm32")]
                    {
                        let room_opt = app.room.read().clone();
                        let mut voice_h = voice;
                        let controls_h = controls;
                        let mut ui_h = ui;
                        let ch_id_wasm = ch_id_v;
                        spawn(async move {
                            let Some(room) = room_opt else {
                                ui_h.toast.set(Some(("未连接".into(), true)));
                                voice_h.active_channel.set(None);
                                return;
                            };
                            if let Err(e) =
                                crate::voice_web::connect_voice(voice_h, controls_h, room, ch_id_wasm).await
                            {
                                tracing::warn!("[voice-web] connect: {}", e);
                                ui_h.toast.set(Some((format!("语音连接失败: {}", e), true)));
                                voice_h.active_channel.set(None);
                            }
                        });
                    }
                    #[cfg(all(
                        not(target_arch = "wasm32"),
                        not(feature = "desktop-media")
                    ))]
                    mobile_voice_unavailable(ui, voice);
                } else {
                    // 私密频道能出现在列表里 = 已是成员，直接打开；
                    // 未加入的频道通过“按 ID 加入”弹窗进入。
                    ui.selected_channel.set(Some(ch_id_click.clone()));
                }
            },
            oncontextmenu: move |e| {
                e.prevent_default();
                ui.channel_menu.set(Some(ch_id_menu.clone()));
            },

            span {
                style: "color: var(--fg-faint); display: flex; align-items: center;",
                if is_voice { IconVolume2 { size: 14 } } else { IconHash { size: 14 } }
            }
            span {
                style: "flex: 1; display: flex; align-items: center; gap: 4px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;",
                if is_private {
                    span {
                        style: "font-size: 11px; opacity: 0.7; flex-shrink: 0;",
                        title: "私密频道",
                        IconLock { size: 12 }
                    }
                    if let Some(tier) = channel.hash_tier.as_deref() {
                        span {
                            style: "font-size: 10px; font-family: ui-monospace, monospace; padding: 1px 5px; border-radius: 3px; background: var(--fg-muted); color: var(--inner-bg); flex-shrink: 0; font-weight: 600; letter-spacing: 0.3px;",
                            title: "Argon2id 哈希强度档位",
                            "HASH-{tier}"
                        }
                    }
                }
                span {
                    style: "overflow: hidden; text-overflow: ellipsis;",
                    "{display}"
                }
                if unread_count > 0 {
                    span {
                        title: if mention_count > 0 { "@ 提及" } else { "未读消息" },
                        style: if mention_count > 0 {
                            "flex-shrink: 0; min-width: 18px; text-align: center; padding: 1px 5px; border-radius: 9px; background: var(--negative); color: #fff; font-size: 10px; font-weight: 700;"
                        } else {
                            "flex-shrink: 0; min-width: 18px; text-align: center; padding: 1px 5px; border-radius: 9px; background: var(--active-bg); color: var(--fg); font-size: 10px; font-weight: 600;"
                        },
                        "{unread_count}"
                    }
                }
            }
        }
    }
}

#[component]
pub fn ChannelContextMenu() -> Element {
    let mut ui = use_context::<UiState>();
    let mut app = use_context::<AppState>();

    let ch_id_opt = ui.channel_menu.read().clone();
    let Some(ch_id) = ch_id_opt else {
        return rsx! {};
    };

    let (ch_name, sid, ch_private) = {
        let Some(sid) = app.current_server.read().clone() else {
            return rsx! {};
        };
        let chans = app.channels.read();
        match chans
            .get(&sid)
            .and_then(|l| l.iter().find(|c| c.id == ch_id).cloned())
        {
            Some(c) => (c.name, sid, c.is_private),
            None => return rsx! {},
        }
    };

    let rename_input = ui.channel_rename_input.read().clone();
    let is_editing = rename_input.is_some();
    let rename_input_str = rename_input.clone().unwrap_or_default();
    let ch_name_for_btn = ch_name.clone();
    let ch_id_for_copy = ch_id.clone();
    let ch_id_for_leave = ch_id.clone();
    let ch_id_for_members = ch_id.clone();
    let sid_for_leave = sid.clone();

    rsx! {
        div {
            class: "server-modal-backdrop",
            style: "background: rgba(0,0,0,0.15);",
            onclick: move |_| {
                ui.channel_menu.set(None);
                ui.channel_rename_input.set(None);
            },
            div {
                onclick: move |e| e.stop_propagation(),
                style: "background: var(--inner-bg); border-radius: 12px; padding: 8px; min-width: 220px; box-shadow: 0 8px 32px rgba(0,0,0,0.15);",
                div { style: "padding: 8px 12px; font-size: 11px; color: var(--fg-faint); text-transform: uppercase; letter-spacing: 0.8px;", "# {ch_name}" }
                div {
                    style: "display: flex; align-items: center; gap: 6px; padding: 0 12px 8px 12px; margin-top: -4px;",
                    span {
                        style: "flex: 1; font-size: 11px; color: var(--fg-muted); font-family: ui-monospace, monospace; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;",
                        title: "频道 ID",
                        "{ch_id_for_copy}"
                    }
                    button {
                        style: "flex-shrink: 0; padding: 3px 10px; font-size: 11px; border-radius: 4px; background: var(--inner-bg); color: var(--fg-muted); border: none; cursor: pointer;",
                        onclick: {
                            let id = ch_id_for_copy.clone();
                            move |_| {
                                // C17: 用 {:?}（Debug 格式）保证 id 在 JS 字符串里被正确转义
                                let script = format!(
                                    "navigator.clipboard.writeText({:?}).catch(()=>{{}})",
                                    id
                                );
                                let _ = dioxus::document::eval(&script);
                                ui.toast.set(Some(("已复制频道 ID".into(), false)));
                            }
                        },
                        "复制"
                    }
                }
                if is_editing {
                    div {
                        style: "padding: 8px; display: flex; flex-direction: column; gap: 6px;",
                        input {
                            r#type: "text",
                            value: "{rename_input_str}",
                            placeholder: "新频道名",
                            oninput: move |e| ui.channel_rename_input.set(Some(e.value())),
                            style: "padding: 6px 10px; border-radius: 6px; border: 1px solid rgba(128,128,128,0.35); background: rgba(0,0,0,0.05); color: var(--fg); font-size: 14px; outline: none;",
                        }
                        div {
                            style: "display: flex; gap: 6px;",
                            button {
                                class: "btn-secondary",
                                style: "flex: 1;",
                                onclick: move |_| ui.channel_rename_input.set(None),
                                "取消"
                            }
                            button {
                                class: "btn-primary",
                                style: "flex: 1;",
                                onclick: {
                                    let ch_id = ch_id.clone();
                                    let sid = sid.clone();
                                    move |_| {
                                        let new_name = ui.channel_rename_input.read().clone().unwrap_or_default();
                                        let new_name = new_name.trim().to_string();
                                        if new_name.is_empty() { return; }
                                        let Some(room) = app.room.read().clone() else { return; };
                                        let ch_id_inner = ch_id.clone();
                                        let sid_inner = sid.clone();
                                        spawn(async move {
                                            let patch = kaede_client::http::UpdateChannelReq {
                                                name: Some(new_name),
                                                position: None,
                                            };
                                            match room.update_channel(&ch_id_inner, patch).await {
                                                Ok(updated) => {
                                                    if let Some(list) = app.channels.write().get_mut(&sid_inner) {
                                                        if let Some(slot) = list.iter_mut().find(|c| c.id == ch_id_inner) {
                                                            *slot = updated;
                                                        }
                                                        list.sort_by_key(|c| c.position);
                                                    }
                                                    ui.channel_rename_input.set(None);
                                                    ui.channel_menu.set(None);
                                                    ui.toast.set(Some(("频道已重命名".into(), false)));
                                                }
                                                Err(e) => {
                                                    ui.toast.set(Some((format!("重命名失败: {}", e), true)));
                                                }
                                            }
                                        });
                                    }
                                },
                                "保存"
                            }
                        }
                    }
                } else {
                    button {
                        style: "width: 100%; text-align: left; padding: 8px 12px; border-radius: 6px; font-size: 14px;",
                        onclick: move |_| {
                            ui.channel_rename_input.set(Some(ch_name_for_btn.clone()));
                        },
                        "重命名频道"
                    }
                    button {
                        style: "width: 100%; text-align: left; padding: 8px 12px; border-radius: 6px; font-size: 14px;",
                        onclick: move |_| {
                            ui.channel_members_open.set(Some(ch_id_for_members.clone()));
                            ui.channel_menu.set(None);
                            ui.channel_rename_input.set(None);
                        },
                        "查看成员"
                    }
                    if ch_private {
                        button {
                            style: "width: 100%; text-align: left; padding: 8px 12px; border-radius: 6px; color: var(--negative); font-size: 14px;",
                            onclick: move |_| {
                                let Some(room) = app.room.read().clone() else { return; };
                                let ch_leave = ch_id_for_leave.clone();
                                let sid_clone = sid_for_leave.clone();
                                ui.channel_menu.set(None);
                                ui.channel_rename_input.set(None);
                                spawn(async move {
                                    match room.leave_channel(&ch_leave).await {
                                        Ok(()) => {
                                            if let Some(list) = app.channels.write().get_mut(&sid_clone) {
                                                list.retain(|c| c.id != ch_leave);
                                            }
                                            if ui.selected_channel.read().as_deref()
                                                == Some(ch_leave.as_str())
                                            {
                                                ui.selected_channel.set(None);
                                            }
                                            ui.toast
                                                .set(Some(("已退出私密频道".into(), false)));
                                        }
                                        Err(e) => {
                                            ui.toast.set(Some((
                                                format!("退出频道失败: {}", e),
                                                true,
                                            )));
                                        }
                                    }
                                });
                            },
                            "退出频道"
                        }
                    }
                    button {
                        style: "width: 100%; text-align: left; padding: 8px 12px; border-radius: 6px; color: var(--negative); font-size: 14px;",
                        onclick: move |_| {
                            let Some(room) = app.room.read().clone() else { return; };
                            let ch_del = ch_id.clone();
                            let sid_clone = sid.clone();
                            ui.channel_menu.set(None);
                            ui.channel_rename_input.set(None);
                            spawn(async move {
                                match room.delete_channel(&ch_del).await {
                                    Ok(_) => {
                                        if let Some(list) = app.channels.write().get_mut(&sid_clone) {
                                            list.retain(|c| c.id != ch_del);
                                        }
                                        if ui.selected_channel.read().as_deref() == Some(ch_del.as_str()) {
                                            ui.selected_channel.set(None);
                                        }
                                        ui.toast.set(Some(("频道已删除".into(), false)));
                                    }
                                    Err(e) => {
                                        ui.toast.set(Some((format!("删除失败: {}", e), true)));
                                    }
                                }
                            });
                        },
                        "删除频道"
                    }
                }
            }
        }
    }
}

#[component]
pub fn CreateChannelModal() -> Element {
    let mut ui = use_context::<UiState>();
    let app = use_context::<AppState>();
    let mut name = use_signal(String::new);
    let busy = use_signal(|| false);
    let mut is_private = use_signal(|| false);
    let mut password = use_signal(String::new);
    let mut hash_tier = use_signal(|| "64".to_string());

    let Some(kind) = *ui.create_channel_kind.read() else {
        return rsx! {};
    };

    let title = if kind == "voice" {
        "新建语音频道"
    } else {
        "新建文字频道"
    };
    let is_busy = *busy.read();

    rsx! {
        div {
            class: "server-modal-backdrop",
            onclick: move |_| ui.create_channel_kind.set(None),
            div {
                class: "server-modal",
                style: "width: 420px; height: auto; padding: 24px; flex-direction: column;",
                onclick: move |e| e.stop_propagation(),
                div { class: "server-modal-title", "{title}" }
                div { class: "form-field",
                    label { class: "form-label", "频道名（支持 分类/名称 语法）" }
                    input {
                        class: "form-input",
                        placeholder: "例：工作/报告，或 general",
                        value: "{name}",
                        disabled: is_busy,
                        oninput: move |e| name.set(e.value()),
                        onkeydown: {
                            let pass = password();
                            let tier = hash_tier.read().clone();
                            move |e: KeyboardEvent| {
                                if e.key() == Key::Enter {
                                    e.prevent_default();
                                    do_create(ui, app, name, busy, kind, *is_private.read(), pass.clone(), tier.clone());
                                }
                            }
                        },
                    }
                }
                div { class: "form-field",
                    label {
                        style: "display: flex; align-items: center; gap: 8px; cursor: pointer;",
                        input {
                            r#type: "checkbox",
                            checked: *is_private.read(),
                            disabled: is_busy,
                            onchange: move |e| is_private.set(e.value() == "true"),
                        }
                        span { class: "form-label", style: "margin: 0;", "私密频道（需密码进入）" }
                    }
                }
                if *is_private.read() {
                    div { class: "form-field",
                        label { class: "form-label", "频道密码（4-128 字符）" }
                        input {
                            class: "form-input",
                            r#type: "password",
                            placeholder: "输入频道密码",
                            value: "{password}",
                            disabled: is_busy,
                            oninput: move |e| password.set(e.value()),
                        }
                    }
                    div { class: "form-field",
                        label { class: "form-label", "哈希强度（Argon2id）" }
                        div {
                            style: "display: flex; gap: 8px;",
                            for (val, label, desc) in [
                                ("19", "HASH-19", "标准 · 19MiB"),
                                ("64", "HASH-64", "推荐 · 64MiB"),
                                ("256", "HASH-256", "安全 · 256MiB"),
                            ] {
                                {
                                    let is_cur = hash_tier.read().as_str() == val;
                                    let cls = if is_cur { "btn-primary" } else { "btn-secondary" };
                                    let val_owned = val.to_string();
                                    rsx! {
                                        button {
                                            key: "{val}",
                                            class: "{cls}",
                                            style: "flex: 1; padding: 8px 4px; font-size: 12px; flex-direction: column; gap: 2px;",
                                            disabled: is_busy,
                                            onclick: move |_| hash_tier.set(val_owned.clone()),
                                            span { style: "font-weight: 600;", "{label}" }
                                            span { style: "font-size: 10px; opacity: 0.7;", "{desc}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                div { class: "form-actions",
                    div { style: "flex:1;" }
                    button {
                        class: "btn-secondary",
                        disabled: is_busy,
                        onclick: move |_| ui.create_channel_kind.set(None),
                        "取消"
                    }
                    button {
                        class: "btn-primary",
                        disabled: is_busy,
                        onclick: {
                            let pass = password();
                            let tier = hash_tier.read().clone();
                            move |_| do_create(ui, app, name, busy, kind, *is_private.read(), pass.clone(), tier.clone())
                        },
                        if is_busy { "创建中..." } else { "创建" }
                    }
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn do_create(
    mut ui: UiState,
    mut app: AppState,
    mut name: Signal<String>,
    mut busy: Signal<bool>,
    kind: &'static str,
    is_private: bool,
    password: String,
    hash_tier: String,
) {
    if *busy.read() {
        return;
    }
    let n = name.read().trim().to_string();
    if n.is_empty() {
        return;
    }
    let Some(room) = app.room.read().clone() else {
        return;
    };
    let Some(sid) = app.current_server.read().clone() else {
        return;
    };

    {
        let chans = app.channels.read();
        if let Some(list) = chans.get(&sid) {
            if list.iter().any(|c| c.name == n) {
                ui.toast.set(Some(("该频道名称已存在".to_string(), true)));
                return;
            }
        }
    }

    if is_private && (password.len() < 4 || password.len() > 128) {
        ui.toast
            .set(Some(("私密频道密码长度须 4-128 字符".to_string(), true)));
        return;
    }

    busy.set(true);

    let pw_owned: Option<String> = if is_private { Some(password) } else { None };
    let tier_owned: Option<String> = if is_private { Some(hash_tier) } else { None };
    spawn(async move {
        match room
            .create_channel_with(
                &n,
                kind,
                is_private,
                pw_owned.as_deref(),
                tier_owned.as_deref(),
            )
            .await
        {
            Ok(ch) => {
                let mut chans = app.channels.write();
                if let Some(list) = chans.get_mut(&sid) {
                    if !list.iter().any(|c| c.id == ch.id) {
                        list.push(ch.clone());
                        list.sort_by_key(|c| c.position);
                    }
                }
                drop(chans);
                if kind != "voice" {
                    ui.selected_channel.set(Some(ch.id.clone()));
                }
                ui.create_channel_kind.set(None);
                name.set(String::new());
                ui.toast.set(Some((
                    format!("频道已创建：{}  右键可复制 ID: {}", ch.name, ch.id),
                    false,
                )));
            }
            Err(e) => {
                ui.toast.set(Some((format!("创建失败: {}", e), true)));
            }
        }
        busy.set(false);
    });
}

#[component]
pub fn JoinChannelModal() -> Element {
    let mut ui = use_context::<UiState>();
    let mut app = use_context::<AppState>();
    #[allow(unused_mut)] // native 分支需要 mut（active_channel.set），wasm 分支不需要
    let mut voice = use_context::<VoiceState>();
    #[allow(unused_variables)]
    let controls = use_context::<VoiceControls>();
    let mut channel_id = use_signal(String::new);
    let mut password = use_signal(String::new);
    let mut busy = use_signal(|| false);
    let mut error = use_signal(|| Option::<String>::None);

    // 弹窗常驻组件；关闭/重新打开时必须重置本地输入，否则会锁在上一次的频道。
    use_effect(move || {
        let open = *ui.join_channel_open.read();
        let prefill = ui.join_channel_prefill_id.read().clone();
        if !open {
            channel_id.set(String::new());
            password.set(String::new());
            error.set(None);
        } else if let Some(p) = prefill {
            channel_id.set(p);
            password.set(String::new());
            error.set(None);
        }
    });

    if !*ui.join_channel_open.read() {
        return rsx! {};
    }

    // 如果是从双击私密频道触发，预填 channel_id
    let prefill = ui.join_channel_prefill_id.read().clone();
    if let Some(p) = prefill.as_ref() {
        if channel_id.read().is_empty() {
            channel_id.set(p.clone());
        }
    }
    let id_locked = prefill.is_some();

    let is_busy = *busy.read();

    let submit = move |_| {
        if *busy.read() {
            return;
        }
        let id = channel_id.read().trim().to_string();
        let pw = password.read().clone();
        if id.is_empty() || pw.is_empty() {
            error.set(Some("频道 ID 和密码都不能为空".into()));
            return;
        }

        let Some(room) = app.room.read().clone() else {
            error.set(Some("未连接".into()));
            return;
        };
        let Some(sid) = app.current_server.read().clone() else {
            error.set(Some("未选择服务器".into()));
            return;
        };

        busy.set(true);
        error.set(None);

        spawn(async move {
            match room.join_channel(&id, &pw).await {
                Ok(()) => {
                    match room.list_channels().await {
                        Ok(list) => {
                            app.channels.write().insert(sid.clone(), list);
                        }
                        Err(e) => {
                            error.set(Some(format!("刷新频道失败: {}", e)));
                        }
                    }
                    ui.join_channel_open.set(false);
                    ui.join_channel_prefill_id.set(None);
                    // 文字频道加入后直接打开；语音频道交给下面的自动连接逻辑。
                    let is_voice = app
                        .channels
                        .read()
                        .get(&sid)
                        .and_then(|l| l.iter().find(|c| c.id == id).map(|c| c.kind == "voice"))
                        .unwrap_or(false);
                    if !is_voice {
                        ui.selected_channel.set(Some(id.clone()));
                    }
                    // 如果双击的是私密语音频道，密码通过后自动连语音
                    if *ui.join_channel_then_voice.read() {
                        ui.join_channel_then_voice.set(false);
                        let ch_id_v = id.clone();
                        let _ch_name_v = app
                            .channels
                            .read()
                            .get(&sid)
                            .and_then(|l| {
                                l.iter().find(|c| c.id == ch_id_v).map(|c| c.name.clone())
                            })
                            .unwrap_or_else(|| ch_id_v.clone());
                        #[cfg(all(not(target_arch = "wasm32"), feature = "desktop-media"))]
                        {
                            voice.active_channel.set(Some(VoiceChannel {
                                id: ch_id_v.clone(),
                                name: _ch_name_v,
                                members: vec![VoiceMember {
                                    uid: "me".into(),
                                    name: "我".into(),
                                    initial: "我".into(),
                                    speaking: false,
                                }],
                            }));
                            spawn(async move {
                                if let Err(e) = connect_voice(app, voice, controls, ch_id_v).await {
                                    tracing::warn!("connect_voice failed: {}", e);
                                    ui.toast.set(Some((format!("语音连接失败: {}", e), true)));
                                    voice.active_channel.set(None);
                                }
                            });
                        }
                        #[cfg(target_arch = "wasm32")]
                        {
                            let room_opt = app.room.read().clone();
                            let mut voice_h = voice;
                            let controls_h = controls;
                            let mut ui_h = ui;
                            let ch_id_wasm = ch_id_v;
                            spawn(async move {
                                let Some(room) = room_opt else {
                                    ui_h.toast.set(Some(("未连接".into(), true)));
                                    voice_h.active_channel.set(None);
                                    return;
                                };
                                if let Err(e) = crate::voice_web::connect_voice(
                                    voice_h, controls_h, room, ch_id_wasm,
                                )
                                .await
                                {
                                    tracing::warn!("[voice-web] connect: {}", e);
                                    ui_h.toast.set(Some((format!("语音连接失败: {}", e), true)));
                                    voice_h.active_channel.set(None);
                                }
                            });
                        }
                    }
                    #[cfg(all(not(target_arch = "wasm32"), not(feature = "desktop-media")))]
                    mobile_voice_unavailable(ui, voice);
                }
                Err(e) => {
                    let msg = match &e {
                        kaede_client::ClientError::Http { status: 401, .. } => {
                            "密码错误".to_string()
                        }
                        kaede_client::ClientError::Http { status: 404, .. } => {
                            "频道不存在".to_string()
                        }
                        kaede_client::ClientError::Http { status: 409, .. } => {
                            "加入冲突，请重试".to_string()
                        }
                        _ => format!("加入失败: {}", e),
                    };
                    error.set(Some(msg));
                }
            }
            busy.set(false);
        });
    };

    rsx! {
        div {
            class: "server-modal-backdrop",
            onclick: move |_| {
                ui.join_channel_open.set(false);
                ui.join_channel_prefill_id.set(None);
                ui.join_channel_then_voice.set(false);
            },
            div {
                class: "server-modal",
                style: "width: 420px; height: auto; padding: 24px; flex-direction: column;",
                onclick: move |e| e.stop_propagation(),
                div { class: "server-modal-title", "加入私密频道" }
                div { class: "form-field",
                    label { class: "form-label", "频道 ID" }
                    input {
                        class: "form-input",
                        placeholder: "ch_xxxxxxxxxx",
                        value: "{channel_id}",
                        disabled: is_busy || id_locked,
                        oninput: move |e| channel_id.set(e.value()),
                    }
                }
                div { class: "form-field",
                    label { class: "form-label", "频道密码" }
                    input {
                        class: "form-input",
                        r#type: "password",
                        placeholder: "输入频道密码",
                        value: "{password}",
                        disabled: is_busy,
                        oninput: move |e| password.set(e.value()),
                    }
                }
                if let Some(err) = error.read().as_ref() {
                    div { style: "color: var(--negative); font-size: 13px;", "{err}" }
                }
                div { class: "form-actions",
                    div { style: "flex:1;" }
                    button {
                        class: "btn-secondary",
                        disabled: is_busy,
                        onclick: move |_| {
                            ui.join_channel_open.set(false);
                            ui.join_channel_prefill_id.set(None);
                            ui.join_channel_then_voice.set(false);
                        },
                        "取消"
                    }
                    button {
                        class: "btn-primary",
                        disabled: is_busy,
                        onclick: submit,
                        if is_busy { "加入中..." } else { "加入" }
                    }
                }
            }
        }
    }
}

/// 语音面板：**始终渲染在 Sidebar 底部**，与当前 Tab 无关。
///
/// # 为什么不能随 Tab 走
///
/// 用户可能在 Server 频道里进语音，然后切到 Dm 查看私信。
/// 如果 VoicePanel 跟随 `MainTab` 卸载，`MediaSessionHandle` 会被 drop，
/// 语音连接断掉。因此 VoicePanel 只在 `VoiceState.active_channel` 为 `None`
/// 时才会视觉消失（用户主动退出）。
///
/// 状态本身存在全局 `VoiceState`（`app.rs` 里 `use_context_provider`），
/// Sidebar 的重渲染不影响它。
#[component]
fn VoicePanel() -> Element {
    let i18n = use_i18n();
    let mut ui = use_context::<UiState>();
    #[allow(unused_variables)]
    let app = use_context::<AppState>();
    // app 在 native / wasm 两个分支都被使用（native: NativeHandle；wasm: app.room）
    let mut voice = use_context::<VoiceState>();
    let mut controls = use_context::<VoiceControls>();

    let ch = voice.active_channel.read().clone();
    let Some(ch) = ch else {
        return rsx! {};
    };

    let mic = *controls.mic_on.read();
    let spk = *controls.speaker_on.read();
    let scr = *controls.screen_on.read();
    let profile_label = voice.current_profile.read().label();

    // 主群 epoch 变化 → 媒体 E2EE key 已变，自动重建媒体会话。
    use_effect(move || {
        let req = voice.reconnect_request.read().clone();
        #[allow(unused_variables)]
        let Some(ch) = req else {
            return;
        };
        voice.reconnect_request.set(None);
        #[cfg(all(not(target_arch = "wasm32"), feature = "desktop-media"))]
        {
            let h = voice.handle.write().take();
            let consumers = std::mem::take(&mut *voice.media_consumers.write());
            let mut ui2 = ui;
            spawn(async move {
                for (_, c) in consumers {
                    c.stop().await;
                }
                if let Some(h) = h {
                    h.inner().disconnect().await;
                }
                if let Err(e) = connect_voice(app, voice, controls, ch).await {
                    ui2.toast.set(Some((format!("语音重连失败: {}", e), true)));
                }
            });
        }
        #[cfg(target_arch = "wasm32")]
        {
            let room_opt = app.room.read().clone();
            let voice_h = voice;
            let controls_h = controls;
            let mut ui_h = ui;
            let ch_id_wasm = ch;
            spawn(async move {
                crate::voice_web::leave_voice().await;
                let Some(room) = room_opt else {
                    return;
                };
                if let Err(e) =
                    crate::voice_web::connect_voice(voice_h, controls_h, room, ch_id_wasm).await
                {
                    tracing::warn!("[voice-web] reconnect: {}", e);
                    ui_h.toast.set(Some((format!("语音重连失败: {}", e), true)));
                }
            });
        }
    });

    rsx! {
        div {
            class: "voice-panel",
            div {
                class: "voice-panel-head",
                span { class: "voice-panel-icon", IconVolume2 { size: 13 } }
                span { class: "voice-panel-name", "{ch.name}" }
                span { class: "voice-panel-count", "{ch.members.len()}" }
            }
            div {
                id: "kaede-remote-videos",
                style: "display: flex; flex-direction: column; gap: 4px; padding: 6px; max-height: 240px; overflow-y: auto; background: rgba(0,0,0,0.15); border-radius: 4px; empty-cells: hide;",
            }
            if let Some(stats) = voice.screen_stats.read().as_ref() {
                {
                    let ratio = if stats.fps_target > 0.0 {
                        stats.fps_actual / stats.fps_target
                    } else {
                        0.0
                    };
                    let color = if ratio >= 0.9 {
                        "var(--positive)"
                    } else if ratio >= 0.5 {
                        "#e6a23c"
                    } else {
                        "var(--negative)"
                    };
                    let res_label = match (stats.width, stats.height) {
                        (w, _h) if w >= 7680 => "8K".to_string(),
                        (w, _h) if w >= 3840 => "4K".to_string(),
                        (w, _h) if w >= 2560 => "1440p".to_string(),
                        (w, _h) if w >= 1920 => "1080p".to_string(),
                        _ => format!("{}x{}", stats.width, stats.height),
                    };
                    rsx! {
                        div {
                            class: "voice-panel-stats",
                            style: "padding: 4px 12px 8px 12px; font-size: 11px; color: {color}; display: flex; align-items: center; gap: 6px;",
                            span { "{res_label} {stats.fps_actual:.0}/{stats.fps_target:.0} fps" }
                            span {
                                if stats.bitrate_actual_kbps > 0 {
                                    "· 目标 {stats.bitrate_kbps} / 实际 {stats.bitrate_actual_kbps} kbps"
                                } else {
                                    "· 目标 {stats.bitrate_kbps} kbps"
                                }
                            }
                        }
                    }
                }
            }
            div {
                class: "voice-panel-actions",
                button {
                    class: if mic { "vp-btn vp-on" } else { "vp-btn vp-off" },
                    title: "{i18n.t(\"voice.mic\")}",
                    onclick: move |_| {
                        #[cfg(all(not(target_arch = "wasm32"), feature = "desktop-media"))]
                        {
                            let new_val = !*controls.mic_on.read();
                            let h = voice.handle.read().clone();
                            let mut ui2 = ui;
                            spawn(async move {
                                let Some(handle) = h else { return; };
                                if new_val {
                                    let result = handle
                                        .inner()
                                        .publish(kaede_media_session::PublishSource::Mic {
                                            sample_rate: 48_000,
                                            channels: 2,
                                            denoise: true,
                                        })
                                        .await;
                                    if let Err(e) = result {
                                        tracing::warn!("mic publish: {}", e);
                                        return;
                                    }
                                } else {
                                    if let Err(e) = handle
                                        .inner()
                                        .unpublish(kaede_media_session::TrackKind::VoiceAudio)
                                        .await
                                    {
                                        tracing::warn!("mic unpublish: {}", e);
                                        ui2.toast.set(Some((format!("关麦失败: {}", e), true)));
                                        return;
                                    }
                                }
                                controls.mic_on.set(new_val);
                            });
                        }
                        #[cfg(target_arch = "wasm32")]
                        {
                            let new_val = !*controls.mic_on.read();
                            let mut controls_h = controls;
                            let mut ui_h = ui;
                            spawn(async move {
                                if let Err(e) = crate::voice_web::set_microphone(new_val).await {
                                    tracing::warn!("[voice-web] mic: {}", e);
                                    ui_h.toast.set(Some((format!("麦克风: {}", e), true)));
                                    return;
                                }
                                controls_h.mic_on.set(new_val);
                            });
                        }
                    },
                    IconMic { size: 15 }
                }
                button { class: if spk { "vp-btn vp-on" } else { "vp-btn vp-off" }, title: "{i18n.t(\"voice.speaker\")}", onclick: move |_| { let v = *controls.speaker_on.read(); controls.speaker_on.set(!v); }, IconVolume2 { size: 15 } }
                button {
                    class: if scr { "vp-btn vp-on" } else { "vp-btn vp-off" },
                    title: "{i18n.t(\"voice.screen\")}",
                    onclick: move |_| {
                        #[cfg(all(not(target_arch = "wasm32"), feature = "desktop-media"))]
                        {
                            let new_val = !*controls.screen_on.read();
                            let h = voice.handle.read().clone();
                            let profile = *voice.current_profile.read();
                            let bitrate = *voice.current_bitrate.read();
                            let mut ui2 = ui;
                            spawn(async move {
                                let Some(handle) = h else {
                                    ui2.toast.set(Some(("未连接媒体会话".into(), true)));
                                    return;
                                };
                                if new_val {
                                    let bps = profile.bitrate_bps(bitrate);
                                    let config = kaede_capture::VideoConfig {
                                        encoder: kaede_capture::EncoderKind::Auto,
                                        fps: profile.fps,
                                        bitrate_kbps: (bps / 1000) as u32,
                                        show_cursor: true,
                                        display_index: 0,
                                        width: Some(profile.width),
                                        height: Some(profile.height),
                                    };
                                    match handle
                                        .inner()
                                        .publish(kaede_media_session::PublishSource::Screen { config })
                                        .await
                                    {
                                        Ok(()) => {
                                            controls.screen_on.set(true);
                                            tracing::info!(
                                                "screen share started: {} @ {} ({} kbps)",
                                                profile.label(),
                                                bitrate.label(),
                                                bps / 1000
                                            );
                                        }
                                        Err(e) => {
                                            ui2.toast.set(Some((format!("屏幕共享失败: {}", e), true)));
                                        }
                                    }
                                } else {
                                    match handle
                                        .inner()
                                        .unpublish(kaede_media_session::TrackKind::ScreenVideo)
                                        .await
                                    {
                                        Ok(()) => {
                                            controls.screen_on.set(false);
                                            let mut v = voice;
                                            v.screen_stats.set(None);
                                        }
                                        Err(e) => {
                                            tracing::warn!("screen unpublish: {}", e);
                                            ui2.toast.set(Some((format!("停止共享失败: {}", e), true)));
                                        }
                                    }
                                }
                            });
                        }
                        #[cfg(target_arch = "wasm32")]
                        {
                            let new_val = !*controls.screen_on.read();
                            if new_val && !crate::voice_web::screen_share_supported() {
                                ui.toast.set(Some((
                                    "当前浏览器不支持屏幕共享（iOS Safari 常见）".into(),
                                    true,
                                )));
                                return;
                            }
                            let mut controls_h = controls;
                            let mut ui_h = ui;
                            let profile = *voice.current_profile.read();
                            spawn(async move {
                                if let Err(e) =
                                    crate::voice_web::set_screen_share(new_val, profile).await
                                {
                                    tracing::warn!("[voice-web] screen: {}", e);
                                    ui_h.toast.set(Some((format!("屏幕共享: {}", e), true)));
                                    return;
                                }
                                controls_h.screen_on.set(new_val);
                            });
                        }
                    },
                    IconMonitor { size: 15 }
                }
                div { style: "flex:1;" }
                button {
                    class: "vp-btn",
                    title: "画质档位",
                    onclick: move |_| ui.video_profile_picker_open.set(true),
                    "{profile_label}"
                }
                button {
                    class: "vp-btn vp-leave",
                    title: "{i18n.t(\"voice.leave\")}",
                    onclick: move |_| {
                        #[allow(unused_variables)]
                        let leaving = voice
                            .active_channel
                            .read()
                            .as_ref()
                            .map(|c| c.id.clone());
                        #[cfg(all(not(target_arch = "wasm32"), feature = "desktop-media"))]
                        {
                            // H25: 先清空本地状态，再执行异步断开，避免 UI 残留
                            let h = voice.handle.write().take();
                            let consumers = std::mem::take(&mut *voice.media_consumers.write());
                            let room = app.room.read().clone();
                            spawn(async move {
                                for (_, c) in consumers {
                                    c.stop().await;
                                }
                                if let Some(h) = h {
                                    h.inner().disconnect().await;
                                }
                                // 通知服务器结束媒体会话（seed 生命周期）。
                                if let (Some(room), Some(ch)) = (room, leaving) {
                                    let _ = room.leave_media_room(&ch).await;
                                }
                            });
                        }
                        #[cfg(target_arch = "wasm32")]
                        {
                            let room_opt = app.room.read().clone();
                            let leave_ch = leaving.clone();
                            spawn(async move {
                                crate::voice_web::leave_voice().await;
                                if let (Some(room), Some(ch)) = (room_opt, leave_ch) {
                                    let _ = room.leave_media_room(&ch).await;
                                }
                            });
                        }
                        voice.active_channel.set(None);
                    },
                    IconX { size: 16 }
                }
            }
        }
    }
}

#[component]
fn UserPanel() -> Element {
    let ui = use_context::<UiState>();

    let (name, initial, status) = {
        let acc = ui.common_account.read();
        let n = if acc.login_id.is_empty() {
            "未登录".to_string()
        } else {
            acc.login_id.clone()
        };
        let i = n
            .chars()
            .next()
            .map(|c| c.to_string())
            .unwrap_or("?".into());
        let s = if acc.login_id.is_empty() {
            "点设置配置账户"
        } else {
            "在线"
        };
        (n, i, s)
    };

    rsx! {
        div {
            style: "flex-shrink: 0; padding: 16px 12px; display: flex; align-items: center; gap: 12px;",
            div {
                style: "width: 32px; height: 32px; border-radius: 50%; background: var(--active-bg); color: var(--fg); display: flex; align-items: center; justify-content: center; font-size: 14px; flex-shrink: 0;",
                "{initial}"
            }
            div {
                style: "min-width: 0; flex: 1;",
                div { style: "font-size: 14px; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;", "{name}" }
                div { style: "font-size: 12px; color: var(--fg-faint);", "{status}" }
            }
        }
    }
}
// ============================================================================
// Voice 连接（native only）
// ============================================================================

/// 移动端原生构建（不含桌面媒体栈）进入语音频道时的提示。
#[cfg(all(not(target_arch = "wasm32"), not(feature = "desktop-media")))]
fn mobile_voice_unavailable(mut ui: UiState, mut voice: VoiceState) {
    voice.active_channel.set(None);
    spawn(async move {
        ui.toast.set(Some((
            "当前移动端构建不含语音/屏幕共享；请使用 web-shell 构建或网页版".into(),
            true,
        )));
    });
}

#[cfg(all(not(target_arch = "wasm32"), feature = "desktop-media"))]
pub(crate) async fn connect_voice(
    app: AppState,
    mut voice: VoiceState,
    mut controls: VoiceControls,
    channel_id: String,
) -> Result<(), String> {
    use kaede_media::AudioPlayer;
    use kaede_media_session::{ConnectConfig, MediaSessionHandle, NativeHandle};
    use std::sync::Arc;

    let room = app
        .room
        .read()
        .clone()
        .ok_or_else(|| "not connected".to_string())?;

    // 重连/重建媒体会话时，清掉上一轮残留的远端成员（只保留自己）。
    {
        let current = voice.active_channel.read().clone();
        if let Some(mut ch) = current {
            ch.members.retain(|m| m.uid == "me");
            voice.active_channel.set(Some(ch));
        }
    }

    // P0-2: begin_media_session 现在返回 (token, e2ee_key)。
    // - token 自带 room-level e2ee_seed
    // - e2ee_key = MLS exporter(channel_id, "livekit-e2ee", seed)
    // - 同房间所有人 seed 相同 → key 相同 → 多人语音能通
    // - 服务端不知道 exporter 输出 → 仍是 E2EE
    let (token, key) = room
        .begin_media_session(&channel_id)
        .await
        .map_err(|e| format!("begin_media_session: {}", e))?;
    let cfg = ConnectConfig {
        ws_url: token.ws_url,
        token: token.token,
        // H19: Zeroizing 包装，出作用域时清零原始 key 数组
        e2ee_key: key, // Option<Zeroizing>,
        room_name: token.room,
    };

    let handle = NativeHandle::connect(cfg)
        .await
        .map_err(|e| format!("connect: {}", e))?;

    tracing::info!(
        "[voice] connected to {} (uid={})",
        handle.room_name(),
        handle.local_uid()
    );

    // 事件循环：TrackAvailable(Audio) → attach_audio → GStreamer 播放
    //
    // 已知限制（8c-1）：attach_audio 返回的 ConsumerHandle 被 drop 泄漏，
    // 不会主动 stop。房间断开后 remote track 消失，任务自然结束。
    // 后续 8c-1.5 会引入 Signal<Arc<Mutex<Vec<ConsumerHandle>>>> 做显式清理。
    let handle_for_loop = handle.clone();
    spawn(async move {
        use kaede_media_session::{MediaEvent, RemoteKind};
        let mut rx = handle_for_loop.events();
        loop {
            match rx.recv().await {
                Ok(MediaEvent::TrackAvailable {
                    track,
                    kind: RemoteKind::Audio,
                }) => match AudioPlayer::new(48_000, 2) {
                    Ok(player) => {
                        let player = Arc::new(player);
                        match handle_for_loop.inner().attach_audio(track.clone(), player) {
                            Ok(h) => {
                                tracing::info!("[voice] audio attached");
                                // C16: 将 handle 存入全局状态，防止泄漏，并支持主动清理
                                voice.media_consumers.write().push((track, h));
                            }
                            Err(e) => tracing::warn!("[voice] attach_audio: {}", e),
                        }
                    }
                    Err(e) => tracing::warn!("[voice] AudioPlayer::new: {}", e),
                },
                Ok(MediaEvent::TrackAvailable {
                    track,
                    kind: RemoteKind::Video,
                }) => {
                    // 桌面端远端视频：解码后的 I420 帧送 GStreamer。
                    match kaede_media::LiveKitVideoSink::new() {
                        Ok(sink) => {
                            let sink = std::sync::Arc::new(sink);
                            match handle_for_loop.inner().attach_video(track.clone(), sink) {
                                Ok(h) => {
                                    tracing::info!("[voice] video attached");
                                    voice.media_consumers.write().push((track, h));
                                }
                                Err(e) => tracing::warn!("[voice] attach_video: {}", e),
                            }
                        }
                        Err(e) => tracing::warn!("[voice] LiveKitVideoSink::new: {}", e),
                    }
                }
                Ok(MediaEvent::TrackGone { track, .. }) => {
                    // C16: 对方关闭音频流，主动清理对应的 consumer handle
                    let mut to_stop = None;
                    {
                        let mut list = voice.media_consumers.write();
                        if let Some(pos) = list.iter().position(|(id, _)| *id == track) {
                            to_stop = Some(list.remove(pos).1);
                        }
                    }
                    if let Some(h) = to_stop {
                        spawn(async move {
                            h.stop().await;
                        });
                    }
                }
                Ok(MediaEvent::PeerJoined { uid }) => {
                    let current = voice.active_channel.read().clone();
                    if let Some(mut ch) = current {
                        if !ch.members.iter().any(|m| m.uid == uid) {
                            let initial = uid
                                .chars()
                                .next()
                                .map(|c| c.to_string())
                                .unwrap_or_default();
                            ch.members.push(crate::state::VoiceMember {
                                name: uid.clone(),
                                initial,
                                uid,
                                speaking: false,
                            });
                            voice.active_channel.set(Some(ch));
                        }
                    }
                }
                Ok(MediaEvent::PeerLeft { uid }) => {
                    let current = voice.active_channel.read().clone();
                    if let Some(mut ch) = current {
                        ch.members.retain(|m| m.uid != uid);
                        voice.active_channel.set(Some(ch));
                    }
                }
                Ok(MediaEvent::ScreenStats(s)) => {
                    let mut v = voice;
                    v.screen_stats.set(Some(crate::state::ScreenStatsView {
                        width: s.width,
                        height: s.height,
                        fps_actual: s.fps_actual,
                        fps_target: s.fps_target,
                        bitrate_kbps: s.bitrate_kbps,
                        bitrate_actual_kbps: s.bitrate_actual_kbps,
                    }));
                }
                Ok(ev) => tracing::info!("[voice] {:?}", ev),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("[voice] event lagged: {}", n);
                }
                Err(_) => break,
            }
        }
    });

    voice.handle.set(Some(handle.clone()));

    // 进入语音后自动推麦克风（若用户未主动关麦）
    if *controls.mic_on.read() {
        let result = handle
            .publish(kaede_media_session::PublishSource::Mic {
                sample_rate: 48_000,
                channels: 1,
                denoise: true,
            })
            .await;
        match result {
            Ok(()) => tracing::info!("[voice] mic published"),
            Err(e) => {
                tracing::warn!("[voice] auto mic publish failed: {}", e);
                controls.mic_on.set(false);
            }
        }
    }

    Ok(())
}

/// 频道成员列表弹窗。
#[component]
pub fn ChannelMembersModal() -> Element {
    let mut ui = use_context::<UiState>();
    let app = use_context::<AppState>();
    let mut rows = use_signal(|| None::<Result<Vec<kaede_client::http::ChannelMember>, String>>);

    use_effect(move || {
        let ch = ui.channel_members_open.read().clone();
        let Some(ch) = ch else {
            rows.set(None);
            return;
        };
        let room_opt = app.room.read().clone();
        rows.set(None);
        spawn(async move {
            let Some(room) = room_opt else {
                rows.set(Some(Err("未连接服务器".into())));
                return;
            };
            match room.list_channel_members(&ch).await {
                Ok(list) => rows.set(Some(Ok(list))),
                Err(e) => rows.set(Some(Err(e.to_string()))),
            }
        });
    });

    let Some(ch_id) = ui.channel_members_open.read().clone() else {
        return rsx! {};
    };

    let (my_uid, can_remove) = {
        let me = ui.common_account.read().login_id.clone();
        let sid = app.current_server.read().clone();
        let op5 = {
            let members = app.members.read();
            sid.as_ref()
                .and_then(|s| members.get(s))
                .and_then(|list| list.iter().find(|m| m.login_id == me))
                .map(|m| m.op == "op5")
                .unwrap_or(false)
        };
        let is_private = {
            let channels = app.channels.read();
            sid.as_ref()
                .and_then(|s| channels.get(s))
                .and_then(|list| list.iter().find(|c| c.id == ch_id))
                .map(|c| c.is_private)
                .unwrap_or(false)
        };
        (me, op5 && is_private)
    };

    let body = match rows() {
        None => rsx! {
            div {
                style: "padding: 24px 0; text-align: center; color: var(--fg-faint); font-size: 13px;",
                "加载中…"
            }
        },
        Some(Err(e)) => rsx! {
            div {
                style: "padding: 24px 0; text-align: center; color: var(--negative); font-size: 13px;",
                "{e}"
            }
        },
        Some(Ok(list)) => rsx! {
            div {
                style: "display: flex; flex-direction: column; gap: 4px; min-height: 0; overflow-y: auto;",
                if list.is_empty() {
                    div {
                        style: "padding: 24px 0; text-align: center; color: var(--fg-faint); font-size: 13px;",
                        "暂无成员"
                    }
                }
                for m in list {
                    div {
                        key: "{m.login_id}",
                        style: "display: flex; align-items: center; gap: 10px; padding: 8px 10px; border-radius: 6px; background: rgba(0,0,0,0.03);",
                        div {
                            style: "width: 28px; height: 28px; border-radius: 50%; background: var(--active-bg); color: var(--fg); display: flex; align-items: center; justify-content: center; font-size: 12px; flex-shrink: 0;",
                            "{m.nickname.chars().next().map(|c| c.to_string()).unwrap_or_default()}"
                        }
                        div {
                            style: "flex: 1; min-width: 0;",
                            div {
                                style: "font-size: 14px; color: var(--fg); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;",
                                "{m.nickname}"
                            }
                            div {
                                style: "font-size: 11px; color: var(--fg-faint); margin-top: 2px;",
                                "@{m.login_id} · 加入于 {crate::observer::day_label(m.joined_at)} {crate::observer::format_hhmm(m.joined_at)}"
                            }
                        }
                        if can_remove && m.login_id != my_uid {
                            button {
                                class: "btn-secondary",
                                style: "padding: 4px 10px; font-size: 12px; color: var(--negative);",
                                onclick: {
                                    let target = m.login_id.clone();
                                    let ch = ch_id.clone();
                                    move |_| {
                                        let target = target.clone();
                                        let ch = ch.clone();
                                        let mut ui2 = ui;
                                        spawn(async move {
                                            let Some(room) = app.room.read().clone() else { return };
                                            match room.remove_channel_member(&ch, &target).await {
                                                Ok(()) => {
                                                    let mut next = rows.read().clone();
                                                    if let Some(Ok(list)) = &mut next {
                                                        list.retain(|x| x.login_id != target);
                                                    }
                                                    rows.set(next);
                                                    ui2.toast.set(Some(("已移除成员".into(), false)));
                                                }
                                                Err(e) => {
                                                    ui2.toast.set(Some((format!("移除失败: {}", e), true)));
                                                }
                                            }
                                        });
                                    }
                                },
                                "移除"
                            }
                        }
                    }
                }
            }
        },
    };

    rsx! {
        div {
            class: "server-modal-backdrop",
            onclick: move |_| ui.channel_members_open.set(None),
            div {
                class: "server-modal",
                style: "width: 420px; height: auto; max-height: 70vh; padding: 24px; flex-direction: column;",
                onclick: move |e| e.stop_propagation(),
                div { class: "server-modal-title", "频道成员" }
                div {
                    style: "font-size: 11px; color: var(--fg-muted); font-family: ui-monospace, monospace; margin-bottom: 12px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;",
                    "{ch_id}"
                }
                {body}
                div { class: "form-actions", style: "margin-top: 16px;",
                    div { style: "flex: 1;" }
                    button {
                        class: "btn-secondary",
                        onclick: move |_| ui.channel_members_open.set(None),
                        "关闭"
                    }
                }
            }
        }
    }
}
