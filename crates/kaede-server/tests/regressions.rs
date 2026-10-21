//! 回归测试：连接接管、离线 Welcome 补投。

use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_util::{SinkExt, StreamExt};
use kaede_protocol::{ClientMessage, ErrorCode, ServerMessage, UserId, WireMessage};
use kaede_server::{channels, config::Config, db, http, members, state::AppState, ws};
use sqlx::SqlitePool;
use tokio::time::timeout;
use tokio_tungstenite::{connect_async, tungstenite::Message};

const T: Duration = Duration::from_secs(5);

async fn start_server() -> (String, SqlitePool) {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let unique = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let db_path =
        std::env::temp_dir().join(format!("kaede_reg_{}_{}.db", std::process::id(), unique));
    let db_url = format!("sqlite://{}?mode=rwc", db_path.display());
    let cfg = Config {
        bind: "127.0.0.1:0".into(),
        database_url: db_url,
        session_ttl_hours: 1,
        livekit_api_key: "k".into(),
        livekit_api_secret: "s".into(),
        livekit_ws_url: "ws://localhost:7880".into(),
    };
    let pool = db::init(&cfg.database_url).await.unwrap();
    let state = AppState::new(pool.clone(), cfg);
    let app = http::router(state.clone())
        .merge(kaede_server::friends::router(state.clone()))
        .merge(kaede_server::dm::router(state.clone()))
        .merge(channels::router(state.clone()))
        .merge(members::router(state.clone()))
        .merge(kaede_server::reads::router(state.clone()))
        .merge(kaede_server::sessions_api::router(state.clone()))
        .merge(ws::router(state));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await;
    });
    (format!("ws://{}/ws", addr), pool)
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn send(ws: &mut Ws, msg: ClientMessage) {
    ws.send(Message::Binary(msg.encode().unwrap()))
        .await
        .unwrap();
}

async fn recv(ws: &mut Ws) -> ServerMessage {
    let m = timeout(T, ws.next())
        .await
        .expect("recv timeout")
        .expect("ws closed")
        .expect("ws error");
    match m {
        Message::Binary(b) => ServerMessage::decode(&b).expect("decode"),
        other => panic!("expected binary, got {:?}", other),
    }
}

async fn recv_until<F>(ws: &mut Ws, pred: F) -> ServerMessage
where
    F: Fn(&ServerMessage) -> bool,
{
    for _ in 0..30 {
        let m = recv(ws).await;
        if pred(&m) {
            return m;
        }
    }
    panic!("recv_until: no match");
}

async fn register(base: &str, login_id: &str) -> String {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/register", base))
        .json(&serde_json::json!({
            "login_id": login_id,
            "password": "password1234",
            "nickname": login_id,
        }))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "register {} failed", login_id);
    let v: serde_json::Value = resp.json().await.unwrap();
    v["session_token"].as_str().unwrap().to_string()
}

async fn connect_auth(url: &str, token: &str) -> Ws {
    let (mut ws, _) = connect_async(url).await.unwrap();
    send(
        &mut ws,
        ClientMessage::ClientHello {
            protocol_version: kaede_protocol::PROTOCOL_VERSION,
            client_name: "test".into(),
            client_version: "0".into(),
            session_token: Some(token.to_string()),
        },
    )
    .await;
    assert!(matches!(
        recv(&mut ws).await,
        ServerMessage::ServerHello { .. }
    ));
    ws
}

async fn join(ws: &mut Ws) {
    send(ws, ClientMessage::Join).await;
    recv_until(ws, |m| matches!(m, ServerMessage::Joined { .. })).await;
}

