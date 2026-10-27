const fs = require('fs')
// 1. 清理 MainView 里临时 console.log
let s = fs.readFileSync('src/components/MainView.tsx', 'utf8')
s = s.replace(
  `    console.log('[dm-rec] skip self echo', ev.DmReceived.conv_id, ev.DmReceived.ts)`,
  `    // defense: server dm.rs B3 comment claims sender is excluded, but
    // probe6 (2026-10-27) confirmed server DOES echo back. Filter client-side.
    // See HANDOFF: DM self-echo bug in server state.send_to.`
)
fs.writeFileSync('src/components/MainView.tsx', s)
console.log('CLEANED MainView')

// 2. 清理 DMView 里临时 console.log，恢复原始
let d = fs.readFileSync('src/components/DMView.tsx', 'utf8')
d = d.replace(
  `      const ts = Date.now()
      appendDmMessage(activeConvId, { sender: '__me__', ts, plaintext: text })
      console.log('[dm-send] local append ts=', ts)
      try {
        const sent: any = await c.sendDm(activeConvId, text)
        console.log('[dm-send] server resp=', sent, 'type=', typeof sent)
      }
      catch (e: any) { setError(String(e)) }
      finally { setBusy(false) }`,
  `      const ts = Date.now()
      appendDmMessage(activeConvId, { sender: '__me__', ts, plaintext: text })
      try { await c.sendDm(activeConvId, text) }
      catch (e: any) { setError(String(e)) }
      finally { setBusy(false) }`
)
fs.writeFileSync('src/components/DMView.tsx', d)
console.log('CLEANED DMView')
