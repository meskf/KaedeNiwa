//! SFrame 接收端会话。
//!
//! # 职责
//!
//! 1. 校验 `key_id`（防 epoch 错配）
//! 2. 解密 SFrame 帧
//! 3. 重放 / 乱序检测（每个 sender 独立维护 last_seq）
//!
//! # 不做
//!
//! - 不做 MLS 密钥管理（key 由上层提供）
//! - 不做解码（H.264 / PCM 由 decoder 负责）
//! - 不做信道恢复（丢包由 RTP / LiveKit 负责）
//!
//! # 使用
//!
//! ```text
//! 发送端：EncodedVideoStream → MediaSession（加密）→ Sink（LiveKit / 文件）
//! 接收端：Source（LiveKit / 文件）→ ReceiverSession（解密）→ Decoder
//! ```

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

// `decrypt` 在此模块内是设计意图的调用：
// ReceiverSession 就是 sframe::decrypt 的上层防护封装（sender 白名单 + 重放检测）。
// 因此本模块的 import 和调用都显式 allow deprecated。
#[allow(deprecated)]
use kaede_crypto::sframe::{
    SframeError, SframeKey, decrypt, read_counter, read_key_id, sender_id_of, seq_of,
};

use crate::sink::EncryptedFrame;

/// 解密后的帧。
#[derive(Debug, Clone)]
pub struct DecryptedFrame {
    pub data: Vec<u8>,
    pub is_keyframe: bool,
    pub pts_ms: i64,
    /// 发送者 id（从 counter 高 32 位提取）。
    pub sender_id: u32,
    /// 帧序号（从 counter 低 32 位提取）。
    pub seq: u32,
}

/// 接收端错误。
#[derive(Debug, thiserror::Error)]
pub enum ReceiveError {
    #[error("decrypt failed: {0}")]
    Decrypt(#[from] SframeError),
    #[error("decode failed: {0}")]
    Decode(String),
    #[error("key_id mismatch: expected {expected:#010x}, got {actual:#010x}")]
    WrongKeyId { expected: u32, actual: u32 },
    #[error("sender_id mismatch: expected {expected:#010x}, got {actual:#010x}")]
    WrongSender { expected: u32, actual: u32 },
    #[error("replay/out-of-order: sender {sender:#010x} last_seq={last}, got={got}")]
    Replay { sender: u32, last: u32, got: u32 },
}

/// 接收端统计。
#[derive(Debug, Clone, Copy, Default)]
pub struct ReceiveStats {
    pub decrypted: u64,
    pub rejected: u64,
}

/// SFrame 接收会话。
///
/// **无状态密钥**：key / key_id 由上层提供（一般在 MLS epoch 前进时重建）。
pub struct ReceiverSession {
    key: SframeKey,
    expected_key_id: u32,
    /// 只接收某个 sender 的帧（可选）。
    expected_sender_id: Option<u32>,
    /// sender_id -> last accepted seq。用于重放 / 乱序检测。
    last_seq_by_sender: Mutex<HashMap<u32, u32>>,
    decrypted: AtomicU64,
    rejected: AtomicU64,
}

impl ReceiverSession {
    /// 新建。`key_id` 一般等于 MLS epoch。
    pub fn new(key: SframeKey, key_id: u32) -> Self {
        Self {
            key,
            expected_key_id: key_id,
            expected_sender_id: None,
            last_seq_by_sender: Mutex::new(HashMap::new()),
            decrypted: AtomicU64::new(0),
            rejected: AtomicU64::new(0),
        }
    }

    /// 只接收某个 sender 的帧。多 sender 场景（多方通话）不要设这个。
    pub fn with_sender(mut self, sender_id: u32) -> Self {
        self.expected_sender_id = Some(sender_id);
        self
    }

