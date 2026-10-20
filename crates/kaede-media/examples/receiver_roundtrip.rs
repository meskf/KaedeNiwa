//! 本地回环：SFrame 加密 → 解密 → 字节比对（含负向测试）。
//!
//! 不需要真采集硬件。跑：
//! ```sh
//! cargo run --release -p kaede-media --example receiver_roundtrip
//! ```

use kaede_crypto::sframe::{
    Counter, KeyId, SframeKey, encrypt, make_counter_start, sender_id_from_uid,
};
use kaede_media::{EncryptedFrame, ReceiveError, ReceiverSession};

const KEY_BYTES: [u8; 32] = [0xAB; 32];
const KEY_ID: u32 = 42;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key = SframeKey::from_bytes(KEY_BYTES);
    let alice_sid = sender_id_from_uid("alice");

    // ---- [1] 发送端：生成 100 个假帧，模拟加密 ----
    let total: u32 = 100;
    let mut frames: Vec<(EncryptedFrame, Vec<u8>)> = Vec::with_capacity(total as usize);
    for i in 0..total {
        let payload = format!("frame #{} payload ({} bytes)", i, 32 + i as usize).into_bytes();
        let counter = make_counter_start(alice_sid) + i as u64;
        let ct = encrypt(&key, KeyId(KEY_ID), Counter(counter), &payload)?;
        frames.push((
            EncryptedFrame {
                data: ct,
                is_keyframe: i == 0,
                pts_ms: i as i64 * 16,
            },
            payload,
        ));
    }
    println!("[1] sender: {} frames encrypted", total);

    // ---- [2] 接收端：解密全部 ----
    let receiver =
        ReceiverSession::new(SframeKey::from_bytes(KEY_BYTES), KEY_ID).with_sender(alice_sid);

    let mut ok = 0u32;
    for (enc, expected) in &frames {
        let dec = receiver.decrypt(enc)?;
        if dec.data != *expected {
            return Err(format!("frame #{} content mismatch", dec.seq).into());
        }
        if dec.sender_id != alice_sid {
            return Err("sender_id mismatch".into());
        }
        ok += 1;
    }
    println!("[2] receiver: {}/{} frames decrypted & verified", ok, total);

    // ---- [3] 负向测试 ----

    // 3a. 篡改 payload
    let mut tampered = frames[0].0.clone();
    tampered.data[20] ^= 0xFF;
    let tamper_res = receiver.decrypt(&tampered);
    let tamper_rejected = matches!(tamper_res, Err(ReceiveError::Decrypt(_)));

    // 3b. 重放
    let replay_res = receiver.decrypt(&frames[50].0);
    let replay_rejected = matches!(replay_res, Err(ReceiveError::Replay { .. }));

    // 3c. 错误 key_id
    let wrong_key_receiver = ReceiverSession::new(SframeKey::from_bytes(KEY_BYTES), 99);
    let wrong_key_res = wrong_key_receiver.decrypt(&frames[0].0);
    let wrong_key_rejected = matches!(wrong_key_res, Err(ReceiveError::WrongKeyId { .. }));

    // 3d. 错误 sender
    let wrong_sender_receiver =
        ReceiverSession::new(SframeKey::from_bytes(KEY_BYTES), KEY_ID).with_sender(0xDEADBEEF);
    let wrong_sender_res = wrong_sender_receiver.decrypt(&frames[0].0);
    let wrong_sender_rejected = matches!(wrong_sender_res, Err(ReceiveError::WrongSender { .. }));

    println!("[3] negative tests:");
    println!("      tamper rejected      = {}", tamper_rejected);
    println!("      replay rejected      = {}", replay_rejected);
    println!("      wrong key_id         = {}", wrong_key_rejected);
    println!("      wrong sender_id      = {}", wrong_sender_rejected);

    // ---- [4] 统计 ----
    let s = receiver.stats();
    println!(
        "[4] receiver stats: decrypted={} rejected={}",
        s.decrypted, s.rejected
    );

    // 合法帧 100 个 + 篡改 1 个 + 重放 1 个 = 解密成功 100, 拒绝 2
    // （wrong_key/wrong_sender 用的是别的 receiver 实例，不计入）
    if s.decrypted != total as u64 {
        return Err(format!("expected decrypted={}, got {}", total, s.decrypted).into());
    }
    if s.rejected != 2 {
        return Err(format!("expected rejected=2, got {}", s.rejected).into());
    }

    if !tamper_rejected || !replay_rejected || !wrong_key_rejected || !wrong_sender_rejected {
        return Err("negative tests failed".into());
    }

    println!("PASS receiver_roundtrip");
    Ok(())
}
