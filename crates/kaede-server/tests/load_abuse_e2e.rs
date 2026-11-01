//! Beta 负载 / 滥用回归（默认 `#[ignore]`，手动或夜间流水线执行）。
//!
//! 为什么忽略：单个用例会注册多个账号（Argon2）并搬运几十 MB 数据，
//! 不适合每个 PR 都跑。发布前执行：
//!
//! ```powershell
//! cargo test -p kaede-server --test load_abuse_e2e -- --ignored --test-threads=1
//! ```
//!
//! 覆盖：
//! 1. 扇出：12 人在线，1 人发 1 MiB 消息，其余 11 人全部收到；
//! 2. 顶替风暴：同一账号 9 条连接并发 Join，最终只保留 1 个在线 peer；
//! 3. 4 MiB 洪泛：连续 5 个 4 MiB 消息必须触发字节预算拒绝，且连接不被断开；
//! 4. 超大帧：超过协议上限的帧必须被服务端关闭连接。

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use kaede_protocol::{ClientMessage, ErrorCode, PROTOCOL_VERSION, ServerMessage, WireMessage};
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn spawn() -> String {
    let addr = kaede_server::spawn_test_server()
        .await
        .expect("spawn_test_server");
    format!("http://{addr}")
}

async fn register(base: &str, login_id: &str) -> String {
    let client = reqwest::Client::new();
    for attempt in 0..15 {
        let resp = client
            .post(format!("{base}/api/register"))
            .json(&serde_json::json!({
                "login_id": login_id,
                "password": "loadtest-123",
                "nickname": "",
            }))
            .send()
            .await
            .expect("register send");

        if resp.status().is_success() {
            let body: serde_json::Value = resp.json().await.expect("register json");
            return body["session_token"].as_str().unwrap().to_string();
        }

        // 如果遇到限流 (400 Bad Request)，等待 500ms 后重试
        if resp.status() == reqwest::StatusCode::BAD_REQUEST && attempt < 14 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            continue;
        }

        panic!("register {login_id}: {}", resp.status());
    }
    panic!("register {login_id}: too many retries");
}

fn ws_url(base: &str) -> String {
    base.replacen("http://", "ws://", 1) + "/ws"
}

async fn connect(base: &str, token: &str) -> Ws {
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url(base))
        .await
        .expect("ws connect");
    send(
        &mut ws,
        &ClientMessage::ClientHello {
            protocol_version: PROTOCOL_VERSION,
            client_name: "loadtest".into(),
            client_version: "0.0.1".into(),
            session_token: Some(token.to_string()),
        },
    )
    .await;
    match recv(&mut ws, Duration::from_secs(10)).await {
        ServerMessage::ServerHello { .. } => {}
        other => panic!("expected ServerHello, got {other:?}"),
    }
    ws
}

async fn join(ws: &mut Ws) {
    send(ws, &ClientMessage::Join).await;
    loop {
        match recv(ws, Duration::from_secs(10)).await {
            ServerMessage::Joined { .. } => return,
            ServerMessage::PresenceUpdated { .. } | ServerMessage::PeerJoined { .. } => continue,
            other => panic!("expected Joined, got {other:?}"),
        }
    }
}

async fn send(ws: &mut Ws, msg: &ClientMessage) {
    let bytes = msg.encode().expect("encode");
    ws.send(tokio_tungstenite::tungstenite::Message::Binary(bytes))
        .await
        .expect("ws send");
}

async fn recv(ws: &mut Ws, timeout: Duration) -> ServerMessage {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let msg = tokio::time::timeout(remaining, ws.next())
            .await
            .expect("recv timeout")
            .expect("ws closed")
            .expect("ws error");
        match msg {
            tokio_tungstenite::tungstenite::Message::Binary(bytes) => {
                return ServerMessage::decode(&bytes).expect("decode server message");
            }
            tokio_tungstenite::tungstenite::Message::Ping(_)
            | tokio_tungstenite::tungstenite::Message::Pong(_) => continue,
            other => panic!("unexpected ws frame: {other:?}"),
        }
    }
}

async fn metric_u64(base: &str, name: &str) -> u64 {
    let body = reqwest::get(format!("{base}/metrics"))
        .await
        .expect("metrics")
        .text()
        .await
        .expect("metrics body");
    body.lines()
        .find_map(|line| {
            line.strip_prefix(name)?
                .strip_prefix(' ')?
                .trim()
                .parse()
                .ok()
        })
        .unwrap_or_else(|| panic!("metric {name} missing"))
}

