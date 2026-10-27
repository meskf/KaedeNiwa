const fs = require('fs')
const p = 'crates/kaede-server/Cargo.toml'
let s = fs.readFileSync(p, 'utf8')
if (s.includes('default-run')) {
  console.log('SKIP: default-run already set')
} else {
  // 在 [package] 下加 default-run = "kaede-server"
  // 找到 [package] 块内 name = "kaede-server" 那行后插入
  const m = s.match(/(\[package\][\s\S]*?name\s*=\s*"kaede-server"\s*\r?\n)/)
  if (!m) { console.error('FAIL: package name anchor'); process.exit(1) }
  s = s.replace(m[1], m[1] + 'default-run = "kaede-server"\n')
  fs.writeFileSync(p, s)
  console.log('OK: default-run added')
}
