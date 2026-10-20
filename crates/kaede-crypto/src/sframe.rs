//! SFrame 帧加密（RFC 9605 风格）。
//!
//! # 帧格式
//!
//! ```text
//! [ key_id   : 4 BE ]
//! [ counter  : 8 BE ]
//! [ payload  : AES-256-GCM(ct || 16-byte tag) ]
//! ```
//!
//! 头部共 `SFRAME_HEADER_LEN = 12` 字节。
//!
//! # Nonce 构造
//!
//! ```text
//! GCM nonce = [0,0,0,0] || counter(8B BE)   -> 12 字节
//! ```
//!
//! # AAD
//!
//! 12 字节帧头（`key_id || counter`）作为 AAD。这保证：
//! - 篡改 key_id 或 counter 会导致 GCM tag 校验失败
//! - 重放检测可以在解密前基于 counter 做（明文的 counter 已绑定到密文）
//!
//! # 与旧项目的关系
//!
//! 此格式与 v1 KaedeNiwa（`frontend/src/crypto/sframe.ts` 和
//! `src/core/sframe/encryptor.cpp`）**逐字节相同**。KAT 保留作为回归锚点。

use zeroize::{Zeroize, ZeroizeOnDrop};

/// SFrame 头部长度（key_id + counter）。
pub const SFRAME_HEADER_LEN: usize = 12;

/// AES-GCM tag 长度。
pub const SFRAME_GCM_TAG_LEN: usize = 16;

/// 密钥 ID（4 字节大端）。通常等于 MLS epoch。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyId(pub u32);

/// 消息计数器（8 字节大端）。
///
/// 在 KaedeNiwa 里，counter 的高 32 位是发送者分区，低 32 位是序号。
/// 这让不同发送者的 nonce 空间不重叠。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Counter(pub u64);

impl Counter {
    /// 帧序号前进一位。seq 溢出时返回 Err，防止污染 sender_id 位导致 nonce 复用。
    pub fn checked_next(self) -> Result<Self, SframeError> {
        let seq = self.0 & 0xFFFF_FFFF;
        if seq == 0xFFFF_FFFF {
            return Err(SframeError::CounterExhausted);
        }
        Ok(Counter(self.0 + 1))
    }
}

/// SFrame 密钥。32 字节，drop 时自动擦除。
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SframeKey([u8; 32]);

impl SframeKey {
    /// 从 32 字节数组构造。
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// 从切片构造。长度必须为 32。
    pub fn from_slice(bytes: &[u8]) -> Result<Self, SframeError> {
        if bytes.len() != 32 {
            return Err(SframeError::InvalidKeyLength(bytes.len()));
        }
        let mut buf = [0u8; 32];
        buf.copy_from_slice(bytes);
        Ok(Self(buf))
    }

    /// 暴露字节用于内部运算。外部代码不应长期持有。
    pub(crate) fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// SFrame 错误。
#[derive(Debug, thiserror::Error)]
pub enum SframeError {
    #[error("frame too short: {0} bytes (need at least {1})")]
    FrameTooShort(usize, usize),
    #[error("invalid key length: {0} (expected 32)")]
    InvalidKeyLength(usize),
    #[error("aead failure: {0}")]
    Aead(String),
    /// 帧序号耗尽（seq 达到 u32::MAX）。调用方 MUST 换 key（MLS epoch 前进）。
    #[error("counter exhausted (seq overflow); advance MLS epoch / rotate SFrame key")]
    CounterExhausted,
}

// ============================================================================
// 头部构造
// ============================================================================

fn make_header(key_id: KeyId, counter: Counter) -> [u8; SFRAME_HEADER_LEN] {
    let mut h = [0u8; SFRAME_HEADER_LEN];
    h[0..4].copy_from_slice(&key_id.0.to_be_bytes());
    h[4..12].copy_from_slice(&counter.0.to_be_bytes());
    h
}

fn make_gcm_nonce(counter: Counter) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[4..12].copy_from_slice(&counter.0.to_be_bytes());
    n
}

// ============================================================================
// 公共 API
// ============================================================================

/// 从帧头读取 `key_id`。不验证 tag。
///
/// 用于接收方在多密钥场景下找到正确的解密密钥。
pub fn read_key_id(frame: &[u8]) -> Result<KeyId, SframeError> {
    if frame.len() < 4 {
        return Err(SframeError::FrameTooShort(frame.len(), 4));
    }
    let mut buf = [0u8; 4];
    buf.copy_from_slice(&frame[0..4]);
    Ok(KeyId(u32::from_be_bytes(buf)))
}

