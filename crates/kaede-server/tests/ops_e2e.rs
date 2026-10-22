//! Beta 运维端点回归：`/health`、`/ready`、`/metrics`。
//!
//! 这三个端点必须在**无 Authorization** 的情况下可用，否则负载均衡器与
//! Prometheus 无法探活/抓取。

use reqwest::StatusCode;

async fn base() -> String {
    let addr = kaede_server::spawn_test_server()
        .await
        .expect("spawn_test_server");
    format!("http://{}", addr)
}

#[tokio::test]
async fn health_is_public_and_ok() {
    let base = base().await;
    let resp = reqwest::get(format!("{base}/health")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "ok");
}

#[tokio::test]
async fn ready_checks_database_without_auth() {
    let base = base().await;
    let resp = reqwest::get(format!("{base}/ready")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "ready");
}

#[tokio::test]
async fn metrics_exposes_prometheus_text_without_auth() {
    let base = base().await;
    let resp = reqwest::get(format!("{base}/metrics")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        content_type.starts_with("text/plain"),
        "expected Prometheus text content type, got {content_type}"
    );

    let body = resp.text().await.unwrap();
    for series in [
        "kaede_build_info{",
        "kaede_uptime_seconds",
        "kaede_ws_connections",
        "kaede_ws_connection_limit",
        "kaede_online_peers",
        "kaede_db_pool_connections",
        "kaede_messages_total",
        "kaede_rate_limited_total",
        "kaede_broadcast_budget_rejected_total",
        "kaede_slow_peer_dropped_total",
        "kaede_auth_failures_total",
    ] {
        assert!(body.contains(series), "missing metric {series}:\n{body}");
    }
    assert!(
        body.contains(&format!("kaede_ws_connection_limit {}", 5000)),
        "connection limit must match the semaphore"
    );
}

/// 认证失败计数必须真实增长：无效 token -> 401 + 指标 +1。
#[tokio::test]
async fn auth_failures_are_counted() {
    let base = base().await;

    let before = metric_value(&base, "kaede_auth_failures_total").await;

    // 带 Authorization 头才能穿过前置门槛，进入真正的 token 校验。
    let resp = reqwest::Client::new()
        .get(format!("{base}/api/me"))
        .bearer_auth("definitely-not-a-valid-token")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    let after = metric_value(&base, "kaede_auth_failures_total").await;
    assert_eq!(
        after,
        before + 1,
        "auth failure counter must increment (before={before}, after={after})"
    );
}

async fn metric_value(base: &str, name: &str) -> u64 {
    let body = reqwest::get(format!("{base}/metrics"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    body.lines()
        .find_map(|line| {
            let rest = line.strip_prefix(name)?.strip_prefix(' ')?;
            rest.trim().parse::<u64>().ok()
        })
        .unwrap_or_else(|| panic!("metric {name} not found in:\n{body}"))
}
