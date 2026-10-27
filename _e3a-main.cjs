const fs = require('fs')
const p = 'crates/kaede-server/src/main.rs'
let s = fs.readFileSync(p, 'utf8')
const log = []

// 1. import 里加 categories
const oldImport = `use kaede_server::{
    admin, attachments, channels, config::Config, db, dm, friends, http, livekit, members, reads,
    sessions_api, state::AppState, ws,
};`
const newImport = `use kaede_server::{
    admin, attachments, categories, channels, config::Config, db, dm, friends, http, livekit,
    members, reads, sessions_api, state::AppState, ws,
};`
if (!s.includes(oldImport)) { console.error('FAIL: import'); process.exit(1) }
s = s.replace(oldImport, newImport)
log.push('OK: import categories')

// 2. router merge 里加 categories
const oldMerge = `            .merge(livekit::router(state.clone()))
            .merge(channels::router(state.clone()))`
const newMerge = `            .merge(livekit::router(state.clone()))
            .merge(categories::router(state.clone()))
            .merge(channels::router(state.clone()))`
if (!s.includes(oldMerge)) { console.error('FAIL: merge'); process.exit(1) }
s = s.replace(oldMerge, newMerge)
log.push('OK: merge categories::router')

fs.writeFileSync(p, s)
log.forEach(l => console.log(l))
