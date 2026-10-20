//! 安全 e2e 测试（E-4，2026-10-19 全量审查补充）
//!
//! 覆盖 3 个安全边界：
//! 1. **附件路径穿越**：`GET /api/attachments/../../etc/passwd` → 400
//! 2. **踢人后旧 token 失效**：A（op5）踢 B → B 的旧 session token 立刻 401
//! 3. **DM 越权读取**：C 尝试读/写 A-B 的 DM → 401
//!
//! 这些测试直接走 HTTP API（不经过 `RoomSession` / `kaede-client`），
//! 是最贴近外部攻击者视角的验证方式。
//!
//! # 测试隔离
//!
//! 每个测试用独立的 `spawn_test_server()`（内部生成唯一的临时 SQLite DB），
//! 可以并行跑。但**建议**用 `--test-threads=1` 减少 CI 上的偶发失败。

use std::net::SocketAddr;

use reqwest::StatusCode;
use serde_json::{Value, json};

/// 启动 in-process 服务器，返回监听地址。
async fn spawn_server() -> SocketAddr {
    kaede_server::spawn_test_server()
        .await
        .expect("spawn_test_server should succeed")
}

/// 注册用户，返回 `(login_id, session_token)`。
///
/// 第一个注册用户自动是 op5（服主），后续是 op2（成员）。
async fn register(base: &str, login_id: &str, password: &str) -> (String, String) {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/register", base))
        .json(&json!({
            "login_id": login_id,
            "password": password,
            "nickname": "",
        }))
        .send()
        .await
        .unwrap_or_else(|e| panic!("register {} send failed: {}", login_id, e));

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "register {} returned {}",
        login_id,
        resp.status()
    );

    let body: Value = resp.json().await.expect("register response should be JSON");
    let uid = body["login_id"]
        .as_str()
        .expect("login_id in response")
        .to_string();
    let token = body["session_token"]
        .as_str()
        .expect("session_token in response")
        .to_string();
    (uid, token)
}

/// **E-4 #1**：附件路径穿越必须被拒绝。
///
/// 攻击者构造 `%2e%2e%2f%2e%2e%2fetc%2fpasswd`（URL 编码的 `../../etc/passwd`），
/// axum 的 `Path<String>` 会自动 URL-decode 成 `../../etc/passwd`，
/// `download` handler 的 `id.contains('/')` 检查应该拒绝 → 400。
///
/// 用 `%2e%2e%2f` 而不是字面量 `../` 是为了绕过 reqwest 的 URL 规范化。
#[tokio::test]
async fn attachment_path_traversal_rejected() {
    let addr = spawn_server().await;
    let base = format!("http://{}", addr);

    let (_uid, token) = register(&base, "traversal-user", "password123").await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!(
            "{}/api/attachments/%2e%2e%2f%2e%2e%2fetc%2fpasswd",
            base
        ))
        .bearer_auth(&token)
        .send()
        .await
        .expect("attachment request should complete");

    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "path traversal must be rejected with 400"
    );
}

/// **E-4 #2**：踢人后旧 session token 立刻失效。
///
/// 步骤：
/// 1. `kick-alice` 先注册 → 自动 op5（服主）
/// 2. `kick-bob` 后注册 → 自动 op2（成员）
/// 3. 验证 bob 的 token 在踢人前有效（`GET /api/me` → 200）
/// 4. alice 踢 bob（`DELETE /api/members/kick-bob`）→ 204
/// 5. 验证 bob 的 token 立刻失效（`GET /api/me` → 401）
///
/// 关键实现：`members.rs::remove_member` 会 `DELETE FROM sessions WHERE login_id = ?`，
/// `authenticate` 查不到 session → 401。
#[tokio::test]
async fn kick_invalidates_token() {
    let addr = spawn_server().await;
    let base = format!("http://{}", addr);

    let (_alice, alice_token) = register(&base, "kick-alice", "password123").await;
    let (bob, bob_token) = register(&base, "kick-bob", "password123").await;

    let client = reqwest::Client::new();

    // 踢人前：bob token 有效
    let resp = client
        .get(format!("{}/api/me", base))
        .bearer_auth(&bob_token)
        .send()
        .await
        .expect("me-before-kick send");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "bob token must be valid before kick"
    );

    // alice (op5) 踢 bob
    let resp = client
        .delete(format!("{}/api/members/{}", base, bob))
        .bearer_auth(&alice_token)
        .send()
        .await
        .expect("kick send");
    assert_eq!(
        resp.status(),
        StatusCode::NO_CONTENT,
        "op5 kick should succeed"
    );

    // 踢人后：bob token 立刻 401
    let resp = client
        .get(format!("{}/api/me", base))
        .bearer_auth(&bob_token)
        .send()
        .await
        .expect("me-after-kick send");
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "bob token must be invalidated after kick"
    );
}