#[tokio::test]
async fn dm_conversation_index_and_monotonic_ts() {
    let (url, _pool) = start_server().await;
    let base = url.replace("ws://", "http://").replace("/ws", "");
    let alice = register(&base, "alice").await;
    let bob = register(&base, "bob").await;
    let client = reqwest::Client::new();

    // 打开 DM 会话（服务端只校验双方存在）
    let resp = client
        .post(format!("{}/api/dm/bob", base))
        .bearer_auth(&alice)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body = resp.text().await.unwrap();
    assert!(status.is_success(), "open_dm {}: {}", status, body);
    let conv: serde_json::Value = serde_json::from_str(&body).unwrap();
    let conv_id = conv["conv_id"].as_str().unwrap().to_string();

    // 连发 3 条：ts 必须严格递增（否则分页会丢消息）
    for i in 0..3 {
        let resp = client
            .post(format!("{}/api/dm/{}/messages", base, conv_id))
            .bearer_auth(&alice)
            .json(&serde_json::json!({
                "ts": 0,
                "sender": "alice",
                "ciphertext": "AQID",
                "_i": i,
            }))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success());
    }

    let msgs: serde_json::Value = client
        .get(format!("{}/api/dm/{}/messages?limit=10", base, conv_id))
        .bearer_auth(&bob)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ts: Vec<i64> = msgs["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["ts"].as_i64().unwrap())
        .collect();
    assert_eq!(ts.len(), 3);
    assert!(
        ts.windows(2).all(|w| w[0] < w[1]),
        "ts must be strictly increasing: {:?}",
        ts
    );

    // dm_conversations 索引表应让双方都能列出该会话
    let convs: serde_json::Value = client
        .get(format!("{}/api/dm", base))
        .bearer_auth(&bob)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        convs["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["conv_id"].as_str() == Some(conv_id.as_str())),
        "dm_conversations index missing: {:?}",
        convs
    );
}

