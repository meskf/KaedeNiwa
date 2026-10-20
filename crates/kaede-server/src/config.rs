//! 服务器配置。全部从环境变量读。

use anyhow::Result;
use std::env;

#[derive(Clone)]
pub struct Config {
    /// 监听地址，例如 `0.0.0.0:8080`。
    pub bind: String,
    /// SQLite 连接串。
    pub database_url: String,
    /// 会话 token 有效期（小时）。
    pub session_ttl_hours: i64,
    /// LiveKit API key。生产从 LIVEKIT_API_KEY 读；测试可注入假值。
    pub livekit_api_key: String,
    /// LiveKit API secret。生产从 LIVEKIT_API_SECRET 读；测试可注入假值。
    pub livekit_api_secret: String,
    /// 返回给客户端的 LiveKit WebSocket URL。
    pub livekit_ws_url: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            bind: env::var("KAEDE_BIND").unwrap_or_else(|_| "0.0.0.0:8080".into()),
            database_url: env::var("KAEDE_DB")
                .unwrap_or_else(|_| "sqlite://kaede.db?mode=rwc".into()),
            session_ttl_hours: env::var("KAEDE_SESSION_TTL_HOURS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(24 * 7),
            livekit_api_key: env::var("LIVEKIT_API_KEY").unwrap_or_default(),
            livekit_api_secret: env::var("LIVEKIT_API_SECRET").unwrap_or_default(),
            livekit_ws_url: env::var("LIVEKIT_WS_URL")
                .unwrap_or_else(|_| "ws://localhost:7880".into()),
        })
    }
}
