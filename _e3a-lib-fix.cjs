const fs = require('fs')
const p = 'crates/kaede-protocol/src/lib.rs'
let s = fs.readFileSync(p, 'utf8')
const log = []

// 1. Channel struct: add category_id
const oldChan = `    /// 哈希强度档位（"19" / "64" / "256"）。仅 UI 展示，verify 走 PHC 串真实参数。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash_tier: Option<String>,
}

/// 在线状态。`
const newChan = `    /// 哈希强度档位（"19" / "64" / "256"）。仅 UI 展示，verify 走 PHC 串真实参数。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash_tier: Option<String>,
    /// 所属分类（categories.id）。None = 未分类。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category_id: Option<String>,
}

/// 频道分类（Discord 风格的可折叠分组）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Category {
    pub id: String,
    pub name: String,
    /// 排序位置（越小越前）。
    pub position: i64,
    pub created_at: i64,
}

/// 在线状态。`
if (!s.includes(oldChan)) { console.error('FAIL: Channel struct anchor'); process.exit(1) }
s = s.replace(oldChan, newChan)
log.push('OK: Channel.category_id + Category struct')

fs.writeFileSync(p, s)
log.forEach(l => console.log(l))
console.log('DONE')
