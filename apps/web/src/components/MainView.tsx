import { useState, useEffect, useRef } from 'react'
import { useAuthStore } from '../stores/auth'
import { useChatStore } from '../stores/chat'
import { getClient, scheduleReconnect } from '../wasm/client'
import { connectMedia, disconnectMedia, updateE2EEKey } from '../lib/livekit'
import { useMediaState } from '../hooks/useLiveKitRoom'
import { MediaBar } from './MediaBar'
import { ScreenShareView } from './ScreenShareView'
import { IconHash, IconVolume, IconSun, IconMoon, IconSend, IconSearch, IconUsers } from './icons'
import { DMView } from './DMView'
import { FriendsView } from './FriendsView'

export function MainView({ onLogout }: { onLogout: () => void }) {
  const loginId = useAuthStore(s => s.loginId)
  const channelList = useChatStore(s => s.channelList)
  const serverMembers = useChatStore(s => s.serverMembers)
  const setServerMembers = useChatStore(s => s.setServerMembers)
  const setChannelList = useChatStore(s => s.setChannelList)
  const {
    currentChannel, setChannel,
    activeView, setActiveView,
    messages, setMessages, addMessage,
    pendingMessages, addPending, removePending, markPendingFailed, incrementRetry,
    peerStatus,
  } = useChatStore()
  const [input, setInput] = useState('')
  const [theme, setTheme] = useState<'dark' | 'light'>('light')
  const [showMembers, setShowMembers] = useState(true)
  const addMemberQueueRef = useRef<Promise<void>>(Promise.resolve())
  const messagesEndRef = useRef<HTMLDivElement>(null)

  const media = useMediaState()
  const mediaRef = useRef(media)
  useEffect(() => { mediaRef.current = media }, [media])

  const currentMessages = messages[currentChannel] || []
  const currentPending = pendingMessages.filter(p => p.channelId === currentChannel)
  const inVoice = media.connected && media.channelId === currentChannel

  useEffect(() => {
    let cancelled = false
    const loadChannels = async () => {
      const client = getClient()
      if (!client) { setTimeout(loadChannels, 200); return }
      try {
        const raw = await client.listChannels()
        const arr = typeof raw === 'string' ? JSON.parse(raw) : raw
        if (cancelled) return
        const mapped = (arr || []).map((c: any) => ({
          id: c.id,
          name: c.name,
          kind: c.kind,
          position: Number(c.position),
          is_private: !!c.is_private,
        }))
        setChannelList(mapped)
      } catch (e) {
        console.error('[channels] list:', e)
      }
    }
    loadChannels()
    return () => { cancelled = true }
  }, [setChannelList])

  useEffect(() => {
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
  }, [currentChannel, setMessages])

  useEffect(() => {
    let cancelled = false
    const tryRegister = () => {
      if (cancelled) return
      const client = getClient()
      if (!client) { setTimeout(tryRegister, 200); return }

      client.setOnEvent((rawEv: any) => {
        let ev = rawEv
        if (typeof rawEv === 'string') {
          try { ev = JSON.parse(rawEv) } catch { return }
        }
        if (ev.Disconnected) { scheduleReconnect(); return }

        const eventDiv = document.createElement('div')
        eventDiv.setAttribute('data-testid', 'raw-event')
        eventDiv.innerText = 'EVENT: ' + JSON.stringify(ev)
        eventDiv.style.display = 'none'
        document.body.appendChild(eventDiv)
        const all = document.querySelectorAll('[data-testid="raw-event"]')
        if (all.length > 500) all[0].remove()

        const st = useChatStore.getState()
        const ch = st.currentChannel

        if (ev.Joined) {
          const peers = ev.Joined.peers || []
          setServerMembers([loginId, ...peers])
          if (peers.length === 0) {
            try {
              const created = client.ensure_channel(ch)
              if (created) useChatStore.getState().setIsAuthority(true)
            } catch (e) { console.error('ensure_channel failed:', e) }
          }
        }
        if (ev.ChannelMemberJoined) {
          const { channel_id, requester } = ev.ChannelMemberJoined
          const st = useChatStore.getState()
          console.log(`[CMJ] ENTER ch=${channel_id} req=${requester} isAuth=${st.isAuthority} curCh=${st.currentChannel}`)
          if (channel_id !== st.currentChannel) { console.log('[CMJ] SKIP: not current channel'); return }
          if (!st.isAuthority) { console.log('[CMJ] SKIP: not authority'); return }
          console.log('[CMJ] calling add_member_to_channel...')
          client.add_member_to_channel(useChatStore.getState().currentChannel, requester).then(async () => {
            console.log('[CMJ] add_member_to_channel OK')
            useChatStore.getState().setPeerStatus(`Added ${requester}`)
            // ★ epoch 已推进，必须刷新 E2EE key（否则本端 keyProvider 还是旧 epoch 的 key）
            try {
              const m = mediaRef.current
              const c = getClient()
              console.log(`[CMJ] rotation check: hasClient=${!!c} connected=${m.connected} mediaCh=${m.channelId} target=${channel_id}`)
              if (c && m.connected && m.channelId === channel_id) {
                const tokenJson = await c.begin_media_session(channel_id)
                const tokenData = typeof tokenJson === 'string' ? JSON.parse(tokenJson) : tokenJson
                if (tokenData.e2ee_seed) {
                  await updateE2EEKey(tokenData.e2ee_seed)
                  console.log(`[E2EE] Key rotated for ${channel_id} after add_member`)
                }
              }
            } catch (e) { console.error('E2EE key rotate after add failed:', e) }
          }).catch((e: any) => useChatStore.getState().setPeerStatus(`Add failed: ${e}`))
        }
        if (ev.PeerJoined) {
          const uid = ev.PeerJoined.uid
          setServerMembers(Array.from(new Set([...useChatStore.getState().serverMembers, uid])))
          if (!useChatStore.getState().isAuthority) return
          useChatStore.getState().setPeerStatus(`Adding ${uid}...`)
          addMemberQueueRef.current = addMemberQueueRef.current.then(async () => {
            try {
              await client.add_member_to_channel(useChatStore.getState().currentChannel, uid)
              useChatStore.getState().setPeerStatus(`Added ${uid}`)
              // ★ 同 ChannelMemberJoined：epoch 变了要刷新 key
              try {
                const m = mediaRef.current
                const curCh = useChatStore.getState().currentChannel
                if (m.connected && m.channelId === curCh) {
                  const tokenJson = await client.begin_media_session(curCh)
                  const tokenData = typeof tokenJson === 'string' ? JSON.parse(tokenJson) : tokenJson
                  if (tokenData.e2ee_seed) {
                    await updateE2EEKey(tokenData.e2ee_seed)
                    console.log(`[E2EE] Key rotated for ${curCh} after PeerJoined add`)
                  }
                }
              } catch (e) { console.error('E2EE key rotate after peer add failed:', e) }
              await new Promise<void>(r => setTimeout(r, 150))
            } catch (e) {
              useChatStore.getState().setPeerStatus(`Add failed: ${e}`)
            }
          })
        }
        if (ev.ChannelSync) {
          const channelId = ev.ChannelSync.channel_id
          const payload = new Uint8Array(ev.ChannelSync.payload)
          const firstByte = payload.length > 0 ? payload[0] : -1
          console.log(`[ChannelSync] ch=${channelId} epoch=${ev.ChannelSync.epoch} len=${payload.length} first=0x${firstByte.toString(16).padStart(2,'0')}`)
          client.process_channel_sync(channelId, payload).then(async () => {
            useChatStore.getState().setPeerStatus('Synced')
            console.log(`[ChannelSync] ch=${channelId} OK`)
            const m = mediaRef.current
            if (m.connected && m.channelId === channelId) {
              try {
                const c = getClient()
                if (!c) return
                const tokenJson = await c.begin_media_session(channelId)
                const tokenData = typeof tokenJson === 'string' ? JSON.parse(tokenJson) : tokenJson
                if (tokenData.e2ee_seed) await updateE2EEKey(tokenData.e2ee_seed)
                console.log(`[E2EE] Key rotated for ${channelId} after ChannelSync`)
              } catch (e) { console.error('E2EE key rotate failed:', e) }
            }
          }).catch((e: any) => {
            console.error(`[ChannelSync] ch=${channelId} FAILED:`, String(e))
            useChatStore.getState().setPeerStatus(`Sync failed: ${e}`)
          })
        }
        if (ev.MlsCommit) {
          console.log(`[MlsCommit] ch=${ev.MlsCommit.channel_id} epoch=${ev.MlsCommit.epoch} len=${ev.MlsCommit.payload?.length}`)
        }
        if (ev.MlsEpochChanged) {
          const m = mediaRef.current
          const curCh = useChatStore.getState().currentChannel
          if (m.connected && m.channelId === curCh) {
            (async () => {
              try {
                const c = getClient()
                if (!c) return
                const tokenJson = await c.begin_media_session(curCh)
                const tokenData = typeof tokenJson === 'string' ? JSON.parse(tokenJson) : tokenJson
                if (tokenData.e2ee_seed) await updateE2EEKey(tokenData.e2ee_seed)
              } catch (e) { console.error('E2EE key rotate failed:', e) }
            })()
          }
        }
        if (ev.ChannelMessage) {
          if (ev.ChannelMessage.channel_id !== useChatStore.getState().currentChannel) return
          try {
            const plaintext = ev.ChannelMessage.plaintext
            let text = ''
            if (Array.isArray(plaintext)) text = new TextDecoder().decode(new Uint8Array(plaintext))
            else if (typeof plaintext === 'string') text = plaintext
            useChatStore.getState().addMessage(useChatStore.getState().currentChannel, {
              sender: ev.ChannelMessage.from || 'Unknown',
              plaintext: text,
              ts: ev.ChannelMessage.ts || Date.now(),
            })
          } catch (err) { console.error('Parse message failed:', err) }
          return
        }
        if (ev.DmReceived) {
          try {
            const plaintext = ev.DmReceived.plaintext
            let text = ''
            if (Array.isArray(plaintext)) text = new TextDecoder().decode(new Uint8Array(plaintext))
            else if (typeof plaintext === 'string') text = plaintext
            useChatStore.getState().appendDmMessage(ev.DmReceived.conv_id, {
              sender: ev.DmReceived.from || 'Unknown',
              plaintext: text,
              ts: ev.DmReceived.ts || Date.now(),
            })
          } catch (err) { console.error('Parse DM failed:', err) }
          return
        }
        if (ev.FriendsChanged) {
          const c = getClient()
          if (c) {
            c.listFriends().then((raw: any) => {
              const arr = typeof raw === 'string' ? JSON.parse(raw) : raw
              useChatStore.getState().setFriends(arr || [])
            }).catch((e: any) => console.error('[friends] refresh:', e))
          }
          return
        }
        if (ev.AppMessageReceived) {
          if (ev.AppMessageReceived.channel_id !== useChatStore.getState().currentChannel) return
          try {
            const text = new TextDecoder().decode(new Uint8Array(ev.AppMessageReceived.payload))
            useChatStore.getState().addMessage(useChatStore.getState().currentChannel, {
              sender: ev.AppMessageReceived.from || 'Unknown',
              plaintext: text,
              ts: Date.now(),
            })
          } catch (err) { console.error('Parse message failed:', err) }
        }
      })
    }
    tryRegister()
    return () => { cancelled = true }
  }, [])

  useEffect(() => {
    const timer = setInterval(async () => {
      const client = getClient()
      if (!client) return
      const failed = useChatStore.getState().pendingMessages.filter(
        p => p.status === 'failed' && p.channelId === currentChannel && p.retries < 5
      )
      for (const p of failed) {
        try {
          incrementRetry(p.id)
          await client.send_message(currentChannel, p.text)
          removePending(p.id)
          addMessage(currentChannel, { sender: loginId, plaintext: p.text, ts: p.ts })
        } catch (e) {
          markPendingFailed(p.id)
        }
      }
    }, 5000)
    return () => clearInterval(timer)
  }, [currentChannel, loginId, incrementRetry, removePending, addMessage, markPendingFailed])

  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' })
  }, [currentMessages.length, currentPending.length])

  useEffect(() => {
    document.documentElement.setAttribute('data-theme', theme === 'light' ? 'light' : 'dark')
  }, [theme])

  const handleJoinMedia = async () => {
    const client = getClient()
    if (!client) return
    try {
      useChatStore.getState().setPeerStatus('Joining voice...')
      await client.join_channel(currentChannel)
      await new Promise(r => setTimeout(r, 500))
      const tokenJson = await client.begin_media_session(currentChannel)
      const tokenData = typeof tokenJson === 'string' ? JSON.parse(tokenJson) : tokenJson
      await connectMedia({
        wsUrl: tokenData.ws_url,
        token: tokenData.token,
        channelId: currentChannel,
        e2eeKeyB64: tokenData.e2ee_seed,
        disableAdaptive: (window as any).__KAEDE_TEST_DISABLE_ADAPTIVE__ === true,
      })
      useChatStore.getState().setPeerStatus('Joined voice' + (tokenData.e2ee_seed ? ' (E2EE)' : ''))
    } catch (e: any) {
      useChatStore.getState().setPeerStatus('Join voice failed: ' + e)
    }
  }

  const handleLeaveMedia = async () => {
    const client = getClient()
    if (client) {
      try { await client.leave_media_room(currentChannel) } catch (e) { /* ignore */ }
    }
    await disconnectMedia()
    useChatStore.getState().setPeerStatus('')
  }

  const handleSend = async (e?: React.FormEvent) => {
    e?.preventDefault()
    if (!input.trim()) return
    const text = input.trim()
    const client = getClient()
    if (!client) return
    const id = Date.now().toString() + Math.random().toString(36).slice(2)
    addPending({ id, channelId: currentChannel, text, ts: Date.now(), status: 'sending', retries: 0 })
    setInput('')
    try {
      await client.send_message(currentChannel, text)
      removePending(id)
      addMessage(currentChannel, { sender: loginId, plaintext: text, ts: Date.now() })
    } catch (e) {
      console.error('send failed:', e)
      markPendingFailed(id)
    }
  }

  const getInitial = (str: string) => str ? str.charAt(0).toUpperCase() : '?'

  const textChannels = channelList.filter(c => c.kind === 'text')
  const voiceChannels = channelList.filter(c => c.kind === 'voice')
  const currentChannelInfo = channelList.find(c => c.id === currentChannel)
  const currentChannelName = currentChannelInfo?.name || currentChannel
  const isVoiceChannel = currentChannelInfo ? currentChannelInfo.kind === 'voice' : false

  return (
    <div className="h-screen w-screen bg-[var(--bg-app)] flex flex-col text-[var(--text-main)] overflow-hidden font-sans">
      
      {/* 上半部分：适度留白包裹 */}
      <div className="flex-1 flex flex-col min-h-0 px-8 md:px-12 lg:px-16 pt-8 md:pt-12 pb-4">
        
        {/* 顶栏 */}
        <header className="relative h-[40px] shrink-0 flex items-center mb-10">
          <div className="w-[200px] shrink-0 flex items-center text-[16px] font-bold tracking-wide">
            KaedeNiwa
          </div>
          <div className="absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 w-full max-w-[420px] px-4">
            <div className="relative flex items-center">
              <IconSearch className="absolute left-3.5 w-4 h-4 text-[var(--text-muted)]" />
              <input
                type="text"
                placeholder="Search..."
                className="w-full bg-[var(--bg-hover)] text-[var(--text-main)] text-[13px] rounded-md pl-10 pr-4 py-2 outline-none placeholder-[var(--text-muted)]"
              />
            </div>
          </div>
          <div className="flex-1" />
          <div className="flex items-center gap-2 shrink-0">
            <button
              onClick={() => setShowMembers(!showMembers)}
              className={`w-8 h-8 flex items-center justify-center rounded-md transition-colors ${showMembers ? 'text-[var(--text-main)] bg-[var(--bg-hover)]' : 'text-[var(--text-muted)] hover:text-[var(--text-main)] hover:bg-[var(--bg-hover)]'}`}
              title="Toggle Members"
            >
              <IconUsers className="w-4 h-4" />
            </button>
            <button
              onClick={() => setTheme(t => t === 'dark' ? 'light' : 'dark')}
              className="w-8 h-8 flex items-center justify-center rounded-md text-[var(--text-muted)] hover:text-[var(--text-main)] hover:bg-[var(--bg-hover)] transition-colors"
            >
              {theme === 'dark' ? <IconSun className="w-4 h-4" /> : <IconMoon className="w-4 h-4" />}
            </button>
          </div>
        </header>

        {/* 主体区 (左右侧边栏绝对定位，主聊天区绝对居中) */}
        <div className="flex-1 relative flex min-h-0">
          
          {/* 左侧侧边栏 */}
          <aside className="absolute left-0 top-0 bottom-0 w-[200px] flex flex-col z-10">
            <div className="h-[40px] flex items-center gap-3 mb-6">
              <div className="w-8 h-8 rounded-md bg-[var(--bg-hover)] flex items-center justify-center text-[var(--text-main)] font-bold text-sm shrink-0">
                K
              </div>
              <div className="flex flex-col min-w-0">
                <span className="text-[14px] font-semibold leading-tight truncate">Kaede Server</span>
                <span className="text-[11px] text-[var(--text-muted)] truncate">Workspace</span>
              </div>
            </div>

            <div className="flex-1 overflow-y-auto pb-4">
              <div className="mb-2 text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase">
                Text Channels
              </div>
              {textChannels.map(ch => (
  <div
    key={ch.id}
    data-testid={`channel-item-${ch.id}`}
    onClick={() => { setChannel(ch.id); setActiveView('chat'); }}
    className={`px-3 py-2 mb-1 rounded-md text-[14px] cursor-pointer flex items-center gap-2.5 transition-colors ${
      currentChannel === ch.id && activeView === 'chat'
        ? 'bg-[var(--bg-active)] text-[var(--text-main)] font-medium'
        : 'text-[var(--text-muted)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-main)]'
    }`}
  >
    <IconHash className="w-4 h-4 opacity-50" /> {ch.name}
  </div>
))}

              <div className="mt-8 mb-2 text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase">
                Voice Channels
              </div>
              {voiceChannels.map(vc => (
                <div
                  key={vc.id}
                  data-testid={`channel-item-${vc.id}`}
                  onClick={() => { setChannel(vc.id); setActiveView('chat'); }}
                  className={`px-3 py-2 mb-1 rounded-md text-[14px] cursor-pointer flex flex-col gap-2 transition-colors ${
                    currentChannel === vc.id && activeView === 'chat'
                      ? 'bg-[var(--bg-active)] text-[var(--text-main)] font-medium'
                      : 'text-[var(--text-muted)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-main)]'
                  }`}
                >
                  <div className="flex items-center gap-2.5">
                    <IconVolume className="w-4 h-4 opacity-50" /> {vc.name}
                  </div>
                  {inVoice && currentChannel === vc.id && (
                    <div className="pl-6 flex flex-col gap-1 mt-1">
                      {Object.values(media.participants || {}).map((p: any) => (
                        <div key={p.identity} className="flex items-center gap-2">
                          <div className="w-5 h-5 rounded-full bg-[var(--bg-active)] flex items-center justify-center text-[10px] font-bold text-[var(--text-main)]">
                            {getInitial(p.identity)}
                          </div>
                          <span className="text-[12px] text-[var(--text-main)]">{p.identity}</span>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              ))}
            </div>
          </aside>

          {/* 右侧成员列表 */}
          {showMembers && activeView === 'chat' && (
            <aside className="absolute right-0 top-0 bottom-0 w-[200px] flex flex-col z-10 hidden md:flex">
              <div className="text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase mb-4 mt-0">
                Members
              </div>
              <div className="flex flex-col gap-3">
                {(serverMembers.length > 0 ? serverMembers : [loginId]).map(uid => (
                  <div key={uid} className="flex items-center gap-3">
                    <div className="relative">
                      <div className="w-7 h-7 rounded-full bg-[var(--bg-active)] flex items-center justify-center text-[11px] font-bold text-[var(--text-main)]">
                        {getInitial(uid)}
                      </div>
                      <div className="absolute bottom-0 right-0 w-2.5 h-2.5 bg-green-500 border-2 border-[var(--bg-app)] rounded-full"></div>
                    </div>
                    <span className="text-[13px] font-medium text-[var(--text-main)]">{uid}</span>
                  </div>
                ))}
              </div>
            </aside>
          )}

          {/* 主区域 (绝对居中，留出左右安全距离) */}
          <main className="flex-1 flex flex-col min-w-0 px-[220px]">
            {activeView === 'chat' ? (
              <div className="flex-1 flex flex-col w-full max-w-[900px] mx-auto min-h-0">

                {/* 超大标题区 */}
                <div className="shrink-0 flex items-end justify-between mb-8 mt-0 w-full">
                  <div>
                    <h1 className="text-4xl md:text-5xl font-extrabold tracking-tight mb-3 text-[var(--text-main)]">
                      #{currentChannelName}
                    </h1>
                    <p className="text-[13px] text-[var(--text-muted)]">
                      Welcome to the beginning of the #{currentChannelName} channel.
                    </p>
                  </div>
                  <div className="flex items-center gap-3 pb-2">
                    {isVoiceChannel && (inVoice ? (
                      <button
                        data-testid="media-leave"
                        onClick={handleLeaveMedia}
                        className="text-[13px] px-5 py-2 rounded-md bg-[var(--bg-hover)] hover:bg-[var(--bg-active)] transition-colors font-medium text-red-500"
                      >
                        Leave Voice
                      </button>
                    ) : (
                      <button
                        data-testid="media-join"
                        onClick={handleJoinMedia}
                        className="text-[13px] px-5 py-2 rounded-md bg-[var(--text-main)] text-[var(--bg-app)] hover:opacity-90 transition-opacity font-semibold"
                      >
                        Join Voice
                      </button>
                    ))}
                  </div>
                </div>

                {inVoice && <MediaBar channelId={currentChannel} />}
                {inVoice && <ScreenShareView />}

                {/* 消息列表 */}
                <div className="flex-1 overflow-y-auto overflow-x-hidden mb-2 pr-2 w-full">
                  <div className="flex flex-col gap-4 w-full">
                    {currentMessages.map((msg, idx) => (
                      <div key={(msg as any).id || `${msg.ts}-${msg.sender}-${idx}`} className="group flex gap-3 hover:bg-[var(--bg-hover)] -mx-3 px-3 py-2 rounded-md transition-colors">
                        <div className="w-9 h-9 rounded-full flex items-center justify-center text-[13px] font-medium shrink-0 bg-[var(--bg-active)] text-[var(--text-main)] mt-0.5">
                          {getInitial(msg.sender)}
                        </div>
                        <div className="min-w-0 flex-1">
                          <div className="flex items-baseline gap-2">
                            <span className="font-semibold text-[14px] text-[var(--text-main)]">{msg.sender}</span>
                            <span className="text-[11px] text-[var(--text-muted)]">{new Date(msg.ts).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}</span>
                          </div>
                          <div className="text-[14px] mt-0.5 whitespace-pre-wrap leading-relaxed text-[var(--text-main)]">
                            {(msg as any).plaintext ?? (msg as any).text ?? ''}
                          </div>
                        </div>
                      </div>
                    ))}
                    {currentPending.map(p => (
                      <div key={p.id} data-testid="pending-msg" data-status={p.status} className="flex gap-3 opacity-50 -mx-3 px-3 py-2 rounded-md">
                        <div className="w-9 h-9 rounded-full flex items-center justify-center text-[13px] font-medium shrink-0 bg-[var(--bg-active)] text-[var(--text-main)] mt-0.5">
                          {getInitial(loginId)}
                        </div>
                        <div className="min-w-0 flex-1">
                          <div className="flex items-baseline gap-2">
                            <span className="font-semibold text-[14px] text-[var(--text-main)]">{loginId}</span>
                            <span className="text-[11px] text-[var(--text-muted)]">
                              {p.status === 'sending' ? 'Sending...' : `Failed (retry ${p.retries}/5)`}
                            </span>
                          </div>
                          <div className="text-[14px] mt-0.5 whitespace-pre-wrap text-[var(--text-main)]">{p.text}</div>
                        </div>
                      </div>
                    ))}
                    <div ref={messagesEndRef} className="h-4" />
                  </div>
                </div>

                <div data-testid="peer-status" className="text-[11px] text-[var(--text-muted)] h-5 w-full mt-2">{peerStatus}</div>

                <form onSubmit={handleSend} className="w-full flex items-center gap-2 bg-[var(--bg-hover)] rounded-md px-4 py-3 shrink-0 mb-2">
                  <input
                    data-testid="message-input"
                    type="text"
                    value={input}
                    onChange={e => setInput(e.target.value)}
                    placeholder={`Message #${currentChannelName}`}
                    className="flex-1 bg-transparent outline-none text-[14px] placeholder-[var(--text-muted)] text-[var(--text-main)]"
                  />
                  <button type="submit" className="text-[var(--text-muted)] hover:text-[var(--text-main)] transition-colors p-1" title="Send">
                    <IconSend className="w-4 h-4" />
                  </button>
                </form>
              </div>
            ) : activeView === 'dms' ? (
              <div className="flex-1 py-4">
                <DMView />
              </div>
            ) : activeView === 'friends' ? (
              <div className="flex-1 py-4">
                <FriendsView />
              </div>
            ) : (
              <div className="flex-1 flex flex-col items-center justify-center text-[var(--text-muted)]">
                <div className="text-xl font-bold mb-2 capitalize">{activeView}</div>
                <div className="text-sm">Under construction</div>
              </div>
            )}
          </main>
        </div>
      </div>

      {/* 底部栏 */}
      <footer className="shrink-0 bg-[var(--bg-footer)]">
        <div className="px-8 md:px-12 lg:px-16 h-[72px] flex items-center relative transition-colors">
          <div className="w-[200px] shrink-0 flex items-center gap-3">
            <div className="w-7 h-7 rounded-md bg-[var(--bg-active)] flex items-center justify-center text-[11px] text-[var(--text-main)] font-bold">
              {getInitial(loginId)}
            </div>
            <div className="flex flex-col">
              <span className="text-[13px] font-semibold leading-tight text-[var(--text-main)]">{loginId}</span>
              <button onClick={onLogout} className="text-[11px] text-[var(--text-muted)] hover:text-[var(--text-main)] text-left transition-colors mt-0.5">
                Logout
              </button>
            </div>
          </div>

          <div className="absolute left-1/2 -translate-x-1/2 flex items-center gap-8 text-[13px] font-medium text-[var(--text-muted)]">
            <span onClick={() => setActiveView('chat')} className={`cursor-pointer transition-colors hover:text-[var(--text-main)] ${activeView === 'chat' ? 'text-[var(--text-main)]' : ''}`}>Groups</span>
            <span onClick={() => setActiveView('dms')} className={`cursor-pointer transition-colors hover:text-[var(--text-main)] ${activeView === 'dms' ? 'text-[var(--text-main)]' : ''}`}>DMs</span>
            <span onClick={() => setActiveView('friends')} className={`cursor-pointer transition-colors hover:text-[var(--text-main)] ${activeView === 'friends' ? 'text-[var(--text-main)]' : ''}`}>Friends</span>
            <span onClick={() => setActiveView('settings')} className={`cursor-pointer transition-colors hover:text-[var(--text-main)] ${activeView === 'settings' ? 'text-[var(--text-main)]' : ''}`}>Settings</span>
            <span onClick={() => setActiveView('more')} className={`cursor-pointer transition-colors hover:text-[var(--text-main)] ${activeView === 'more' ? 'text-[var(--text-main)]' : ''}`}>More</span>
          </div>

          <div className="ml-auto w-[200px] text-right text-[11px] text-[var(--text-muted)] font-mono">
            v0.0.1-alpha
          </div>
        </div>
      </footer>
    </div>
  )
}