/// **E-4 #3**：DM 越权读取/写入必须被拒绝。
///
/// 步骤：
/// 1. `dm-alice` / `dm-bob` / `dm-carol` 都注册
/// 2. alice 调 `POST /api/dm/dm-bob` 拿 conv_id（`dm:dm-alice:dm-bob`）
/// 3. bob 能读 A-B DM（`GET /api/dm/{conv_id}/messages` → 200）
/// 4. carol **不能读** A-B DM → 401
/// 5. carol **不能写** A-B DM → 401
///
/// 关键实现：`dm.rs::is_member` 检查 conv_id 里的两个 login_id 是否包含 caller。
#[tokio::test]
async fn dm_cross_user_rejected() {
    let addr = spawn_server().await;
    let base = format!("http://{}", addr);

    let (_alice, alice_token) = register(&base, "dm-alice", "password123").await;
    let (_bob, bob_token) = register(&base, "dm-bob", "password123").await;
    let (_carol, carol_token) = register(&base, "dm-carol", "password123").await;

    let client = reqwest::Client::new();

    // alice 开 A-B 会话
    let resp = client
        .post(format!("{}/api/dm/dm-bob", base))
        .bearer_auth(&alice_token)
        .send()
        .await
        .expect("open-dm send");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "alice should open DM with bob"
    );
    let body: Value = resp.json().await.expect("open-dm json");
    let conv_id = body["conv_id"]
        .as_str()
        .expect("conv_id in response")
        .to_string();

    // bob 可以读
    let resp = client
        .get(format!("{}/api/dm/{}/messages", base, conv_id))
        .bearer_auth(&bob_token)
        .send()
        .await
        .expect("bob-read-dm send");
    assert_eq!(resp.status(), StatusCode::OK, "bob should read A-B DM");

    // carol **不能读**
    let resp = client
        .get(format!("{}/api/dm/{}/messages", base, conv_id))
        .bearer_auth(&carol_token)
        .send()
        .await
        .expect("carol-read-dm send");
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "carol must not read A-B DM"
    );

    // carol **不能写**
    let resp = client
        .post(format!("{}/api/dm/{}/messages", base, conv_id))
        .bearer_auth(&carol_token)
        .json(&json!({
            "ts": 1_700_000_000_000i64,
            "sender": "dm-carol",
            "ciphertext": "AAAA",
        }))
        .send()
        .await
        .expect("carol-write-dm send");
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "carol must not write A-B DM"
    );
}

/// **P1-AD**：avatar_url 的 data: URI 白名单必须拒绝 SVG。
///
/// SVG 可以内嵌 `<script>`，部分渲染器（`<object>` / `<iframe>` / `<embed>`）
/// 会执行，构成 XSS。当前白名单限定 `data:image/{png,jpeg,gif,webp}`。
///
/// 同时验证：合法的 PNG data URI 和 HTTPS URL 仍被接受（白名单不过度严格）。
#[tokio::test]
async fn avatar_svg_data_uri_rejected() {
    let addr = spawn_server().await;
    let base = format!("http://{}", addr);

    let (alice, alice_token) = register(&base, "avatar-svg", "password123").await;
    let client = reqwest::Client::new();

    // 1. 恶意 SVG data URI 必须被拒绝
    let resp = client
        .patch(format!("{}/api/members/{}", base, alice))
        .bearer_auth(&alice_token)
        .json(&json!({
            "avatar_url": "data:image/svg+xml;base64,PHN2ZyBvbmxvYWQ9ImFsZXJ0KDEpIi8+"
        }))
        .send()
        .await
        .expect("patch svg-avatar send");
    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "SVG data URI must be rejected with 400"
    );

    // 2. 合法 PNG data URI 必须被接受
    let resp = client
        .patch(format!("{}/api/members/{}", base, alice))
        .bearer_auth(&alice_token)
        .json(&json!({
            "avatar_url": "data:image/png;base64,iVBORw0KGgo="
        }))
        .send()
        .await
        .expect("patch png-avatar send");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "PNG data URI should be accepted"
    );

    // 3. 合法 HTTPS URL 必须被接受
    let resp = client
        .patch(format!("{}/api/members/{}", base, alice))
        .bearer_auth(&alice_token)
        .json(&json!({
            "avatar_url": "https://example.com/avatar.png"
        }))
        .send()
        .await
        .expect("patch https-avatar send");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "HTTPS URL should be accepted"
    );

    // 4. 非法 scheme（`javascript:`）也必须被拒绝
    let resp = client
        .patch(format!("{}/api/members/{}", base, alice))
        .bearer_auth(&alice_token)
        .json(&json!({
            "avatar_url": "javascript:alert(1)"
        }))
        .send()
        .await
        .expect("patch js-avatar send");
    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "javascript: scheme must be rejected with 400"
    );
}

