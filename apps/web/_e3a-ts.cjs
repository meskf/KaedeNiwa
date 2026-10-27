const fs = require('fs')
let s = fs.readFileSync('src/protocol/Channel.ts', 'utf8')
const old = `hash_tier: string | null, };`
const new_ = `hash_tier: string | null,
/**
 * 所属分类（categories.id）。None = 未分类。
 */
category_id: string | null, };`
if (!s.includes(old)) { console.error('FAIL'); process.exit(1) }
s = s.replace(old, new_)
fs.writeFileSync('src/protocol/Channel.ts', s)
console.log('OK Channel.ts')
