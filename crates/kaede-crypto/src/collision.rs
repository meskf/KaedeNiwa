//! Sender_id 碰撞检测。
//!
//! # 背景
//!
//! SFrame 的 `sender_id` = `SHA256(uid)[0..4]`，32 位。同一个 channel 里
//! 不同 sender 各自从 counter 高 32 位 = sender_id 开始加密，靠这个机制
//! 分离 AES-GCM nonce 空间。
//!
//! 碰撞概率（生日悖论）：n 个 sender 碰撞概率约 1 - exp(-n^2/2^33)。
//! 几十人约 1e-7，实际可忽略。但一旦碰撞，两个 sender 的 counter 空间
//! 重叠 → nonce 复用 → AES-GCM 灾难性失效。
//!
//! # 用法
//!
//! 在 Add 新成员到 MLS 群之前，调用方用**当前群内 uid 列表**检查：
//!
//! ```ignore
//! let members: Vec<&str> = group.members().map(|m| m.uid.as_str()).collect();
//! if let Some(other) = find_sender_id_collision(new_uid, members) {
//!     // 拒绝 Add，提示用户换 uid
//! }
//! ```
//!
//! uid 是 login_id（不可改），所以碰撞时的应对是**拒绝该成员加入** +
//! 提示用户改名。这是唯一正确做法 —— sender_id 是纯函数，不能靠
//! "退避"绕过（双端必须独立算出同一个 sender_id）。

use crate::sframe::sender_id_from_uid;

/// 检测 `candidate` 的 sender_id 是否与 `existing` 中任一 uid 碰撞。
///
/// - 排除 `candidate == uid` 自身
/// - 返回第一个碰撞的 uid
/// - 无碰撞返回 `None`
pub fn find_sender_id_collision<'a, I>(candidate: &str, existing: I) -> Option<String>
where
    I: IntoIterator<Item = &'a str>,
{
    let target = sender_id_from_uid(candidate);
    for uid in existing {
        if uid != candidate && sender_id_from_uid(uid) == target {
            return Some(uid.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn no_collision_in_small_set() {
        let existing = ["alice", "bob", "carol", "dave", "eve"];
        for uid in &existing {
            assert!(
                find_sender_id_collision(uid, existing.iter().copied()).is_none(),
                "{} unexpectedly collides",
                uid
            );
        }
    }

    #[test]
    fn self_excluded() {
        let existing = ["alice"];
        assert!(find_sender_id_collision("alice", existing.iter().copied()).is_none());
    }

    #[test]
    fn candidate_not_in_set_returns_none_for_normal_names() {
        let existing = ["alice", "bob"];
        assert!(find_sender_id_collision("carol", existing.iter().copied()).is_none());
    }

    #[test]
    fn empty_set_returns_none() {
        assert!(find_sender_id_collision("alice", std::iter::empty::<&str>()).is_none());
    }

    #[test]
    fn detects_synthetic_collision() {
        // 生日悖论：150k 个 uid 里 32 位 sender_id 碰撞的期望数 ≈ 2.6，
        // 几乎肯定能找到。找到就验证检测；找不到（概率 <8%）跳过。
        let mut seen: HashMap<u32, String> = HashMap::new();
        let mut found: Option<(String, String)> = None;
        for i in 0..150_000u32 {
            let uid = format!("u{}", i);
            let sid = sender_id_from_uid(&uid);
            if let Some(prev) = seen.get(&sid) {
                found = Some((prev.clone(), uid));
                break;
            }
            seen.insert(sid, uid);
        }
        if let Some((a, b)) = found {
            let got = find_sender_id_collision(&b, [a.as_str()]);
            assert_eq!(got.as_deref(), Some(a.as_str()));
        }
    }
}
