const fs = require('fs')
let s = fs.readFileSync('src/components/MainView.tsx', 'utf8')
// 在 ChannelEpochChanged 分支前插入 ChannelJoined 分支
const anchor = `        if (ev.ChannelEpochChanged) {`
const insert = `        if (ev.ChannelJoined) {
          const jchId = (ev.ChannelJoined as any).channel_id || (ev.ChannelJoined as any).id || useChatStore.getState().currentChannel
          if (jchId === useChatStore.getState().currentChannel) {
            (async () => {
              try {
                const c = getClient()
                if (!c) return
                // 稍等，确保 group 状态稳定
                await new Promise(r => setTimeout(r, 800))
                const hist = await c.load_history(jchId, 50)
                const arr = typeof hist === 'string' ? JSON.parse(hist) : hist
                const list = Array.isArray(arr) ? arr : []
                if (list.length > 0) {
                  useChatStore.getState().setMessages(jchId, list)
                  console.log('[ChannelJoined] reloaded history: ' + list.length + ' msgs')
                } else {
                  console.log('[ChannelJoined] history still empty (group might not be ready)')
                }
              } catch (e) { console.error('[ChannelJoined] history reload failed:', e) }
            })()
          }
        }
${anchor}`
if (!s.includes(anchor)) { console.error('FAIL anchor'); process.exit(1) }
s = s.replace(anchor, insert)
fs.writeFileSync('src/components/MainView.tsx', s)
console.log('OK MainView')