#[tokio::test]
async fn same_account_second_connection_takes_over() {
    let (url, _pool) = start_server().await;
    let base = url.replace("ws://", "http://").replace("/ws", "");
    let token = register(&base, "alice").await;

    let mut a = connect_auth(&url, &token).await;
    join(&mut a).await;

    let mut b = connect_auth(&url, &token).await;
    join(&mut b).await;

    // 旧连接必须收到 Kicked。
    recv_until(&mut a, |m| {
        matches!(
            m,
            ServerMessage::Error {
                code: ErrorCode::Kicked,
                ..
            }
        )
    })
    .await;

    // 新连接必须真正成为 peers 里的 handle：给自己发一条 MlsWelcome，
    // 服务器会转发到 alice 的 handle（也就是 B 自己）。
    send(
        &mut b,
        ClientMessage::MlsWelcome {
            to: UserId::new("alice"),
            payload: b"v=0".to_vec(),
        },
    )
    .await;
    let got = recv_until(&mut b, |m| matches!(m, ServerMessage::MlsWelcome { .. })).await;
    match got {
        ServerMessage::MlsWelcome { from, .. } => assert_eq!(from.as_str(), "alice"),
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn missed_commits_are_replayed_on_epoch_report() {
    let (url, _pool) = start_server().await;
    let base = url.replace("ws://", "http://").replace("/ws", "");
    let alice = register(&base, "alice").await;
    let bob = register(&base, "bob").await;

    let mut a = connect_auth(&url, &alice).await;
    join(&mut a).await;

    // bob 离线时发生 epoch 5 的 commit。
    send(
        &mut a,
        ClientMessage::MlsCommit {
            epoch: 5,
            payload: b"commit-five".to_vec(),
        },
    )
    .await;
    send(&mut a, ClientMessage::Ping { nonce: 9 }).await;
    recv_until(&mut a, |m| matches!(m, ServerMessage::Pong { nonce: 9 })).await;

    // bob 上线，上报自己还在 epoch 2 → 服务器补投 epoch 5 的 commit。
    let mut b = connect_auth(&url, &bob).await;
    join(&mut b).await;
    send(&mut b, ClientMessage::ReportEpoch { epoch: 2 }).await;

    let got = recv_until(&mut b, |m| matches!(m, ServerMessage::MlsCommit { .. })).await;
    match got {
        ServerMessage::MlsCommit { epoch, payload, .. } => {
            assert_eq!(epoch, 5);
            assert_eq!(payload, b"commit-five");
        }
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn offline_welcome_is_delivered_on_join() {
    let (url, _pool) = start_server().await;
    let base = url.replace("ws://", "http://").replace("/ws", "");
    let alice = register(&base, "alice").await;
    let bob = register(&base, "bob").await;

    let mut a = connect_auth(&url, &alice).await;
    join(&mut a).await;

    // bob 离线，Welcome 应被服务器暂存。
    send(
        &mut a,
        ClientMessage::MlsWelcome {
            to: UserId::new("bob"),
            payload: b"welcome-bytes".to_vec(),
        },
    )
    .await;
    // 屏障：确认服务器已经处理完上面的 Welcome（写入 pending 表）。
    send(&mut a, ClientMessage::Ping { nonce: 7 }).await;
    recv_until(&mut a, |m| matches!(m, ServerMessage::Pong { nonce: 7 })).await;

    let mut b = connect_auth(&url, &bob).await;
    join(&mut b).await;

    let got = recv_until(&mut b, |m| matches!(m, ServerMessage::MlsWelcome { .. })).await;
    match got {
        ServerMessage::MlsWelcome { from, payload } => {
            assert_eq!(from.as_str(), "alice");
            assert_eq!(payload, b"welcome-bytes");
        }
        _ => unreachable!(),
    }
}

// ============================================================================
// 第二轮审计回归
// ============================================================================

async fn create_private_channel(base: &str, token: &str, name: &str, password: &str) -> String {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/channels", base))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "name": name,
            "kind": "text",
            "is_private": true,
            "password": password,
            "hash_tier": "19",
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let v: serde_json::Value = resp.json().await.unwrap();
    assert!(
        status.is_success(),
        "create channel failed: {} {:?}",
        status,
        v
    );
    v["id"].as_str().unwrap().to_string()
}

async fn join_private(
    base: &str,
    token: &str,
    channel_id: &str,
    password: &str,
    key_package: Option<&str>,
) {
    let client = reqwest::Client::new();
    let body = match key_package {
        Some(kp) => serde_json::json!({ "password": password, "key_package": kp }),
        None => serde_json::json!({ "password": password }),
    };
    let resp = client
        .post(format!("{}/api/channels/{}/join", base, channel_id))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let text = resp.text().await.unwrap();
    assert!(
        status.is_success(),
        "join private failed: {} {}",
        status,
        text
    );
}

/// R2-01：私密频道消息必须实时推给其他成员。
#[tokio::test]
async fn private_channel_message_is_broadcast_realtime() {
    let (url, _pool) = start_server().await;
    let base = url.replace("ws://", "http://").replace("/ws", "");
    let alice = register(&base, "alice").await;
    let bob = register(&base, "bob").await;

    let ch = create_private_channel(&base, &alice, "secret", "pass1234").await;
    join_private(&base, &bob, &ch, "pass1234", None).await;

    let mut a = connect_auth(&url, &alice).await;
    join(&mut a).await;
    let mut b = connect_auth(&url, &bob).await;
    join(&mut b).await;

    let ct = URL_SAFE_NO_PAD.encode([0x03u8, 1, 2, 3, 4]);
    let resp = reqwest::Client::new()
        .post(format!("{}/api/messages", base))
        .bearer_auth(&alice)
        .json(&serde_json::json!({
            "channel_id": ch,
            "ts": 0,
            "sender": "alice",
            "ciphertext": ct,
        }))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "post message failed");

    let got = recv_until(&mut b, |m| {
        matches!(m, ServerMessage::ChannelMessage { .. })
    })
    .await;
    match got {
        ServerMessage::ChannelMessage {
            channel_id,
            from,
            ts,
            ciphertext,
        } => {
            assert_eq!(channel_id, ch);
            assert_eq!(from.as_str(), "alice");
            assert!(ts > 0, "server ts must be authoritative");
            assert_eq!(ciphertext, URL_SAFE_NO_PAD.decode(ct).unwrap());
        }
        _ => unreachable!(),
    }
}

/// R2-09：权威成员离线时，加入请求必须暂存并在其上线后补投。
#[tokio::test]
async fn offline_authority_receives_join_request_on_join() {
    let (url, _pool) = start_server().await;
    let base = url.replace("ws://", "http://").replace("/ws", "");
    let alice = register(&base, "alice").await;
    let bob = register(&base, "bob").await;

    let ch = create_private_channel(&base, &alice, "offline-auth", "pass1234").await;

    // Bob 提交合法 KeyPackage；alice（权威）此刻离线。
    let provider = kaede_mls::KaedeMlsProvider::new();
    let identity = kaede_mls::MlsIdentity::generate("bob").unwrap();
    let kp = identity.generate_key_package(&provider).unwrap();
    let kp_b64 = URL_SAFE_NO_PAD.encode(kp);
    join_private(&base, &bob, &ch, "pass1234", Some(&kp_b64)).await;

    // Alice 上线 Join：应收到暂存的 ChannelMemberJoined。
    let mut a = connect_auth(&url, &alice).await;
    join(&mut a).await;
    let got = recv_until(&mut a, |m| {
        matches!(m, ServerMessage::ChannelMemberJoined { .. })
    })
    .await;
    match got {
        ServerMessage::ChannelMemberJoined {
            channel_id,
            requester,
            key_package,
        } => {
            assert_eq!(channel_id, ch);
            assert_eq!(requester.as_str(), "bob");
            assert_eq!(key_package, kp_b64);
        }
        _ => unreachable!(),
    }
}

/// R2-08：删除成员必须清理 channel_members / dm_conversations。
#[tokio::test]
async fn removing_member_cleans_orphan_rows() {
    let (url, pool) = start_server().await;
    let base = url.replace("ws://", "http://").replace("/ws", "");
    let alice = register(&base, "alice").await;
    let bob = register(&base, "bob").await;

    let ch = create_private_channel(&base, &alice, "cleanup", "pass1234").await;
    join_private(&base, &bob, &ch, "pass1234", None).await;

    // 建立 DM 索引（先打开会话，再发一条消息）。
    let client = reqwest::Client::new();
    let conv: serde_json::Value = client
        .post(format!("{}/api/dm/bob", base))
        .bearer_auth(&alice)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let conv_id = conv["conv_id"].as_str().unwrap().to_string();
    let resp = client
        .post(format!("{}/api/dm/{}/messages", base, conv_id))
        .bearer_auth(&alice)
        .json(&serde_json::json!({
            "ts": 0,
            "sender": "alice",
            "ciphertext": "AQID",
        }))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());

    let resp = client
        .delete(format!("{}/api/members/bob", base))
        .bearer_auth(&alice)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::NO_CONTENT);

    let (channel_members,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM channel_members WHERE login_id = 'bob'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(channel_members, 0, "channel_members orphan rows");

    let (dm_rows,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM dm_conversations WHERE a = 'bob' OR b = 'bob'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(dm_rows, 0, "dm_conversations orphan rows");
}

/// v5：好友请求 / 接受 / 删除必须实时推送给双方。
#[tokio::test]
async fn friend_changes_push_realtime() {
    let (url, _pool) = start_server().await;
    let base = url.replace("ws://", "http://").replace("/ws", "");
    let alice = register(&base, "alice").await;
    let bob = register(&base, "bob").await;

    let mut a = connect_auth(&url, &alice).await;
    join(&mut a).await;
    let mut b = connect_auth(&url, &bob).await;
    join(&mut b).await;

    let client = reqwest::Client::new();

    // 请求
    let resp = client
        .post(format!("{}/api/friends/bob", base))
        .bearer_auth(&alice)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::CREATED);
    recv_until(&mut a, |m| matches!(m, ServerMessage::FriendsChanged)).await;
    recv_until(&mut b, |m| matches!(m, ServerMessage::FriendsChanged)).await;

    // 接受
    let resp = client
        .post(format!("{}/api/friends/alice/accept", base))
        .bearer_auth(&bob)
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
    recv_until(&mut a, |m| matches!(m, ServerMessage::FriendsChanged)).await;
    recv_until(&mut b, |m| matches!(m, ServerMessage::FriendsChanged)).await;

    // 删除
    let resp = client
        .delete(format!("{}/api/friends/bob", base))
        .bearer_auth(&alice)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::NO_CONTENT);
    recv_until(&mut a, |m| matches!(m, ServerMessage::FriendsChanged)).await;
    recv_until(&mut b, |m| matches!(m, ServerMessage::FriendsChanged)).await;
}

/// 频道成员列表：公开可看、私密仅成员可看。
#[tokio::test]
async fn channel_members_visibility() {
    let (url, _pool) = start_server().await;
    let base = url.replace("ws://", "http://").replace("/ws", "");
    let alice = register(&base, "alice").await;
    let bob = register(&base, "bob").await;
    let carol = register(&base, "carol").await;
    let client = reqwest::Client::new();

    // 公开频道：任意登录用户可看（创建者自动在 channel_members 中）。
    let public: serde_json::Value = client
        .post(format!("{}/api/channels", base))
        .bearer_auth(&alice)
        .json(&serde_json::json!({ "name": "public", "kind": "text" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let public_id = public["id"].as_str().unwrap().to_string();

    let resp = client
        .get(format!("{}/api/channels/{}/members", base, public_id))
        .bearer_auth(&carol)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["members"][0]["login_id"], "alice");

    // 私密频道：非成员 403，成员 200。
    let ch = create_private_channel(&base, &alice, "members", "pass1234").await;

    let resp = client
        .get(format!("{}/api/channels/{}/members", base, ch))
        .bearer_auth(&bob)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::FORBIDDEN);

    join_private(&base, &bob, &ch, "pass1234", None).await;
    let resp = client
        .get(format!("{}/api/channels/{}/members", base, ch))
        .bearer_auth(&bob)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    let ids: Vec<&str> = body["members"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m["login_id"].as_str())
        .collect();
    assert!(ids.contains(&"alice"));
    assert!(ids.contains(&"bob"));

    // 不存在的频道 → 404。
    let resp = client
        .get(format!("{}/api/channels/nope/members", base))
        .bearer_auth(&alice)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);
}

/// 未读计数与已读位置：频道 + DM。
#[tokio::test]
async fn unread_counts_track_reads() {
    let (url, _pool) = start_server().await;
    let base = url.replace("ws://", "http://").replace("/ws", "");
    let alice = register(&base, "alice").await;
    let bob = register(&base, "bob").await;
    let client = reqwest::Client::new();

    // 公开频道 + alice 发一条消息。
    let ch: serde_json::Value = client
        .post(format!("{}/api/channels", base))
        .bearer_auth(&alice)
        .json(&serde_json::json!({ "name": "unread", "kind": "text" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ch_id = ch["id"].as_str().unwrap().to_string();

    let resp = client
        .post(format!("{}/api/messages", base))
        .bearer_auth(&alice)
        .json(&serde_json::json!({
            "channel_id": ch_id,
            "ts": 0,
            "sender": "alice",
            "ciphertext": "AQID",
        }))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());

    // bob 有 1 条未读；alice 自己的消息不计。
    let unread: serde_json::Value = client
        .get(format!("{}/api/unread", base))
        .bearer_auth(&bob)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let bob_ch = unread["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["channel_id"] == ch_id.as_str())
        .cloned()
        .expect("bob should have unread channel");
    assert_eq!(bob_ch["count"], 1);

    let alice_unread: serde_json::Value = client
        .get(format!("{}/api/unread", base))
        .bearer_auth(&alice)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        alice_unread["channels"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["channel_id"] != ch_id.as_str()),
        "own message must not be unread"
    );

    // 标记已读后清零。
    let resp = client
        .post(format!("{}/api/reads", base))
        .bearer_auth(&bob)
        .json(&serde_json::json!({ "channel_id": ch_id, "ts": i64::MAX }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::NO_CONTENT);

    let unread: serde_json::Value = client
        .get(format!("{}/api/unread", base))
        .bearer_auth(&bob)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        unread["channels"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["channel_id"] != ch_id.as_str())
    );

    // DM 未读。
    let conv: serde_json::Value = client
        .post(format!("{}/api/dm/bob", base))
        .bearer_auth(&alice)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let conv_id = conv["conv_id"].as_str().unwrap().to_string();

    let resp = client
        .post(format!("{}/api/dm/{}/messages", base, conv_id))
        .bearer_auth(&alice)
        .json(&serde_json::json!({
            "ts": 0,
            "sender": "alice",
            "ciphertext": "AQID",
        }))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());

    let unread: serde_json::Value = client
        .get(format!("{}/api/unread", base))
        .bearer_auth(&bob)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let dm = unread["dms"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["conv_id"] == conv_id.as_str())
        .cloned()
        .expect("bob should have unread dm");
    assert_eq!(dm["count"], 1);

    let resp = client
        .post(format!("{}/api/reads", base))
        .bearer_auth(&bob)
        .json(&serde_json::json!({ "conv_id": conv_id, "ts": i64::MAX }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::NO_CONTENT);

    let unread: serde_json::Value = client
        .get(format!("{}/api/unread", base))
        .bearer_auth(&bob)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        unread["dms"]
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d["conv_id"] != conv_id.as_str())
    );
}

/// 登录设备：列表、撤销其他会话、撤销当前会话。
#[tokio::test]
async fn sessions_list_and_revoke() {
    let (url, _pool) = start_server().await;
    let base = url.replace("ws://", "http://").replace("/ws", "");
    let client = reqwest::Client::new();

    let first = register(&base, "alice").await;

    let login = |device: &'static str| {
        let client = client.clone();
        let base = base.clone();
        async move {
            let v: serde_json::Value = client
                .post(format!("{}/api/login", base))
                .json(&serde_json::json!({
                    "login_id": "alice",
                    "password": "password1234",
                    "device_name": device,
                }))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            v["session_token"].as_str().unwrap().to_string()
        }
    };
    let second = login("second-device").await;

    let sessions: serde_json::Value = client
        .get(format!("{}/api/sessions", base))
        .bearer_auth(&second)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let list = sessions["sessions"].as_array().unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list.iter().filter(|s| s["current"] == true).count(), 1);
    assert!(
        list.iter()
            .any(|s| s["device_name"] == "second-device" && s["current"] == true)
    );
    let current_id = list.iter().find(|s| s["current"] == true).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // 撤销其他设备 → 第一个 token 失效。
    let resp = client
        .post(format!("{}/api/sessions/revoke-others", base))
        .bearer_auth(&second)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::NO_CONTENT);

    let resp = client
        .get(format!("{}/api/me", base))
        .bearer_auth(&first)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    // 撤销当前会话 → 该 token 立即失效。
    let resp = client
        .delete(format!("{}/api/sessions/{}", base, current_id))
        .bearer_auth(&second)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::NO_CONTENT);

    let resp = client
        .get(format!("{}/api/me", base))
        .bearer_auth(&second)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
}

