//! KaedeNiwa signaling server 入口。

use anyhow::Result;
use kaede_server::avatars;
use kaede_server::{
    admin, attachments, categories, channels, config::Config, db, dm, friends, http, livekit,
    members, reads, sessions_api, state::AppState, ws,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// 随服务端一起拉起的 LiveKit 子进程。`Drop` 时回收，避免早退路径留下孤儿进程。
struct LivekitChild {
    child: Option<std::process::Child>,
    /// 临时生成的 LiveKit 配置文件，进程退出时删除。
    config_file: Option<std::path::PathBuf>,
}

impl Drop for LivekitChild {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(path) = self.config_file.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// 同目录存在 `livekit-server.exe` / `livekit-server` 时自动拉起，实现单目录开箱即用。
///
/// 只在**服务端配置校验通过、监听成功后**调用：配置错误不会留下孤儿进程。
/// `--dev` 使用 LiveKit 的默认 devkey/secret，因此只绑回环，避免把无认证的
/// 开发实例暴露到局域网/公网。
fn spawn_bundled_livekit() -> LivekitChild {
    let Some(exe_path) = std::env::current_exe().ok().and_then(|p| {
        let dir = p.parent()?;
        ["livekit-server.exe", "livekit-server"]
            .iter()
            .map(|name| dir.join(name))
            .find(|path| path.exists())
    }) else {
        return LivekitChild {
            child: None,
            config_file: None,
        };
    };

    // 从 env 读 API key/secret：server 签 token、livekit 验 token 必须用同一组值。
    // 为空时跳过启动（比用 devkey 起来但 token 永远验不过更诚实）。
    let api_key = std::env::var("LIVEKIT_API_KEY").unwrap_or_default();
    let api_secret = std::env::var("LIVEKIT_API_SECRET").unwrap_or_default();
    if api_key.is_empty() || api_secret.is_empty() {
        tracing::warn!(
            "bundled livekit skipped: LIVEKIT_API_KEY/SECRET not set (voice will be unavailable)"
        );
        return LivekitChild {
            child: None,
            config_file: None,
        };
    }

    // bind IP 从 KAEDE_BIND 推导：
    //   `0.0.0.0:18080`   → `0.0.0.0`（局域网/公网可访问）
    //   `127.0.0.1:18080` → `127.0.0.1`（仅本机）
    let bind_ip = std::env::var("KAEDE_BIND")
        .ok()
        .and_then(|s| s.rsplit_once(':').map(|(ip, _)| ip.to_string()))
        .unwrap_or_else(|| "0.0.0.0".into());

    let ws_port: u32 = std::env::var("KAEDE_LIVEKIT_WS_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(18082);
    let tcp_port: u32 = std::env::var("KAEDE_LIVEKIT_TCP_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(18083);
    let udp_port: u32 = std::env::var("KAEDE_LIVEKIT_UDP_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(18084);

    // LiveKit 没有 --port 参数，必须通过 YAML 配置端口。
    // `keys:` 段必须与 server 的 LIVEKIT_API_KEY/SECRET 一致，否则 token 校验失败。
    let config = format!(
        "port: {ws_port}\n\
         rtc:\n  tcp_port: {tcp_port}\n  udp_port: {udp_port}\n  use_external_ip: false\n\
         keys:\n  {api_key}: {api_secret}\n\
         logging:\n  level: info\n  json: false\n\
         room:\n  auto_create: true\n"
    );
    let config_path =
        std::env::temp_dir().join(format!("kaede-livekit-{}.yaml", std::process::id()));
    if let Err(e) = std::fs::write(&config_path, &config) {
        tracing::error!("failed to write bundled LiveKit config: {}", e);
        return LivekitChild {
            child: None,
            config_file: None,
        };
    }

    tracing::info!(
        "starting bundled LiveKit server from {:?} (bind={}, ws={}, tcp={}, udp={})",
        exe_path,
        bind_ip,
        ws_port,
        tcp_port,
        udp_port
    );
    match std::process::Command::new(&exe_path)
        .args(["--bind", bind_ip.as_str(), "--config"])
        .arg(&config_path)
        .spawn()
    {
        Ok(child) => LivekitChild {
            child: Some(child),
            config_file: Some(config_path),
        },
        Err(e) => {
            tracing::error!("failed to spawn livekit-server: {}", e);
            let _ = std::fs::remove_file(&config_path);
            LivekitChild {
                child: None,
                config_file: None,
            }
        }
    }
}

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
            .merge(avatars::router(state.clone()))
            .merge(friends::router(state.clone()))
            .merge(dm::router(state.clone()))
            .merge(livekit::router(state.clone()))
            .merge(categories::router(state.clone()))
            .merge(channels::router(state.clone()))
            .merge(members::router(state.clone()))
            .merge(reads::router(state.clone()))
            .merge(sessions_api::router(state.clone()))
            .merge(ws::router(state.clone()))
            // 静态文件托管：所有非 API 请求 fallback 到 dist/web/public
            .fallback_service(
                tower_http::services::ServeDir::new("web")
                    .fallback(tower_http::services::ServeFile::new("web/index.html")),
            ),
    );

    // 管理面板：独立 task + 独立 listener（默认 127.0.0.1:18081）
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

    // 定期清理过期数据，防止 SQLite 膨胀
    {
        let pool = state.pool.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
            loop {
                interval.tick().await;
                if let Err(e) = kaede_server::db::cleanup_expired(&pool).await {
                    tracing::warn!("periodic cleanup failed: {}", e);
                }
                // 附件表与磁盘文件对账（上传/删除崩溃残留的孤儿）。
                let (files, rows) = kaede_server::attachments::sweep_orphans(&pool).await;
                if files > 0 || rows > 0 {
                    tracing::info!("[attachments] swept {} file(s), {} row(s)", files, rows);
                }
            }
        });
    }

    let listener = tokio::net::TcpListener::bind(&bind).await?;

    // 端口绑定成功后才拉起 LiveKit：配置/绑定失败不会留下孤儿进程。
    // guard 在 main 返回（含提前 `?`）时统一回收子进程。
    let _livekit = spawn_bundled_livekit();

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
