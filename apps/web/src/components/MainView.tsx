import { useState, useEffect, useRef } from 'react'
import { useAuthStore } from '../stores/auth'
import { useChatStore } from '../stores/chat'
import { getClient, scheduleReconnect } from '../wasm/client'
import { connectMedia, disconnectMedia, updateE2EEKey } from '../lib/livekit'
import { useMediaState } from '../hooks/useLiveKitRoom'
import { MediaBar } from './MediaBar'
import { ScreenShareView } from './ScreenShareView'

const CHANNELS = ['general', 'dev', 'random']
const VOICE_CHANNELS = ['Lounge']

export function MainView({ onLogout }: { onLogout: () => void }) {
  const loginId = useAuthStore(s => s.loginId)
  const {
    currentChannel, setChannel,
    activeView, setActiveView,
    messages, setMessages, addMessage,
    pendingMessages, addPending, removePending, markPendingFailed, incrementRetry,
    peerStatus,
  } = useChatStore()
  const [input, setInput] = useState('')
  const [theme, setTheme] = useState<'dark' | 'light'>('dark')
  const addMemberQueueRef = useRef<Promise<void>>(Promise.resolve())
  const messagesEndRef = useRef<HTMLDivElement>(null)

  const media = useMediaState()
  const mediaRef = useRef(media)
  useEffect(() => { mediaRef.current = media }, [media])

  const currentMessages = messages[currentChannel] || []
  const currentPending = pendingMessages.filter(p => p.channelId === currentChannel)
  const inVoice = media.connected && media.channelId === currentChannel

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
          if (peers.length === 0) {
            try {
              const created = client.ensure_channel(ch)
              if (created) useChatStore.getState().setIsAuthority(true)
            } catch (e) { console.error('ensure_channel failed:', e) }
          }
        }
        if (ev.ChannelMemberJoined) {
          const { channel_id, requester } = ev.ChannelMemberJoined
          if (channel_id !== useChatStore.getState().currentChannel) return
          if (!useChatStore.getState().isAuthority) return
          client.add_member_to_channel(useChatStore.getState().currentChannel, requester).then(() => {
            useChatStore.getState().setPeerStatus(`Added ${requester}`)
          }).catch((e: any) => useChatStore.getState().setPeerStatus(`Add failed: ${e}`))
        }
        if (ev.PeerJoined) {
          const uid = ev.PeerJoined.uid
          if (!useChatStore.getState().isAuthority) return
          useChatStore.getState().setPeerStatus(`Adding ${uid}...`)
          addMemberQueueRef.current = addMemberQueueRef.current.then(async () => {
            try {
              await client.add_member_to_channel(useChatStore.getState().currentChannel, uid)
              useChatStore.getState().setPeerStatus(`Added ${uid}`)
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
          client.process_channel_sync(channelId, payload).then(() => {
            useChatStore.getState().setPeerStatus('Synced')
            console.log(`[ChannelSync] ch=${channelId} OK`)
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

  return (
    <div className={`h-screen w-screen flex flex-col font-bold ${theme === 'dark' ? 'bg-[#18191c] text-white' : 'bg-white text-black'}`}>
      <header className="relative h-[56px] shrink-0 flex items-center border-b border-[#2a2b2f]">
        <div className="w-[240px] shrink-0 h-full flex items-center px-4 text-[16px] tracking-wide font-bold z-10">
          KaedeNiwa
        </div>
        <div className="absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 w-full max-w-[480px] px-4 z-10">
          <input
            type="text"
            placeholder="Search..."
            className="w-full bg-[#1e1f22] text-white text-[13px] rounded-md pl-4 pr-3 py-1.5 outline-none placeholder-gray-500 border border-transparent"
          />
        </div>
        <div className="flex-1" />
        <div className="flex items-center gap-1 shrink-0 pr-2 z-10">
          <button onClick={() => setTheme(t => t === 'dark' ? 'light' : 'dark')} className="w-10 h-10 flex items-center justify-center text-gray-400 hover:text-white">
            {theme === 'dark' ? '☀' : '🌙'}
          </button>
          <div className="w-[138px] h-full" />
        </div>
      </header>

      <div className="flex-1 flex min-h-0">
        <aside className="w-[240px] shrink-0 flex flex-col border-r border-[#2a2b2f]">
          <div className="h-[56px] shrink-0 flex items-center px-4 border-b border-[#2a2b2f]">
            <div className="w-8 h-8 rounded-lg bg-[#5865f2] flex items-center justify-center text-white font-bold text-sm shrink-0">
              K
            </div>
            <div className="ml-3 flex flex-col min-w-0">
              <span className="text-[14px] font-bold leading-tight truncate">Kaede Server</span>
              <span className="text-[11px] font-normal text-gray-400 truncate">Workspace</span>
            </div>
          </div>

          <div className="flex-1 overflow-y-auto py-3">
            <div className="px-4 mb-2 text-[11px] font-bold tracking-widest text-gray-400 uppercase">
              Text Channels
            </div>
            {CHANNELS.map(ch => (
              <div
                key={ch}
                data-testid={`channel-item-${ch}`}
                onClick={() => { setChannel(ch); setActiveView('chat'); }}
                className={`px-3 py-2 mx-2 rounded text-[15px] cursor-pointer flex items-center gap-2 ${
                  currentChannel === ch && activeView === 'chat' ? 'bg-[#2b2d31] text-white' : 'text-gray-400 hover:bg-[#1e1f22]'
                }`}
              >
                <span className="text-[18px] font-light leading-none">#</span> {ch}
              </div>
            ))}

            <div className="px-4 mt-6 mb-2 text-[11px] font-bold tracking-widest text-gray-400 uppercase">
              Voice Channels
            </div>
            {VOICE_CHANNELS.map(vc => (
              <div
                key={vc}
                className="px-3 py-2 mx-2 text-[15px] text-gray-400 flex items-center gap-2 hover:bg-[#1e1f22] cursor-pointer rounded"
              >
                <span className="text-[16px]">🔊</span> {vc}
              </div>
            ))}
          </div>
        </aside>

        <main className="flex-1 flex flex-col min-w-0">
          {activeView === 'chat' ? (
            <>
              <div className="h-[56px] shrink-0 flex items-center justify-between px-4 border-b border-[#2a2b2f]">
                <div className="flex items-center gap-3">
                  <span className="text-[24px] font-light text-gray-400 leading-none pt-1">#</span>
                  <span className="font-bold text-[16px]">{currentChannel}</span>
                </div>
                <div className="flex items-center gap-3">
                  {inVoice ? (
                    <button
                      data-testid="media-leave"
                      onClick={handleLeaveMedia}
                      className="text-xs px-3 py-1.5 rounded bg-[#da373c] hover:bg-[#a12829] text-white font-bold"
                    >
                      Leave Voice
                    </button>
                  ) : (
                    <button
                      data-testid="media-join"
                      onClick={handleJoinMedia}
                      className="text-xs px-3 py-1.5 rounded bg-[#248046] hover:bg-[#1a6334] text-white font-bold"
                    >
                      Join Voice
                    </button>
                  )}
                </div>
              </div>

              <MediaBar channelId={currentChannel} />
              <ScreenShareView />

              <div className="flex-1 overflow-y-auto px-6 py-4">
                {(currentMessages.length > 0 || currentPending.length > 0) && (
                  <div className="space-y-5 max-w-4xl mx-auto mt-2">
                    {currentMessages.map((msg, idx) => (
                      <div key={(msg as any).id || `${msg.ts}-${msg.sender}-${idx}`} className="flex gap-4 hover:bg-[#1e1f22] -mx-2 px-2 py-1 rounded">
                        <div className="w-10 h-10 rounded-full flex items-center justify-center text-[15px] font-bold shrink-0 bg-[#5865f2] text-white">
                          {getInitial(msg.sender)}
                        </div>
                        <div className="min-w-0">
                          <div className="flex items-baseline gap-2">
                            <span className="font-bold text-[15px]">{msg.sender}</span>
                            <span className="text-[11px] font-normal text-gray-400">{new Date(msg.ts).toLocaleTimeString()}</span>
                          </div>
                          <div className="text-[15px] font-normal mt-0.5 whitespace-pre-wrap">
                            {(msg as any).plaintext ?? (msg as any).text ?? ''}
                          </div>
                        </div>
                      </div>
                    ))}
                    {currentPending.map(p => (
                      <div key={p.id} data-testid="pending-msg" data-status={p.status} className="flex gap-4 opacity-60 -mx-2 px-2 py-1 rounded">
                        <div className="w-10 h-10 rounded-full flex items-center justify-center text-[15px] font-bold shrink-0 bg-[#5865f2] text-white">
                          {getInitial(loginId)}
                        </div>
                        <div className="min-w-0">
                          <div className="flex items-baseline gap-2">
                            <span className="font-bold text-[15px]">{loginId}</span>
                            <span className="text-[11px] font-normal text-gray-400">
                              {p.status === 'sending' ? 'Sending...' : `Failed (retry ${p.retries}/5)`}
                            </span>
                          </div>
                          <div className="text-[15px] font-normal mt-0.5 whitespace-pre-wrap">{p.text}</div>
                        </div>
                      </div>
                    ))}
                    <div ref={messagesEndRef} />
                  </div>
                )}
              </div>

              <div data-testid="peer-status" className="px-6 text-[11px] text-yellow-400">{peerStatus}</div>
              <div className="px-6 pb-6 pt-2 shrink-0">
                <form onSubmit={handleSend} className="flex items-center gap-3 rounded-lg px-4 py-3 bg-[#1e1f22]">
                  <span className="text-gray-400 text-xl font-light">+</span>
                  <input
                    type="text"
                    value={input}
                    onChange={e => setInput(e.target.value)}
                    placeholder={`Message #${currentChannel}`}
                    data-testid="message-input"
                    className="bg-transparent outline-none flex-1 text-[15px] font-normal placeholder-gray-500 text-white"
                  />
                  <button type="submit" className="text-gray-400 hover:text-white">
                    Send
                  </button>
                </form>
              </div>
            </>
          ) : (
            <div className="flex-1 flex flex-col items-center justify-center text-gray-500">
              <div className="text-2xl font-bold uppercase mb-2">{activeView}</div>
              <div className="text-sm font-normal">Under construction</div>
            </div>
          )}
        </main>
      </div>

      <footer className="relative h-[56px] shrink-0 flex items-center border-t border-[#2a2b2f] px-4">
        <div className="w-[240px] shrink-0 flex items-center gap-3">
          <div className="w-8 h-8 rounded-full bg-[#5865f2] flex items-center justify-center text-xs text-white font-bold">
            {getInitial(loginId)}
          </div>
          <div className="flex flex-col">
            <span className="text-[14px] font-bold leading-tight">{loginId}</span>
            <button onClick={onLogout} className="text-[11px] font-normal text-gray-400 text-left hover:underline">Logout</button>
          </div>
        </div>

        <div className="absolute left-1/2 -translate-x-1/2 flex items-center gap-4 text-[13px] font-bold text-gray-400">
          <span onClick={() => setActiveView('chat')} className={`cursor-pointer ${activeView === 'chat' ? 'text-white' : ''}`}>Groups</span>
          <span className="text-gray-600 font-normal">|</span>
          <span onClick={() => setActiveView('dms')} className={`cursor-pointer ${activeView === 'dms' ? 'text-white' : ''}`}>DMs</span>
          <span className="text-gray-600 font-normal">|</span>
          <span onClick={() => setActiveView('friends')} className={`cursor-pointer ${activeView === 'friends' ? 'text-white' : ''}`}>Friends</span>
          <span className="text-gray-600 font-normal">|</span>
          <span onClick={() => setActiveView('settings')} className={`cursor-pointer ${activeView === 'settings' ? 'text-white' : ''}`}>Settings</span>
          <span className="text-gray-600 font-normal">|</span>
          <span onClick={() => setActiveView('more')} className={`cursor-pointer ${activeView === 'more' ? 'text-white' : ''}`}>More</span>
        </div>

        <div className="ml-auto w-[100px] text-right text-[12px] font-normal text-gray-500 font-mono">
          v0.0.1-alpha
        </div>
      </footer>
    </div>
  )
}