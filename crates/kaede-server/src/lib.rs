//! KaedeNiwa signaling server。
//!
//! - HTTP：账户 / KeyPackage 目录
//! - WebSocket：实时信令 + MLS 消息转发
//!
//! **服务器永不解析 MLS payload。**

pub mod attachments;
pub mod channels;
pub mod config;
pub mod db;
pub mod dm;
pub mod error;
pub mod friends;
pub mod http;
pub mod livekit;
pub mod members;
pub mod state;
pub mod ws;

use std::net::SocketAddr;

/// 起一个 in-process 测试服务器。返回监听的地址。
///
/// 用临时文件 SQLite（`sqlite::memory:` 在 sqlx pool 里 per-connection
/// 独立，会跟连接池语义冲突）。
pub async fn spawn_test_server() -> std::io::Result<SocketAddr> {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let db_path = std::env::temp_dir().join(format!("kaede_test_{}.db", unique));
    let db_url = format!("sqlite://{}?mode=rwc", db_path.display());

    let cfg = config::Config {
        bind: "127.0.0.1:0".into(),
        database_url: db_url,
        session_ttl_hours: 1,
        livekit_api_key: "testkey".into(),
        livekit_api_secret: "testsecret".into(),
        livekit_ws_url: "ws://localhost:7880".into(),
    };
    let pool = db::init(&cfg.database_url).await.expect("db init");
    ensure_default_channels(&pool).await;
    let state = state::AppState::new(pool, cfg);
    let app = http::router(state.clone())
        .merge(attachments::router(state.clone()))
        .merge(friends::router(state.clone()))
        .merge(dm::router(state.clone()))
        .merge(livekit::router(state.clone()))
        .merge(channels::router(state.clone()))
        .merge(members::router(state.clone()))
        .merge(ws::router(state));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok(addr)
}

/// 首次启动建默认频道（general + voice）。
async fn ensure_default_channels(pool: &sqlx::SqlitePool) {
    let (count,): (i64,) = match sqlx::query_as("SELECT COUNT(*) FROM channels")
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
        "INSERT INTO channels (id, name, kind, position, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind("general")
    .bind("general")
    .bind("text")
    .bind(0i64)
    .bind(now)
    .execute(pool)
    .await;

    let _ = sqlx::query(
        "INSERT INTO channels (id, name, kind, position, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind("voice")
    .bind("语音")
    .bind("voice")
    .bind(1i64)
    .bind(now)
    .execute(pool)
    .await;
}
