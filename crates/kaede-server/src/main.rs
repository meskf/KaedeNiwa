//! KaedeNiwa signaling server 入口。

use anyhow::Result;
use kaede_server::{
    attachments, channels, config::Config, db, dm, friends, http, livekit, members,
    state::AppState, ws,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,kaede_server=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cfg = Config::from_env()?;
    tracing::info!(
        "KaedeNiwa server v{} bind={} db={}",
        env!("CARGO_PKG_VERSION"),
        cfg.bind,
        cfg.database_url
    );

    // 启动时 fail-fast：LIVEKIT 环境变量缺失会导致 /api/livekit/* 端点 500。
    // 生产必须设置；开发可设 KAEDE_SKIP_LIVEKIT_ENV_CHECK=1 跳过（端点仍会 500）。
    let skip_livekit_env = std::env::var("KAEDE_SKIP_LIVEKIT_ENV_CHECK").is_ok();
    if skip_livekit_env {
        tracing::warn!("KAEDE_SKIP_LIVEKIT_ENV_CHECK set; /api/livekit/* will return 500");
    } else {
        for var in ["LIVEKIT_API_KEY", "LIVEKIT_API_SECRET"] {
            if std::env::var(var).is_err() {
                anyhow::bail!(
                    "{} must be set before starting the server. \
                     Set KAEDE_SKIP_LIVEKIT_ENV_CHECK=1 to bypass (dev only, NOT for production).",
                    var
                );
            }
        }
        tracing::info!("livekit env: ok");
    }
    let bind = cfg.bind.clone();
    let pool = db::init(&cfg.database_url).await?;
    // 全新数据库必须补齐默认频道，否则客户端默认的 `general` 会 404。
    kaede_server::ensure_default_channels(&pool).await;
    let state = AppState::new(pool, cfg);

    let app = kaede_server::with_middleware(
        http::router(state.clone())
            .merge(attachments::router(state.clone()))
            .merge(friends::router(state.clone()))
            .merge(dm::router(state.clone()))
            .merge(livekit::router(state.clone()))
            .merge(channels::router(state.clone()))
            .merge(members::router(state.clone()))
            .merge(ws::router(state)),
    );

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!("listening on http://{}", listener.local_addr()?);
    // B2: CORS 允许任意 origin（Bearer token，不走 cookie，无 CSRF 风险）；
    // 生产若需收紧可改为 allow_origin([...]) 白名单。
    let app = app.layer(
        tower_http::cors::CorsLayer::new()
            .allow_origin(tower_http::cors::Any)
            .allow_methods(tower_http::cors::Any)
            .allow_headers(tower_http::cors::Any),
    );
    axum::serve(listener, app).await?;

    Ok(())
}
