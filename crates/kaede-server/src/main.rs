//! KaedeNiwa signaling server 入口。

use anyhow::Result;
use kaede_server::{
    admin, attachments, channels, config::Config, db, dm, friends, http, livekit, members, reads,
    sessions_api, state::AppState, ws,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<()> {
    // 共享 log buffer：subscriber 写、admin /api/logs 读
    let log_buffer = admin::logs::LogBuffer::new();
    let log_filter = || {
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "info,kaede_server=debug".into())
    };
    // 生产日志默认人类可读；`KAEDE_LOG_FORMAT=json` 输出单行 JSON，
    // 便于 Loki / ELK / CloudWatch 采集。
    let json_logs =
        std::env::var("KAEDE_LOG_FORMAT").is_ok_and(|v| v.eq_ignore_ascii_case("json") || v == "1");
    if json_logs {
        tracing_subscriber::registry()
            .with(log_filter())
            .with(tracing_subscriber::fmt::layer().json())
            .with(log_buffer.layer())
            .init();
    } else {
        tracing_subscriber::registry()
            .with(log_filter())
            .with(tracing_subscriber::fmt::layer())
            .with(log_buffer.layer())
            .init();
    }

    let mut cfg = Config::from_env()?;
    if cfg.admin_enabled && cfg.admin_token.is_empty() {
        let token = db::random_token();
        tracing::warn!(
            target: admin::logs::SECRET_TARGET,
            "KAEDE_ADMIN_TOKEN not set; generated for this run: {}",
            token
        );
        tracing::warn!("set KAEDE_ADMIN_TOKEN env var to keep it stable across restarts");
        cfg.admin_token = token;
    }
    // fail-fast：非法配置直接退出，不带着半坏配置起服务。
    cfg.validate()?;
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

    // 引导预警：注册接口是公开的，且第一个注册者会自动成为 op5（房主）。
    // 若先暴露端口再注册，房主身份可能被他人抢注。
    match sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM members")
        .fetch_one(&pool)
        .await
    {
        Ok(0) => tracing::warn!(
            "no accounts yet: the FIRST registration becomes op5 (room owner). \
             Register the owner before exposing this port to the internet."
        ),
        Ok(_) => {}
        Err(e) => tracing::warn!("bootstrap owner check failed: {}", e),
    }
    let state = AppState::new_with_log(pool, cfg, log_buffer);

    let app = kaede_server::with_middleware(
        http::router(state.clone())
            .merge(attachments::router(state.clone()))
            .merge(friends::router(state.clone()))
            .merge(dm::router(state.clone()))
            .merge(livekit::router(state.clone()))
            .merge(channels::router(state.clone()))
            .merge(members::router(state.clone()))
            .merge(reads::router(state.clone()))
            .merge(sessions_api::router(state.clone()))
            .merge(ws::router(state.clone())),
    );

    // 管理面板：独立 task + 独立 listener（默认 127.0.0.1:3001）
    if state.cfg.admin_enabled {
        let admin_state = state.clone();
        let admin_bind = state.cfg.admin_bind.clone();
        tokio::spawn(async move {
            let app = admin::router(admin_state);
            match tokio::net::TcpListener::bind(&admin_bind).await {
                Ok(listener) => {
                    tracing::info!("admin panel on http://{}", admin_bind);
                    if let Err(e) = axum::serve(listener, app).await {
                        tracing::error!("admin serve failed: {}", e);
                    }
                }
                Err(e) => tracing::error!("admin bind {} failed: {}", admin_bind, e),
            }
        });
    }

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!("listening on http://{}", listener.local_addr()?);
    // CORS 已由 `with_middleware` 统一配置（KAEDE_CORS_ORIGINS）。
    // 这里**不能**再叠加一层 allow_origin(Any)，否则响应会出现两条
    // Access-Control-Allow-Origin，浏览器按规范拒绝。
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    tracing::info!("server stopped");
    Ok(())
}

/// 等待 SIGINT / SIGTERM。收到后 axum 停止接收新连接，等在途请求完成。
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(e) => tracing::warn!("cannot install SIGTERM handler: {}", e),
        }
    };

    #[cfg(unix)]
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    #[cfg(not(unix))]
    ctrl_c.await;

    tracing::info!("shutdown signal received");
}
