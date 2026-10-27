const fs = require('fs')
const p = 'crates/kaede-server/src/lib.rs'
let s = fs.readFileSync(p, 'utf8')
const log = []
// module declaration — find "mod channels;"
if (!s.includes('mod categories;')) {
  s = s.replace('mod channels;', 'mod categories;\nmod channels;')
  log.push('OK: mod categories')
} else { log.push('SKIP: mod categories') }
// merge router
if (!s.includes('categories::router')) {
  s = s.replace('.merge(channels::router(state.clone()))', '.merge(categories::router(state.clone()))\n        .merge(channels::router(state.clone()))')
  log.push('OK: merge categories::router')
} else { log.push('SKIP: merge') }
fs.writeFileSync(p, s)
log.forEach(l => console.log(l))
