//! **P1-AJ + H8 回归测试**：`HttpApi::fetch_key_package` 必须校验 server 返回的
//! `target` 字段与请求的 target 一致，并校验 KP 内部 credential。
//!
//! # 攻击场景
//!
//! 恶意 / 故障 server 对被请求 `GET /api/keypackages/alice` 返回
//! `{"target": "bob", "package": "<bob's KP>"}`。
//!
//! 如果客户端不校验 target：
//! 1. host 以为加的是 alice，实际加的是 bob 的 KP
//! 2. bob 会收到 Welcome（进入群）
//! 3. **alice 进不来**，群成员列表错误
//! 4. 更糟：如果 bob 是攻击者控制的账号，攻击者凭空进入群 → 密钥泄漏

use std::net::SocketAddr;

use axum::{Json, Router, routing::get};
use base64::Engine;
use kaede_client::MlsSession;
use kaede_client::http::HttpApi;
use kaede_protocol::UserId;
use serde_json::json;

/// 启动一个 mock server：`/api/keypackages/{login_id}` 返回固定的
/// `{"target": <target>, "package": <package>}`。
async fn spawn_mock_kp_server(target: String, package_b64: String) -> SocketAddr {
    let app = Router::new().route(
        "/api/keypackages/{login_id}",
        get(move || {
            let t = target.clone();
            let p = package_b64.clone();
            async move {
                Json(json!({
                    "target": t,
                    "package": p,
                }))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock listener");
    let addr = listener.local_addr().expect("local_addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    addr
}

#[tokio::test]
async fn p1aj_wrong_target_in_kp_response_rejected() {
    // mock 返回 target="bob"（错误）——但客户端请求 alice
    // target 校验在 base64 decode 之前短路，所以 package 无所谓
    let addr = spawn_mock_kp_server("bob".to_string(), "AAAA".to_string()).await;
    let base = format!("http://{}", addr);
    let api = HttpApi::new(&base).expect("HttpApi::new");
    api.set_token("dummy-token");

    let err = api
        .fetch_key_package(&UserId::new("alice"))
        .await
        .expect_err("client should reject mismatched KP target");

    let msg = format!("{}", err);
    assert!(
        msg.contains("target mismatch"),
        "error should mention 'target mismatch', got: {}",
        msg
    );
    assert!(
        msg.contains("alice"),
        "error should mention requested target 'alice', got: {}",
        msg
    );
    assert!(
        msg.contains("bob"),
        "error should mention returned target 'bob', got: {}",
        msg
    );
}

#[tokio::test]
async fn p1aj_correct_target_passes_through() {
    // 生成一个真实的 alice KeyPackage，因为 H8 会校验 KP 内部 credential
    let alice_session = MlsSession::new("alice").expect("create alice session");
    let kp_bytes = alice_session.generate_key_package().expect("gen kp");

    // 编码为 base64 URL-safe
    let kp_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&kp_bytes);

    let addr = spawn_mock_kp_server("alice".to_string(), kp_b64).await;
    let base = format!("http://{}", addr);
    let api = HttpApi::new(&base).expect("HttpApi::new");
    api.set_token("dummy-token");

    let kp = api
        .fetch_key_package(&UserId::new("alice"))
        .await
        .expect("correct target should decode base64 package");

    assert_eq!(kp, kp_bytes, "fetched KP bytes should match generated KP");
}
