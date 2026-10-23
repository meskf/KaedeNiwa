//! KaedeNiwa 客户端核心。
//!
//! # 设计
//!
//! - [`KaedeClient`]：多 server 容器。每个 server 独立连接、独立状态机、
//!   独立 MLS 组。
//! - [`ServerConnection`]：单个 server 的连接 + 状态 + MLS 会话。
//! - [`Transport`]：网络抽象。native / wasm / test 各有实现。
//!
//! # 不含
//!
//! - UI 状态：由 `kaede-ui` 层持有
//! - 具体网络实现：放在 `transport` 子模块，用 feature 开关
//!
//! # 线程模型
//!
//! [`KaedeClient`] 用 `std::sync::Mutex` 保护 `HashMap`，**不跨 await 持锁**。
//! 网络 IO 完全交给 [`Transport`]。

#![cfg_attr(target_arch = "wasm32", allow(clippy::arc_with_non_send_sync))]
// wasm 单线程：ServerConnection 构造时同样需要 allow

pub mod connection;
pub mod endpoint;
pub mod error;
pub mod event;
pub mod http;
pub mod keystore;

#[cfg(target_arch = "wasm32")]
pub mod keystore_web;
pub mod mls_session;
pub mod payload;
pub mod room;
pub mod state;
pub mod transport;

pub use connection::{PendingWelcomeInfo, ServerConnection};
pub use endpoint::{EndpointError, ServerEndpoint, parse_server_url};
pub use error::ClientError;
pub use event::ClientEvent;
pub use http::HttpApi;
pub use mls_session::MlsSession;
pub use payload::{DecodedPayload, MediaMetaPayload, MessagePayload, ReactionPayload, decode};
pub use room::{DEFAULT_CHANNEL, HistoryMessage, RoomSession, open_default_store};
pub use state::{ClientState, ClientStateMachine, TransitionError};
pub use transport::{MockTransport, Transport};

// 从 kaede-mls 重导出，UI 层不需要直接依赖 kaede-mls
pub use kaede_mls::SframePurpose;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// 事件回调。UI 层实现此 trait 接收来自所有 server 的事件。
pub trait ClientObserver: Send + Sync + 'static {
    fn on_event(&self, event: ClientEvent);
}

/// KaedeNiwa 客户端。
///
/// 管理 N 个 server 连接。用户可以同时在线多个 server，每个 server
/// 使用独立的身份（login_id + MLS identity）。
pub struct KaedeClient {
    servers: Mutex<HashMap<String, Arc<ServerConnection>>>,
    observer: Option<Arc<dyn ClientObserver>>,
}

impl KaedeClient {
    /// 创建客户端。
    pub fn new() -> Self {
        Self {
            servers: Mutex::new(HashMap::new()),
            observer: None,
        }
    }

    /// 设置事件观察者。整个 client 生命周期内只设一次。
    pub fn set_observer(&mut self, obs: Arc<dyn ClientObserver>) {
        self.observer = Some(obs);
    }

    /// 注册一个新 server 连接（未连接状态）。
    pub fn add_server(
        &self,
        server_id: impl Into<String>,
        transport: Arc<dyn Transport>,
    ) -> Arc<ServerConnection> {
        let server_id = server_id.into();
        let conn = Arc::new(ServerConnection::new(
            server_id.clone(),
            transport,
            self.observer.clone(),
        ));
        self.servers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(server_id, conn.clone());
        conn
    }

    /// 获取某个 server 连接。
    pub fn server(&self, server_id: &str) -> Option<Arc<ServerConnection>> {
        self.servers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(server_id)
            .cloned()
    }

    /// 当前所有 server ID。
    pub fn server_ids(&self) -> Vec<String> {
        self.servers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect()
    }

    /// 移除一个 server 连接。
    pub fn remove_server(&self, server_id: &str) -> Option<Arc<ServerConnection>> {
        self.servers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(server_id)
    }
}

impl Default for KaedeClient {
    fn default() -> Self {
        Self::new()
    }
}

/// crate 版本。
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_client() {
        let c = KaedeClient::new();
        assert_eq!(c.server_ids().len(), 0);
    }

    #[test]
    fn add_and_remove_servers() {
        let c = KaedeClient::new();
        let t1 = Arc::new(MockTransport::new());
        let t2 = Arc::new(MockTransport::new());
        c.add_server("s1", t1);
        c.add_server("s2", t2);
        assert_eq!(c.server_ids().len(), 2);
        assert!(c.server("s1").is_some());
        c.remove_server("s1");
        assert_eq!(c.server_ids().len(), 1);
    }
}