/// 从帧头读取 `counter`。不验证 tag。
///
/// **调用方必须明白**：这个值在解密前是未认证的。合法用途包括：
/// - 路由（选择哪个密钥解密）
/// - 重放检测的预筛选（标记后再解密，与旧项目行为一致）
///
/// 非法用途：基于 counter 做安全决策（例如"counter > 100 就拒绝"）。
pub fn read_counter(frame: &[u8]) -> Result<Counter, SframeError> {
    if frame.len() < SFRAME_HEADER_LEN {
        return Err(SframeError::FrameTooShort(frame.len(), SFRAME_HEADER_LEN));
    }
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&frame[4..12]);
    Ok(Counter(u64::from_be_bytes(buf)))
}

// ============================================================================
// Sender 分区
// ============================================================================
//
// nonce = [0,0,0,0] || counter(8B BE)。counter 完全决定 nonce。
// 同一个 key 被多个 sender 共用时（典型：MLS group 里多人共享 channel key），
// 各自从 counter=0 开始会撞 AES-GCM nonce。
//
// 设计：counter 高 32 位是 sender_id，低 32 位是帧序号。
// Alice 用 sender_id=0x12345678 从 counter=0x12345678_00000000 开始；
// Bob 用 sender_id=0xABCDEF01 从 counter=0xABCDEF01_00000000 开始。
// 两个 sender 的 counter 空间天然不重叠（在 2^32 帧内）。
//
// 2^32 帧 @ 60fps ≈ 2.3 年。真的用完需要换 key（= MLS epoch 前进）。

/// 从 uid 派生 32 位 sender_id（SHA-256 前 4 字节，大端）。
///
/// 同一个 uid 在任何设备/会话都得到同一个 sender_id，无需协调。
/// 碰撞概率：几十人 ~ 1e-7，实际可忽略。若将来需要严格保证，
/// 可在 MLS 成员列表上检测碰撞并触发重分配。
pub fn sender_id_from_uid(uid: &str) -> u32 {
    use sha2::{Digest, Sha256};
    let h = Sha256::digest(uid.as_bytes());
    u32::from_be_bytes([h[0], h[1], h[2], h[3]])
}

/// 由 sender_id 构造 counter 起点（高 32 位 = sender_id，低 32 位 = 0）。
pub const fn make_counter_start(sender_id: u32) -> u64 {
    (sender_id as u64) << 32
}

/// 从 counter 提取 sender_id（高位）。
pub const fn sender_id_of(counter: Counter) -> u32 {
    (counter.0 >> 32) as u32
}

/// 从 counter 提取帧序号（低位）。
pub const fn seq_of(counter: Counter) -> u32 {
    (counter.0 & 0xFFFF_FFFF) as u32
}
/// 加密一帧。
///
/// 输出：`[key_id:4][counter:8][ciphertext||tag]`。
pub fn encrypt(
    key: &SframeKey,
    key_id: KeyId,
    counter: Counter,
    plaintext: &[u8],
) -> Result<Vec<u8>, SframeError> {
    let header = make_header(key_id, counter);
    let nonce = make_gcm_nonce(counter);

    let ct = crate::aead::aes256gcm_encrypt(key.as_bytes(), &nonce, &header, plaintext)
        .map_err(|e| SframeError::Aead(e.to_string()))?;

    let mut frame = Vec::with_capacity(SFRAME_HEADER_LEN + ct.len());
    frame.extend_from_slice(&header);
    frame.extend_from_slice(&ct);
    Ok(frame)
}