    /// 解密一帧。
    ///
    /// # 安全性质
    ///
    /// - **篡改检测**：SFrame 头部（key_id + counter）是 AEAD 的 AAD，
    ///   任何字节被改都会导致 GCM tag 校验失败
    /// - **重放检测**：每个 sender 独立维护 last_seq，seq 必须严格递增
    ///   （允许 u32 回绕 `MAX -> 0`）。LiveKit 的 RTP 层会做乱序重排，
    ///   所以到达这里时应该大致有序
    /// - **key_id 校验**：防止用旧 epoch 的帧污染新 epoch 的解密器
    // ReceiverSession 是 sframe::decrypt 的上层封装：
    // - sframe::decrypt 裸 API 无防护（已 deprecated）
    // - ReceiverSession 在此之上做了 sender 白名单 + 重放检测
    // 因此内部调用旧的 sframe::decrypt 是设计意图，非疏忽。
    #[allow(deprecated)]
    pub fn decrypt(&self, frame: &EncryptedFrame) -> Result<DecryptedFrame, ReceiveError> {
        // 1. key_id 校验（快速拒绝，不解密）
        let key_id = read_key_id(&frame.data)?;
        if key_id.0 != self.expected_key_id {
            self.rejected.fetch_add(1, Ordering::Relaxed);
            return Err(ReceiveError::WrongKeyId {
                expected: self.expected_key_id,
                actual: key_id.0,
            });
        }

        // 2. counter 提取
        let counter = read_counter(&frame.data)?;
        let sender_id = sender_id_of(counter);
        let seq = seq_of(counter);

        // 3. sender 白名单（可选）
        if let Some(expected) = self.expected_sender_id
            && sender_id != expected
        {
            self.rejected.fetch_add(1, Ordering::Relaxed);
            return Err(ReceiveError::WrongSender {
                expected,
                actual: sender_id,
            });
        }

        // 4. 解密（AAD 校验：counter 已被 tag 保护）
        let data = match decrypt(&self.key, &frame.data) {
            Ok(d) => d,
            Err(e) => {
                self.rejected.fetch_add(1, Ordering::Relaxed);
                return Err(ReceiveError::Decrypt(e));
            }
        };

        // 5. 重放 / 乱序检测（解密成功后才信任 counter）
        // M12: 删除 u32 wrap 分支——发送端 Counter::checked_next 保证
        // 不产生 MAX→0 回绕；攻击者伪造的 seq=0 帧会被 AAD 校验拒绝。
        {
            // M11: Mutex poison 恢复 —— 一次 decrypt panic 不应让整个
            // 接收链永久死锁。取 into_inner 继续（replay 保护可能损失一次）。
            let mut map = match self.last_seq_by_sender.lock() {
                Ok(g) => g,
                Err(p) => {
                    tracing::warn!(
                        "receiver mutex poisoned (prior panic); recovering — some replay protection may be lost"
                    );
                    p.into_inner()
                }
            };
            if let Some(last) = map.get(&sender_id).copied()
                && seq <= last
            {
                drop(map);
                self.rejected.fetch_add(1, Ordering::Relaxed);
                return Err(ReceiveError::Replay {
                    sender: sender_id,
                    last,
                    got: seq,
                });
            }
            map.insert(sender_id, seq);
        }

        self.decrypted.fetch_add(1, Ordering::Relaxed);

        Ok(DecryptedFrame {
            data,
            is_keyframe: frame.is_keyframe,
            pts_ms: frame.pts_ms,
            sender_id,
            seq,
        })
    }

    /// 统计快照。
    pub fn stats(&self) -> ReceiveStats {
        ReceiveStats {
            decrypted: self.decrypted.load(Ordering::Relaxed),
            rejected: self.rejected.load(Ordering::Relaxed),
        }
    }
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use kaede_crypto::sframe::{Counter, KeyId, encrypt, make_counter_start, sender_id_from_uid};

    const TEST_KEY: [u8; 32] = [0xAB; 32];
    const TEST_KEY_ID: u32 = 42;

    fn make_receiver() -> ReceiverSession {
        ReceiverSession::new(SframeKey::from_bytes(TEST_KEY), TEST_KEY_ID)
    }

    fn encrypt_frame(uid: &str, seq: u32, payload: &[u8], is_keyframe: bool) -> EncryptedFrame {
        let key = SframeKey::from_bytes(TEST_KEY);
        let sid = sender_id_from_uid(uid);
        let counter = make_counter_start(sid) + seq as u64;
        let ct = encrypt(&key, KeyId(TEST_KEY_ID), Counter(counter), payload).unwrap();
        EncryptedFrame {
            data: ct,
            is_keyframe,
            pts_ms: seq as i64 * 16,
        }
    }

    #[test]
    fn decrypt_roundtrip() {
        let r = make_receiver();
        let frame = encrypt_frame("alice", 0, b"hello", true);
        let dec = r.decrypt(&frame).unwrap();
        assert_eq!(dec.data, b"hello");
        assert!(dec.is_keyframe);
        assert_eq!(dec.pts_ms, 0);
        assert_eq!(dec.sender_id, sender_id_from_uid("alice"));
        assert_eq!(dec.seq, 0);
    }

