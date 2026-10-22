//! KaedeNiwa 管理后台（Dioxus Desktop）。
//!
//! 通过 `X-Admin-Token` 调用独立的管理 HTTP listener。
//! 支持远程服务端地址；token 只保存在内存里，不落盘。

use std::path::PathBuf;

use dioxus::prelude::*;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

const DEFAULT_BASE: &str = "http://127.0.0.1:3001";

fn main() {
    tracing_subscriber::fmt::init();
    dioxus::launch(App);
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Dashboard,
    Users,
    Channels,
    Logs,
    Service,
}

#[derive(Deserialize, Clone)]
struct Dashboard {
    version: String,
    uptime_secs: u64,
    users_online: usize,
    channels: i64,
    messages: i64,
    accounts: i64,
    banned: i64,
}

#[derive(Deserialize, Clone)]
struct UserRow {
    login_id: String,
    nickname: String,
    banned: bool,
    #[serde(default)]
    banned_reason: Option<String>,
}

#[derive(Deserialize, Clone)]
struct ChannelRow {
    id: String,
    name: String,
    is_private: bool,
    members: i64,
}

#[derive(Deserialize, Clone)]
struct ServiceStatus {
    platform: String,
    unit_exists: bool,
    current_mode: String,
    hint: String,
}

#[derive(Deserialize, Clone)]
struct LogEntry {
    ts: i64,
    level: String,
    target: String,
    message: String,
}

#[derive(Deserialize, Clone)]
struct ModeResp {
    mode: String,
    command: String,
    note: String,
}

#[derive(Serialize)]
struct BanReq {
    reason: String,
}

// ---------------------------------------------------------------------------
// base URL 记忆（只记地址，不记 token）
// ---------------------------------------------------------------------------

fn config_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|base| base.join("kaede-admin"))
}

fn load_base() -> String {
    config_dir()
        .and_then(|dir| std::fs::read_to_string(dir.join("base_url")).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_BASE.to_string())
}

