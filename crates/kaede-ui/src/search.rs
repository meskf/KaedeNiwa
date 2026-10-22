//! 本地消息搜索弹窗。
//!
//! 只搜索客户端已解密/已加载的内容：`history_cache`、本地发送缓存、
//! 当前会话已加载的消息。服务端只有密文，无法参与搜索。

use dioxus::prelude::*;

use crate::state::{AppState, MainTab, UiState};

#[derive(Clone, PartialEq)]
struct SearchHit {
    is_dm: bool,
    /// 频道 id / conv_id。
    target: String,
    /// 展示用的频道名或 conv 标识。
    label: String,
    author: String,
    ts: i64,
    text: String,
}

async fn do_search(app: AppState, query: String) -> Vec<SearchHit> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let Some(sid) = app.current_server.read().clone() else {
        return Vec::new();
    };
    let prefix = format!("{}::", sid);
    let channels = app.channels.read().get(&sid).cloned().unwrap_or_default();
    let label_of = |id: &str| {
        channels
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| id.to_string())
    };
    let dm_convs = app
        .dm_conversations
        .read()
        .get(&sid)
        .cloned()
        .unwrap_or_default();
    let dm_label_of = |id: &str| {
        dm_convs
            .iter()
            .find(|c| c.conv_id == id)
            .map(|c| c.peer_nickname.clone())
            .unwrap_or_else(|| id.to_string())
    };

    let mut hits = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // 当前会话已加载的频道消息。
    {
        let messages = app.messages.read();
        for (key, list) in messages.iter() {
            let Some(ch) = key.strip_prefix(&prefix) else {
                continue;
            };
            for m in list {
                if m.text.to_lowercase().contains(&q)
                    && seen.insert((format!("c:{ch}"), m.ts, m.author.clone()))
                {
                    hits.push(SearchHit {
                        is_dm: false,
                        target: ch.to_string(),
                        label: label_of(ch),
                        author: m.author.clone(),
                        ts: m.ts,
                        text: m.text.clone(),
                    });
                }
            }
        }
    }

    // 已加载的 DM 消息。
    {
        let dms = app.dm_messages.read();
        for (conv, list) in dms.iter() {
            for m in list {
                if m.text.to_lowercase().contains(&q)
                    && seen.insert((format!("d:{conv}"), m.ts, m.author.clone()))
                {
                    hits.push(SearchHit {
                        is_dm: true,
                        target: conv.clone(),
                        label: dm_label_of(conv),
                        author: m.author.clone(),
                        ts: m.ts,
                        text: m.text.clone(),
                    });
                }
            }
        }
    }

    // 持久化的频道历史缓存（可能比已加载的多）。
    if let Some(room) = app.room.read().clone() {
        let ids: Vec<String> = channels
            .iter()
            .filter(|c| c.kind != "voice")
            .map(|c| c.id.clone())
            .collect();
        let dm_ids: Vec<String> = dm_convs.iter().map(|c| c.conv_id.clone()).collect();
        for h in room.search_local(&query, &ids, &dm_ids, 200).await {
            let seen_key = if h.is_dm {
                format!("d:{}", h.channel_id)
            } else {
                format!("c:{}", h.channel_id)
            };
            if seen.insert((seen_key, h.ts, h.sender.clone())) {
                hits.push(SearchHit {
                    is_dm: h.is_dm,
                    label: if h.is_dm {
                        dm_label_of(&h.channel_id)
                    } else {
                        label_of(&h.channel_id)
                    },
                    target: h.channel_id,
                    author: h.sender,
                    ts: h.ts,
                    text: h.text,
                });
            }
        }
    }

    hits.sort_by_key(|h| std::cmp::Reverse(h.ts));
    hits.truncate(200);
    hits
}

