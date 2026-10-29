//! KaedeNiwa signaling server。
//!
//! - HTTP：账户 / KeyPackage 目录
//! - WebSocket：实时信令 + MLS 消息转发
//!
//! **服务器永不解析 MLS payload。**

pub mod admin;
pub mod attachments;
pub mod avatars;
pub mod categories;
pub mod channels;
mod nodes;
pub mod config;
pub mod db;
pub mod dm;
pub mod error;
pub mod friends;
pub mod http;
pub mod livekit;
pub mod members;
pub mod metrics;
pub mod reads;
pub mod sessions_api;
pub mod state;
pub mod ws;

use std::sync::atomic::{AtomicU64, Ordering};

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderValue, Method};
use tower_http::cors::CorsLayer;

static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(1);

/// 起一个 in-process 测试服务器。返回监听的地址。
///
/// 用临时文件 SQLite（`sqlite::memory:` 在 sqlx pool 里 per-connection
/// 独立，会跟连接池语义冲突）。
pub async fn spawn_test_server() -> std::io::Result<std::net::SocketAddr> {
    let pid = std::process::id();
    let id = NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed);
    let db_path = std::env::temp_dir().join(format!("kaede_test_{}_{}.db", pid, id));
    let db_url = format!("sqlite://{}?mode=rwc", db_path.display());

    let cfg = config::Config {
        bind: "127.0.0.1:0".into(),
        database_url: db_url,
        session_ttl_hours: 1,
        livekit_api_key: "testkey".into(),
        livekit_api_secret: "testsecret".into(),
        livekit_ws_url: "ws://localhost:18082".into(),
        admin_enabled: false,
        admin_bind: "127.0.0.1:18081".into(),
        admin_token: String::new(),
    };
    let pool = db::init(&cfg.database_url).await.expect("db init");
    ensure_default_channels(&pool).await;
    let state = state::AppState::new(pool, cfg);
    let app = with_middleware(
        http::router(state.clone())
            .merge(attachments::router(state.clone()))
            .merge(avatars::router(state.clone()))
            .merge(friends::router(state.clone()))
            .merge(dm::router(state.clone()))
            .merge(livekit::router(state.clone()))
            .merge(categories::router(state.clone()))
        .merge(channels::router(state.clone()))
            .merge(members::router(state.clone()))
            .merge(reads::router(state.clone()))
            .merge(sessions_api::router(state.clone()))
            .merge(ws::router(state)),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await;
    });
    Ok(addr)
}

/// 给最终 Router 加上全局中间件：CORS + 请求体上限。
///
/// - Body limit 必须 > `attachments::MAX_ATTACHMENT_BYTES`（32 MiB）。
///   axum 默认只有 2 MiB，会让 32 MiB 的常量失效。
/// - CORS 默认只允许常见本地开发端口；生产用 `KAEDE_CORS_ORIGINS`
///   逗号分隔配置，`*` 表示允许任意来源。
pub async fn ensure_default_channels(pool: &sqlx::SqlitePool) {
    // 修复历史乱码：早期 seed 的 "语音" 因 GBK->UTF-8 错误落成 PUA 字符
    let _ = sqlx::query("UPDATE nodes SET name = ?1 WHERE id = 'voice' AND name != ?1")
        .bind("\u{8BED}\u{97F3}")
        .execute(pool)
        .await;

    let (count,): (i64,) = match sqlx::query_as("SELECT COUNT(*) FROM nodes")
        .fetch_one(pool)
        .await
    {
        Ok(r) => r,
        Err(_) => return,
    };
    if count > 0 {
        return;
    }
    let now = db::now_ms();
    let _ = sqlx::query(
        "INSERT INTO nodes (id, name, kind, position, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind("general")
    .bind("general")
    .bind("text")
    .bind(0i64)
    .bind(now)
    .execute(pool)
    .await;

    let _ = sqlx::query(
        "INSERT INTO nodes (id, name, kind, position, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind("voice")
    .bind("语音")
    .bind("voice")
    .bind(1i64)
    .bind(now)
    .execute(pool)
    .await;
}

#[allow(clippy::items_after_test_module)]
#[cfg(test)]
mod default_channel_tests {
    use super::*;

    #[tokio::test]
    async fn empty_database_gets_default_channels_idempotently() {
        let id = NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("kaede_default_{}_{}.db", std::process::id(), id));
        let url = format!("sqlite://{}?mode=rwc", path.display());
        let pool = db::init(&url).await.unwrap();

        ensure_default_channels(&pool).await;
        ensure_default_channels(&pool).await;

        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT id, kind FROM nodes ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(rows.len(), 2, "default channels must be created once");
        assert!(
            rows.iter()
                .any(|(id, kind)| id == "general" && kind == "text")
        );
        assert!(
            rows.iter()
                .any(|(id, kind)| id == "voice" && kind == "voice")
        );

        pool.close().await;
        let _ = std::fs::remove_file(path);
    }
}

pub fn with_middleware(app: Router) -> Router {
    let raw = std::env::var("KAEDE_CORS_ORIGINS").unwrap_or_else(|_| {
        "http://localhost:18080,http://127.0.0.1:18080,http://localhost:18085,http://127.0.0.1:18085,http://localhost:5173,http://127.0.0.1:5173"
            .to_string()
    });
    let cors = if raw.trim() == "*" {
        CorsLayer::new()
            .allow_origin(tower_http::cors::Any)
            .allow_methods(tower_http::cors::Any)
            .allow_headers(tower_http::cors::Any)
    } else {
        let origins: Vec<HeaderValue> = raw
            .split(',')
            .filter_map(|s| s.trim().parse::<HeaderValue>().ok())
            .collect();
        CorsLayer::new()
            .allow_origin(origins)
            .allow_methods([
                Method::GET,
                Method::POST,
                Method::PATCH,
                Method::DELETE,
                Method::OPTIONS,
            ])
            .allow_headers([AUTHORIZATION, CONTENT_TYPE])
    };

    app.layer(DefaultBodyLimit::max(34 * 1024 * 1024))
        .layer(axum::middleware::from_fn(require_auth_header))
        .layer(cors)
}

/// 在 body 被缓冲**之前**拒绝没有 Authorization 头的请求。
///
/// 真正的 token 校验仍在 handler/extractor 内完成；这里只做廉价的
/// 前置门槛，避免未认证请求把 32–34 MiB body 读进内存。
async fn require_auth_header(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, axum::http::StatusCode> {
    let path = req.uri().path();
    if req.method() == axum::http::Method::OPTIONS {
        return Ok(next.run(req).await);
    }

    // 头像/图标是公开元数据（成员列表所有成员可见），无 Bearer token 也能访问。
    // 让 <img src="/api/avatars/..."> 可以直接加载，避免前端为每张头像做 blob URL。
    // POST/DELETE 头像、POST 图标仍在 handler 内 authenticate。
    if path.starts_with("/api/avatars/") || path == "/api/server/icon" {
        return Ok(next.run(req).await);
    }

    // 放行：静态文件、WebSocket (业务层会通过 ClientHello 验证 token)、公开认证接口
    if !path.starts_with("/api")
        || path.starts_with("/ws")
        || path == "/api/login"
        || path == "/api/register"
    {
        return Ok(next.run(req).await);
    }

    // 其他 /api 路径必须携带 Authorization 头
    if req
        .headers()
        .contains_key(axum::http::header::AUTHORIZATION)
    {
        Ok(next.run(req).await)
    } else {
        Err(axum::http::StatusCode::UNAUTHORIZED)
    }
}
