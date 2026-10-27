const fs = require('fs')
const p = 'crates/kaede-server/src/lib.rs'
let s = fs.readFileSync(p, 'utf8')
// 修复：把 "pub mod categories;\nmod channels;" 改成两行都 pub
const bad = 'pub mod categories;\nmod channels;'
const good = 'pub mod categories;\npub mod channels;'
if (s.includes(bad)) {
  s = s.replace(bad, good)
  console.log('OK: fixed mod channels -> pub')
} else {
  // 兜底：查找真实内容
  const lines = s.split('\n')
  for (let i = 0; i < 20; i++) {
    if (/mod (categories|channels);/.test(lines[i])) console.log('L' + (i+1) + ': ' + lines[i])
  }
}
fs.writeFileSync(p, s)
