const fs = require('fs')

// ===== 1. MainView: size 覆盖 + Add failed 静默 =====
let s = fs.readFileSync('src/components/MainView.tsx', 'utf8')

const old1 = `        const raw = await client.uploadAttachment(currentChannel, p.name, p.mime, p.bytes)
        const att = typeof raw === 'string' ? JSON.parse(raw) : raw
        uploaded.push(att)`
const new1 = `        const raw = await client.uploadAttachment(currentChannel, p.name, p.mime, p.bytes)
        const att = typeof raw === 'string' ? JSON.parse(raw) : raw
        // 服务端只看到密文，size 是密文大小（原文 + 29B 加密开销）。
        // 覆盖成原文大小；这个 att 会被 JSON.stringify 进 E2EE payload，
        // 所以 Alice 本地显示和 Bob 收到的都是真实原文大小。
        att.size = p.size
        if (!att.mime || att.mime === 'application/octet-stream') {
          att.mime = p.mime
        }
        uploaded.push(att)`
if (!s.includes(old1)) { console.error('FAIL: upload size'); process.exit(1) }
s = s.replace(old1, new1)
console.log('OK: upload size override')

// Add failed 静默（2 处）
const old2a = `          client.add_member_to_channel(useChatStore.getState().currentChannel, requester).then(() => {
            useChatStore.getState().setPeerStatus(\`Added \${requester}\`)
          }).catch((e: any) => useChatStore.getState().setPeerStatus(\`Add failed: \${e}\`))`
const new2a = `          client.add_member_to_channel(useChatStore.getState().currentChannel, requester).then(() => {
            useChatStore.getState().setPeerStatus(\`Added \${requester}\`)
          }).catch((e: any) => {
            const msg = String(e)
            if (msg.includes('already in group') || msg.includes('already_member')) return
            useChatStore.getState().setPeerStatus(\`Add failed: \${msg}\`)
          })`
if (!s.includes(old2a)) { console.error('FAIL: add1'); process.exit(1) }
s = s.replace(old2a, new2a)
console.log('OK: Add failed silence (1)')

const old2b = `            } catch (e) {
              useChatStore.getState().setPeerStatus(\`Add failed: \${e}\`)
            }`
const new2b = `            } catch (e) {
              const msg = String(e)
              if (msg.includes('already in group') || msg.includes('already_member')) return
              useChatStore.getState().setPeerStatus(\`Add failed: \${msg}\`)
            }`
if (!s.includes(old2b)) { console.error('FAIL: add2'); process.exit(1) }
s = s.replace(old2b, new2b)
console.log('OK: Add failed silence (2)')

fs.writeFileSync('src/components/MainView.tsx', s)
console.log('OK MainView')

// ===== 2. MessageAttachments: 宽度约束 =====
let m = fs.readFileSync('src/components/MessageAttachments.tsx', 'utf8')
const oldBtn = `      className="mt-1 flex items-center gap-2 px-3 py-2 rounded-lg bg-[var(--bg-active)] hover:bg-[var(--bg-hover)] text-left transition-colors"`
const newBtn = `      className="mt-1 inline-flex items-center gap-2 px-3 py-2 rounded-lg bg-[var(--bg-active)] hover:bg-[var(--bg-hover)] text-left transition-colors max-w-[400px]"`
if (!m.includes(oldBtn)) { console.error('FAIL: att btn'); process.exit(1) }
m = m.replace(oldBtn, newBtn)
// 内层 name div 加 min-w-0 让 truncate 生效
const oldName = `<div className="text-xs font-medium text-[var(--text-main)] truncate max-w-[200px]">{att.name}</div>`
const newName = `<div className="text-xs font-medium text-[var(--text-main)] truncate max-w-[280px]">{att.name}</div>`
if (!m.includes(oldName)) { console.error('FAIL: att name'); process.exit(1) }
m = m.replace(oldName, newName)
fs.writeFileSync('src/components/MessageAttachments.tsx', m)
console.log('OK MessageAttachments')

console.log('DONE')
