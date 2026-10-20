//! 所有 CSS。集中管理，避免散落各文件。

use crate::theme::{DARK, FONT_STACK, LIGHT};

pub fn global() -> String {
    let mut s = String::with_capacity(16384);
    s.push_str(&base());
    s.push_str(SHELL);
    s.push_str(SIDEBAR);
    s.push_str(CONTENT);
    s.push_str(MEMBERS);
    s.push_str(LOGIN);
    s.push_str(MODAL);
    s.push_str(TOPTAB_CSS);
    s.push_str(FIX_CSS);
    s
}

const FIX_CSS: &str = r#"
/* 语音面板：启用态 hover 保持白底 */
.vp-btn.vp-on:hover { background: var(--inner-bg); }
.vp-btn:hover:not(.vp-on) { background: var(--active-bg); }

/* 发送按钮禁用态 */
.composer-send:disabled { opacity: 0.4; cursor: not-allowed; }
.composer-send:disabled:hover { background: var(--accent-bg); }

/* 服务器选中项 hover 保持强调色 */
.srv-selected:hover { background: var(--accent-bg) !important; color: var(--accent-fg) !important; }

/* 频道行：选中/悬停状态完全由 class 驱动（行内 style 已移除） */
.channel-row {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-radius: 10px;
    background: transparent;
    color: var(--fg-muted);
    font-size: 14px;
    text-align: left;
    transition: background 120ms, color 120ms;
    appearance: none;
    -webkit-appearance: none;
    outline: none;
    border: none;
    cursor: pointer;
}
.channel-row:hover:not(.selected) { background: var(--hover-bg); color: var(--fg); }
.channel-row.selected { background: var(--active-bg); color: var(--fg); }
.channel-row.selected:hover { background: var(--active-bg); color: var(--fg); }

/* ============== 批次 A 消息操作样式 ============== */
.msg-group {
    position: relative;
}
.msg-hover-actions {
    position: absolute;
    top: -12px;
    right: 12px;
    display: flex;
    gap: 2px;
    background: var(--inner-bg);
    border-radius: 8px;
    padding: 2px;
    box-shadow: 0 2px 8px rgba(0,0,0,0.08);
    z-index: 2;
}
.msg-hover-btn {
    width: 28px;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 6px;
    color: var(--fg-muted);
    font-size: 14px;
    cursor: pointer;
}
.msg-hover-btn:hover {
    background: var(--hover-bg);
    color: var(--fg);
}
.msg-reactions {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
    margin-top: 6px;
}
.reaction-pill {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 8px;
    border-radius: 999px;
    background: var(--hover-bg);
    border: 1px solid transparent;
    cursor: pointer;
    font-size: 13px;
    transition: background 120ms, border-color 120ms;
}
.reaction-pill:hover {
    background: var(--active-bg);
}
.reaction-pill.reaction-mine {
    background: var(--active-bg);
    border-color: var(--fg-faint);
}
.reaction-emoji {
    font-size: 14px;
}
.reaction-count {
    font-size: 12px;
    color: var(--fg-muted);
    font-weight: 600;
}
.reply-preview {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    margin-bottom: 6px;
    background: var(--hover-bg);
    border-radius: 8px;
    border-left: 3px solid var(--accent-bg);
    font-size: 13px;
}
.reply-preview-icon { color: var(--fg-faint); }
.reply-preview-author { color: var(--fg); font-weight: 600; }
.reply-preview-text {
    color: var(--fg-muted);
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}
.reply-preview-cancel {
    color: var(--fg-faint);
    padding: 2px 6px;
    border-radius: 4px;
}
.reply-preview-cancel:hover {
    background: var(--active-bg);
    color: var(--fg);
}"#;
const TOPTAB_CSS: &str = r#"
.top-tab {
    height: 36px;
    padding: 0 14px;
    display: flex;
    align-items: center;
    gap: 7px;
    border-radius: 10px;
    background: transparent;
    color: var(--fg-muted);
    font-size: 14px;
    font-weight: 500;
    transition: background 120ms, color 120ms;
    flex-shrink: 0;
    max-width: 220px;
    appearance: none;
    -webkit-appearance: none;
    outline: none;
    border: none;
    cursor: pointer;
}
.top-tab.active {
    background: var(--inner-bg) !important;
    color: var(--fg) !important;
    font-weight: 600;
}
.top-tab:hover {
    background: var(--hover-bg);
}
.top-tab.active:hover {
    background: var(--inner-bg) !important;
}
"#;
fn base() -> String {
    let l = LIGHT;
    let d = DARK;
    format!(
        r#"
* {{ box-sizing: border-box; margin: 0; padding: 0; }}

:root {{
  --outer-bg: {lo};
  --inner-bg: {li};
  --fg: {lf};
  --fg-muted: {lm};
  --fg-faint: {lfa};
  --accent-bg: {lab};
  --accent-fg: {laf};
  --hover-bg: {lh};
  --active-bg: {la};
  --positive: {lp};
  --negative: {ln};
  font-family: {font};
  font-size: 15px;
  line-height: 1.5;
  font-weight: 500;
}}

[data-theme="dark"] {{
  --outer-bg: {doo};
  --inner-bg: {dii};
  --fg: {dff};
  --fg-muted: {dmm};
  --fg-faint: {dffa};
  --accent-bg: {dab};
  --accent-fg: {daf};
  --hover-bg: {dhh};
  --active-bg: {daa};
  --positive: {dpp};
  --negative: {dnn};
}}

html, body, #main {{
  height: 100%; width: 100%;
  background: var(--outer-bg);
  color: var(--fg);
  overflow: hidden;
}}