/// 1 MiB × 1 发送者 → 11 个在线接收者全部收到。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "load test: run before release"]
async fn fanout_one_mib_reaches_every_peer() {
    let base = spawn().await;
    const N: usize = 12;

    let mut clients = Vec::with_capacity(N);
    for i in 0..N {
        let token = register(&base, &format!("fanout-{i}")).await;
        clients.push(connect(&base, &token).await);
    }
    for ws in &mut clients {
        join(ws).await;
    }

    let payload = vec![0xABu8; 1024 * 1024];
    send(
        &mut clients[0],
        &ClientMessage::MlsAppMessage {
            channel_id: "general".into(),
            payload: payload.clone(),
            id: 0,
            ts: 0,
        },
    )
    .await;

    for (i, ws) in clients.iter_mut().enumerate().skip(1) {
        loop {
            match recv(ws, Duration::from_secs(15)).await {
                ServerMessage::MlsAppMessage { payload: got, .. } => {
                    assert_eq!(got.len(), payload.len(), "peer {i} got truncated payload");
                    break;
                }
                ServerMessage::PresenceUpdated { .. } | ServerMessage::PeerJoined { .. } => {
                    continue;
                }
                other => panic!("peer {i} unexpected message: {other:?}"),
            }
        }
    }

    // 发送者本人不回显（客户端本地回显）。
    assert_eq!(metric_u64(&base, "kaede_online_peers").await, N as u64);
}

/// 同一账号 9 条并发连接：服务端必须只保留最后一个 peer，旧连接被踢。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "load test: run before release"]
async fn reconnect_storm_keeps_single_peer() {
    let base = spawn().await;
    let token = register(&base, "storm-user").await;

    let mut first = connect(&base, &token).await;
    join(&mut first).await;

    let mut last: Option<Ws> = None;
    for _ in 0..8 {
        let mut ws = connect(&base, &token).await;
        join(&mut ws).await;
        last = Some(ws);
    }
    let mut last = last.expect("at least one reconnect");

    // 旧连接必须被显式踢掉（Kicked）或关闭，不能继续存活。
    let kicked = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(frame) = first.next().await {
            match frame {
                Ok(tokio_tungstenite::tungstenite::Message::Binary(bytes)) => {
                    if let Ok(ServerMessage::Error { code, .. }) = ServerMessage::decode(&bytes)
                        && matches!(code, ErrorCode::Kicked)
                    {
                        return true;
                    }
                }
                Ok(_) => continue,
                Err(_) => return true,
            }
        }
        true
    })
    .await;
    assert!(
        kicked.is_ok_and(|v| v),
        "oldest connection was not kicked or closed"
    );

    // 最终在线 peer 只能有 1 个。
    let mut peers = 0;
    for _ in 0..40 {
        peers = metric_u64(&base, "kaede_online_peers").await;
        if peers == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(peers, 1, "reconnect storm left multiple peers");

    // 最新连接仍然可用。
    send(&mut last, &ClientMessage::Ping { nonce: 7 }).await;
    loop {
        match recv(&mut last, Duration::from_secs(5)).await {
            ServerMessage::Pong { nonce } => {
                assert_eq!(nonce, 7);
                break;
            }
            _ => continue,
        }
    }
}

/// 连续 5 个 4 MiB 消息：超出 16 MiB 突发预算后必须返回 RateLimited，
/// 且连接不能被断开（后续 Ping 仍可用）。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "load test: run before release"]
async fn four_mib_flood_hits_byte_budget_without_disconnect() {
    let base = spawn().await;
    let token = register(&base, "flood-user").await;
    let mut ws = connect(&base, &token).await;
    join(&mut ws).await;

    let payload = vec![0xCDu8; 4 * 1024 * 1024];
    for _ in 0..8 {
        send(
            &mut ws,
            &ClientMessage::MlsAppMessage {
                channel_id: "general".into(),
                payload: payload.clone(),
                id: 0,
                ts: 0,
            },
        )
        .await;
    }

    let mut limited = false;
    for _ in 0..20 {
        match recv(&mut ws, Duration::from_secs(10)).await {
            ServerMessage::Error {
                code: ErrorCode::RateLimited,
                ..
            } => {
                limited = true;
                break;
            }
            ServerMessage::PresenceUpdated { .. } | ServerMessage::PeerJoined { .. } => continue,
            _ => continue,
        }
    }
    assert!(limited, "byte budget never rejected after 32 MiB burst");

    // 连接仍然可用。
    send(&mut ws, &ClientMessage::Ping { nonce: 99 }).await;
    loop {
        match recv(&mut ws, Duration::from_secs(5)).await {
            ServerMessage::Pong { nonce } => {
                assert_eq!(nonce, 99);
                break;
            }
            _ => continue,
        }
    }
}

/// 超过 `MAX_MESSAGE_BYTES` 的帧必须被服务端拒绝（连接关闭），
/// 不能把 17 MiB 读进内存后继续服务。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "load test: run before release"]
async fn oversized_frame_closes_connection() {
    let base = spawn().await;
    let token = register(&base, "oversize-user").await;
    let mut ws = connect(&base, &token).await;
    join(&mut ws).await;

    let oversized = vec![0u8; kaede_protocol::MAX_MESSAGE_BYTES + 1024];
    let _ = ws
        .send(tokio_tungstenite::tungstenite::Message::Binary(oversized))
        .await;

    let closed = tokio::time::timeout(Duration::from_secs(15), async {
        while let Some(frame) = ws.next().await {
            if frame.is_err() {
                return true;
            }
        }
        true
    })
    .await;
    assert!(
        closed.is_ok(),
        "oversized frame did not close the connection"
    );
}
