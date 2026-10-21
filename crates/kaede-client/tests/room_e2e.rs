//! 端到端集成测试：真 server + 2 个 `RoomSession` 走完 MLS 建组 + 加密聊天。
//!
//! 需要 `--features native`。

#![cfg(feature = "native")]
#![allow(clippy::await_holding_lock)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use kaede_client::{ClientEvent, ClientObserver, RoomSession};
use kaede_protocol::UserId;
use tokio::time::{sleep, timeout};

const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// 事件收集器。
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

    fn count<F: Fn(&ClientEvent) -> bool>(&self, f: F) -> usize {
        self.events.lock().unwrap_or_else(|e| e.into_inner()).iter().filter(|e| f(e)).count()
    }

    /// 等待某事件出现（最多等 3 秒）。
    async fn wait_for<F: Fn(&ClientEvent) -> bool>(&self, f: F) -> bool {
        for _ in 0..60 {
            if self.count(&f) > 0 {
                return true;
            }
            sleep(Duration::from_millis(50)).await;
        }
        false
    }
}

async fn setup_two_users() -> (RoomSession, RoomSession, Arc<Collector>, Arc<Collector>) {
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

    alice.spawn_event_loop();
    bob.spawn_event_loop();

    (alice, bob, alice_obs, bob_obs)
}

