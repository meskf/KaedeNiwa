//! 端到端集成测试：真 WebSocket 客户端 × 2 走完整信令流程。

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use kaede_protocol::{ClientMessage, ErrorCode, ServerMessage, UserId, WireMessage};
use kaede_server::{config::Config, db, http, state::AppState, ws};
use tokio::time::timeout;
use tokio_tungstenite::{connect_async, tungstenite::Message};

const TEST_TIMEOUT: Duration = Duration::from_secs(5);

async fn start_server() -> String {
    // 每个测试独立的临时文件 SQLite。
    // 注意：sqlx 的 `sqlite::memory:` 是 per-connection 独立内存库，
    // 用 pool 时不同连接看到不同 DB，必须用文件或 shared cache。
    // 用进程内原子计数，避免并行测试拿到相同 nanos 导致 DB 锁冲突。
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let unique = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let db_path =
        std::env::temp_dir().join(format!("kaede_test_{}_{}.db", std::process::id(), unique));
    let db_url = format!("sqlite://{}?mode=rwc", db_path.display());

    let cfg = Config {
        bind: "127.0.0.1:0".into(),
        database_url: db_url,
        session_ttl_hours: 1,
        livekit_api_key: "testkey".into(),
        livekit_api_secret: "testsecret".into(),
        livekit_ws_url: "ws://localhost:7880".into(),
    };
    let pool = db::init(&cfg.database_url).await.unwrap();
    let state = AppState::new(pool, cfg);
    let app = http::router(state.clone()).merge(ws::router(state));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    // 等待服务器就绪：尝试 TCP 连接，最多 20 次 × 20ms = 400ms
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            break;
        }
    }

    format!("ws://{}/ws", addr)
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn send(ws: &mut Ws, msg: ClientMessage) {
    let bytes = msg.encode().unwrap();
    ws.send(Message::Binary(bytes)).await.unwrap();
}

async fn recv(ws: &mut Ws) -> ServerMessage {
    let m = timeout(TEST_TIMEOUT, ws.next())
        .await
        .expect("recv timeout")
        .expect("ws closed")
        .expect("ws error");
    let bytes = match m {
        Message::Binary(b) => b.to_vec(),
        other => panic!("expected binary, got {:?}", other),
    };
    ServerMessage::decode(&bytes).expect("decode")
}

/// 等待指定 nonce 的 Pong，跳过 NotJoined 等非致命错误。
async fn recv_pong(ws: &mut Ws, nonce: u64) {
    for _ in 0..10 {
        match recv(ws).await {
            ServerMessage::Pong { nonce: n } => {
                assert_eq!(n, nonce);
                return;
            }
            ServerMessage::Error {
                code: ErrorCode::NotJoined,
                ..
            } => continue,
            other => panic!("expected Pong({}), got {:?}", nonce, other),
        }
    }
    panic!("recv_pong: no Pong({}) in 10 tries", nonce);
}

/// 一直收消息，直到匹配 `pred`（最多尝试 20 次）。
/// 用于跳过 presence 之类的无关广播。
async fn recv_until<F>(ws: &mut Ws, pred: F) -> ServerMessage
where
    F: Fn(&ServerMessage) -> bool,
{
    for _ in 0..20 {
        let msg = recv(ws).await;
        if pred(&msg) {
            return msg;
        }
    }
    panic!("recv_until: no matching message in 20 tries");
}

async fn connect_and_hello(url: &str) -> Ws {
    let (mut ws, _) = connect_async(url).await.expect("connect");
    send(
        &mut ws,
        ClientMessage::ClientHello {
            protocol_version: kaede_protocol::PROTOCOL_VERSION,
            client_name: "test".into(),
            client_version: "0.1.0".into(),
            session_token: None,
        },
    )
    .await;
    match recv(&mut ws).await {
        ServerMessage::ServerHello { .. } => ws,
        other => panic!("expected ServerHello, got {:?}", other),
    }
}

// ============================================================================
// 测试
// ============================================================================

#[tokio::test]
async fn hello_roundtrip() {
    let url = start_server().await;
    let _ws = connect_and_hello(&url).await;
}

