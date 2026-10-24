//! Round 6：Pending Main Welcome（Signal 模型）。
//!
//! 收到主群 Welcome 时不立即入群，而是排队等用户在 UI 上确认
//! （[`ServerConnection::accept_pending_welcome`] /
//! [`ServerConnection::reject_pending_welcome`]），防止恶意 Welcome
//! 静默替换本地主群。

use kaede_protocol::UserId;

use crate::error::ClientError;
use crate::event::ClientEvent;
use crate::state::ClientState;

use super::{ServerConnection, unpack_welcome_payload};

/// 等待用户确认的主群 Welcome。
pub(crate) struct PendingMainWelcome {
    /// 服务器投递的原始 payload（含 group_id 前缀 + Welcome bytes）。
    pub(crate) payload: Vec<u8>,
    /// 发送者。
    pub(crate) from: UserId,
    /// 收到时本地是否已有主群。true 表示接受会重建主群（丢旧历史）。
    pub(crate) is_rebuild: bool,
    /// 收到时间，用于 UI 排序和可选超时清理。
    pub(crate) received_at: web_time::Instant,
}

/// 给 UI 展示用的只读视图。
#[derive(Debug, Clone)]
pub struct PendingWelcomeInfo {
    pub sender_login_id: String,
    pub is_rebuild: bool,
    pub age_secs: u64,
}

impl ServerConnection {
    /// 列出所有 pending 主群 Welcome。UI 层用来渲染确认对话框。
    pub fn pending_welcome_info(&self) -> Vec<PendingWelcomeInfo> {
        let map = self
            .pending_main_welcomes
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        map.iter()
            .map(|(sender, p)| PendingWelcomeInfo {
                sender_login_id: sender.clone(),
                is_rebuild: p.is_rebuild,
                age_secs: p.received_at.elapsed().as_secs(),
            })
            .collect()
    }

    /// 用户确认接受某个 sender 的 Welcome。
    ///
    /// 成功：从 pending 移除 + 应用 Welcome + 迁移状态 + emit
    /// `PendingWelcomeResolved { accepted: true }`。
    ///
    /// 失败（Welcome 解析失败 / 成员校验失败）：
    /// - pending 保留（用户仍可拒绝）
    /// - emit `MlsSyncError`
    pub fn accept_pending_welcome(&self, sender_login_id: &str) -> Result<(), ClientError> {
        let pending = {
            let mut map = self
                .pending_main_welcomes
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            map.remove(sender_login_id)
        };
        let Some(pending) = pending else {
            return Err(ClientError::Mls(format!(
                "no pending welcome from {}",
                sender_login_id
            )));
        };

        let was_ready = self.mls_is_ready();
        let (gid, welcome) = unpack_welcome_payload(&pending.payload);
        let result = self.mls_join_or_reset_welcome(welcome, gid, pending.from.as_str());
        match result {
            Ok(false) => {
                // 重复（理论上收到时已过滤，但用户可能在 pending 存活期间
                // 通过其他路径入群）
                tracing::debug!(
                    "[client:{}] accept_pending_welcome: duplicate, ignored",
                    self.server_id
                );
            }
            Ok(true) => {
                if was_ready {
                    tracing::warn!(
                        "[client:{}] main group rebuilt via accepted welcome from {}",
                        self.server_id,
                        pending.from
                    );
                }
                if self.state() == ClientState::Authenticated
                    && let Err(e) = self.transition(ClientState::MlsJoining)
                {
                    tracing::warn!("[client:{}] -> MlsJoining failed: {}", self.server_id, e);
                }
                if let Err(e) = self.transition(ClientState::MlsReady) {
                    tracing::debug!("[client:{}] -> MlsReady skipped: {}", self.server_id, e);
                }
            }
            Err(e) => {
                // 失败时把 pending 放回去，用户还能拒绝
                {
                    let mut map = self
                        .pending_main_welcomes
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    map.insert(sender_login_id.to_string(), pending);
                }
                self.emit(ClientEvent::MlsSyncError {
                    server_id: self.server_id.clone(),
                    user_message: "无法应用 Welcome，请重试或拒绝".into(),
                    recoverable: true,
                    detail: e.to_string(),
                });
                return Err(e);
            }
        }
        self.emit(ClientEvent::PendingWelcomeResolved {
            server_id: self.server_id.clone(),
            sender_login_id: sender_login_id.to_string(),
            accepted: true,
        });
        Ok(())
    }

    /// 用户拒绝某个 sender 的 Welcome。仅从 pending 移除，不影响 MLS 状态。
    pub fn reject_pending_welcome(&self, sender_login_id: &str) {
        let removed = {
            let mut map = self
                .pending_main_welcomes
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            map.remove(sender_login_id)
        };
        if removed.is_none() {
            return;
        }
        self.emit(ClientEvent::PendingWelcomeResolved {
            server_id: self.server_id.clone(),
            sender_login_id: sender_login_id.to_string(),
            accepted: false,
        });
    }
}
