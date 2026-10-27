const fs = require('fs')
const p = 'start-all.ps1'
let s = fs.readFileSync(p, 'utf8')
const old = 'cargo run -p kaede-server --features dev-mode'
const new_ = 'cargo run -p kaede-server --bin kaede-server --features dev-mode'
if (s.includes(new_)) { console.log('SKIP: --bin already set') }
else if (s.includes(old)) {
  s = s.replace(old, new_)
  fs.writeFileSync(p, s)
  console.log('OK: start-all.ps1 --bin added')
} else {
  console.log('MISS: cargo run line not found in start-all.ps1')
}
