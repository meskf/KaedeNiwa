const fs = require('fs')
const log = []
function rep(file, from, to, name) {
  let s = fs.readFileSync(file, 'utf8')
  if (!s.includes(from)) { log.push('MISS: ' + name); return }
  s = s.replace(from, to)
  fs.writeFileSync(file, s)
  log.push('OK: ' + name)
}

// ---- channel.rs struct + category_id ----
rep(
  'crates/kaede-protocol/src/channel.rs',
  `      #[serde(default, skip_serializing_if = "Option::is_none")]
      pub hash_tier: Option<String>,
  }`,
  `      #[serde(default, skip_serializing_if = "Option::is_none")]
      pub hash_tier: Option<String>,
      /// 所属分类（categories.id）。None = 未分类。
      #[serde(default, skip_serializing_if = "Option::is_none")]
      pub category_id: Option<String>,
  }`,
  'Channel.category_id')

// ---- new Category struct (append at end of channel.rs) ----
const catStruct = `

/// 频道分类（Discord 风格的可折叠分组）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Category {
    pub id: String,
    pub name: String,
    /// 排序位置（越小越前）。
    pub position: i64,
    pub created_at: i64,
}
`
fs.appendFileSync('crates/kaede-protocol/src/channel.rs', catStruct)
log.push('OK: Category struct appended')
log.forEach(l => console.log(l))
