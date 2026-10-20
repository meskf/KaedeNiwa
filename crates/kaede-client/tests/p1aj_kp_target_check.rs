//! **P1-AJ 回归测试**：`HttpApi::fetch_key_package` 必须校验 server 返回的
//! `target` 字段与请求的 target 一致。
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
//!
//! 修复位置：`kaede-client/src/http.rs::fetch_key_package`
//!   `if body.target != target.as_str() { return Err(...); }`

use std::net::SocketAddr;

use axum::{Json, Router, routing::get};
use kaede_client::http::HttpApi;
use kaede_protocol::UserId;
use serde_json::json;

/// 启动一个 mock server：`/api/keypackages/{login_id}` 永远返回固定的
/// `{"target": <bad_target>, "package": "AAAA"}`。
///
/// 不需要真实的 KeyPackage 内容——因为 target 校验发生在 base64 decode 之前，
/// 只要 target 不匹配就一定短路返回 `Protocol` 错误。
async fn spawn_mock_kp_server(bad_target: &'static str) -> SocketAddr {
    let app = Router::new().route(
        "/api/keypackages/{login_id}",
        get(move || async move {
            Json(json!({
                "target": bad_target,
                "package": "AAAA", // 无效 base64 也无所谓（校验在前）
            }))
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
    let addr = spawn_mock_kp_server("bob").await;
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
    // mock 返回 target="alice"（正确）
    let addr = spawn_mock_kp_server("alice").await;
    let base = format!("http://{}", addr);
    let api = HttpApi::new(&base).expect("HttpApi::new");
    api.set_token("dummy-token");

    // 校验通过后，进入 base64 decode。这里 package="AAAA" 是有效的
    // base64（解出 3 个 null 字节），所以应当 Ok，返回 3 字节。
    let kp = api
        .fetch_key_package(&UserId::new("alice"))
        .await
        .expect("correct target should decode base64 package");
    assert_eq!(
        kp,
        vec![0u8, 0u8, 0u8],
        "AAAA base64 decodes to 3 zero bytes"
    );
}
