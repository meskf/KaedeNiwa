//! 管理面板。
//!
//! 独立的 HTTP server（默认 127.0.0.1:18081），通过 X-Admin-Token header 认证。
//! 面板看不到任何 E2EE 内容——只管理元数据（用户/频道/配置/日志）。

pub mod auth;
pub mod handlers;
pub mod logs;
pub mod service;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, post};

use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/dashboard", get(handlers::dashboard))
        .route("/api/users", get(handlers::list_users))
        .route("/api/users/{id}/ban", post(handlers::ban_user))
        .route("/api/users/{id}/unban", post(handlers::unban_user))
        // P1-12：op 晋升/降级。admin token 是最高权限，不受用户 op 体系约束。
        // 用于恢复误操作、转移房主（op5）、临时指派 op4 代理人。
        // 所有变更强制写入 admin_audit。
        .route("/api/users/{id}/op", post(handlers::set_op))
        .route("/api/channels", get(handlers::list_channels))
        .route("/api/channels/{id}", delete(handlers::delete_channel))
        .route("/api/logs", get(handlers::list_logs))
        .route("/api/service", get(handlers::get_service))
        .route("/api/service/mode", post(handlers::set_mode))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .with_state(state)
}
