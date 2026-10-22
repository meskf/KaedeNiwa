//! 协议解码健壮性：任意/截断/超大输入都只能返回 `Err`，不能 panic。
//!
//! 这是服务端 WS 输入面的最低保证：攻击者可以发送任意字节，
//! 解码器崩溃等于远程 DoS。测试用确定性伪随机序列，失败可复现。

use kaede_protocol::{
    ClientMessage, MAX_MESSAGE_BYTES, PROTOCOL_VERSION, ServerMessage, WireMessage,
};

/// 确定性 xorshift64*，避免引入额外依赖。
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn fill(&mut self, buf: &mut [u8]) {
        for chunk in buf.chunks_mut(8) {
            let v = self.next_u64().to_le_bytes();
            chunk.copy_from_slice(&v[..chunk.len()]);
        }
    }
}

#[test]
fn random_bytes_never_panic_client_decode() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for len in 0..=2048usize {
        let mut buf = vec![0u8; len];
        rng.fill(&mut buf);
        let _ = ClientMessage::decode(&buf);
    }
}

#[test]
fn random_bytes_never_panic_server_decode() {
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    for len in 0..=2048usize {
        let mut buf = vec![0u8; len];
        rng.fill(&mut buf);
        let _ = ServerMessage::decode(&buf);
    }
}

#[test]
fn every_truncation_of_valid_messages_is_an_error_not_panic() {
    let client = ClientMessage::ClientHello {
        protocol_version: PROTOCOL_VERSION,
        client_name: "fuzz".into(),
        client_version: "0.0.1".into(),
        session_token: Some("token".into()),
    }
    .encode()
    .expect("encode client hello");

    for cut in 0..client.len() {
        let _ = ClientMessage::decode(&client[..cut]);
    }

    let server = ServerMessage::ServerHello {
        protocol_version: PROTOCOL_VERSION,
        server_id: "s".into(),
        server_name: "n".into(),
    }
    .encode()
    .expect("encode server hello");

    for cut in 0..server.len() {
        let _ = ServerMessage::decode(&server[..cut]);
    }
}

#[test]
fn oversized_payload_is_rejected_before_decoding() {
    let huge = vec![0u8; MAX_MESSAGE_BYTES + 1];
    assert!(ClientMessage::decode(&huge).is_err());
    assert!(ServerMessage::decode(&huge).is_err());
}

#[test]
fn bit_flips_in_valid_message_never_panic() {
    let encoded = ClientMessage::Ping { nonce: 42 }
        .encode()
        .expect("encode ping");
    for i in 0..encoded.len() * 8 {
        let mut mutated = encoded.clone();
        mutated[i / 8] ^= 1 << (i % 8);
        let _ = ClientMessage::decode(&mutated);
    }
}