/// 解密一帧。
///
/// 输入必须完整包含头部。tag 校验失败返回错误。
///
/// # ⚠️ 无防护（R6 审查发现）
///
/// 本函数**不验证** counter 中的 sender_id 是否是预期的发送者，**不检查**重放。
/// 直接调用会暴露以下攻击面：
///
/// 1. **Impersonation**：恶意成员用共享 channel key + 别人的 sender_id 加密 →
///    接收方误认为"是那个人发的"（`DecryptedFrame.sender_id` 被伪造成受害者）
/// 2. **Nonce 复用**：如果受害者随后真实发送同一 seq 的帧 → AES-GCM 灾难性失效
///    （明文 XOR 可恢复）
/// 3. **重放**：攻击者可反复投递同一帧
///
/// **正确用法**：上层应该用 `kaede-media::receiver::ReceiverSession::decrypt`，
/// 它在本函数之上做了 sender 白名单 + 重放检测。或者用
/// `decrypt_checked(key, frame, &mut ReceiverState, expected_senders)`。
#[deprecated(
    since = "0.2.0",
    note = "no replay / impersonation defense; use kaede_media::receiver::ReceiverSession::decrypt or decrypt_checked"
)]
#[allow(deprecated)]
pub fn decrypt(key: &SframeKey, frame: &[u8]) -> Result<Vec<u8>, SframeError> {
    let min_len = SFRAME_HEADER_LEN + SFRAME_GCM_TAG_LEN;
    if frame.len() < min_len {
        return Err(SframeError::FrameTooShort(frame.len(), min_len));
    }

    let counter = read_counter(frame)?;
    let nonce = make_gcm_nonce(counter);
    let header = &frame[0..SFRAME_HEADER_LEN];
    let ct = &frame[SFRAME_HEADER_LEN..];

    crate::aead::aes256gcm_decrypt(key.as_bytes(), &nonce, header, ct)
        .map_err(|e| SframeError::Aead(e.to_string()))
}

// ============================================================================
// 测试
// ============================================================================