fn save_base(base: &str) {
    let Some(dir) = config_dir() else {
        return;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let _ = std::fs::write(dir.join("base_url"), base);
}

// ---------------------------------------------------------------------------
// HTTP helpers
// ---------------------------------------------------------------------------

fn join(base: &str, path: &str) -> String {
    format!("{}{}", base.trim_end_matches('/'), path)
}

async fn get_json<T: DeserializeOwned>(base: &str, token: &str, path: &str) -> Result<T, String> {
    let resp = reqwest::Client::new()
        .get(join(base, path))
        .header("X-Admin-Token", token)
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    resp.json::<T>()
        .await
        .map_err(|e| format!("响应解析失败: {e}"))
}

async fn post_action(base: &str, token: &str, path: &str) -> Result<(), String> {
    let resp = reqwest::Client::new()
        .post(join(base, path))
        .header("X-Admin-Token", token)
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    Ok(())
}

async fn post_ban(base: &str, token: &str, login_id: &str, reason: &str) -> Result<(), String> {
    let resp = reqwest::Client::new()
        .post(join(
            base,
            &format!("/api/users/{}/ban", url_encode(login_id)),
        ))
        .header("X-Admin-Token", token)
        .json(&BanReq {
            reason: reason.to_string(),
        })
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    Ok(())
}

async fn post_mode(base: &str, token: &str, mode: &str) -> Result<ModeResp, String> {
    let resp = reqwest::Client::new()
        .post(join(base, "/api/service/mode"))
        .header("X-Admin-Token", token)
        .json(&serde_json::json!({ "mode": mode }))
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    resp.json::<ModeResp>()
        .await
        .map_err(|e| format!("响应解析失败: {e}"))
}

async fn delete_channel(base: &str, token: &str, id: &str) -> Result<(), String> {
    let resp = reqwest::Client::new()
        .delete(join(base, &format!("/api/channels/{}", url_encode(id))))
        .header("X-Admin-Token", token)
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    Ok(())
}

/// 只用于路径段的百分号编码，避免 login_id / channel_id 中的特殊字符破坏 URL。
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn fmt_uptime(secs: u64) -> String {
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let mins = (secs % 3600) / 60;
    if days > 0 {
        format!("{days} 天 {hours} 小时")
    } else if hours > 0 {
        format!("{hours} 小时 {mins} 分")
    } else {
        format!("{mins} 分")
    }
}

/// UTC 时间，仅用于日志列表。
fn fmt_hms(ts_ms: i64) -> String {
    let secs = ts_ms / 1000;
    format!(
        "{:02}:{:02}:{:02} UTC",
        (secs / 3600).rem_euclid(24),
        (secs / 60).rem_euclid(60),
        secs.rem_euclid(60)
    )
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

#[component]
fn App() -> Element {
    let mut base_url = use_signal(load_base);
    let mut token = use_signal(String::new);
    let mut authed = use_signal(|| false);
    let mut tab = use_signal(|| Tab::Dashboard);
    let mut error_msg = use_signal(String::new);
    let mut busy = use_signal(|| false);

    if !*authed.read() {
        let is_remote_plain_http = {
            let b = base_url.read();
            let b = b.trim();
            !b.starts_with("https://")
                && !b.starts_with("http://127.0.0.1")
                && !b.starts_with("http://localhost")
        };
        return rsx! {
            style { dangerous_inner_html: CSS }
            div { class: "login-wrap",
                div { class: "login-card",
                    h2 { "KaedeNiwa 管理后台" }
                    p { class: "muted", "输入服务端管理地址与启动时打印的 Admin Token。" }
                    label { class: "field-label", "管理地址" }
                    input {
                        r#type: "text",
                        class: "input",
                        value: "{base_url}",
                        placeholder: DEFAULT_BASE,
                        oninput: move |e| base_url.set(e.value()),
                    }
                    if is_remote_plain_http {
                        p { class: "warn", "远程地址使用明文 HTTP；建议通过 HTTPS 反向代理或 SSH 隧道访问。" }
                    }
                    label { class: "field-label", "Admin Token" }
                    input {
                        r#type: "password",
                        class: "input",
                        value: "{token}",
                        placeholder: "X-Admin-Token",
                        oninput: move |e| token.set(e.value()),
                    }
                    if !error_msg.read().is_empty() {
                        p { class: "error", "{error_msg}" }
                    }
                    button {
                        class: "btn primary full",
                        disabled: *busy.read(),
                        onclick: move |_| {
                            if *busy.read() {
                                return;
                            }
                            let base = base_url.read().trim().trim_end_matches('/').to_string();
                            let t = token.read().clone();
                            if !(base.starts_with("http://") || base.starts_with("https://")) {
                                error_msg.set("管理地址必须以 http:// 或 https:// 开头".into());
                                return;
                            }
                            if t.is_empty() {
                                error_msg.set("Admin Token 不能为空".into());
                                return;
                            }
                            base_url.set(base.clone());
                            save_base(&base);
                            busy.set(true);
                            error_msg.set(String::new());
                            let mut a = authed;
                            let mut err = error_msg;
                            let mut b = busy;
                            spawn(async move {
                                match get_json::<Dashboard>(&base, &t, "/api/dashboard").await {
                                    Ok(_) => a.set(true),
                                    Err(e) => err.set(format!("连接失败: {e}")),
                                }
                                b.set(false);
                            });
                        },
                        if *busy.read() { "连接中..." } else { "连接" }
                    }
                }
            }
        };
    }

    let base = base_url().trim_end_matches('/').to_string();
    let token_value = token();
    let current_tab = *tab.read();

    rsx! {
        style { dangerous_inner_html: CSS }
        div { class: "shell",
            aside { class: "sidebar",
                div { class: "brand", "KaedeAdmin" }
                div { class: "brand-sub", "{base}" }
                NavItem { label: "仪表盘", active: current_tab == Tab::Dashboard, on_click: move |_| tab.set(Tab::Dashboard) }
                NavItem { label: "用户管理", active: current_tab == Tab::Users, on_click: move |_| tab.set(Tab::Users) }
                NavItem { label: "频道管理", active: current_tab == Tab::Channels, on_click: move |_| tab.set(Tab::Channels) }
                NavItem { label: "日志", active: current_tab == Tab::Logs, on_click: move |_| tab.set(Tab::Logs) }
                NavItem { label: "系统服务", active: current_tab == Tab::Service, on_click: move |_| tab.set(Tab::Service) }
                div { class: "sidebar-foot",
                    button {
                        class: "btn ghost",
                        onclick: move |_| {
                            authed.set(false);
                            token.set(String::new());
                        },
                        "断开"
                    }
                }
            }
            main { class: "content",
                match current_tab {
                    Tab::Dashboard => rsx! { DashboardView { base: base.clone(), token: token_value.clone() } },
                    Tab::Users => rsx! { UsersView { base: base.clone(), token: token_value.clone() } },
                    Tab::Channels => rsx! { ChannelsView { base: base.clone(), token: token_value.clone() } },
                    Tab::Logs => rsx! { LogsView { base: base.clone(), token: token_value.clone() } },
                    Tab::Service => rsx! { ServiceView { base: base.clone(), token: token_value.clone() } },
                }
            }
        }
    }
}

#[component]
fn NavItem(label: String, active: bool, on_click: EventHandler<()>) -> Element {
    rsx! {
        div {
            class: if active { "nav-item active" } else { "nav-item" },
            onclick: move |_| on_click.call(()),
            "{label}"
        }
    }
}

#[component]
fn DashboardView(base: String, token: String) -> Element {
    let mut refresh = use_signal(|| 0u32);
    let base_res = base.clone();
    let token_res = token.clone();
    let data = use_resource(move || {
        let _ = *refresh.read();
        let base = base_res.clone();
        let token = token_res.clone();
        async move { get_json::<Dashboard>(&base, &token, "/api/dashboard").await }
    });

    rsx! {
        div { class: "page-head",
            h2 { "仪表盘" }
            button { class: "btn", onclick: move |_| refresh += 1, "刷新" }
        }
        match &*data.read_unchecked() {
            Some(Ok(d)) => rsx! {
                div { class: "stats",
                    StatCard { label: "在线用户", value: "{d.users_online}" }
                    StatCard { label: "账号总数", value: "{d.accounts}" }
                    StatCard { label: "封禁中", value: "{d.banned}" }
                    StatCard { label: "频道", value: "{d.channels}" }
                    StatCard { label: "消息", value: "{d.messages}" }
                    StatCard { label: "运行时长", value: fmt_uptime(d.uptime_secs) }
                }
                p { class: "muted", "服务端版本 {d.version}" }
            },
            Some(Err(e)) => rsx! { p { class: "error", "加载失败: {e}" } },
            None => rsx! { p { class: "muted", "加载中..." } },
        }
    }
}

#[component]
fn StatCard(label: String, value: String) -> Element {
    rsx! {
        div { class: "stat",
            div { class: "stat-value", "{value}" }
            div { class: "stat-label", "{label}" }
        }
    }
}

// ---------------------------------------------------------------------------
// Users
// ---------------------------------------------------------------------------

#[component]
fn UsersView(base: String, token: String) -> Element {
    let mut refresh = use_signal(|| 0u32);
    let mut query = use_signal(String::new);
    let mut applied = use_signal(String::new);
    let mut reason = use_signal(String::new);
    let error = use_signal(String::new);
    let busy = use_signal(|| false);
    let base_res = base.clone();
    let token_res = token.clone();

    let data = use_resource(move || {
        let _ = *refresh.read();
        let q = applied.read().clone();
        let base = base_res.clone();
        let token = token_res.clone();
        async move {
            let path = if q.is_empty() {
                "/api/users?limit=200".to_string()
            } else {
                format!("/api/users?limit=200&q={}", url_encode(&q))
            };
            get_json::<Vec<UserRow>>(&base, &token, &path).await
        }
    });

    let apply = move |_| applied.set(query.read().trim().to_string());

    rsx! {
        div { class: "page-head",
            h2 { "用户管理" }
            button { class: "btn", onclick: move |_| refresh += 1, "刷新" }
        }
        div { class: "toolbar",
            input {
                r#type: "text",
                class: "input compact",
                placeholder: "搜索 login_id / 昵称",
                value: "{query}",
                oninput: move |e| query.set(e.value()),
            }
            button { class: "btn", onclick: apply, "搜索" }
            input {
                r#type: "text",
                class: "input compact",
                placeholder: "封禁原因（可选）",
                value: "{reason}",
                oninput: move |e| reason.set(e.value()),
            }
        }
        if !error.read().is_empty() {
            p { class: "error", "{error}" }
        }
        match &*data.read_unchecked() {
            Some(Ok(users)) => rsx! {
                table { class: "table",
                    thead { tr {
                        th { "Login ID" }
                        th { "昵称" }
                        th { "状态" }
                        th { "原因" }
                        th { "操作" }
                    }}
                    tbody { for u in users.clone() {
                        tr { key: "{u.login_id}",
                            td { class: "mono", "{u.login_id}" }
                            td { "{u.nickname}" }
                            td {
                                if u.banned {
                                    span { class: "badge danger", "已封禁" }
                                } else {
                                    span { class: "badge ok", "正常" }
                                }
                            }
                            td { class: "muted", "{u.banned_reason.clone().unwrap_or_default()}" }
                            td {
                                {
                                    let id = u.login_id.clone();
                                    let is_banned = u.banned;
                                    let base2 = base.clone();
                                    let token2 = token.clone();
                                    let reason2 = reason.read().clone();
                                    let err = error;
                                    let mut b = busy;
                                    let r = refresh;
                                    rsx! {
                                        button {
                                            class: if is_banned { "btn small" } else { "btn small danger" },
                                            disabled: *busy.read(),
                                            onclick: move |_| {
                                                if *b.read() {
                                                    return;
                                                }
                                                let base = base2.clone();
                                                let token = token2.clone();
                                                let id = id.clone();
                                                let reason = reason2.clone();
                                                let mut err = err;
                                                let mut b2 = b;
                                                let mut r = r;
                                                b.set(true);
                                                spawn(async move {
                                                    let result = if is_banned {
                                                        post_action(&base, &token, &format!("/api/users/{}/unban", url_encode(&id))).await
                                                    } else {
                                                        post_ban(&base, &token, &id, &reason).await
                                                    };
                                                    match result {
                                                        Ok(()) => {
                                                            r += 1;
                                                        }
                                                        Err(e) => err.set(e),
                                                    }
                                                    b2.set(false);
                                                });
                                            },
                                            if is_banned { "解封" } else { "封禁" }
                                        }
                                    }
                                }
                            }
                        }
                    }}
                }
            },
            Some(Err(e)) => rsx! { p { class: "error", "加载失败: {e}" } },
            None => rsx! { p { class: "muted", "加载中..." } },
        }
    }
}

// ---------------------------------------------------------------------------
// Channels
// ---------------------------------------------------------------------------

#[component]
fn ChannelsView(base: String, token: String) -> Element {
    let mut refresh = use_signal(|| 0u32);
    let confirm = use_signal(|| Option::<String>::None);
    let error = use_signal(String::new);
    let busy = use_signal(|| false);
    let base_res = base.clone();
    let token_res = token.clone();

    let data = use_resource(move || {
        let _ = *refresh.read();
        let base = base_res.clone();
        let token = token_res.clone();
        async move { get_json::<Vec<ChannelRow>>(&base, &token, "/api/channels").await }
    });

    rsx! {
        div { class: "page-head",
            h2 { "频道管理" }
            button { class: "btn", onclick: move |_| refresh += 1, "刷新" }
        }
        if !error.read().is_empty() {
            p { class: "error", "{error}" }
        }
        match &*data.read_unchecked() {
            Some(Ok(channels)) => rsx! {
                table { class: "table",
                    thead { tr {
                        th { "ID" }
                        th { "名称" }
                        th { "类型" }
                        th { "成员数" }
                        th { "操作" }
                    }}
                    tbody { for c in channels.clone() {
                        tr { key: "{c.id}",
                            td { class: "mono", "{c.id}" }
                            td { "{c.name}" }
                            td {
                                if c.is_private {
                                    span { class: "badge", "私密" }
                                } else {
                                    span { class: "badge muted", "公开" }
                                }
                            }
                            td { "{c.members}" }
                            td {
                                if c.id == "general" || c.id == "voice" {
                                    span { class: "muted", "内置频道" }
                                } else {
                                    {
                                        let id = c.id.clone();
                                        let is_confirm = confirm.read().as_deref() == Some(c.id.as_str());
                                        let base2 = base.clone();
                                        let token2 = token.clone();
                                        let err = error;
                                        let mut b = busy;
                                        let r = refresh;
                                        let mut cf = confirm;
                                        rsx! {
                                            button {
                                                class: if is_confirm { "btn small danger" } else { "btn small" },
                                                disabled: *busy.read(),
                                                onclick: move |_| {
                                                    if *b.read() {
                                                        return;
                                                    }
                                                    if !is_confirm {
                                                        cf.set(Some(id.clone()));
                                                        return;
                                                    }
                                                    let base = base2.clone();
                                                    let token = token2.clone();
                                                    let id = id.clone();
                                                    let mut err = err;
                                                    let mut b2 = b;
                                                    let mut r = r;
                                                    let mut cf = cf;
                                                    b.set(true);
                                                    spawn(async move {
                                                        match delete_channel(&base, &token, &id).await {
                                                            Ok(()) => {
                                                                cf.set(None);
                                                                r += 1;
                                                            }
                                                            Err(e) => err.set(e),
                                                        }
                                                        b2.set(false);
                                                    });
                                                },
                                                if is_confirm { "确认删除" } else { "删除" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }}
                }
            },
            Some(Err(e)) => rsx! { p { class: "error", "加载失败: {e}" } },
            None => rsx! { p { class: "muted", "加载中..." } },
        }
    }
}

// ---------------------------------------------------------------------------
// Logs
// ---------------------------------------------------------------------------

#[component]
fn LogsView(base: String, token: String) -> Element {
    let mut refresh = use_signal(|| 0u32);
    let base_res = base.clone();
    let token_res = token.clone();
    let data = use_resource(move || {
        let _ = *refresh.read();
        let base = base_res.clone();
        let token = token_res.clone();
        async move { get_json::<Vec<LogEntry>>(&base, &token, "/api/logs?limit=300").await }
    });

    rsx! {
        div { class: "page-head",
            h2 { "日志" }
            button { class: "btn", onclick: move |_| refresh += 1, "刷新" }
        }
        p { class: "muted", "内存环形缓冲最近 300 条；不含 Admin Token 生成日志。" }
        match &*data.read_unchecked() {
            Some(Ok(logs)) => rsx! {
                table { class: "table logs",
                    thead { tr {
                        th { "时间" }
                        th { "级别" }
                        th { "Target" }
                        th { "内容" }
                    }}
                    tbody { for l in logs.clone() {
                        tr { key: "{l.ts}-{l.target}-{l.message}",
                            td { class: "mono muted", "{fmt_hms(l.ts)}" }
                            td {
                                span { class: level_class(&l.level), "{l.level}" }
                            }
                            td { class: "mono muted", "{l.target}" }
                            td { class: "mono log-msg", "{l.message}" }
                        }
                    }}
                }
            },
            Some(Err(e)) => rsx! { p { class: "error", "加载失败: {e}" } },
            None => rsx! { p { class: "muted", "加载中..." } },
        }
    }
}

fn level_class(level: &str) -> &'static str {
    match level {
        "ERROR" => "badge danger",
        "WARN" => "badge warn",
        "INFO" => "badge ok",
        _ => "badge muted",
    }
}

const CSS: &str = r#"
* { box-sizing: border-box; }
html, body { margin: 0; height: 100%; }
body { background: #1a1a1e; color: #e6e6e8; font-family: system-ui, -apple-system, "Segoe UI", sans-serif; }
.login-wrap { display: flex; align-items: center; justify-content: center; height: 100vh; background: #1a1a1e; }
.login-card { width: 420px; padding: 32px; background: #232328; border: 1px solid #35353d; border-radius: 12px; }
.login-card h2 { margin: 0 0 8px; font-size: 20px; }
.field-label { display: block; margin: 16px 0 6px; color: #a1a1aa; font-size: 12px; }
.input { width: 100%; padding: 10px 12px; border: 1px solid #3a3a44; border-radius: 8px; background: #1a1a1e; color: #e6e6e8; font-size: 14px; }
.input:focus { outline: none; border-color: #0fdc78; }
.input.compact { width: 260px; }
.btn { padding: 8px 14px; border: 1px solid #3a3a44; border-radius: 8px; background: #2a2a31; color: #e6e6e8; font-size: 13px; cursor: pointer; }
.btn:hover { background: #34343d; }
.btn:disabled { opacity: .5; cursor: default; }
.btn.primary { background: #0fdc78; border-color: #0fdc78; color: #004d26; font-weight: 600; }
.btn.ghost { background: transparent; }
.btn.full { width: 100%; margin-top: 20px; padding: 10px; }
.btn.small { padding: 4px 10px; font-size: 12px; }
.btn.danger { border-color: #e8463a; color: #ff8f86; }
.muted { color: #9a9aa4; font-size: 13px; }
.warn { color: #efaa17; font-size: 12px; }
.error { color: #ff8f86; font-size: 13px; }
.shell { display: flex; height: 100vh; background: #1a1a1e; }
.sidebar { width: 216px; padding: 20px 0; background: #232328; border-right: 1px solid #35353d; display: flex; flex-direction: column; }
.brand { padding: 0 20px; font-size: 18px; font-weight: 700; color: #32f08c; }
.brand-sub { padding: 4px 20px 16px; font-size: 11px; color: #6f6f78; word-break: break-all; }
.nav-item { padding: 11px 20px; color: #9a9aa4; font-size: 14px; cursor: pointer; }
.nav-item:hover { background: #2a2a31; color: #e6e6e8; }
.nav-item.active { background: #34343d; color: #fff; border-left: 2px solid #0fdc78; }
.sidebar-foot { margin-top: auto; padding: 0 20px; }
.content { flex: 1; padding: 32px 40px; overflow-y: auto; }
.page-head { display: flex; align-items: center; justify-content: space-between; margin-bottom: 16px; }
.page-head h2 { margin: 0; font-size: 22px; }
.stats { display: grid; grid-template-columns: repeat(3, minmax(160px, 1fr)); gap: 16px; margin: 16px 0; }
.stat { padding: 20px; background: #232328; border: 1px solid #35353d; border-radius: 10px; text-align: center; }
.stat-value { font-size: 24px; font-weight: 700; margin-bottom: 6px; }
.stat-label { color: #9a9aa4; font-size: 13px; }
.toolbar { display: flex; gap: 10px; align-items: center; margin-bottom: 14px; flex-wrap: wrap; }
.card { padding: 20px; background: #232328; border: 1px solid #35353d; border-radius: 10px; margin: 12px 0; }
.table { width: 100%; margin-top: 8px; border-collapse: collapse; }
.table th { text-align: left; padding: 10px; border-bottom: 1px solid #3a3a44; color: #9a9aa4; font-size: 12px; font-weight: 500; }
.table td { padding: 10px; border-bottom: 1px solid #2a2a31; font-size: 13px; vertical-align: middle; }
.table.logs td { vertical-align: top; }
.mono { font-family: ui-monospace, "SF Mono", Menlo, Consolas, monospace; }
.log-msg { white-space: pre-wrap; word-break: break-word; max-width: 640px; }
.badge { display: inline-block; padding: 2px 8px; border-radius: 999px; font-size: 11px; background: #34343d; color: #c9c9d0; }
.badge.ok { background: rgba(15, 220, 120, .16); color: #5cf5a8; }
.badge.warn { background: rgba(239, 170, 23, .16); color: #efaa17; }
.badge.danger { background: rgba(232, 70, 58, .16); color: #ff8f86; }
.badge.muted { color: #9a9aa4; }
.command { padding: 12px; background: #1a1a1e; border: 1px solid #35353d; border-radius: 8px; color: #c9c9d0; font-family: ui-monospace, "SF Mono", Menlo, Consolas, monospace; font-size: 12px; white-space: pre-wrap; }
"#;

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

#[component]
fn ServiceView(base: String, token: String) -> Element {
    let mut refresh = use_signal(|| 0u32);
    let result = use_signal(|| Option::<Result<ModeResp, String>>::None);
    let busy = use_signal(|| false);
    let base_res = base.clone();
    let token_res = token.clone();

    let data = use_resource(move || {
        let _ = *refresh.read();
        let base = base_res.clone();
        let token = token_res.clone();
        async move { get_json::<ServiceStatus>(&base, &token, "/api/service").await }
    });

    let switch = move |mode: &'static str| {
        let base = base.clone();
        let token = token.clone();
        let result = result;
        let mut b = busy;
        move |_| {
            if *b.read() {
                return;
            }
            let base = base.clone();
            let token = token.clone();
            let mut result = result;
            let mut b2 = b;
            b.set(true);
            spawn(async move {
                result.set(Some(post_mode(&base, &token, mode).await));
                b2.set(false);
            });
        }
    };

    rsx! {
        div { class: "page-head",
            h2 { "系统服务" }
            button { class: "btn", onclick: move |_| refresh += 1, "刷新" }
        }
        match &*data.read_unchecked() {
            Some(Ok(s)) => rsx! {
                div { class: "card",
                    p { "平台: {s.platform}" }
                    p { "Unit 已安装: {s.unit_exists}" }
                    p { "当前模式: {s.current_mode}" }
                    p { class: "muted", "{s.hint}" }
                }
                div { class: "toolbar",
                    button {
                        class: "btn",
                        disabled: *busy.read(),
                        onclick: switch("on_demand"),
                        "切换为按需启动"
                    }
                    button {
                        class: "btn",
                        disabled: *busy.read(),
                        onclick: switch("keep_alive"),
                        "切换为常驻"
                    }
                }
            },
            Some(Err(e)) => rsx! { p { class: "error", "加载失败: {e}" } },
            None => rsx! { p { class: "muted", "加载中..." } },
        }
        match &*result.read() {
            Some(Ok(r)) => rsx! {
                div { class: "card",
                    p { "模式: {r.mode}" }
                    p { class: "muted", "{r.note}" }
                    pre { class: "command", "{r.command}" }
                }
            },
            Some(Err(e)) => rsx! { p { class: "error", "{e}" } },
            None => rsx! {},
        }
    }
}