#[tokio::test]
async fn full_mls_chat_flow() {
    let (alice, bob, alice_obs, bob_obs) = setup_two_users().await;

    // --- Alice 建组 ---
    alice.become_host().await.expect("alice become host");
    alice.join_room().await.expect("alice join");

    // --- Bob 上传 KP + join ---
    bob.become_guest().await.expect("bob become guest");
    bob.join_room().await.expect("bob join");

    // --- 等 Alice 收到 PeerJoined ---
    let got_peer = timeout(
        TEST_TIMEOUT,
        alice_obs.wait_for(|e| matches!(e, ClientEvent::PeerJoined { .. })),
    )
    .await
    .expect("timeout waiting for PeerJoined");
    assert!(got_peer, "alice did not see PeerJoined");

    // --- Alice add Bob ---
    // 直接调 host_add_peer，不等 PeerJoined 事件里的具体 uid
    alice
        .host_add_peer(&UserId::from("bob"))
        .await
        .expect("alice add bob");

    // --- 等 Bob 收到 Welcome 并进入 MlsReady ---
    for _ in 0..60 {
        if bob.is_mls_ready() && bob.member_count() == 2 {
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    assert!(bob.is_mls_ready(), "bob did not become MlsReady");
    assert_eq!(bob.member_count(), 2, "bob's group should have 2 members");
    assert_eq!(
        alice.member_count(),
        2,
        "alice's group should have 2 members"
    );

    // --- epoch 必须一致 ---
    assert_eq!(alice.epoch(), bob.epoch(), "epochs must match");

    // --- Alice → Bob 加密消息 ---
    alice.send_text("hello bob").await.expect("alice send");

    let got_app = timeout(
        TEST_TIMEOUT,
        bob_obs.wait_for(|e| matches!(e, ClientEvent::AppMessage { .. })),
    )
    .await
    .expect("timeout waiting for AppMessage");
    assert!(got_app, "bob did not receive AppMessage");

    // 检查内容：现在的实现把文本包成 MessagePayload JSON，
    // 因此优先按 MessagePayload 解析，拿 t 字段。
    let events = bob_obs.events.lock().unwrap_or_else(|e| e.into_inner());
    let texts: Vec<String> = events
        .iter()
        .filter_map(|e| match e {
            ClientEvent::AppMessage { plaintext, .. } => {
                match kaede_client::MessagePayload::parse(plaintext) {
                    Some(p) => Some(p.t),
                    None => Some(String::from_utf8_lossy(plaintext).to_string()),
                }
            }
            _ => None,
        })
        .collect();
    assert!(
        texts.iter().any(|t| t == "hello bob"),
        "expected 'hello bob', got {:?}",
        texts
    );

    // --- Bob → Alice 加密消息 ---
    drop(events);
    bob.send_text("hi alice").await.expect("bob send");

    let got_reply = timeout(
        TEST_TIMEOUT,
        alice_obs.wait_for(|e| matches!(e, ClientEvent::AppMessage { .. })),
    )
    .await
    .expect("timeout waiting for reply");
    assert!(got_reply, "alice did not receive reply");
}

#[tokio::test]
async fn sframe_key_agrees_between_members() {
    let (alice, bob, _, _) = setup_two_users().await;

    alice.become_host().await.unwrap();
    alice.join_room().await.unwrap();
    bob.become_guest().await.unwrap();
    bob.join_room().await.unwrap();

    sleep(Duration::from_millis(200)).await;
    alice.host_add_peer(&UserId::from("bob")).await.unwrap();

    for _ in 0..60 {
        if bob.is_mls_ready() && bob.member_count() == 2 {
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }

    let a_key = alice.export_sframe_key("ch-general").unwrap();
    let b_key = bob.export_sframe_key("ch-general").unwrap();
    assert_eq!(a_key, b_key, "SFrame keys must agree");

    let a_voice = alice.export_sframe_key("ch-voice").unwrap();
    assert_ne!(a_key, a_voice);
}

#[tokio::test]
#[ignore = "requires LIVEKIT_API_KEY/SECRET; validates the full begin_media_session() \
            HTTP+MLS path end-to-end. Core P0-Z invariant (epoch rotation => key change) \
            is covered by begin_media_session_key_rotates_per_epoch which does NOT need LiveKit."]
async fn begin_media_session_returns_token_and_rotating_key() {
    let (alice, _bob, _, _) = setup_two_users().await;
    alice.become_host().await.unwrap();
    alice.join_room().await.unwrap();

    // P0-α 修复后，连续两次 begin_media_session 各自 mls_self_update → epoch 前进两次
    // → 即使 seed 相同，key 也必然不同（P0-Z：无 nonce 复用）
    let (token1, key1) = alice.begin_media_session("voice").await.unwrap();
    let (_token2, key2) = alice.begin_media_session("voice").await.unwrap();

    assert_ne!(
        key1, key2,
        "successive begin_media_session must rotate key (P0-Z; same seed is OK because epoch advances)"
    );
    assert_eq!(token1.room, "kaede-voice");
}
// ============================================================================
// R2c: P0-Z 回归守卫
// ============================================================================
// 验证 `ServerConnection::mls_self_update` 确实前进 epoch。
// 这是 P0-Z（媒体 E2EE key 每次会话轮换）的必要条件：
//   - 若 epoch 不前进，key 不变 → LiveKit SDK ratchet 重启时 nonce 复用
//   - 若 epoch 前进，exporter secret 改变 → export_sframe_key_for_with_extra
//     派生的 key 改变（MLS 标准保证）→ 无 nonce 复用
//
// 不依赖 LIVEKIT_API_KEY/SECRET，直接测 MLS 层。
#[tokio::test]
async fn mls_self_update_advances_epoch() {
    let (alice, _bob, _, _) = setup_two_users().await;
    alice.become_host().await.unwrap();
    alice.join_room().await.unwrap();

    let epoch0 = alice.epoch();
    let commit = alice.connection().mls_self_update().unwrap();
    alice
        .connection()
        .send_commit(alice.epoch(), commit)
        .await
        .unwrap();
    let epoch1 = alice.epoch();

    assert!(
        epoch1 > epoch0,
        "mls_self_update must advance epoch (P0-Z guard): {} -> {}",
        epoch0,
        epoch1
    );
}
// ============================================================================
// R5+: P0-Z 核心假设 — 同 seed + 不同 epoch => 不同 key
// ============================================================================
// 原 begin_media_session_returns_token_and_stable_key 需要 LIVEKIT_API_KEY/SECRET，
// 无法在 CI 跑。本测试覆盖不依赖 LiveKit 的核心假设：
//   1. mls_self_update 前进 epoch
//   2. 相同 seed + 不同 epoch => 不同 key
// 注意：`begin_media_session` 现在不再每次进频道都 self_update（否则多人 key
// 不一致）；跨会话 key 轮换由 server seed 生命周期保证。
#[tokio::test]
async fn begin_media_session_key_rotates_per_epoch() {
    let (alice, _bob, _, _) = setup_two_users().await;
    alice.become_host().await.unwrap();
    alice.join_room().await.unwrap();

    let seed = [0x42u8; 32];

    // Session 1：epoch0 下派生 key
    let epoch1 = alice.epoch();
    let key1 = alice
        .connection()
        .mls_export_sframe_key_for_with_extra("voice", kaede_mls::SframePurpose::LivekitE2ee, &seed)
        .unwrap();

    // 手动推进 epoch，验证同 seed / 不同 epoch 会派生出不同 key
    let commit = alice.connection().mls_self_update().unwrap();
    alice
        .connection()
        .send_commit(alice.epoch(), commit)
        .await
        .unwrap();

    // Session 2：epoch1 下派生 key（同 seed）
    let epoch2 = alice.epoch();
    let key2 = alice
        .connection()
        .mls_export_sframe_key_for_with_extra("voice", kaede_mls::SframePurpose::LivekitE2ee, &seed)
        .unwrap();

    assert!(
        epoch2 > epoch1,
        "epoch must advance: {} -> {}",
        epoch1,
        epoch2
    );
    assert_ne!(
        key1, key2,
        "same seed + different epoch must yield different key (P0-Z: no nonce reuse)"
    );
}