/// op5 移除私密频道成员：目标收到 ChannelRemoved，权威收到 ChannelMemberLeft。
#[tokio::test]
async fn remove_channel_member_notifies() {
    let (url, _pool) = start_server().await;
    let base = url.replace("ws://", "http://").replace("/ws", "");
    let alice = register(&base, "alice").await;
    let bob = register(&base, "bob").await;
    let charlie = register(&base, "charlie").await;

    let ch = create_private_channel(&base, &alice, "kick", "pass1234").await;
    join_private(&base, &bob, &ch, "pass1234", None).await;

    let mut a = connect_auth(&url, &alice).await;
    join(&mut a).await;
    let mut b = connect_auth(&url, &bob).await;
    join(&mut b).await;

    // 非 op5 不能移除别人。
    let resp = reqwest::Client::new()
        .delete(format!("{}/api/channels/{}/members/alice", base, ch))
        .bearer_auth(&charlie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::FORBIDDEN);

    let resp = reqwest::Client::new()
        .delete(format!("{}/api/channels/{}/members/bob", base, ch))
        .bearer_auth(&alice)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::NO_CONTENT);

    recv_until(&mut b, |m| {
        matches!(m, ServerMessage::ChannelRemoved { .. })
    })
    .await;
    let evt = recv_until(&mut a, |m| {
        matches!(m, ServerMessage::ChannelMemberLeft { .. })
    })
    .await;
    match evt {
        ServerMessage::ChannelMemberLeft { channel_id, member } => {
            assert_eq!(channel_id, ch);
            assert_eq!(member.as_str(), "bob");
        }
        _ => unreachable!(),
    }

    // bob 的频道列表不再包含该私密频道。
    let chans: serde_json::Value = reqwest::Client::new()
        .get(format!("{}/api/channels", base))
        .bearer_auth(&bob)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        chans["channels"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["id"] != ch.as_str())
    );
}