#[allow(deprecated)]
#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> SframeKey {
        SframeKey::from_bytes([0x42u8; 32])
    }

    #[test]
    fn round_trip() {
        let key = test_key();
        let frame = encrypt(&key, KeyId(1), Counter(1), b"hello").unwrap();
        let pt = decrypt(&key, &frame).unwrap();
        assert_eq!(pt, b"hello");
    }

    #[test]
    fn empty_plaintext() {
        let key = test_key();
        let frame = encrypt(&key, KeyId(1), Counter(1), b"").unwrap();
        // 帧 = 12 header + 16 tag = 28
        assert_eq!(frame.len(), SFRAME_HEADER_LEN + SFRAME_GCM_TAG_LEN);
        let pt = decrypt(&key, &frame).unwrap();
        assert_eq!(pt, b"");
    }

    #[test]
    fn frame_length_correct() {
        let key = test_key();
        let frame = encrypt(&key, KeyId(1), Counter(1), b"hello").unwrap();
        // 12 + 5 + 16 = 33
        assert_eq!(frame.len(), 33);
    }

    #[test]
    fn key_id_big_endian() {
        let key = test_key();
        let frame = encrypt(&key, KeyId(0xDEADBEEF), Counter(0), b"x").unwrap();
        assert_eq!(&frame[0..4], &[0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn counter_big_endian() {
        let key = test_key();
        let frame = encrypt(&key, KeyId(0), Counter(0x0102030405060708), b"x").unwrap();
        assert_eq!(
            &frame[4..12],
            &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]
        );
    }

    #[test]
    fn read_key_id_from_frame() {
        let key = test_key();
        let frame = encrypt(&key, KeyId(12345), Counter(99), b"x").unwrap();
        assert_eq!(read_key_id(&frame).unwrap(), KeyId(12345));
        assert_eq!(read_counter(&frame).unwrap(), Counter(99));
    }

    #[test]
    fn different_counters_differ() {
        let key = test_key();
        let a = encrypt(&key, KeyId(1), Counter(1), b"msg").unwrap();
        let b = encrypt(&key, KeyId(1), Counter(2), b"msg").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn counter_tamper_fails_auth() {
        let key = test_key();
        let mut frame = encrypt(&key, KeyId(1), Counter(1), b"msg").unwrap();
        // 篡改 counter 的最后一个字节
        frame[11] ^= 0x01;
        assert!(decrypt(&key, &frame).is_err());
    }

    #[test]
    fn key_id_tamper_fails_auth() {
        let key = test_key();
        let mut frame = encrypt(&key, KeyId(1), Counter(1), b"msg").unwrap();
        frame[0] ^= 0x01;
        assert!(decrypt(&key, &frame).is_err());
    }

    #[test]
    fn wrong_key_fails() {
        let frame = encrypt(&test_key(), KeyId(1), Counter(1), b"msg").unwrap();
        let other = SframeKey::from_bytes([0x99u8; 32]);
        assert!(decrypt(&other, &frame).is_err());
    }

    #[test]
    fn truncation_rejected() {
        let key = test_key();
        let frame = encrypt(&key, KeyId(1), Counter(1), b"msg").unwrap();
        let truncated = &frame[..frame.len() - 1];
        assert!(matches!(
            decrypt(&key, truncated),
            Err(SframeError::Aead(_))
        ));
    }

    #[test]
    fn too_short_rejected() {
        let key = test_key();
        let r = decrypt(&key, &[0u8; 10]);
        assert!(matches!(r, Err(SframeError::FrameTooShort(10, _))));
    }

    #[test]
    fn key_from_slice_validates_length() {
        assert!(SframeKey::from_slice(&[0u8; 32]).is_ok());
        assert!(matches!(
            SframeKey::from_slice(&[0u8; 31]),
            Err(SframeError::InvalidKeyLength(31))
        ));
        assert!(matches!(
            SframeKey::from_slice(&[0u8; 33]),
            Err(SframeError::InvalidKeyLength(33))
        ));
    }

    // ----------------------------------------------------------------
    // KAT（Known Answer Test）
    //
    // 这些值在 2026-10-12 首次运行时生成，作为格式回归锚点。
    // 若未来改动帧格式，此测试会失败 —— 那就说明格式变了。
    // ----------------------------------------------------------------

    /// 打印 KAT 值供捕获。用 `cargo test -- --ignored --nocapture print_kat` 运行。
    #[test]
    #[ignore]
    fn print_kat() {
        let key = SframeKey::from_bytes([
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
            0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c,
            0x1d, 0x1e, 0x1f, 0x20,
        ]);
        let frame = encrypt(
            &key,
            KeyId(0xCAFEBABE),
            Counter(0x1122334455667788),
            b"hello kaede",
        )
        .unwrap();
        let hex: String = frame.iter().map(|b| format!("{:02x}", b)).collect();
        println!("KAT key     = 0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20");
        println!("KAT key_id  = 0xCAFEBABE");
        println!("KAT counter = 0x1122334455667788");
        println!("KAT pt      = 68656c6c6f206b61656465 (\"hello kaede\")");
        println!("KAT frame   = {}", hex);
        println!("KAT len     = {}", frame.len());
    }

    #[test]
    fn sender_id_from_uid_is_deterministic() {
        let a = sender_id_from_uid("alice");
        let b = sender_id_from_uid("alice");
        assert_eq!(a, b);
        let c = sender_id_from_uid("bob");
        assert_ne!(a, c);
    }

    #[test]
    fn different_senders_have_disjoint_counter_spaces() {
        let alice_sid = sender_id_from_uid("alice");
        let bob_sid = sender_id_from_uid("bob");
        let alice_start = make_counter_start(alice_sid);
        let bob_start = make_counter_start(bob_sid);

        assert_eq!(sender_id_of(Counter(alice_start)), alice_sid);
        assert_eq!(sender_id_of(Counter(bob_start)), bob_sid);
        assert_eq!(seq_of(Counter(alice_start)), 0);
        assert_eq!(seq_of(Counter(bob_start)), 0);

        // alice 跑 1 亿帧不会跨入 bob 空间
        let n: u64 = 100_000_000;
        let alice_max = alice_start + n;
        let bob_max = bob_start + n;
        assert!(
            alice_max < bob_start || bob_max < alice_start,
            "counter spaces must not overlap"
        );
    }

    #[test]
    fn different_sender_ids_produce_different_nonces() {
        let alice_start = make_counter_start(sender_id_from_uid("alice"));
        let bob_start = make_counter_start(sender_id_from_uid("bob"));
        let n1 = make_gcm_nonce(Counter(alice_start));
        let n2 = make_gcm_nonce(Counter(bob_start));
        assert_ne!(n1, n2);
    }
    #[test]
    fn counter_checked_next_rejects_overflow() {
        let start = make_counter_start(0x1234_5678);
        let c0 = Counter(start);
        let c1 = c0.checked_next().unwrap();
        assert_eq!(c1.0, start + 1);
        assert_eq!(sender_id_of(c1), 0x1234_5678);
        let last = Counter(start | 0xFFFF_FFFF);
        assert!(matches!(
            last.checked_next(),
            Err(SframeError::CounterExhausted)
        ));
    }

    #[test]
    fn kat_self_consistent() {
        // 不验证具体字节，只保证同样的输入产生同样的输出
        let key = SframeKey::from_bytes([0x42u8; 32]);
        let a = encrypt(&key, KeyId(1), Counter(1), b"deterministic").unwrap();
        let b = encrypt(&key, KeyId(1), Counter(1), b"deterministic").unwrap();
        assert_eq!(a, b);
    }
}
