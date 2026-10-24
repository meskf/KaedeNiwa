//! connection 单元测试（自 connection.rs 内联 `mod tests` 原样拆出，
//! 仅去掉一层缩进与模块包裹，测试内容未改动）。

use super::*;
use crate::transport::MockTransport;
use kaede_protocol::ErrorCode;

async fn spawn_hello_server(server_t: crate::transport::MockServerSide) {
    let msg = server_t.recv().await.unwrap();
    assert!(matches!(msg, ClientMessage::ClientHello { .. }));
    server_t
        .send(ServerMessage::ServerHello {
            protocol_version: kaede_protocol::PROTOCOL_VERSION,
            server_id: "srv_test".into(),
            server_name: "Test".into(),
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn connect_happy_path() {
    let (client_t, server_t) = MockTransport::pair();
    let conn = ServerConnection::new("s1", Arc::new(client_t), None);
    let h = tokio::spawn(spawn_hello_server(server_t));
    conn.connect_with_token("test", "0.1.0", Some("tok".into()))
        .await
        .unwrap();
    h.await.unwrap();
    assert_eq!(conn.state(), ClientState::Connecting);
}

#[tokio::test]
async fn full_join_flow() {
    let (client_t, server_t) = MockTransport::pair();
    let conn = ServerConnection::new("s1", Arc::new(client_t), None);

    let h = tokio::spawn(async move {
        let _ = server_t.recv().await.unwrap();
        server_t
            .send(ServerMessage::ServerHello {
                protocol_version: kaede_protocol::PROTOCOL_VERSION,
                server_id: "srv_test".into(),
                server_name: "Test".into(),
            })
            .await
            .unwrap();
        let msg = server_t.recv().await.unwrap();
        assert!(matches!(msg, ClientMessage::Join));
        server_t
            .send(ServerMessage::Joined {
                peers: vec![UserId::new("bob")],
            })
            .await
            .unwrap();
    });

    conn.connect_with_token("test", "0.1.0", Some("tok".into()))
        .await
        .unwrap();
    conn.authenticate().unwrap();
    conn.join().await.unwrap();
    h.await.unwrap();

    // join 现在只发不收；状态已进 MlsJoining，但 peers 要等
    // event_loop 处理 Joined 后才填充。
    assert_eq!(conn.state(), ClientState::MlsJoining);

    // 手动喂一条 Joined，验证 event_loop 路径
    conn.handle_server_message(ServerMessage::Joined {
        peers: vec![UserId::new("bob")],
    })
    .await
    .unwrap();
    assert_eq!(conn.peers().len(), 1);
}

#[tokio::test]
async fn cannot_join_before_auth() {
    let (client_t, _server_t) = MockTransport::pair();
    let conn = ServerConnection::new("s1", Arc::new(client_t), None);
    let err = conn.join().await.unwrap_err();
    assert!(matches!(err, ClientError::NotAuthenticated));
}

#[tokio::test]
async fn server_error_terminates() {
    let (client_t, server_t) = MockTransport::pair();
    let conn = ServerConnection::new("s1", Arc::new(client_t), None);

    let h = tokio::spawn(async move {
        let _ = server_t.recv().await.unwrap();
        server_t
            .send(ServerMessage::Error {
                code: ErrorCode::UnsupportedVersion,
                message: "need v3".into(),
            })
            .await
            .unwrap();
    });

    let err = conn
        .connect_with_token("test", "0.1.0", Some("tok".into()))
        .await
        .unwrap_err();
    h.await.unwrap();
    assert!(matches!(
        err,
        ClientError::Server {
            code: ErrorCode::UnsupportedVersion,
            ..
        }
    ));
}

// =================================================================
// MLS 集成测试
// =================================================================

#[tokio::test]
async fn mls_create_group_transitions_to_ready() {
    let (client_t, _server_t) = MockTransport::pair();
    let conn = ServerConnection::new("s1", Arc::new(client_t), None);
    conn.state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .force(ClientState::MlsJoining);

    conn.mls_init("alice").unwrap();
    assert!(!conn.mls_is_ready());

    conn.mls_create_group().unwrap();
    assert!(conn.mls_is_ready());
    assert_eq!(conn.state(), ClientState::MlsReady);
    assert_eq!(conn.mls_member_count(), 1);
}

#[tokio::test]
async fn mls_send_app_goes_through_transport() {
    let (client_t, server_t) = MockTransport::pair();
    let conn = ServerConnection::new("s1", Arc::new(client_t), None);
    conn.state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .force(ClientState::MlsJoining);

    conn.mls_init("alice").unwrap();
    conn.mls_create_group().unwrap();

    let _ct = conn.mls_send_app(b"hello").await.unwrap();

    // 服务器端应收到一条 MlsAppMessage
    let got = server_t.recv().await.unwrap();
    match got {
        ClientMessage::MlsAppMessage { payload } => {
            assert!(!payload.is_empty(), "ciphertext should not be empty");
        }
        other => panic!("expected MlsAppMessage, got {:?}", other),
    }
}

#[tokio::test]
async fn mls_receive_app_message_decrypts() {
    // 建立两个 MlsSession，通过它构造一条给 alice 的合法密文。
    // 然后把它注入 alice 的 ServerConnection，检查事件是否触发。
    use std::sync::Mutex as StdMutex;

    struct CollectingObserver {
        events: StdMutex<Vec<ClientEvent>>,
    }
    impl crate::ClientObserver for CollectingObserver {
        fn on_event(&self, event: ClientEvent) {
            self.events
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(event);
        }
    }

    // --- 准备：离线构造 alice + bob 的 MLS 组 ---
    let mut bob_session = MlsSession::new("bob").unwrap();
    let bob_kp = bob_session.generate_key_package().unwrap();

    let mut alice_offline = MlsSession::new("alice").unwrap();
    alice_offline.create_group().unwrap();
    let (_c, welcome) = alice_offline.add_member(&bob_kp).unwrap();
    bob_session.join_with_welcome(&welcome).unwrap();
    assert_eq!(alice_offline.member_count(), 2);
    assert_eq!(bob_session.member_count(), 2);

    // bob 加密一条消息（此时双方 epoch 一致）
    let ct = bob_session.encrypt(b"hi alice").unwrap();

    // --- 拿到 alice_offline 的 MlsSession 也没法直接塞进 ServerConnection，
    //     因为 ServerConnection.mls_init 会创建新 session。
    //     所以这个测试改成：直接验证 alice_offline 能解密，
    //     并验证 ServerConnection 的 handle_server_message 走通了事件链路
    //     使用 MockTransport 无 group 时不会 panic。
    let pt = alice_offline.decrypt(&ct).unwrap();
    assert_eq!(pt, b"hi alice");

    // --- 事件链路测试：给 ServerConnection 一个 无 MLS 状态，
    //     收到 AppMessage 应该静默丢弃（不 panic）---
    let observer = Arc::new(CollectingObserver {
        events: StdMutex::new(Vec::new()),
    });
    let (client_t, _server_t) = MockTransport::pair();
    let conn = ServerConnection::new("s1", Arc::new(client_t), Some(observer.clone()));

    // 未初始化 MLS，直接送一条 AppMessage
    conn.handle_server_message(ServerMessage::MlsAppMessage {
        from: UserId::new("bob"),
        payload: ct.clone(),
    })
    .await
    .unwrap();

    // 应该没有 AppMessage 事件（因为没 MLS state）
    let events = observer.events.lock().unwrap_or_else(|e| e.into_inner());
    let app_events = events
        .iter()
        .filter(|e| matches!(e, ClientEvent::AppMessage { .. }))
        .count();
    assert_eq!(app_events, 0, "no MLS state => no AppMessage event");
}

#[tokio::test]
async fn mls_remove_member_by_uid_via_connection() {
    use std::sync::Arc;
    // 直接用两个 MlsSession 模拟握手，不经过网络
    let bob = MlsSession::new("bob").unwrap();
    let bob_kp = bob.generate_key_package().unwrap();

    let (client_t, _server_t) = MockTransport::pair();
    let conn = ServerConnection::new("s1", Arc::new(client_t), None);
    conn.state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .force(ClientState::MlsJoining);
    conn.mls_init("alice").unwrap();
    conn.mls_create_group().unwrap();

    let _ = conn.mls_add_member(&bob_kp).unwrap();
    assert_eq!(conn.mls_member_count(), 2);

    let commit = conn.mls_remove_member_by_uid("bob").unwrap();
    assert!(!commit.is_empty());
    assert_eq!(conn.mls_member_count(), 1);
}
#[tokio::test]
async fn mls_export_sframe_key_for_purpose_via_connection() {
    let (client_t, _server_t) = MockTransport::pair();
    let conn = ServerConnection::new("s1", Arc::new(client_t), None);
    conn.state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .force(ClientState::MlsJoining);
    conn.mls_init("alice").unwrap();
    conn.mls_create_group().unwrap();

    let k_va = conn
        .mls_export_sframe_key_for("general", kaede_mls::SframePurpose::VoiceAudio)
        .unwrap();
    let k_vv = conn
        .mls_export_sframe_key_for("general", kaede_mls::SframePurpose::VoiceVideo)
        .unwrap();
    let k_sv = conn
        .mls_export_sframe_key_for("general", kaede_mls::SframePurpose::ScreenVideo)
        .unwrap();
    assert_ne!(k_va, k_vv);
    assert_ne!(k_vv, k_sv);

    // 旧 API 等于显式 Default
    let k_default = conn.mls_export_sframe_key("general").unwrap();
    let k_explicit = conn
        .mls_export_sframe_key_for("general", kaede_mls::SframePurpose::Default)
        .unwrap();
    assert_eq!(k_default, k_explicit);
}
#[tokio::test]
async fn mls_export_sframe_key_via_connection() {
    let (client_t, _server_t) = MockTransport::pair();
    let conn = ServerConnection::new("s1", Arc::new(client_t), None);
    conn.state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .force(ClientState::MlsJoining);

    conn.mls_init("alice").unwrap();
    conn.mls_create_group().unwrap();

    let k1 = conn.mls_export_sframe_key("ch-general").unwrap();
    let k2 = conn.mls_export_sframe_key("ch-general").unwrap();
    assert_eq!(k1, k2);

    let k3 = conn.mls_export_sframe_key("ch-voice").unwrap();
    assert_ne!(k1, k3);
}

// =================================================================
// C-2: MLS handshake 失败 emit MlsSyncError
// =================================================================

#[tokio::test]
async fn mls_commit_failure_emits_sync_error() {
    use std::sync::Mutex as StdMutex;

    struct CollectingObserver {
        events: StdMutex<Vec<ClientEvent>>,
    }
    impl crate::ClientObserver for CollectingObserver {
        fn on_event(&self, event: ClientEvent) {
            self.events
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(event);
        }
    }

    let observer = Arc::new(CollectingObserver {
        events: StdMutex::new(Vec::new()),
    });
    let (client_t, _server_t) = MockTransport::pair();
    let conn = ServerConnection::new("s1", Arc::new(client_t), Some(observer.clone()));

    // 初始化 MLS 并进入 MlsReady
    conn.mls_init("alice").unwrap();
    conn.state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .force(ClientState::MlsJoining);
    conn.mls_create_group().unwrap();
    assert!(conn.mls_is_ready());

    // 送一条无法反序列化的 commit
    conn.handle_server_message(ServerMessage::MlsCommit {
        from: UserId::new("bob"),
        epoch: 1,
        payload: vec![0xFF, 0xFF, 0xFF, 0xFF],
    })
    .await
    .unwrap();

    // 验证 emit 了 MlsSyncError
    let events = observer.events.lock().unwrap_or_else(|e| e.into_inner());
    let has_sync_error = events.iter().any(|e| {
        matches!(
            e,
            ClientEvent::MlsSyncError {
                recoverable: true,
                ..
            }
        )
    });
    assert!(
        has_sync_error,
        "expected MlsSyncError (recoverable=true), got {:?}",
        *events
    );
}

// =================================================================
// R2-02：私密频道内容不得从主群广播通道进入
// =================================================================

#[tokio::test]
async fn main_group_message_targeting_private_channel_is_dropped() {
    use std::sync::Mutex as StdMutex;

    struct CollectingObserver {
        events: StdMutex<Vec<ClientEvent>>,
    }
    impl crate::ClientObserver for CollectingObserver {
        fn on_event(&self, event: ClientEvent) {
            self.events
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(event);
        }
    }

    let observer = Arc::new(CollectingObserver {
        events: StdMutex::new(Vec::new()),
    });

    // 构造一条“主群”消息（正文声称属于私密频道 "ch_secret"）。
    let (client_t, _server_t) = MockTransport::pair();
    let conn = ServerConnection::new("s1", Arc::new(client_t), Some(observer.clone()));
    conn.state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .force(ClientState::MlsJoining);
    conn.mls_init("alice").unwrap();
    conn.mls_create_group().unwrap();

    // 用本地主群加密一条指向私密频道的 payload。
    let payload =
        crate::payload::MessagePayload::new("ch_secret", "bob", "injected", Vec::new(), None)
            .to_bytes()
            .unwrap();
    let ct = conn
        .mls
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_mut()
        .unwrap()
        .encrypt(&payload)
        .unwrap();

    // 先让本地持有 "ch_secret" 的频道 group（模拟私密频道成员）。
    conn.channel_ensure_group("ch_secret").unwrap();
    assert!(conn.channel_has_group("ch_secret"));

    conn.handle_server_message(ServerMessage::MlsAppMessage {
        from: UserId::new("bob"),
        payload: ct,
    })
    .await
    .unwrap();

    let events = observer.events.lock().unwrap_or_else(|e| e.into_inner());
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, ClientEvent::AppMessage { .. })),
        "main-group message targeting a private channel must be dropped, got {:?}",
        *events
    );
}