button {{
  font-family: inherit; font-size: inherit; color: inherit;
  background: none; border: none; cursor: pointer; padding: 0;
}}

input, textarea {{
  font-family: inherit; font-size: inherit; color: inherit;
  background: none; border: none; outline: none;
}}

::-webkit-scrollbar {{ width: 10px; height: 10px; -webkit-appearance: none; background: transparent; }}
::-webkit-scrollbar-track {{ background: transparent; border-radius: 5px; }}
::-webkit-scrollbar-thumb {{ background: var(--active-bg); border-radius: 5px; border: 2px solid transparent; background-clip: padding-box; }}
::-webkit-scrollbar-thumb:hover {{ background: var(--fg-faint); background-clip: padding-box; }}

.content-split {{
  display: flex; flex: 1; min-height: 0; width: 100%;
}}
.content-split > .content {{ flex: 1; min-width: 0; }}
"#,
        lo = l.outer_bg,
        li = l.inner_bg,
        lf = l.fg,
        lm = l.fg_muted,
        lfa = l.fg_faint,
        lab = l.accent_bg,
        laf = l.accent_fg,
        lh = l.hover_bg,
        la = l.active_bg,
        lp = l.positive,
        ln = l.negative,
        doo = d.outer_bg,
        dii = d.inner_bg,
        dff = d.fg,
        dmm = d.fg_muted,
        dffa = d.fg_faint,
        dab = d.accent_bg,
        daf = d.accent_fg,
        dhh = d.hover_bg,
        daa = d.active_bg,
        dpp = d.positive,
        dnn = d.negative,
        font = FONT_STACK,
    )
}

const SHELL: &str = r#"
.srv-bar {
  display: flex; align-items: center; gap: 16px;
  overflow-x: auto; overflow-y: hidden; padding: 0;
}
.srv {
  width: 48px; height: 48px;
  border-radius: 15px;
  background: var(--inner-bg);
  color: var(--fg);
  display: flex; align-items: center; justify-content: center;
  font-size: 17px; font-weight: 600;
  flex-shrink: 0;
  box-shadow: 0 1px 3px rgba(0,0,0,0.06);
  user-select: none;
  transition: background 120ms, color 120ms;
}
.srv-selected { background: var(--accent-bg); color: var(--accent-fg); }
.srv-add { color: var(--fg-faint); font-size: 22px; line-height: 1; background: transparent; border: 1px dashed var(--fg-faint); box-shadow: none; }
"#;

