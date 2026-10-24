//! 服务器配置。全部从环境变量读。

use anyhow::{Context, Result, bail};
use std::env;
use std::net::SocketAddr;
use std::path::Path;

#[derive(Clone)]
pub struct Config {
    /// 监听地址，例如 `0.0.0.0:18080`。
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
    /// 是否启用管理面板（默认关）。
    pub admin_enabled: bool,
    /// 管理面板绑定地址（默认 127.0.0.1:18081，仅本机）。
    pub admin_bind: String,
    /// 管理面板 token（为空则 admin_enabled 时由 main 生成一次）。
    pub admin_token: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            bind: env::var("KAEDE_BIND").unwrap_or_else(|_| "0.0.0.0:18080".into()),
            database_url: env::var("KAEDE_DB")
                .unwrap_or_else(|_| "sqlite://kaede.db?mode=rwc".into()),
            session_ttl_hours: env::var("KAEDE_SESSION_TTL_HOURS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(24 * 7),
            livekit_api_key: env::var("LIVEKIT_API_KEY").unwrap_or_default(),
            livekit_api_secret: env::var("LIVEKIT_API_SECRET").unwrap_or_default(),
            livekit_ws_url: env::var("LIVEKIT_WS_URL")
                .unwrap_or_else(|_| "ws://localhost:18082".into()),
            admin_enabled: env::var("KAEDE_ADMIN_ENABLED")
                .map(|s| s == "1" || s == "true")
                .unwrap_or(false),
            admin_bind: env::var("KAEDE_ADMIN_BIND").unwrap_or_else(|_| "127.0.0.1:18081".into()),
            admin_token: env::var("KAEDE_ADMIN_TOKEN").unwrap_or_default(),
        })
    }

    /// 启动前校验配置。任何一项不合法都直接失败，不带着半坏配置起服务。
    ///
    /// 必须在 `main` 生成临时 admin token **之后**调用，否则
    /// `admin_enabled + 公网绑定 + 空 token` 会被误判为错误。
    pub fn validate(&self) -> Result<()> {
        let bind: SocketAddr = self
            .bind
            .parse()
            .with_context(|| format!("KAEDE_BIND is not a socket address: {}", self.bind))?;

        if !self.database_url.starts_with("sqlite:") {
            bail!("KAEDE_DB must be a sqlite: URL (got {})", self.database_url);
        }
        Self::ensure_database_dir(&self.database_url)?;

        if !(1..=24 * 365).contains(&self.session_ttl_hours) {
            bail!(
                "KAEDE_SESSION_TTL_HOURS must be 1..=8760 (got {})",
                self.session_ttl_hours
            );
        }
        // 可信代理 CIDR 非法必须启动失败，不能静默丢弃（否则真实客户端 IP
        // 会被归并成代理 IP，登录限流/注册限流全部失真）。
        if let Ok(raw) = env::var("KAEDE_TRUSTED_PROXIES") {
            crate::http::validate_trusted_proxies(&raw).map_err(|e| anyhow::anyhow!(e))?;
        }
        if !self.livekit_ws_url.starts_with("ws://") && !self.livekit_ws_url.starts_with("wss://") {
            bail!(
                "LIVEKIT_WS_URL must start with ws:// or wss:// (got {})",
                self.livekit_ws_url
            );
        }

        if self.admin_enabled {
            let admin: SocketAddr = self.admin_bind.parse().with_context(|| {
                format!(
                    "KAEDE_ADMIN_BIND is not a socket address: {}",
                    self.admin_bind
                )
            })?;
            if !admin.ip().is_loopback() {
                if self.admin_token.is_empty() {
                    bail!(
                        "KAEDE_ADMIN_BIND={} is not loopback but KAEDE_ADMIN_TOKEN is empty; \
                         refusing to expose the admin API without a token",
                        self.admin_bind
                    );
                }
                tracing::warn!(
                    "admin panel bound to non-loopback {}; put it behind HTTPS + IP allowlist",
                    self.admin_bind
                );
            }
        }

        if !bind.ip().is_loopback() {
            if env::var("KAEDE_CORS_ORIGINS").is_ok_and(|v| v.trim() == "*") {
                tracing::warn!(
                    "KAEDE_CORS_ORIGINS=* with a public bind; restrict origins in production"
                );
            }
            tracing::info!(
                "server bound to {}; terminate TLS at the reverse proxy (Caddy/Nginx)",
                self.bind
            );
        }
        Ok(())
    }

    /// SQLite 不会自动创建父目录；`mode=rwc` 只创建文件。
    pub(crate) fn ensure_database_dir(url: &str) -> Result<()> {
        let rest = url
            .strip_prefix("sqlite://")
            .or_else(|| url.strip_prefix("sqlite:"));
        let Some(rest) = rest else { return Ok(()) };
        let path = rest.split('?').next().unwrap_or(rest);
        if path.is_empty() || path == ":memory:" {
            return Ok(());
        }
        if let Some(parent) = Path::new(path).parent()
            && !parent.as_os_str().is_empty()
            && !parent.exists()
        {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("cannot create database directory {}", parent.display())
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Config;

    fn base() -> Config {
        Config {
            bind: "127.0.0.1:18080".into(),
            database_url: "sqlite::memory:".into(),
            session_ttl_hours: 168,
            livekit_api_key: "k".into(),
            livekit_api_secret: "s".into(),
            livekit_ws_url: "ws://localhost:18082".into(),
            admin_enabled: false,
            admin_bind: "127.0.0.1:18081".into(),
            admin_token: String::new(),
        }
    }

    #[test]
    fn valid_config_passes() {
        base().validate().expect("default config should validate");
    }

    #[test]
    fn bad_bind_is_rejected() {
        let mut cfg = base();
        cfg.bind = "not-an-address".into();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn non_sqlite_db_is_rejected() {
        let mut cfg = base();
        cfg.database_url = "postgres://localhost/kaede".into();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn zero_ttl_is_rejected() {
        let mut cfg = base();
        cfg.session_ttl_hours = 0;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn public_admin_without_token_is_rejected() {
        let mut cfg = base();
        cfg.admin_enabled = true;
        cfg.admin_bind = "0.0.0.0:18081".into();
        cfg.admin_token.clear();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn public_admin_with_token_passes() {
        let mut cfg = base();
        cfg.admin_enabled = true;
        cfg.admin_bind = "0.0.0.0:18081".into();
        cfg.admin_token = "x".repeat(32);
        cfg.validate().expect("token-protected admin should pass");
    }

    #[test]
    fn trusted_proxy_cidr_validation() {
        assert_eq!(
            crate::http::validate_trusted_proxies("127.0.0.0/8, ::1/128").unwrap(),
            2
        );
        assert_eq!(crate::http::validate_trusted_proxies("").unwrap(), 0);
        assert!(crate::http::validate_trusted_proxies("127.0.0.0/8, not-a-cidr").is_err());
    }
}