// =================================================================
// R2-04：Welcome 驱动的旧群重建
// =================================================================

#[test]
fn welcome_with_new_group_id_resets_existing_group() {
    // bob 先加入 alice1 的旧群。
    let mut bob = MlsSession::new("bob").unwrap();
    let bob_kp1 = bob.generate_key_package().unwrap();
    let mut alice1 = MlsSession::new("alice").unwrap();
    alice1.create_group().unwrap();
    let (_c1, w1) = alice1.add_member(&bob_kp1).unwrap();
    bob.join_with_welcome(&w1).unwrap();
    let gid1 = bob.group_id().unwrap().to_vec();

    // 房主丢失快照后重建群：alice2 新群，使用 bob 的新 KP。
    let mut alice2 = MlsSession::new("alice").unwrap();
    alice2.create_group().unwrap();
    let bob_kp2 = bob.generate_key_package().unwrap();
    let (_c2, w2) = alice2.add_member(&bob_kp2).unwrap();
    let gid2 = alice2.group_id().unwrap().to_vec();
    assert_ne!(gid1, gid2, "rebuild must produce a new group id");

    let (client_t, _server_t) = MockTransport::pair();
    let conn = ServerConnection::new("s1", Arc::new(client_t), None);
    *conn.mls.lock().unwrap_or_else(|e| e.into_inner()) = Some(bob);

    let changed = conn
        .mls_join_or_reset_welcome(&w2, Some(&gid2), "alice")
        .unwrap();
    assert!(changed, "different group id must rebuild local group");

    {
        let mut guard = conn.mls.lock().unwrap_or_else(|e| e.into_inner());
        let s = guard.as_mut().unwrap();
        assert_eq!(s.group_id().unwrap(), &gid2[..]);
        assert_eq!(s.epoch(), alice2.epoch());
        let ct = alice2.encrypt(b"after-reset").unwrap();
        assert_eq!(s.decrypt(&ct).unwrap(), b"after-reset");
        let ct2 = s.encrypt(b"reply").unwrap();
        assert_eq!(alice2.decrypt(&ct2).unwrap(), b"reply");
    }

    // 同一个 Welcome 再投递一次（重复）不应重建。
    assert!(
        !conn
            .mls_join_or_reset_welcome(&w2, Some(&gid2), "alice")
            .unwrap(),
        "duplicate welcome must be ignored"
    );
}

