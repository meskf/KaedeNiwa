#![no_main]
use libfuzzer_sys::fuzz_target;
use kaede_protocol::{ClientMessage, WireMessage};

fuzz_target!(|data: &[u8]| {
    // 契约：ClientMessage::decode 对任意字节输入永不 panic。
    // 攻击面：恶意客户端发送畸形 CBOR 导致服务端进程崩溃。
    let _ = ClientMessage::decode(data);
});