// ============================================================================
// Batch 3：P0-A（并发 login）+ P0-C（并发 register）
// ============================================================================
//
// ⚠️ 诚实说明：
// P0-A / P0-C 的修复本质是 **Argon2 DoS 防护**——修复前/后的功能行为
// 完全一样（"5 次失败锁账号"、"并发注册只成功一次"）。
// 纯 HTTP 行为测试无法区分修复前/后。本批测试的价值是：
//   1. **行为回归保护**：如果未来有人改坏了 login 锁 / register 冲突处理，会立刻红
//   2. **并发安全性验证**：确认 `INSERT ... ON CONFLICT DO NOTHING` 原子性
//   3. **性能测试**（#[ignore]）：手动跑才看得到 Argon2 flood 的避免效果

use std::time::{Duration, Instant};

/// **P0-A**：并发 100 次失败登录后必须锁账号。
///
/// 验证：
/// 1. 100 个并发错误密码 → 全部 401
/// 2. 之后正确密码 → 401（被锁 15 分钟）
/// 3. 已有 session token 仍有效（锁只影响 login，不影响 authenticate）
#[tokio::test]
async fn p0a_concurrent_failures_lock_account() {
    let addr = spawn_server().await;
    let base = format!("http://{}", addr);

    // 注册 alice，记住初始 session_token
    let (_alice, alice_token) = register(&base, "p0a-alice", "pw-correct-123").await;

    // 并发 100 次错误密码
    let mut handles = Vec::new();
    for _ in 0..100 {
        let base = base.clone();
        handles.push(tokio::spawn(async move {
            reqwest::Client::new()
                .post(format!("{}/api/login", base))
                .json(&json!({
                    "login_id": "p0a-alice",
                    "password": "pw-wrong",
                }))
                .send()
                .await
                .map(|r| r.status())
        }));
    }
    let mut unauth_count = 0;
    for h in handles {
        match h.await.expect("join").expect("send") {
            StatusCode::UNAUTHORIZED => unauth_count += 1,
            s => panic!("expected 401 for wrong password, got {}", s),
        }
    }
    assert_eq!(
        unauth_count, 100,
        "all 100 concurrent bad logins must return 401"
    );

    // 正确密码也必须被锁
    let resp = reqwest::Client::new()
        .post(format!("{}/api/login", base))
        .json(&json!({
            "login_id": "p0a-alice",
            "password": "pw-correct-123",
        }))
        .send()
        .await
        .expect("correct-pw login send");
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "after concurrent failures, correct password must be locked out"
    );

    // 已有 session 仍然有效
    let resp = reqwest::Client::new()
        .get(format!("{}/api/me", base))
        .bearer_auth(&alice_token)
        .send()
        .await
        .expect("me send");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "existing session should still be valid (lock is for login, not authentication)"
    );
}