// =================================================================
// R2-05：DM 代际仲裁
// =================================================================

#[test]
fn dm_generation_rebuild_converges() {
    let conv = "dm:alice:bob";
    let bob = MlsSession::new("bob").unwrap();
    let bob_kp1 = bob.generate_key_package().unwrap();
    let bob_kp2 = bob.generate_key_package().unwrap();

    // 发送方（alice）与接收方（bob）各自一个连接，bob 复用同一 MlsSession。
    let (alice_t, _alice_s) = MockTransport::pair();
    let alice = ServerConnection::new("s-alice", Arc::new(alice_t), None);
    alice.mls_init("alice").unwrap();

    let (bob_t, _bob_s) = MockTransport::pair();
    let bob_conn = ServerConnection::new("s-bob", Arc::new(bob_t), None);
    *bob_conn.mls.lock().unwrap_or_else(|e| e.into_inner()) = Some(bob);

    // 第一次建群：gen=1。
    let (w1, gen1) = alice.dm_init_group(conv, &bob_kp1).unwrap();
    assert_eq!(gen1, 1);
    bob_conn.dm_join_group(conv, &w1, gen1).unwrap();
    assert_eq!(bob_conn.dm_generation(conv), 1);

    let ct = alice.dm_seal(conv, b"hello").unwrap();
    assert_eq!(bob_conn.dm_open(conv, "alice", &ct).unwrap(), b"hello");

    // alice 丢失本地 DM 状态后重建：gen=2。
    alice.dm_remove_group(conv);
    assert_eq!(alice.dm_generation(conv), 1);
    let (w2, gen2) = alice.dm_init_group(conv, &bob_kp2).unwrap();
    assert_eq!(gen2, 2);

    // bob 收到 gen=2 的 Welcome 必须采纳，而不是继续保留旧群。
    bob_conn.dm_join_group(conv, &w2, gen2).unwrap();
    assert_eq!(bob_conn.dm_generation(conv), 2);

    let ct2 = alice.dm_seal(conv, b"new-group").unwrap();
    assert_eq!(bob_conn.dm_open(conv, "alice", &ct2).unwrap(), b"new-group");
    let ct3 = bob_conn.dm_seal(conv, b"reply").unwrap();
    assert_eq!(alice.dm_open(conv, "bob", &ct3).unwrap(), b"reply");
}