#[component]
pub fn SearchModal() -> Element {
    let mut ui = use_context::<UiState>();
    let app = use_context::<AppState>();
    let mut query = use_signal(String::new);
    let hits = use_signal(Vec::<SearchHit>::new);
    let busy = use_signal(|| false);
    let searched = use_signal(|| false);

    if !*ui.search_open.read() {
        return rsx! {};
    }

    let run_search = move |_| {
        let q = query.read().clone();
        if q.trim().is_empty() {
            return;
        }
        let mut hits = hits;
        let mut busy = busy;
        let mut searched = searched;
        spawn(async move {
            busy.set(true);
            searched.set(true);
            hits.set(do_search(app, q).await);
            busy.set(false);
        });
    };

    rsx! {
        div {
            class: "server-modal-backdrop",
            onclick: move |_| ui.search_open.set(false),
            div {
                class: "server-modal",
                style: "width: 560px; height: auto; max-height: 78vh; padding: 24px; flex-direction: column;",
                onclick: move |e| e.stop_propagation(),
                div { class: "server-modal-title", "搜索消息" }
                div {
                    style: "display: flex; gap: 8px; margin-bottom: 8px;",
                    input {
                        r#type: "text",
                        value: "{query}",
                        placeholder: "输入关键词…",
                        autofocus: true,
                        oninput: move |e| query.set(e.value()),
                        onkeydown: move |e| {
                            if e.key() == Key::Enter {
                                run_search(());
                            }
                        },
                        style: "flex: 1; padding: 10px 14px; border-radius: 8px; border: 1px solid rgba(128,128,128,0.35); background: rgba(0,0,0,0.05); color: var(--fg); font-size: 14px; outline: none;",
                    }
                    button {
                        class: "btn-primary",
                        onclick: move |_| run_search(()),
                        "搜索"
                    }
                }
                div {
                    style: "font-size: 11px; color: var(--fg-faint); margin-bottom: 10px;",
                    "仅搜索本地已解密历史（未缓存/未解密的消息不会出现在结果中）。"
                }
                div {
                    style: "flex: 1; min-height: 0; overflow-y: auto; display: flex; flex-direction: column; gap: 4px;",
                    if *busy.read() {
                        div {
                            style: "padding: 24px 0; text-align: center; color: var(--fg-faint); font-size: 13px;",
                            "搜索中…"
                        }
                    } else if *searched.read() && hits.read().is_empty() {
                        div {
                            style: "padding: 24px 0; text-align: center; color: var(--fg-faint); font-size: 13px;",
                            "没有匹配的本地消息"
                        }
                    }
                    for h in hits.read().iter() {
                        {
                            let hit = h.clone();
                            let author = h.author.clone();
                            let label = h.label.clone();
                            let snippet: String = h.text.chars().take(120).collect();
                            let time = format!(
                                "{} {}",
                                crate::observer::day_label(h.ts),
                                crate::observer::format_hhmm(h.ts)
                            );
                            rsx! {
                                button {
                                    key: "{h.target}-{h.ts}-{h.author}",
                                    style: "width: 100%; text-align: left; padding: 8px 10px; border-radius: 6px; background: rgba(0,0,0,0.03); border: none; cursor: pointer;",
                                    onclick: move |_| {
                                        if hit.is_dm {
                                            ui.main_tab.set(MainTab::Dm);
                                            ui.selected_dm_conv.set(Some(hit.target.clone()));
                                            let conv = hit.target.clone();
                                            spawn(async move {
                                                crate::dm_view::load_messages(app, conv).await;
                                            });
                                        } else {
                                            ui.main_tab.set(MainTab::Server);
                                            ui.selected_channel.set(Some(hit.target.clone()));
                                        }
                                        ui.search_open.set(false);
                                    },
                                    div {
                                        style: "display: flex; gap: 8px; align-items: baseline;",
                                        span {
                                            style: "font-size: 12px; color: var(--fg-muted); font-weight: 600;",
                                            "#{label}"
                                        }
                                        span {
                                            style: "font-size: 11px; color: var(--fg-faint);",
                                            "{author} · {time}"
                                        }
                                    }
                                    div {
                                        style: "font-size: 13px; color: var(--fg); margin-top: 2px; white-space: pre-wrap; word-wrap: break-word;",
                                        "{snippet}"
                                    }
                                }
                            }
                        }
                    }
                }
                div { class: "form-actions", style: "margin-top: 16px;",
                    div { style: "flex: 1;" }
                    button {
                        class: "btn-secondary",
                        onclick: move |_| ui.search_open.set(false),
                        "关闭"
                    }
                }
            }
        }
    }
}