const SIDEBAR: &str = r#"
.voice-panel {
  margin: 0 8px 8px;
  padding: 10px 12px;
  background: var(--hover-bg);
  border-radius: 12px;
}
.voice-panel-head {
  display: flex; align-items: center; gap: 6px; margin-bottom: 8px;
}
.voice-panel-icon { color: var(--positive); font-size: 13px; }
.voice-panel-name { font-size: 13px; font-weight: 600; flex: 1; }
.voice-panel-count { font-size: 12px; color: var(--fg-faint); }
.voice-panel-actions {
  display: flex; align-items: center; gap: 4px;
}
.vp-btn {
  width: 28px; height: 28px;
  display: flex; align-items: center; justify-content: center;
  border-radius: 8px; font-size: 14px;
  transition: background 120ms, color 120ms;
}
.vp-on { color: var(--fg); background: var(--inner-bg); }
.vp-off { color: var(--fg-faint); background: transparent; }
.vp-btn:hover { background: var(--active-bg); }
.vp-leave { color: var(--negative); font-size: 16px; }
"#;

const CONTENT: &str = r#"
.content {
  display: flex; flex-direction: column;
  height: 100%; min-height: 0;
}
.channel-header {
  height: 64px; flex-shrink: 0;
  display: flex; align-items: center; gap: 10px;
  padding: 8px 24px 0;
}
.channel-header-icon {
  color: var(--fg-faint);
  display: inline-flex;
  align-items: center;
  justify-content: center;
}
.channel-header-name { font-size: 17px; font-weight: 600; line-height: 1; }
.channel-header-desc { font-size: 13px; color: var(--fg-muted); }

.msg-list {
  flex: 1; min-height: 0;
  overflow-y: auto;
  padding: 0 16px 8px;
}
.msg-load-earlier {
  display: block; margin: 12px auto;
  padding: 6px 16px; border-radius: 999px;
  background: var(--hover-bg); color: var(--fg-muted);
  font-size: 13px; transition: background 120ms;
}
.msg-load-earlier:hover { background: var(--active-bg); }

.date-divider {
  display: flex; align-items: center; gap: 12px;
  margin: 20px 8px 12px;
  font-size: 12px; color: var(--fg-faint);
}
.date-divider::before,
.date-divider::after {
  content: ""; flex: 1; height: 1px;
  background: var(--active-bg);
}

.msg-group {
  padding: 6px 12px; border-radius: 10px;
  cursor: pointer; transition: background 120ms;
}
.msg-group:hover { background: var(--hover-bg); }
.msg-group.selected, .msg-group.selected:hover { background: var(--active-bg); }
.msg-group-merged { padding-top: 2px; padding-bottom: 2px; }
.msg-group-merged .msg-body { margin-top: 0; }

