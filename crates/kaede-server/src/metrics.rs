//! 轻量 Prometheus 指标。
//!
//! 手写 text exposition（`text/plain; version=0.0.4`），不引入额外依赖。
//! 只暴露计数/容量，不含账号、频道或消息内容，可安全交给 Prometheus 抓取。

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use crate::state::AppState;

#[derive(Default)]
pub struct Metrics {
    ws_active: AtomicI64,
    ws_rejected_total: AtomicU64,
    messages_total: AtomicU64,
    rate_limited_total: AtomicU64,
    broadcast_budget_rejected_total: AtomicU64,
    slow_peer_dropped_total: AtomicU64,
    auth_failures_total: AtomicU64,
}

impl Metrics {
    /// WS 升级成功后调用；连接结束时必须配对调用 [`Self::ws_disconnected`]。
    pub fn ws_connected(&self) {
        self.ws_active.fetch_add(1, Ordering::Relaxed);
    }

    pub fn ws_disconnected(&self) {
        // 正常配对时不会为负；用 CAS 防御重复调用造成下溢。
        let _ = self
            .ws_active
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                if v > 0 { Some(v - 1) } else { None }
            });
    }

    pub fn ws_rejected(&self) {
        self.ws_rejected_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn message_stored(&self) {
        self.messages_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn rate_limited(&self) {
        self.rate_limited_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn broadcast_budget_rejected(&self) {
        self.broadcast_budget_rejected_total
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn slow_peer_dropped(&self) {
        self.slow_peer_dropped_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn auth_failure(&self) {
        self.auth_failures_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn ws_active(&self) -> i64 {
        self.ws_active.load(Ordering::Relaxed)
    }
}

fn counter(out: &mut String, name: &str, help: &str, value: u64) {
    out.push_str(&format!(
        "# HELP {name} {help}\n# TYPE {name} counter\n{name} {value}\n"
    ));
}

fn gauge(out: &mut String, name: &str, help: &str, value: impl std::fmt::Display) {
    out.push_str(&format!(
        "# HELP {name} {help}\n# TYPE {name} gauge\n{name} {value}\n"
    ));
}

/// 渲染当前指标文本。
pub fn render(state: &AppState) -> String {
    let m = &state.metrics;
    let mut out = String::with_capacity(2048);

    out.push_str(
        "# HELP kaede_build_info Build and protocol information.\n\
         # TYPE kaede_build_info gauge\n",
    );
    out.push_str(&format!(
        "kaede_build_info{{version=\"{}\",protocol=\"{}\"}} 1\n",
        env!("CARGO_PKG_VERSION"),
        kaede_protocol::PROTOCOL_VERSION
    ));

    gauge(
        &mut out,
        "kaede_uptime_seconds",
        "Seconds since the server process started.",
        state.started_at.elapsed().as_secs(),
    );
    gauge(
        &mut out,
        "kaede_ws_connections",
        "Currently active WebSocket connections.",
        m.ws_active(),
    );
    gauge(
        &mut out,
        "kaede_ws_connection_limit",
        "Configured WebSocket connection limit.",
        crate::state::MAX_WS_CONNECTIONS,
    );
    gauge(
        &mut out,
        "kaede_online_peers",
        "Users currently joined to the room.",
        state.peer_count(),
    );
    gauge(
        &mut out,
        "kaede_db_pool_connections",
        "SQLite pool connections currently open.",
        state.pool.size(),
    );
    gauge(
        &mut out,
        "kaede_db_pool_idle",
        "SQLite pool idle connections.",
        state.pool.num_idle(),
    );

    counter(
        &mut out,
        "kaede_ws_rejected_total",
        "WebSocket upgrades rejected due to the connection limit.",
        m.ws_rejected_total.load(Ordering::Relaxed),
    );
    counter(
        &mut out,
        "kaede_messages_total",
        "Messages persisted via HTTP.",
        m.messages_total.load(Ordering::Relaxed),
    );
    counter(
        &mut out,
        "kaede_rate_limited_total",
        "Requests rejected by rate or byte budgets.",
        m.rate_limited_total.load(Ordering::Relaxed),
    );
    counter(
        &mut out,
        "kaede_broadcast_budget_rejected_total",
        "Broadcasts rejected by the room fanout budget.",
        m.broadcast_budget_rejected_total.load(Ordering::Relaxed),
    );
    counter(
        &mut out,
        "kaede_slow_peer_dropped_total",
        "Slow consumers dropped during important broadcasts.",
        m.slow_peer_dropped_total.load(Ordering::Relaxed),
    );
    counter(
        &mut out,
        "kaede_auth_failures_total",
        "Authentication failures (HTTP and WebSocket).",
        m.auth_failures_total.load(Ordering::Relaxed),
    );

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn metrics_render_contains_core_series() {
        use sqlx::sqlite::SqlitePoolOptions;
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let cfg = crate::config::Config {
            bind: "127.0.0.1:0".into(),
            database_url: "sqlite::memory:".into(),
            session_ttl_hours: 1,
            livekit_api_key: "testkey".into(),
            livekit_api_secret: "testsecret".into(),
            livekit_ws_url: "ws://localhost:18082".into(),
            admin_enabled: false,
            admin_bind: "127.0.0.1:18081".into(),
            admin_token: String::new(),
        };
        let state = AppState::new(pool, cfg);
        state.metrics.message_stored();
        state.metrics.rate_limited();
        state.metrics.ws_connected();

        let text = render(&state);
        assert!(text.contains("kaede_messages_total 1"));
        assert!(text.contains("kaede_rate_limited_total 1"));
        assert!(text.contains("kaede_ws_connections 1"));
        assert!(text.contains("kaede_build_info{"));
        assert!(text.contains("# TYPE kaede_uptime_seconds gauge"));

        state.metrics.ws_disconnected();
        state.metrics.ws_disconnected(); // 重复调用不能下溢
        assert_eq!(state.metrics.ws_active(), 0);
    }
}
