const fs = require('fs')
let s = fs.readFileSync('src/components/MainView.tsx', 'utf8')

// ===== Patch 1: initChannel effect 加 retry + safe merge =====
const oldEffect = `  useEffect(() => {
    const initChannel = async () => {
      const client = getClient()
      if (!client) { setTimeout(() => initChannel(), 200); return }
      try {
        await client.join_channel(currentChannel)
        const hist = await client.load_history(currentChannel, 50)
        const parsedHist = typeof hist === 'string' ? JSON.parse(hist) : hist
        setMessages(currentChannel, Array.isArray(parsedHist) ? parsedHist : [])
      } catch (e) {
        console.error('initChannel failed:', e)
        setMessages(currentChannel, [])
      }
    }
    initChannel()
  }, [currentChannel, setMessages])`
const newEffect = `  useEffect(() => {
    let cancelled = false
    let attempt = 0
    const CH = currentChannel
    const initChannel = async () => {
      if (cancelled) return
      const client = getClient()
      if (!client) { setTimeout(initChannel, 200); return }
      try {
        await client.join_channel(CH)
        if (cancelled) return
        const hist = await client.load_history(CH, 50)
        if (cancelled) return
        const parsedHist = typeof hist === 'string' ? JSON.parse(hist) : hist
        const arr = Array.isArray(parsedHist) ? parsedHist : []
        const existing = useChatStore.getState().messages[CH] || []
        if (existing.length === 0 && arr.length > 0) {
          setMessages(CH, arr)
        } else if (arr.length === 0 && existing.length === 0 && attempt < 10) {
          attempt++
          setTimeout(initChannel, 1500)
        }
      } catch (e) {
        console.error('[initChannel] attempt ' + attempt + ' failed:', e)
        if (!cancelled && attempt < 10) {
          attempt++
          setTimeout(initChannel, 1500)
        } else if (!cancelled) {
          const existing = useChatStore.getState().messages[CH] || []
          if (existing.length === 0) setMessages(CH, [])
        }
      }
    }
    initChannel()
    return () => { cancelled = true }
  }, [currentChannel, setMessages])`
if (!s.includes(oldEffect)) { console.error('FAIL: initChannel effect'); process.exit(1) }
s = s.replace(oldEffect, newEffect)
console.log('OK: initChannel retry')

// ===== Patch 2: AppMessageReceived 保留 attachments =====
const oldApp = `        if (ev.AppMessageReceived) {
          if (ev.AppMessageReceived.channel_id !== useChatStore.getState().currentChannel) return
          try {
            const { text } = extractMessage(ev.AppMessageReceived.payload)
            useChatStore.getState().addMessage(useChatStore.getState().currentChannel, {
              sender: ev.AppMessageReceived.from || 'Unknown',
              plaintext: text,
              ts: Date.now(),
            })
          } catch (err) { console.error('Parse message failed:', err) }
        }`
const newApp = `        if (ev.AppMessageReceived) {
          if (ev.AppMessageReceived.channel_id !== useChatStore.getState().currentChannel) return
          try {
            const { text, attachments } = extractMessage(ev.AppMessageReceived.payload)
            useChatStore.getState().addMessage(useChatStore.getState().currentChannel, {
              sender: ev.AppMessageReceived.from || 'Unknown',
              plaintext: text,
              ts: (ev.AppMessageReceived as any).ts || Date.now(),
              attachments: (ev.AppMessageReceived as any).attachments || attachments,
            })
          } catch (err) { console.error('Parse message failed:', err) }
        }`
if (!s.includes(oldApp)) { console.error('FAIL: AppMessageReceived'); process.exit(1) }
s = s.replace(oldApp, newApp)
console.log('OK: AppMessageReceived attachments')

fs.writeFileSync('src/components/MainView.tsx', s)
console.log('DONE')
