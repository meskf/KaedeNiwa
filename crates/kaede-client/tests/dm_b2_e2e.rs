//! DM B2 端到端测试（E-1 修复验证）。
//!
//! 覆盖：
//! - 独立 2 人 MLS group，首条 DM 触发 Welcome 交换
//! - 群成员（Carol）无法获得 DM group，无法解密
//! - Welcome 持久化于 messages 表，通过 dm_load_history 补收

#![cfg(feature = "native")]
#![allow(clippy::await_holding_lock)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use kaede_client::{ClientEvent, ClientObserver, RoomSession};
use kaede_protocol::UserId;
use tokio::time::sleep;

#[derive(Default)]
struct Collector {
    events: Mutex<Vec<ClientEvent>>,
}

impl ClientObserver for Collector {
    fn on_event(&self, event: ClientEvent) {
        self.events.lock().unwrap().push(event);
    }
}

impl Collector {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn dm_texts(&self) -> Vec<String> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                ClientEvent::DmReceived { plaintext, .. } => {
                    match kaede_client::MessagePayload::parse(plaintext) {
                        Some(p) => Some(p.t),
                        None => Some(String::from_utf8_lossy(plaintext).to_string()),
                    }
                }
                _ => None,
            })
            .collect()
    }
}

struct Trio {
    alice: RoomSession,
    bob: RoomSession,
    carol: RoomSession,
    alice_obs: Arc<Collector>,
    bob_obs: Arc<Collector>,
}