    #[test]
    fn sequential_frames_accepted() {
        let r = make_receiver();
        for i in 0..10 {
            let f = encrypt_frame("alice", i, format!("f{}", i).as_bytes(), false);
            assert!(r.decrypt(&f).is_ok(), "frame {} should decrypt", i);
        }
        let s = r.stats();
        assert_eq!(s.decrypted, 10);
        assert_eq!(s.rejected, 0);
    }

    #[test]
    fn tamper_rejected() {
        let r = make_receiver();
        let mut f = encrypt_frame("alice", 0, b"hello", false);
        // 篡改 payload（第一个 ct 字节）
        f.data[12] ^= 0xFF;
        let err = r.decrypt(&f).unwrap_err();
        assert!(matches!(err, ReceiveError::Decrypt(_)), "got {:?}", err);
    }

    #[test]
    fn tamper_counter_rejected() {
        let r = make_receiver();
        let mut f = encrypt_frame("alice", 0, b"hello", false);
        // 篡改 counter（header 字节 11）
        f.data[11] ^= 0x01;
        let err = r.decrypt(&f).unwrap_err();
        assert!(matches!(err, ReceiveError::Decrypt(_)), "got {:?}", err);
    }

    #[test]
    fn wrong_key_id_rejected() {
        let r = make_receiver();
        let key = SframeKey::from_bytes(TEST_KEY);
        let ct = encrypt(&key, KeyId(99), Counter(0), b"x").unwrap();
        let f = EncryptedFrame {
            data: ct,
            is_keyframe: false,
            pts_ms: 0,
        };
        let err = r.decrypt(&f).unwrap_err();
        assert!(matches!(err, ReceiveError::WrongKeyId { .. }));
        assert_eq!(r.stats().rejected, 1);
    }

    #[test]
    fn wrong_sender_rejected() {
        let r = make_receiver().with_sender(sender_id_from_uid("alice"));
        let f = encrypt_frame("bob", 0, b"x", false);
        let err = r.decrypt(&f).unwrap_err();
        assert!(matches!(err, ReceiveError::WrongSender { .. }));
    }

    #[test]
    fn replay_rejected() {
        let r = make_receiver();
        let f = encrypt_frame("alice", 5, b"x", false);
        r.decrypt(&f).unwrap();
        // 重放同一帧
        let err = r.decrypt(&f).unwrap_err();
        assert!(matches!(
            err,
            ReceiveError::Replay {
                last: 5,
                got: 5,
                ..
            }
        ));
    }

    #[test]
    fn out_of_order_rejected() {
        let r = make_receiver();
        r.decrypt(&encrypt_frame("alice", 5, b"a", false)).unwrap();
        // seq 回退
        let err = r
            .decrypt(&encrypt_frame("alice", 3, b"b", false))
            .unwrap_err();
        assert!(matches!(
            err,
            ReceiveError::Replay {
                last: 5,
                got: 3,
                ..
            }
        ));
    }

    #[test]
    fn multi_sender_independent_seq() {
        // 不带 with_sender：允许多 sender，各自独立维护 seq
        let r = make_receiver();
        r.decrypt(&encrypt_frame("alice", 5, b"a", false)).unwrap();
        // Bob 从 0 开始，不应被 Alice 的 seq=5 影响
        r.decrypt(&encrypt_frame("bob", 0, b"b", false)).unwrap();
        r.decrypt(&encrypt_frame("bob", 1, b"c", false)).unwrap();
        // Alice seq=6 仍可
        r.decrypt(&encrypt_frame("alice", 6, b"d", false)).unwrap();
        assert_eq!(r.stats().decrypted, 4);
        assert_eq!(r.stats().rejected, 0);
    }

    #[test]
    fn empty_payload_ok() {
        let r = make_receiver();
        let f = encrypt_frame("alice", 0, b"", false);
        let dec = r.decrypt(&f).unwrap();
        assert!(dec.data.is_empty());
    }

    #[test]
    fn keyframe_flag_preserved() {
        let r = make_receiver();
        let f1 = encrypt_frame("alice", 0, b"idr", true);
        let f2 = encrypt_frame("alice", 1, b"p", false);
        assert!(r.decrypt(&f1).unwrap().is_keyframe);
        assert!(!r.decrypt(&f2).unwrap().is_keyframe);
    }
}