/// **P0-A**：对一个账号的攻击不应阻塞其他账号登录。
///
/// 场景：alice 被 100 并发攻击的同时，bob 正常登录。
/// 期望：bob 200 OK（修复前的 bug：Argon2 flood 占满 semaphore，bob 排队等待）。
#[tokio::test]
async fn p0a_attack_on_alice_does_not_block_bob() {
    let addr = spawn_server().await;
    let base = format!("http://{}", addr);

    let (_alice, _) = register(&base, "p0a-iso-alice", "pw-alice-123").await;
    let (_bob, _) = register(&base, "p0a-iso-bob", "pw-bob-123").await;

    // 先启动 100 个 alice 的错误密码请求
    let mut alice_handles = Vec::new();
    for _ in 0..100 {
        let base = base.clone();
        alice_handles.push(tokio::spawn(async move {
            reqwest::Client::new()
                .post(format!("{}/api/login", base))
                .json(&json!({
                    "login_id": "p0a-iso-alice",
                    "password": "wrong",
                }))
                .send()
                .await
                .map(|r| r.status())
        }));
    }

    // 立即启动 bob 的正确登录（与 alice 攻击竞争 semaphore）
    let bob_handle = {
        let base = base.clone();
        tokio::spawn(async move {
            reqwest::Client::new()
                .post(format!("{}/api/login", base))
                .json(&json!({
                    "login_id": "p0a-iso-bob",
                    "password": "pw-bob-123",
                }))
                .send()
                .await
                .map(|r| r.status())
        })
    };

    // 等所有完成
    for h in alice_handles {
        let _ = h.await;
    }
    let bob_status = bob_handle.await.expect("bob join").expect("bob send");

    assert_eq!(
        bob_status,
        StatusCode::OK,
        "bob's login must succeed despite alice being under attack"
    );
}

/// **P0-C**：并发注册同一 login_id 只有一个成功。
///
/// 验证 SQLite 的 `INSERT ... ON CONFLICT DO NOTHING` 原子性：
/// 100 个并发 → 1 个 200 + 99 个 409（不是 2 个 200）。
///
/// **回归保护**：如果有人把 `ON CONFLICT DO NOTHING` 改回
/// `SELECT-then-INSERT`（TOCTOU），多个客户端会同时插入成功——
/// 本测试会立刻失败。
#[tokio::test]
async fn p0c_concurrent_register_only_one_succeeds() {
    let addr = spawn_server().await;
    let base = format!("http://{}", addr);

    let mut handles = Vec::new();
    for _ in 0..100 {
        let base = base.clone();
        handles.push(tokio::spawn(async move {
            reqwest::Client::new()
                .post(format!("{}/api/register", base))
                .json(&json!({
                    "login_id": "p0c-race",
                    "password": "pw-race-123",
                    "nickname": "",
                }))
                .send()
                .await
                .map(|r| r.status())
        }));
    }

    let mut ok_count = 0;
    let mut conflict_count = 0;
    for h in handles {
        match h.await.expect("join").expect("send") {
            StatusCode::OK => ok_count += 1,
            StatusCode::CONFLICT => conflict_count += 1,
            s => panic!("unexpected status: {}", s),
        }
    }

    assert_eq!(ok_count, 1, "exactly one register should succeed");
    assert_eq!(
        conflict_count, 99,
        "the other 99 should return 409 Conflict"
    );
}

/// **P0-A 性能**：100 并发错误密码应在合理时间内完成。
///
/// `#[ignore]` —— 对 CI 机器性能敏感（Argon2 速度因 CPU 而异）。
///
/// 手动跑：
/// ```text
/// cargo test -p kaede-server --test security_e2e p0a_100_concurrent_bad_logins_complete_quickly -- --ignored --nocapture
/// ```
///
/// 参考：
/// - 修复前（100 次 Argon2，8 并发）：~600-1000 ms
/// - 修复后（5 次 Argon2 + 锁拒绝）：~50-100 ms
#[tokio::test]
#[ignore]
async fn p0a_100_concurrent_bad_logins_complete_quickly() {
    let addr = spawn_server().await;
    let base = format!("http://{}", addr);

    let (_alice, _) = register(&base, "p0a-perf", "pw-perf-123").await;

    let start = Instant::now();
    let mut handles = Vec::new();
    for _ in 0..100 {
        let base = base.clone();
        handles.push(tokio::spawn(async move {
            reqwest::Client::new()
                .post(format!("{}/api/login", base))
                .json(&json!({
                    "login_id": "p0a-perf",
                    "password": "wrong",
                }))
                .send()
                .await
                .map(|r| r.status())
        }));
    }
    for h in handles {
        let _ = h.await;
    }
    let elapsed = start.elapsed();

    println!("P0-A perf: 100 concurrent bad logins took {:?}", elapsed);
    assert!(
        elapsed < Duration::from_secs(3),
        "100 concurrent bad logins took {:?}, expected < 3s (P0-A fix prevents Argon2 flood)",
        elapsed
    );
}