async fn setup_trio() -> Trio {
    let addr = kaede_server::spawn_test_server().await.unwrap();
    let http = format!("http://{}", addr);
    let ws = format!("ws://{}/ws", addr);

    let alice_obs = Collector::new();
    let bob_obs = Collector::new();

    let alice = RoomSession::connect(&http, &ws, "alice", "alice1234", Some(alice_obs.clone()))
        .await
        .expect("alice connect");
    let bob = RoomSession::connect(&http, &ws, "bob", "bob12345", Some(bob_obs.clone()))
        .await
        .expect("bob connect");
    let carol = RoomSession::connect(&http, &ws, "carol", "carol1234", None)
        .await
        .expect("carol connect");

    alice.spawn_event_loop();
    bob.spawn_event_loop();
    carol.spawn_event_loop();

    alice.become_host().await.expect("alice host");
    alice.join_room().await.expect("alice join");

    bob.become_guest().await.expect("bob guest");
    bob.join_room().await.expect("bob join");
    carol.become_guest().await.expect("carol guest");
    carol.join_room().await.expect("carol join");

    sleep(Duration::from_millis(300)).await;

    alice
        .host_add_peer(&UserId::from("bob"))
        .await
        .expect("alice add bob");
    sleep(Duration::from_millis(200)).await;
    alice
        .host_add_peer(&UserId::from("carol"))
        .await
        .expect("alice add carol");

    for _ in 0..120 {
        if alice.is_mls_ready()
            && bob.is_mls_ready()
            && carol.is_mls_ready()
            && alice.member_count() == 3
        {
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(alice.member_count(), 3, "main group size");

    Trio {
        alice,
        bob,
        carol,
        alice_obs,
        bob_obs,
    }
}

#[tokio::test]
async fn dm_b2_full_flow() {
    let t = setup_trio().await;

    // 1. Alice 打开与 Bob 的 DM
    let conv = t.alice.open_dm("bob").await.expect("alice open dm");
    let conv_id = conv.conv_id.clone();
    println!("[test] conv_id: {}", conv_id);
    assert!(conv_id.starts_with("dm:"));
    assert!(conv_id.contains("alice") && conv_id.contains("bob"));

    // 2. Alice 发首条消息（触发 Welcome 交换 + 文本消息）
    t.alice
        .send_dm(&conv_id, "hello bob (dm b2)")
        .await
        .expect("alice send dm");

    // Alice 本地应有 dm group
    assert!(
        t.alice.connection().dm_has_group(&conv_id),
        "alice should have dm group after first send"
    );

    // 3. 等 Bob 从 WS 收到 Welcome 并处理，然后收到文本消息
    for _ in 0..120 {
        if t.bob.connection().dm_has_group(&conv_id) {
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    assert!(
        t.bob.connection().dm_has_group(&conv_id),
        "bob should have dm group after receiving welcome"
    );

    // 4. Bob 收到 DmReceived 事件
    let mut saw_alice_text = false;
    for _ in 0..120 {
        if t.bob_obs
            .dm_texts()
            .iter()
            .any(|s| s == "hello bob (dm b2)")
        {
            saw_alice_text = true;
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    assert!(
        saw_alice_text,
        "bob should decrypt alice's dm; got: {:?}",
        t.bob_obs.dm_texts()
    );
    println!("[test] bob decrypted alice's dm");

    // 5. Bob 反向发消息
    t.bob
        .send_dm(&conv_id, "hi alice (dm b2 reply)")
        .await
        .expect("bob send dm reply");

    let mut saw_bob_reply = false;
    for _ in 0..120 {
        if t.alice_obs
            .dm_texts()
            .iter()
            .any(|s| s == "hi alice (dm b2 reply)")
        {
            saw_bob_reply = true;
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    assert!(
        saw_bob_reply,
        "alice should decrypt bob's reply; got: {:?}",
        t.alice_obs.dm_texts()
    );
    println!("[test] alice decrypted bob's reply");

    // 6. Bob 从历史读取（含 Welcome 持久化的兜底验证）
    let history = t
        .bob
        .dm_load_history(&conv_id, None, 20)
        .await
        .expect("bob dm history");
    let bob_texts: Vec<String> = history.iter().map(|h| h.plaintext.clone()).collect();
    assert!(
        bob_texts.iter().any(|s| s == "hello bob (dm b2)"),
        "bob's history should include alice's text; got: {:?}",
        bob_texts
    );
    println!("[test] bob dm_load_history works");
}

#[tokio::test]
async fn dm_b2_carol_cryptographic_isolation() {
    let t = setup_trio().await;

    // Alice↔Bob 建立 DM
    let conv = t.alice.open_dm("bob").await.expect("open dm");
    let conv_id = conv.conv_id.clone();

    t.alice
        .send_dm(&conv_id, "top secret from alice")
        .await
        .expect("send dm");
    sleep(Duration::from_millis(300)).await;

    // 关键断言 1：Carol 本地没有 dm group
    assert!(
        !t.carol.connection().dm_has_group(&conv_id),
        "carol must NOT have dm group (cryptographic isolation)"
    );

    // 关键断言 2：Carol 尝试读历史 → HTTP 401（服务端准入）
    let r = t.carol.dm_load_history(&conv_id, None, 10).await;
    assert!(
        r.is_err(),
        "carol should not read dm history; got {:?}",
        r.map(|v| v.len())
    );
    println!("[test] carol dm_load_history rejected");

    // 关键断言 3：Carol 尝试发消息 → 服务端拒绝
    let r = t.carol.send_dm(&conv_id, "carol was here").await;
    assert!(
        r.is_err(),
        "carol send_dm should fail (server rejects non-member)"
    );
    println!("[test] carol send_dm rejected");

    // 关键断言 4：Alice 和 Bob 都有 group，Carol 没有（隔离核心证据）
    assert!(t.alice.connection().dm_has_group(&conv_id));
    assert!(t.bob.connection().dm_has_group(&conv_id));
    assert!(!t.carol.connection().dm_has_group(&conv_id));
    println!("[test] isolation verified: alice/bob have group, carol does not");
}

// `dm_b2_bob_offline_welcome_via_history` 已移除（理由保留）。
//
// 原因：MLS Welcome 是一次性的（openmls 消费后删除 KeyPackageBundle 私钥），
// 重放同一个 Welcome 会被协议层拒绝（NoMatchingKeyPackage）。
//
// 真实场景下 Bob 重启后恢复 DM 会话的正确做法是**把 group state 持久化到 storage**
// （Signal / Wire 的做法），而不是重放 Welcome。B3+ 已实现，见下方
// `dm_b2_persists_snapshot_to_storage`。

// ============================================================================
// B3+：DM group 持久化 e2e 测试
// ============================================================================

/// B3+：`send_dm` / 收到 Welcome 之后，两侧 storage 都应出现 `dm/{server_url}/{conv_id}`。
///
/// 这是后续「Bob 重启能恢复 DM group」的必要前提。恢复路径由 `RoomSession::
/// connect_with_storage` 在连接时自动执行（见 `load_aux_snapshots_from_storage`），
/// 本测试只验证「写盘」这一半，读取路径由持久化 hook 的单元测试覆盖。
#[tokio::test]
async fn dm_b2_persists_snapshot_to_storage() {
    let addr = kaede_server::spawn_test_server().await.unwrap();
    let http = format!("http://{}", addr);
    let ws = format!("ws://{}/ws", addr);

    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let alice_db = std::env::temp_dir()
        .join(format!("kaede_dm_alice_{}.db", unique))
        .to_string_lossy()
        .to_string();
    let bob_db = std::env::temp_dir()
        .join(format!("kaede_dm_bob_{}.db", unique))
        .to_string_lossy()
        .to_string();

    let alice_store = kaede_client::open_default_store(&alice_db)
        .await
        .expect("alice store");
    let bob_store = kaede_client::open_default_store(&bob_db)
        .await
        .expect("bob store");

    let alice = RoomSession::connect_with_storage(
        &http,
        &ws,
        "alice",
        "alice1234",
        Some(alice_store.clone()),
        None,
    )
    .await
    .expect("alice connect");
    let bob = RoomSession::connect_with_storage(
        &http,
        &ws,
        "bob",
        "bob12345",
        Some(bob_store.clone()),
        None,
    )
    .await
    .expect("bob connect");

    alice.spawn_event_loop();
    bob.spawn_event_loop();

    alice.become_host().await.expect("alice host");
    alice.join_room().await.expect("alice join");
    bob.become_guest().await.expect("bob guest");
    bob.join_room().await.expect("bob join");
    sleep(Duration::from_millis(300)).await;
    alice
        .host_add_peer(&UserId::from("bob"))
        .await
        .expect("alice add bob");
    for _ in 0..120 {
        if alice.is_mls_ready() && bob.is_mls_ready() && alice.member_count() == 2 {
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(alice.member_count(), 2);

    // 建立 DM（Alice 首条消息 → Welcome → Bob 加入 → dm_join_group）
    let conv = alice.open_dm("bob").await.expect("alice open_dm");
    let conv_id = conv.conv_id.clone();
    alice
        .send_dm(&conv_id, "hello bob (persist test)")
        .await
        .expect("alice send_dm");

    // 等 Bob 收到 Welcome 并处理
    for _ in 0..120 {
        if bob.connection().dm_has_group(&conv_id) {
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    assert!(
        bob.connection().dm_has_group(&conv_id),
        "bob should have dm group after welcome"
    );

    // 等 dirty flush 落地（event loop 每条消息后 flush 一次；dm_join_group 后也应如此）
    sleep(Duration::from_millis(600)).await;

    let alice_keys = alice_store.list_keys("dm").await.expect("list alice dm");
    let bob_keys = bob_store.list_keys("dm").await.expect("list bob dm");

    assert!(
        alice_keys.iter().any(|k| k.ends_with(&conv_id)),
        "alice storage missing dm snapshot for {}: {:?}",
        conv_id,
        alice_keys,
    );
    assert!(
        bob_keys.iter().any(|k| k.ends_with(&conv_id)),
        "bob storage missing dm snapshot for {}: {:?}",
        conv_id,
        bob_keys,
    );

    println!("[test] alice dm keys: {:?}", alice_keys);
    println!("[test] bob   dm keys: {:?}", bob_keys);

    drop(alice);
    drop(bob);
    drop(alice_store);
    drop(bob_store);
    let _ = std::fs::remove_file(&alice_db);
    let _ = std::fs::remove_file(&bob_db);
}

#[tokio::test]
async fn dm_b2_media_isolation() {
    let t = setup_trio().await;

    // Alice 打开与 Bob 的 DM 并发送首条消息（触发 Welcome 交换）
    let conv = t.alice.open_dm("bob").await.expect("open dm");
    let conv_id = conv.conv_id.clone();
    t.alice
        .send_dm(&conv_id, "trigger dm group")
        .await
        .expect("send dm");
    sleep(Duration::from_millis(500)).await;

    // 双方都应有 dm group
    assert!(
        t.alice.connection().dm_has_group(&conv_id),
        "alice should have dm group"
    );
    assert!(
        t.bob.connection().dm_has_group(&conv_id),
        "bob should have dm group"
    );

    // Alice 拿到 DM 媒体 token + key
    let (alice_token, alice_key) = t
        .alice
        .begin_media_session(&conv_id)
        .await
        .expect("alice begin dm media");
    println!("[test] alice dm room: {}", alice_token.room);
    assert!(
        alice_token.room.starts_with("kaede-dm:"),
        "DM room name should be kaede-dm:..., got {}",
        alice_token.room
    );

    // Bob 拿到 DM 媒体 token + key，room 相同
    let (bob_token, bob_key) = t
        .bob
        .begin_media_session(&conv_id)
        .await
        .expect("bob begin dm media");
    assert_eq!(
        alice_token.room, bob_token.room,
        "alice and bob should share the same DM room"
    );

    // Alice 和 Bob 的 dm group epoch 相同 → key 相同
    assert_eq!(
        alice_key, bob_key,
        "DM peers should derive identical media key"
    );
    println!("[test] alice+bob derived identical DM media key");

    // Carol 不在 DM 中 → 服务端 401
    let r = t.carol.begin_media_session(&conv_id).await;
    assert!(
        r.is_err(),
        "carol should not get DM media token; got {:?}",
        r.map(|(t, _)| t.room)
    );
    println!("[test] carol rejected from DM media");
}

// ============================================================================
// B3+：DM 持久化 —— 重启恢复 e2e
// ============================================================================

/// B3+：Bob 断开 + 用同一个 storage 重连后，应自动从 snapshot 恢复 dm group，
/// 且能解密 Alice 在重启后新发的消息（不依赖重放 Welcome）。
///
/// 这验证了持久化的**读取侧**：`RoomSession::connect_with_storage` →
/// `load_aux_snapshots_from_storage` → `dm_restore_from_snapshot_with_cipher`。
#[tokio::test]
async fn dm_b2_bob_restart_recovers_from_snapshot() {
    let addr = kaede_server::spawn_test_server().await.unwrap();
    let http = format!("http://{}", addr);
    let ws = format!("ws://{}/ws", addr);

    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let alice_db = std::env::temp_dir()
        .join(format!("kaede_dm_alice_restart_{}.db", unique))
        .to_string_lossy()
        .to_string();
    let bob_db = std::env::temp_dir()
        .join(format!("kaede_dm_bob_restart_{}.db", unique))
        .to_string_lossy()
        .to_string();

    let alice_store = kaede_client::open_default_store(&alice_db)
        .await
        .expect("alice store");
    let bob_store = kaede_client::open_default_store(&bob_db)
        .await
        .expect("bob store");

    let alice = RoomSession::connect_with_storage(
        &http,
        &ws,
        "alice",
        "alice1234",
        Some(alice_store.clone()),
        None,
    )
    .await
    .expect("alice connect");
    let bob = RoomSession::connect_with_storage(
        &http,
        &ws,
        "bob",
        "bob12345",
        Some(bob_store.clone()),
        None,
    )
    .await
    .expect("bob connect");

    alice.spawn_event_loop();
    bob.spawn_event_loop();

    alice.become_host().await.expect("alice host");
    alice.join_room().await.expect("alice join");
    bob.become_guest().await.expect("bob guest");
    bob.join_room().await.expect("bob join");
    sleep(Duration::from_millis(300)).await;
    alice
        .host_add_peer(&UserId::from("bob"))
        .await
        .expect("alice add bob");
    for _ in 0..120 {
        if alice.is_mls_ready() && bob.is_mls_ready() && alice.member_count() == 2 {
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(alice.member_count(), 2);

    // 建立 DM
    let conv = alice.open_dm("bob").await.expect("alice open_dm");
    let conv_id = conv.conv_id.clone();
    alice
        .send_dm(&conv_id, "hello bob (restart test)")
        .await
        .expect("alice send_dm");
    for _ in 0..120 {
        if bob.connection().dm_has_group(&conv_id) {
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    assert!(
        bob.connection().dm_has_group(&conv_id),
        "bob should have dm group after welcome"
    );

    // 等 snapshot 写盘
    sleep(Duration::from_millis(600)).await;
    let bob_keys = bob_store.list_keys("dm").await.expect("list bob dm");
    assert!(
        bob_keys.iter().any(|k| k.ends_with(&conv_id)),
        "bob storage missing dm snapshot: {:?}",
        bob_keys,
    );

    // === 模拟 Bob 重启 ===
    bob.connection().close().await.ok();
    drop(bob);
    // 等旧 event loop 退出 + 服务端清理 peer
    sleep(Duration::from_millis(300)).await;

    // 用同一个 db 重连
    let bob_restored = RoomSession::connect_with_storage(
        &http,
        &ws,
        "bob",
        "bob12345",
        Some(bob_store.clone()),
        None,
    )
    .await
    .expect("bob restore connect");
    bob_restored.spawn_event_loop();

    // 核心断言：dm group 从 snapshot 恢复
    assert!(
        bob_restored.connection().dm_has_group(&conv_id),
        "bob should recover dm group from storage after restart"
    );

    // Alice 发新消息 → Bob 恢复后能解密
    alice
        .send_dm(&conv_id, "after bob restart")
        .await
        .expect("alice send after restart");
    sleep(Duration::from_millis(500)).await;

    let history = bob_restored
        .dm_load_history(&conv_id, None, 20)
        .await
        .expect("bob load history after restart");
    let texts: Vec<String> = history.iter().map(|h| h.plaintext.clone()).collect();
    assert!(
        texts.iter().any(|s| s == "after bob restart"),
        "bob should decrypt post-restart message; got: {:?}",
        texts
    );
    assert!(
        texts.iter().any(|s| s == "hello bob (restart test)"),
        "bob should recover pre-restart message from history; got: {:?}",
        texts
    );

    println!("[test] restart recovery ok; history: {:?}", texts);

    drop(bob_restored);
    drop(alice);
    drop(alice_store);
    drop(bob_store);
    let _ = std::fs::remove_file(&alice_db);
    let _ = std::fs::remove_file(&bob_db);
}
