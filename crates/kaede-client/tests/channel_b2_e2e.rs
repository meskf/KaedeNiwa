//! B2 私密频道端到端测试。
//!
//! 覆盖：
//! - 完整密码 + MLS Welcome 交换流程
//! - 加密隔离：非成员即使拿到 ciphertext 也解不开
//! - 错密码拒绝
//! - 非成员读历史被服务端拒绝

#![cfg(feature = "native")]
#![allow(clippy::await_holding_lock)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use kaede_client::{ClientEvent, ClientObserver, RoomSession};
use kaede_protocol::UserId;
use tokio::time::{sleep, timeout};

const TEST_TIMEOUT: Duration = Duration::from_secs(15);
const CHANNEL_PASSWORD: &str = "b2secret42";

#[derive(Default)]
struct Collector {
    events: Mutex<Vec<ClientEvent>>,
}

impl ClientObserver for Collector {
    fn on_event(&self, event: ClientEvent) {
        self.events.lock().unwrap_or_else(|e| e.into_inner()).push(event);
    }
}

impl Collector {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

struct Trio {
    alice: RoomSession,
    bob: RoomSession,
    carol: RoomSession,
    _alice_obs: Arc<Collector>,
    _bob_obs: Arc<Collector>,
    _carol_obs: Arc<Collector>,
}

async fn setup_trio() -> Trio {
    let addr = kaede_server::spawn_test_server().await.unwrap();
    let http = format!("http://{}", addr);
    let ws = format!("ws://{}/ws", addr);

    let alice_obs = Collector::new();
    let bob_obs = Collector::new();
    let carol_obs = Collector::new();

    let alice = RoomSession::connect(&http, &ws, "alice", "alice1234", Some(alice_obs.clone()))
        .await
        .expect("alice connect");
    let bob = RoomSession::connect(&http, &ws, "bob", "bob12345", Some(bob_obs.clone()))
        .await
        .expect("bob connect");
    let carol = RoomSession::connect(&http, &ws, "carol", "carol1234", Some(carol_obs.clone()))
        .await
        .expect("carol connect");

    alice.spawn_event_loop();
    bob.spawn_event_loop();
    carol.spawn_event_loop();

    // Alice 建主群
    alice.become_host().await.expect("alice host");
    alice.join_room().await.expect("alice join");

    // Bob / Carol 作为 guest 加入主群
    bob.become_guest().await.expect("bob guest");
    bob.join_room().await.expect("bob join");
    carol.become_guest().await.expect("carol guest");
    carol.join_room().await.expect("carol join");

    // 等 PeerJoined 到达
    sleep(Duration::from_millis(300)).await;

    // Alice 依次把 Bob / Carol 加进主群（各自触发 epoch 前进 + Welcome 交换）
    alice
        .host_add_peer(&UserId::from("bob"))
        .await
        .expect("alice add bob");
    sleep(Duration::from_millis(200)).await;
    alice
        .host_add_peer(&UserId::from("carol"))
        .await
        .expect("alice add carol");

    // 等所有成员都 MlsReady 且成员数一致
    for _ in 0..120 {
        if alice.is_mls_ready()
            && bob.is_mls_ready()
            && carol.is_mls_ready()
            && alice.member_count() == 3
            && bob.member_count() == 3
            && carol.member_count() == 3
        {
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(alice.member_count(), 3, "alice main group size");
    assert_eq!(bob.member_count(), 3, "bob main group size");
    assert_eq!(carol.member_count(), 3, "carol main group size");

    Trio {
        alice,
        bob,
        carol,
        _alice_obs: alice_obs,
        _bob_obs: bob_obs,
        _carol_obs: carol_obs,
    }
}

#[tokio::test]
async fn b2_private_channel_full_flow() {
    let t = setup_trio().await;

    // === 1. Alice 创建私密频道 ===
    let ch = t
        .alice
        .create_channel_with("secret-room", "text", true, Some(CHANNEL_PASSWORD), None)
        .await
        .expect("create private channel");
    assert!(ch.is_private, "channel should be private");
    let ch_id = ch.id.clone();
    println!("[test] private channel: {}", ch_id);

    // Alice 是创建者，必须有本地 channel group
    assert!(
        t.alice.connection().channel_has_group(&ch_id),
        "alice should have channel group after create"
    );

    // === 2. Bob 用密码加入（阻塞等 Welcome 交换完成） ===
    timeout(TEST_TIMEOUT, t.bob.join_channel(&ch_id, CHANNEL_PASSWORD))
        .await
        .expect("bob join_channel timeout")
        .expect("bob join_channel failed");

    assert!(
        t.bob.connection().channel_has_group(&ch_id),
        "bob should have channel group after join"
    );
    println!("[test] bob joined channel via Welcome");

    // === 3. Alice 发消息 → Bob 读历史能解密 ===
    t.alice
        .send_text_to(&ch_id, "hello bob from private channel")
        .await
        .expect("alice send to channel");

    sleep(Duration::from_millis(300)).await;

    let bob_history = t
        .bob
        .load_history(&ch_id, 10)
        .await
        .expect("bob load history");
    let bob_texts: Vec<String> = bob_history.iter().map(|h| h.plaintext.clone()).collect();
    assert!(
        bob_texts
            .iter()
            .any(|s| s == "hello bob from private channel"),
        "bob should decrypt; got: {:?}",
        bob_texts
    );
    println!("[test] bob decrypted alice's message");

    // === 4. Bob 发消息 → Alice 读历史能解密 ===
    t.bob
        .send_text_to(&ch_id, "hi alice from bob")
        .await
        .expect("bob send to channel");

    sleep(Duration::from_millis(300)).await;

    let alice_history = t
        .alice
        .load_history(&ch_id, 10)
        .await
        .expect("alice load history");
    let alice_texts: Vec<String> = alice_history.iter().map(|h| h.plaintext.clone()).collect();
    assert!(
        alice_texts.iter().any(|s| s == "hi alice from bob"),
        "alice should decrypt; got: {:?}",
        alice_texts
    );
    println!("[test] alice decrypted bob's message");
}

#[tokio::test]
async fn b2_wrong_password_rejected() {
    let t = setup_trio().await;

    let ch = t
        .alice
        .create_channel_with("secret", "text", true, Some(CHANNEL_PASSWORD), None)
        .await
        .expect("create");
    let ch_id = ch.id.clone();

    let r = timeout(Duration::from_secs(3), t.bob.join_channel(&ch_id, "WRONG")).await;

    match r {
        Ok(Err(_)) => println!("[test] bob rejected with wrong password (expected)"),
        Ok(Ok(())) => panic!("bob joined with wrong password — server permission bug!"),
        Err(_) => panic!("bob join_channel hung on wrong password (should error fast)"),
    }

    // Bob 没加入，不能有 channel group
    assert!(
        !t.bob.connection().channel_has_group(&ch_id),
        "bob should not have channel group after rejected join"
    );
}

#[tokio::test]
async fn b2_carol_cryptographic_isolation() {
    let t = setup_trio().await;

    let ch = t
        .alice
        .create_channel_with("secret", "text", true, Some(CHANNEL_PASSWORD), None)
        .await
        .expect("create");
    let ch_id = ch.id.clone();

    // Alice 发消息
    t.alice
        .send_text_to(&ch_id, "top secret message")
        .await
        .expect("send");
    sleep(Duration::from_millis(200)).await;

    // 关键断言 1：Carol 不在频道内 → 读历史被服务端拒绝（401）
    let r = t.carol.load_history(&ch_id, 10).await;
    assert!(
        r.is_err(),
        "carol should not read channel history; got: {:?}",
        r.map(|v| v.len())
    );

    // 关键断言 2（密码学隔离）：Carol 本地根本没有这个 channel 的 MLS group
    // 即使 DB dump 泄露、ciphertext 拿到手，她也没有 channel_export_key 能解
    assert!(
        !t.carol.connection().channel_has_group(&ch_id),
        "carol must NOT have channel group (cryptographic isolation)"
    );
    assert!(
        t.alice.connection().channel_has_group(&ch_id),
        "alice (creator) should have channel group"
    );

    // 关键断言 3：Carol 错密码 join 也被拒
    let r = timeout(
        Duration::from_secs(3),
        t.carol.join_channel(&ch_id, "WRONG"),
    )
    .await;
    match r {
        Ok(Err(_)) => {}
        Ok(Ok(())) => panic!("carol joined with wrong password"),
        Err(_) => panic!("carol join hung"),
    }

    // 加入后 Carol 才能拿到 channel group
    // （前提：有成员在线时才能加；这是 B2 设计限制，这里跳过此步骤）
}

#[tokio::test]
async fn b2_private_channel_media_isolation() {
    let t = setup_trio().await;

    let ch = t
        .alice
        .create_channel_with("media-room", "text", true, Some(CHANNEL_PASSWORD), None)
        .await
        .expect("create private channel");
    let ch_id = ch.id.clone();

    // Bob 加入频道（channel_members + Welcome 交换）
    timeout(TEST_TIMEOUT, t.bob.join_channel(&ch_id, CHANNEL_PASSWORD))
        .await
        .expect("bob join timeout")
        .expect("bob join failed");

    // Alice（创建者）能拿到 LiveKit token + key
    let (alice_token, alice_key) = t
        .alice
        .begin_media_session(&ch_id)
        .await
        .expect("alice begin_media_session");
    println!("[test] alice token room: {}", alice_token.room);

    // Bob（成员）也能拿到 token + key，且 room 相同
    let (bob_token, bob_key) = t
        .bob
        .begin_media_session(&ch_id)
        .await
        .expect("bob begin_media_session");
    assert_eq!(
        alice_token.room, bob_token.room,
        "alice and bob should be in the same room"
    );

    // Alice 和 Bob 都在同一个 channel group（同 epoch），key 应相同
    assert_eq!(
        alice_key, bob_key,
        "channel group members should derive identical media key"
    );
    println!("[test] alice+bob derived identical media key");

    // Carol（未加入）被服务端拒绝（HTTP 401）
    let r = t.carol.begin_media_session(&ch_id).await;
    assert!(
        r.is_err(),
        "carol should not get private channel media token; got {:?}",
        r.map(|(t, _)| t.room)
    );
    println!("[test] carol rejected from private channel media");
}
