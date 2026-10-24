#![no_main]
use libfuzzer_sys::fuzz_target;
use kaede_protocol::{ServerMessage, WireMessage};

fuzz_target!(|data: &[u8]| {
    // 契约：ServerMessage::decode 对任意字节输入永不 panic。
    // 攻击面：恶意服务端向客户端发送畸形 CBOR。
    let _ = ServerMessage::decode(data);
});