#[tokio::test]
async fn version_mismatch_rejected() {
    let url = start_server().await;
    let (mut ws, _) = connect_async(&url).await.unwrap();

    let hello = ClientMessage::ClientHello {
        protocol_version: 1, // 太旧
        client_name: "old".into(),
        client_version: "0.0.1".into(),
        session_token: None,
    };
    send(&mut ws, hello).await;

    match recv(&mut ws).await {
        ServerMessage::Error { code, .. } => {
            assert_eq!(code, ErrorCode::UnsupportedVersion);
        }
        other => panic!("expected UnsupportedVersion, got {:?}", other),
    }
}

#[tokio::test]
async fn join_requires_session_token() {
    let url = start_server().await;
    let mut ws = connect_and_hello(&url).await;

    // 没 token 直接 join：必须收到明确的 NotJoined 错误，而不是静默。
    send(&mut ws, ClientMessage::Join).await;
    match recv(&mut ws).await {
        ServerMessage::Error {
            code: ErrorCode::NotJoined,
            ..
        } => {}
        other => panic!("expected NotJoined, got {:?}", other),
    }

    // 连接必须保持可用（不是 fatal error）。
    send(&mut ws, ClientMessage::Ping { nonce: 42 }).await;
    match recv(&mut ws).await {
        ServerMessage::Pong { nonce } => assert_eq!(nonce, 42),
        other => panic!("expected Pong, got {:?}", other),
    }
}