// =================================================================
// S1：MLS 成员集合绑定
// =================================================================

#[test]
fn dm_join_rejects_group_with_extra_member() {
    let conv = "dm:alice:bob";

    // 正常的 bob / carol 接收端。
    let bob = MlsSession::new("bob").unwrap();
    let bob_kp = bob.generate_key_package().unwrap();
    let carol = MlsSession::new("carol").unwrap();
    let carol_kp = carol.generate_key_package().unwrap();

    let (alice_t, _alice_s) = MockTransport::pair();
    let alice = ServerConnection::new("s-alice", Arc::new(alice_t), None);
    alice.mls_init("alice").unwrap();

    // 模拟恶意对端：构造一个包含 alice + carol + bob 的 DM 群。
    let dm_welcome = {
        let guard = alice.mls.lock().unwrap_or_else(|e| e.into_inner());
        let main = guard.as_ref().unwrap();
        let mut dm = main.fork_for_new_group();
        dm.create_group().unwrap();
        dm.add_member(&carol_kp).unwrap();
        dm.add_member(&bob_kp).unwrap().1
    };

    let (bob_t, _bob_s) = MockTransport::pair();
    let bob_conn = ServerConnection::new("s-bob", Arc::new(bob_t), None);
    *bob_conn.mls.lock().unwrap_or_else(|e| e.into_inner()) = Some(bob);

    let err = bob_conn
        .dm_join_group(conv, &dm_welcome, 1)
        .expect_err("3-person DM group must be rejected");
    assert!(
        err.to_string().contains("members"),
        "unexpected error: {err}"
    );
    assert!(
        !bob_conn.dm_has_group(conv),
        "rejected welcome must not create a local DM group"
    );
}

#[test]
fn channel_group_member_validation() {
    let allowed: HashSet<String> = ["alice", "bob"].iter().map(|s| s.to_string()).collect();

    // 完整集合与“尚未全部进群”的子集都允许。
    let full: HashSet<String> = allowed.clone();
    assert!(ServerConnection::ensure_channel_group_members(&full, &allowed, "alice", "ch").is_ok());
    let pending: HashSet<String> = ["alice".to_string()].into();
    assert!(
        ServerConnection::ensure_channel_group_members(&pending, &allowed, "alice", "ch").is_ok()
    );

    // 群外成员、缺少自己都必须拒绝。
    let extra: HashSet<String> = ["alice".to_string(), "carol".to_string()].into();
    assert!(
        ServerConnection::ensure_channel_group_members(&extra, &allowed, "alice", "ch").is_err()
    );
    let missing_self: HashSet<String> = ["bob".to_string()].into();
    assert!(
        ServerConnection::ensure_channel_group_members(&missing_self, &allowed, "alice", "ch")
            .is_err()
    );
}
