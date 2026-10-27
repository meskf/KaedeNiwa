const fs = require('fs')
let s = fs.readFileSync('src/App.tsx', 'utf8')
const old = `    ;(window as any).__kaede_debug__ = {`
const new_ = `    ;(window as any).__kaede_debug__ = {
      loadHistory: async (ch: string, limit = 50) => {
        const c = getClient()
        if (!c) throw new Error('client not ready')
        const raw = await c.load_history(ch, limit)
        const arr = typeof raw === 'string' ? JSON.parse(raw) : raw
        return Array.isArray(arr) ? arr : { raw }
      },
      joinChannel: async (ch: string) => {
        const c = getClient()
        if (!c) throw new Error('client not ready')
        return await c.join_channel(ch)
      },`
if (!s.includes(old)) { console.error('FAIL App.tsx'); process.exit(1) }
s = s.replace(old, new_)
fs.writeFileSync('src/App.tsx', s)
console.log('OK App.tsx')