#[tokio::test]
async fn authenticated_join_and_app_message_broadcast() {
    let url = start_server().await;

    let http_url = url.replace("ws://", "http://").replace("/ws", "");
    let client = reqwest::Client::new();
    let alice = client
        .post(format!("{}/api/register", http_url))
        .json(&serde_json::json!({
            "login_id": "alice",
            "password": "alice1234",
            "nickname": "Alice",
        }))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    let bob = client
        .post(format!("{}/api/register", http_url))
        .json(&serde_json::json!({
            "login_id": "bob",
            "password": "bob12345",
            "nickname": "Bob",
        }))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();

    let alice_token = alice["session_token"].as_str().unwrap().to_string();
    let bob_token = bob["session_token"].as_str().unwrap().to_string();

    // Alice 连
    let (mut a_ws, _) = connect_async(&url).await.unwrap();
    send(
        &mut a_ws,
        ClientMessage::ClientHello {
            protocol_version: kaede_protocol::PROTOCOL_VERSION,
            client_name: "alice".into(),
            client_version: "0.1.0".into(),
            session_token: Some(alice_token),
        },
    )
    .await;
    assert!(matches!(
        recv(&mut a_ws).await,
        ServerMessage::ServerHello { .. }
    ));
    send(&mut a_ws, ClientMessage::Join).await;
    assert!(matches!(
        recv(&mut a_ws).await,
        ServerMessage::Joined { .. }
    ));

    // Bob 连
    let (mut b_ws, _) = connect_async(&url).await.unwrap();
    send(
        &mut b_ws,
        ClientMessage::ClientHello {
            protocol_version: kaede_protocol::PROTOCOL_VERSION,
            client_name: "bob".into(),
            client_version: "0.1.0".into(),
            session_token: Some(bob_token),
        },
    )
    .await;
    assert!(matches!(
        recv(&mut b_ws).await,
        ServerMessage::ServerHello { .. }
    ));
    send(&mut b_ws, ClientMessage::Join).await;
    assert!(matches!(
        recv(&mut b_ws).await,
        ServerMessage::Joined { .. }
    ));

    // Alice 会收到 Bob 的 PeerJoined（可能夹杂 PresenceUpdated）
    match recv_until(&mut a_ws, |m| matches!(m, ServerMessage::PeerJoined { .. })).await {
        ServerMessage::PeerJoined { uid } => assert_eq!(uid, UserId::new("bob")),
        _ => unreachable!(),
    }

    // Alice 发 MlsAppMessage
    let payload = b"ciphertext".to_vec();
    send(
        &mut a_ws,
        ClientMessage::MlsAppMessage {
            payload: payload.clone(),
        },
    )
    .await;

    // Bob 收 MlsAppMessage（可能夹杂 PresenceUpdated）
    match recv_until(&mut b_ws, |m| {
        matches!(m, ServerMessage::MlsAppMessage { .. })
    })
    .await
    {
        ServerMessage::MlsAppMessage { from, payload: got } => {
            assert_eq!(from, UserId::new("alice"));
            assert_eq!(got, payload);
        }
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn direct_message_routing() {
    let url = start_server().await;
    let http_url = url.replace("ws://", "http://").replace("/ws", "");
    let client = reqwest::Client::new();

    let alice = client
        .post(format!("{}/api/register", http_url))
        .json(&serde_json::json!({
            "login_id": "alice",
            "password": "alice1234",
            "nickname": "Alice",
        }))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    let bob = client
        .post(format!("{}/api/register", http_url))
        .json(&serde_json::json!({
            "login_id": "bob",
            "password": "bob12345",
            "nickname": "Bob",
        }))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();

    let a_tok = alice["session_token"].as_str().unwrap().to_string();
    let b_tok = bob["session_token"].as_str().unwrap().to_string();

    let (mut a_ws, _) = connect_async(&url).await.unwrap();
    send(
        &mut a_ws,
        ClientMessage::ClientHello {
            protocol_version: kaede_protocol::PROTOCOL_VERSION,
            client_name: "alice".into(),
            client_version: "0.1.0".into(),
            session_token: Some(a_tok),
        },
    )
    .await;
    let _ = recv(&mut a_ws).await;
    send(&mut a_ws, ClientMessage::Join).await;
    let _ = recv(&mut a_ws).await;

    let (mut b_ws, _) = connect_async(&url).await.unwrap();
    send(
        &mut b_ws,
        ClientMessage::ClientHello {
            protocol_version: kaede_protocol::PROTOCOL_VERSION,
            client_name: "bob".into(),
            client_version: "0.1.0".into(),
            session_token: Some(b_tok),
        },
    )
    .await;
    let _ = recv(&mut b_ws).await;
    send(&mut b_ws, ClientMessage::Join).await;
    let _ = recv(&mut b_ws).await;

    // Alice 会看到 Bob 进来
    let _ = recv(&mut a_ws).await;

    // Alice 定向给 Bob
    send(
        &mut a_ws,
        ClientMessage::MlsWelcome {
            to: UserId::new("bob"),
            payload: b"welcome".to_vec(),
        },
    )
    .await;

    match recv_until(&mut b_ws, |m| matches!(m, ServerMessage::MlsWelcome { .. })).await {
        ServerMessage::MlsWelcome { from, payload } => {
            assert_eq!(from, UserId::new("alice"));
            assert_eq!(payload, b"welcome");
        }
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn rate_limit_kicks_in() {
    let url = start_server().await;
    let mut ws = connect_and_hello(&url).await;

    // burst=60，发 100 条应该触发限流
    for i in 0..100u64 {
        send(&mut ws, ClientMessage::Ping { nonce: i }).await;
    }

    let mut got_rate_limited = false;
    for _ in 0..300 {
        match recv(&mut ws).await {
            ServerMessage::Error {
                code: ErrorCode::RateLimited,
                ..
            } => {
                got_rate_limited = true;
                break;
            }
            _ => continue,
        }
    }
    assert!(got_rate_limited, "expected RateLimited after burst");
}

// ============================================================================
// Batch 4：P0-D（ensure_joined）回归测试
// ============================================================================
//
// 修复位置：`ws.rs::handle_message` 里 SdpOffer/SdpAnswer/IceCandidate/
// MlsWelcome/MlsCommit/MlsProposal/MlsAppMessage/SetPresence 分支都改为
// `ensure_joined(state, my_uid, my_session_id)?`，未 Join 时返回 Err，
// 消息被静默丢弃（**不发**给目标 peer，也不 broadcast）。
//
// 测试策略（避免 flaky）：
//   1. 攻击方 authenticated 但未 Join，尝试发消息
//   2. 用 Ping/Pong 作为"屏障"确认 server 已处理到那条消息
//   3. 用 150ms 短超时断言接收方**没收到**任何东西
//   4. 正向控制：攻击方 Join 后再发一次，接收方能收到

/// Try to receive a message within `duration`. Returns `None` on timeout.
async fn try_recv(ws: &mut Ws, duration: Duration) -> Option<ServerMessage> {
    match timeout(duration, ws.next()).await {
        Err(_) => None,
        Ok(None) => None,
        Ok(Some(Err(e))) => panic!("ws error: {:?}", e),
        Ok(Some(Ok(Message::Binary(b)))) => Some(ServerMessage::decode(&b).expect("decode")),
        Ok(Some(Ok(other))) => panic!("expected binary, got {:?}", other),
    }
}

/// 通过 HTTP 注册用户，返回 session_token。
async fn register_via_http(http_url: &str, login_id: &str, password: &str) -> String {
    let resp = reqwest::Client::new()
        .post(format!("{}/api/register", http_url))
        .json(&serde_json::json!({
            "login_id": login_id,
            "password": password,
            "nickname": "",
        }))
        .send()
        .await
        .unwrap_or_else(|e| panic!("register {} send failed: {}", login_id, e));
    let body: serde_json::Value = resp.json().await.expect("register json");
    body["session_token"]
        .as_str()
        .expect("session_token")
        .to_string()
}

/// 连接 + 带 token 的 ClientHello，返回已握手的 ws。
async fn connect_with_token(url: &str, token: &str) -> Ws {
    let (mut ws, _) = connect_async(url).await.expect("connect");
    send(
        &mut ws,
        ClientMessage::ClientHello {
            protocol_version: kaede_protocol::PROTOCOL_VERSION,
            client_name: "test".into(),
            client_version: "0.1.0".into(),
            session_token: Some(token.to_string()),
        },
    )
    .await;
    match recv(&mut ws).await {
        ServerMessage::ServerHello { .. } => ws,
        other => panic!("expected ServerHello, got {:?}", other),
    }
}

/// 一个攻击者（auth 未 Join）+ 一个受害者（auth + Join）的会话。
///
/// 返回 `(attacker_ws, victim_ws)`。victim 的队列已被清空到 Joined 之后，
/// attacker 已 auth 但未 Join。
async fn setup_attacker_and_victim() -> (Ws, Ws) {
    let url = start_server().await;
    let http_url = url.replace("ws://", "http://").replace("/ws", "");

    let a_tok = register_via_http(&http_url, "p0d-attacker", "pw-attacker-123").await;
    let v_tok = register_via_http(&http_url, "p0d-victim", "pw-victim-123").await;

    let mut a_ws = connect_with_token(&url, &a_tok).await;
    let mut v_ws = connect_with_token(&url, &v_tok).await;

    // victim 先 Join，并清空自身队列
    send(&mut v_ws, ClientMessage::Join).await;
    match recv(&mut v_ws).await {
        ServerMessage::Joined { .. } => {}
        other => panic!("expected Joined, got {:?}", other),
    }
    // 服务器还会对 victim 自己 broadcast 一条 PresenceUpdated
    match recv(&mut v_ws).await {
        ServerMessage::PresenceUpdated { .. } => {}
        other => panic!("expected PresenceUpdated, got {:?}", other),
    }

    // attacker 不动 — 保持 auth 未 Join 状态
    let _ = &mut a_ws;
    (a_ws, v_ws)
}

/// **P0-D #1**：未 Join 发 SdpOffer → victim 收不到。
#[tokio::test]
async fn p0d_sdp_offer_without_join_dropped() {
    let (mut a_ws, mut v_ws) = setup_attacker_and_victim().await;

    // 攻击：未 Join 发 SdpOffer
    send(
        &mut a_ws,
        ClientMessage::SdpOffer {
            to: UserId::new("p0d-victim"),
            sdp: "should-be-dropped".to_string(),
        },
    )
    .await;

    // 屏障：Ping → Pong，确认 server 已处理完前面的 SdpOffer
    send(&mut a_ws, ClientMessage::Ping { nonce: 111 }).await;
    recv_pong(&mut a_ws, 111).await;

    // victim 队列应仍为空（150ms 内无任何消息）
    assert!(
        try_recv(&mut v_ws, Duration::from_millis(150))
            .await
            .is_none(),
        "P0-D violated: SdpOffer without Join leaked to victim"
    );

    // 正向控制：attacker Join 后，SdpOffer 能正常投递
    send(&mut a_ws, ClientMessage::Join).await;
    match recv(&mut a_ws).await {
        ServerMessage::Joined { .. } => {}
        other => panic!("expected Joined, got {:?}", other),
    }
    // victim 收到 PeerJoined(attacker)
    match recv_until(
        &mut v_ws,
        |m| matches!(m, ServerMessage::PeerJoined { uid } if uid.as_str() == "p0d-attacker"),
    )
    .await
    {
        ServerMessage::PeerJoined { uid } => {
            assert_eq!(uid, UserId::new("p0d-attacker"))
        }
        _ => unreachable!(),
    }

    // 再发 SdpOffer — 这次应到达
    send(
        &mut a_ws,
        ClientMessage::SdpOffer {
            to: UserId::new("p0d-victim"),
            sdp: "after-join".to_string(),
        },
    )
    .await;

    match recv_until(&mut v_ws, |m| matches!(m, ServerMessage::SdpOffer { .. })).await {
        ServerMessage::SdpOffer { from, sdp } => {
            assert_eq!(from, UserId::new("p0d-attacker"));
            assert_eq!(
                sdp, "after-join",
                "victim should receive post-join SdpOffer"
            );
        }
        _ => unreachable!(),
    }
}

/// **P0-D #2**：未 Join 发 MlsAppMessage → victim 收不到。
#[tokio::test]
async fn p0d_mls_app_message_without_join_dropped() {
    let (mut a_ws, mut v_ws) = setup_attacker_and_victim().await;

    send(
        &mut a_ws,
        ClientMessage::MlsAppMessage {
            payload: b"should-be-dropped".to_vec(),
        },
    )
    .await;

    send(&mut a_ws, ClientMessage::Ping { nonce: 222 }).await;
    recv_pong(&mut a_ws, 222).await;

    assert!(
        try_recv(&mut v_ws, Duration::from_millis(150))
            .await
            .is_none(),
        "P0-D violated: MlsAppMessage without Join leaked to victim"
    );

    // 正向控制
    send(&mut a_ws, ClientMessage::Join).await;
    match recv(&mut a_ws).await {
        ServerMessage::Joined { .. } => {}
        other => panic!("expected Joined, got {:?}", other),
    }
    match recv_until(
        &mut v_ws,
        |m| matches!(m, ServerMessage::PeerJoined { uid } if uid.as_str() == "p0d-attacker"),
    )
    .await
    {
        ServerMessage::PeerJoined { .. } => {}
        _ => unreachable!(),
    }

    send(
        &mut a_ws,
        ClientMessage::MlsAppMessage {
            payload: b"after-join".to_vec(),
        },
    )
    .await;

    match recv_until(&mut v_ws, |m| {
        matches!(m, ServerMessage::MlsAppMessage { .. })
    })
    .await
    {
        ServerMessage::MlsAppMessage { from, payload } => {
            assert_eq!(from, UserId::new("p0d-attacker"));
            assert_eq!(payload, b"after-join");
        }
        _ => unreachable!(),
    }
}

/// **P0-D #3**：未 Join 发 MlsCommit → victim 收不到。
///
/// 注意：MlsCommit 走 `broadcast_except_important`（带背压 await send），
/// 与 try_send 路径不同，但 `ensure_joined` 同样在分支开头拦截。
#[tokio::test]
async fn p0d_mls_commit_without_join_dropped() {
    let (mut a_ws, mut v_ws) = setup_attacker_and_victim().await;

    send(
        &mut a_ws,
        ClientMessage::MlsCommit {
            epoch: 1,
            payload: b"should-be-dropped".to_vec(),
        },
    )
    .await;

    send(&mut a_ws, ClientMessage::Ping { nonce: 333 }).await;
    recv_pong(&mut a_ws, 333).await;

    assert!(
        try_recv(&mut v_ws, Duration::from_millis(150))
            .await
            .is_none(),
        "P0-D violated: MlsCommit without Join leaked to victim"
    );

    // 正向控制
    send(&mut a_ws, ClientMessage::Join).await;
    match recv(&mut a_ws).await {
        ServerMessage::Joined { .. } => {}
        other => panic!("expected Joined, got {:?}", other),
    }
    match recv_until(
        &mut v_ws,
        |m| matches!(m, ServerMessage::PeerJoined { uid } if uid.as_str() == "p0d-attacker"),
    )
    .await
    {
        ServerMessage::PeerJoined { .. } => {}
        _ => unreachable!(),
    }

    send(
        &mut a_ws,
        ClientMessage::MlsCommit {
            epoch: 1,
            payload: b"after-join".to_vec(),
        },
    )
    .await;

    match recv_until(&mut v_ws, |m| matches!(m, ServerMessage::MlsCommit { .. })).await {
        ServerMessage::MlsCommit { from, payload, .. } => {
            assert_eq!(from, UserId::new("p0d-attacker"));
            assert_eq!(payload, b"after-join");
        }
        _ => unreachable!(),
    }
}

/// **P0-D #4（负向对照）**：Join 后主动 Leave → 再次发消息被拒。
///
/// 验证 `Leave` 语义：清 `my_session_id = 0`，后续 `ensure_joined` 会拒绝
/// （返回 "not joined"）。
#[tokio::test]
async fn p0d_leave_revokes_join() {
    let url = start_server().await;
    let http_url = url.replace("ws://", "http://").replace("/ws", "");

    let _a_tok = register_via_http(&http_url, "p0d-leave-attacker", "pw-123456").await;
    let v_tok = register_via_http(&http_url, "p0d-leave-victim", "pw-654321").await;

    let _a_ws = connect_with_token(&url, &_a_tok).await;
    let mut a_ws = _a_ws;
    let mut v_ws = connect_with_token(&url, &v_tok).await;

    // victim Join
    send(&mut v_ws, ClientMessage::Join).await;
    match recv(&mut v_ws).await {
        ServerMessage::Joined { .. } => {}
        _ => panic!(),
    }
    match recv(&mut v_ws).await {
        ServerMessage::PresenceUpdated { .. } => {}
        _ => panic!(),
    }

    // attacker Join
    send(&mut a_ws, ClientMessage::Join).await;
    match recv(&mut a_ws).await {
        ServerMessage::Joined { .. } => {}
        _ => panic!(),
    }
    // victim 收 PeerJoined + PresenceUpdated
    let _ = recv_until(&mut v_ws, |m| matches!(m, ServerMessage::PeerJoined { .. })).await;

    // attacker 主动 Leave
    send(&mut a_ws, ClientMessage::Leave).await;
    // 用 Ping 作为屏障
    send(&mut a_ws, ClientMessage::Ping { nonce: 444 }).await;

    // attacker 可能收到 PeerLeft/PresenceUpdated（来自自己 Leave）—— 跳过直到 Pong
    let _ = recv_until(&mut a_ws, |m| {
        matches!(m, ServerMessage::Pong { nonce: 444 })
    })
    .await;
    // 清空 victim 的 PeerLeft/PresenceUpdated
    let _ = recv_until(&mut v_ws, |m| {
        matches!(m, ServerMessage::PresenceUpdated { .. })
    })
    .await;

    // 现在 attacker 处于 Leave 后状态 — 再发 SdpOffer 应该被静默丢弃
    send(
        &mut a_ws,
        ClientMessage::SdpOffer {
            to: UserId::new("p0d-leave-victim"),
            sdp: "after-leave".to_string(),
        },
    )
    .await;
    send(&mut a_ws, ClientMessage::Ping { nonce: 555 }).await;
    let _ = recv_until(&mut a_ws, |m| {
        matches!(m, ServerMessage::Pong { nonce: 555 })
    })
    .await;

    // 只断言"没有 SdpOffer"——队列里可能还有 PeerLeft / PresenceUpdated 等
    // 正常广播（来自 attacker 的 Leave），那些不影响 P0-D 的安全性质。
    //
    // 用 250ms 窗口 + 忽略非 SdpOffer 消息：
    let deadline = tokio::time::Instant::now() + Duration::from_millis(250);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break; // 250ms 内没收到 SdpOffer → 通过
        }
        match try_recv(&mut v_ws, remaining).await {
            None => break, // 队列空 → 通过
            Some(ServerMessage::SdpOffer { .. }) => {
                panic!("P0-D violated: SdpOffer after Leave leaked to victim")
            }
            Some(_) => continue, // PeerLeft / PresenceUpdated / 其他 → 忽略
        }
    }
}
