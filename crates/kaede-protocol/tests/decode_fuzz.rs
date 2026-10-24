//! P0-2: CBOR 解码 fuzz（property-based）
//!
//! 目标：`ClientMessage::decode` / `ServerMessage::decode` 对任意输入永不 panic。
//! 这挡住"恶意客户端发送畸形 CBOR 导致服务端 panic"这一攻击面。

use kaede_protocol::{ClientMessage, ServerMessage, WireMessage};
use quickcheck_macros::quickcheck;

#[quickcheck]
fn client_message_decode_never_panics(data: Vec<u8>) -> bool {
    let _ = ClientMessage::decode(&data);
    true
}

#[quickcheck]
fn server_message_decode_never_panics(data: Vec<u8>) -> bool {
    let _ = ServerMessage::decode(&data);
    true
}

#[quickcheck]
fn random_bytes_do_not_panic_both_directions(data: Vec<u8>) -> bool {
    let _ = ClientMessage::decode(&data);
    let _ = ServerMessage::decode(&data);
    true
}

// 定向测试：已知"解码炸弹"模式
#[test]
fn oversized_length_prefix_does_not_panic() {
    // CBOR map header: 0xBB (map with 8-byte length) + 0xFFFFFFFFFFFFFFFF
    let data = vec![0xBB, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
    let _ = ClientMessage::decode(&data);
    let _ = ServerMessage::decode(&data);
}

#[test]
fn empty_input_returns_error() {
    assert!(ClientMessage::decode(&[]).is_err());
    assert!(ServerMessage::decode(&[]).is_err());
}

#[test]
fn truncated_cbor_does_not_panic() {
    // 0xBB 是 8 字节长度前缀，只给 1 字节
    let data = vec![0xBB];
    let _ = ClientMessage::decode(&data);
    let _ = ServerMessage::decode(&data);
}
