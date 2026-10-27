const fs = require('fs')
const p = 'crates/kaede-server/Cargo.toml'
let s = fs.readFileSync(p, 'utf8')
if (s.includes('name = "promote"')) {
  console.log('SKIP: promote already in Cargo.toml')
} else {
  const oldBin = `[[bin]]
name = "kaede-server"
path = "src/main.rs"`
  const newBin = `[[bin]]
name = "kaede-server"
path = "src/main.rs"

[[bin]]
name = "promote"
path = "src/bin/promote.rs"`
  if (!s.includes(oldBin)) { console.error('FAIL: bin anchor'); process.exit(1) }
  s = s.replace(oldBin, newBin)
  fs.writeFileSync(p, s)
  console.log('OK: Cargo.toml promote bin')
}