.msg-head {
  display: flex; align-items: center; gap: 8px; margin-bottom: 3px;
}
.msg-avatar {
  width: 28px; height: 28px; border-radius: 50%;
  background: var(--active-bg); color: var(--fg);
  display: inline-flex; align-items: center; justify-content: center;
  font-size: 12px; font-weight: 600;
  flex-shrink: 0;
}
.msg-author { font-weight: 600; font-size: 14px; }
.msg-time {
  font-size: 11px; color: var(--fg-faint);
  margin-left: 2px; font-weight: 400;
}
.msg-body {
  font-size: 14px; line-height: 1.55;
  word-break: break-word; white-space: pre-wrap;
  margin-left: 36px;
}
.msg-reply {
  margin-left: 36px; margin-bottom: 4px;
  padding: 2px 10px;
  border-left: 2px solid var(--fg-faint);
  font-size: 13px; color: var(--fg-muted);
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.msg-reply-author { font-weight: 600; margin-right: 4px; }

.composer-wrap { flex-shrink: 0; padding: 8px 24px 20px; }
.msg-action-bar {
  display: flex; align-items: center; gap: 6px;
  padding: 6px 8px; margin-bottom: 6px;
  background: var(--hover-bg); border-radius: 12px;
}
.msg-action-bar button {
  padding: 4px 12px; border-radius: 8px;
  color: var(--fg-muted); font-size: 13px;
  transition: background 120ms;
}
.msg-action-bar button:hover {
  background: var(--active-bg); color: var(--fg);
}
.selected-preview {
  flex: 1; padding: 0 8px;
  color: var(--fg-muted); font-size: 13px;
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.composer {
  display: flex; align-items: flex-end; gap: 8px;
  background: var(--hover-bg);
  border-radius: 16px;
  padding: 10px 12px 10px 16px;
}
.composer-input {
  flex: 1; resize: none;
  min-height: 24px; max-height: 140px;
  line-height: 1.5; font-size: 15px; padding: 4px 0;
}
.composer-send {
  padding: 8px 18px; border-radius: 10px;
  background: var(--accent-bg); color: var(--accent-fg);
  font-size: 14px; font-weight: 600;
  flex-shrink: 0; transition: opacity 120ms;
}
.composer-send:disabled { opacity: 0.35; cursor: default; }
"#;

const MEMBERS: &str = r#"
.member-list {
  width: 200px; flex-shrink: 0;
  overflow-y: auto;
  padding: 8px 4px;
}
.member-section {
  padding: 12px 8px 4px;
  font-size: 11px; color: var(--fg-faint);
  letter-spacing: 0.6px; text-transform: uppercase;
}
.member-row {
  width: 100%;
  display: flex; align-items: center; gap: 8px;
  padding: 6px 8px; border-radius: 8px;
  text-align: left;
  transition: background 120ms;
}
.member-row:hover { background: var(--hover-bg); }
.member-offline { opacity: 0.5; }
.member-avatar {
  width: 28px; height: 28px; border-radius: 50%;
  background: var(--active-bg); color: var(--fg);
  display: flex; align-items: center; justify-content: center;
  font-size: 13px; font-weight: 600;
  flex-shrink: 0;
}
.member-name {
  font-size: 14px;
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
"#;

const LOGIN: &str = r#"
.login-wrap {
  height: 100%;
  display: flex; align-items: center; justify-content: center;
}
.login-card {
  width: 360px; padding: 32px;
  background: var(--inner-bg);
  border-radius: 20px;
  box-shadow: 0 4px 24px rgba(0,0,0,0.06);
}
.login-title {
  font-size: 28px; margin-bottom: 24px; text-align: center;
  letter-spacing: 0.5px;
}
.login-input {
  width: 100%;
  padding: 10px 12px; margin-bottom: 12px;
  background: var(--hover-bg);
  border-radius: 10px;
}
.login-error {
  color: var(--negative); font-size: 13px; margin-bottom: 12px;
}
.login-btn {
  width: 100%; padding: 12px;
  background: var(--accent-bg); color: var(--accent-fg);
  border-radius: 12px; font-weight: 600;
}
"#;
const MODAL: &str = r#"
.server-modal-backdrop {
  position: fixed; inset: 0;
  background: rgba(0,0,0,0.35);
  display: flex; align-items: center; justify-content: center;
  z-index: 100;
}
.server-modal {
  position: relative;
  width: 780px; height: 520px;
  max-width: calc(100vw - 32px);
  max-height: calc(100vh - 32px);
  background: var(--inner-bg);
  border-radius: 24px;
  box-shadow: 0 20px 60px rgba(0,0,0,0.18);
  display: flex;
  overflow: hidden;
}
.server-modal-close {
  position: absolute;
  top: 16px; right: 16px;
  width: 36px; height: 36px;
  border-radius: 50%;
  color: var(--fg-muted);
  display: flex; align-items: center; justify-content: center;
  transition: background 120ms, color 120ms;
  z-index: 10;
}
.server-modal-close:hover {
  background: var(--hover-bg);
  color: var(--fg);
}

.server-modal-nav {
  width: 200px; flex-shrink: 0;
  padding: 28px 12px 12px;
  display: flex; flex-direction: column;
  gap: 2px;
  overflow-y: auto;
}
.server-modal-nav > button {
  width: 100%;
  padding: 8px 14px;
  border-radius: 10px;
  font-size: 14px;
  text-align: left;
  color: var(--fg-muted);
  transition: background 120ms, color 120ms;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  line-height: 1.6;
  display: flex; align-items: center; gap: 6px;
}
.server-modal-nav > button:hover {
  background: var(--hover-bg);
  color: var(--fg);
}
.server-modal-nav > button.active {
  background: var(--hover-bg);
  color: var(--fg);
  font-weight: 600;
}
.server-modal-add {
  margin-top: 16px;
  color: var(--fg-faint) !important;
  font-size: 13px !important;
}
.server-modal-add:hover {
  color: var(--fg-muted) !important;
}

.server-modal-content {
  flex: 1; min-width: 0;
  padding: 40px 44px 32px 16px;
  overflow-y: auto;
}
.server-modal-title {
  font-size: 22px;
  font-weight: 600;
  margin-bottom: 32px;
  letter-spacing: 0.3px;
}

.form-field { margin-bottom: 20px; }
.form-label {
  display: block;
  font-size: 13px;
  color: var(--fg-muted);
  margin-bottom: 8px;
}
.form-input {
  width: 100%;
  padding: 12px 16px;
  background: var(--hover-bg);
  border-radius: 12px;
  font-size: 15px;
  transition: background 120ms;
}
.form-input:focus { background: var(--active-bg); }
.form-radio-row { display: flex; gap: 8px; }
.form-radio {
  display: flex; align-items: center; gap: 6px;
  padding: 10px 16px;
  border-radius: 12px;
  background: var(--hover-bg);
  cursor: pointer;
  font-size: 14px;
  color: var(--fg-muted);
  transition: background 120ms, color 120ms;
}
.form-radio.active {
  background: var(--accent-bg);
  color: var(--accent-fg);
}
.form-hint {
  font-size: 13px;
  color: var(--fg-faint);
  margin-top: 8px;
  line-height: 1.5;
}
.form-actions {
  display: flex; align-items: center; gap: 8px;
  margin-top: 32px;
}
.btn-secondary {
  padding: 10px 20px;
  border-radius: 10px;
  color: var(--fg-muted);
  font-size: 14px;
  transition: background 120ms, color 120ms;
}
.btn-secondary:hover { background: var(--hover-bg); color: var(--fg); }
.btn-primary {
  padding: 10px 22px;
  border-radius: 10px;
  background: var(--accent-bg);
  color: var(--accent-fg);
  font-size: 14px; font-weight: 600;
}
.btn-danger {
  padding: 10px 16px;
  border-radius: 10px;
  color: var(--negative);
  font-size: 14px;
  transition: background 120ms;
  display: flex; align-items: center; gap: 6px;
}
.btn-danger:hover { background: var(--hover-bg); }

.srv-menu-btn {
  height: 36px;
  padding: 0 14px;
  display: flex; align-items: center; gap: 8px;
  border-radius: 10px;
  color: var(--fg);
  font-size: 14px;
  background: var(--inner-bg);
  transition: background 120ms;
  flex-shrink: 0;
  max-width: 220px;
}
.srv-menu-btn:hover { background: var(--hover-bg); }
.settings-modal { width: 480px; height: auto; min-height: 240px; }
.srv-menu-btn .label {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
"#;
