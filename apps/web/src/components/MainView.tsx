import { useState, useEffect, useRef } from 'react'
import { useAuthStore } from '../stores/auth'
import { useChatStore } from '../stores/chat'
import { getClient, scheduleReconnect } from '../wasm/client'
import { connectMedia, disconnectMedia, updateE2EEKey } from '../lib/livekit'
import { useMediaState } from '../hooks/useLiveKitRoom'
import { MediaBar } from './MediaBar'
import { ScreenShareView } from './ScreenShareView'
import { IconHash, IconVolume, IconSearch, IconSun, IconMoon, IconPlus, IconSend, IconLogout } from './icons'

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
    <div className="h-screen w-screen flex flex-col bg-[var(--bg-base)] text-[var(--text-primary)]">

      {/* 顶栏 */}
      <header className="relative h-[56px] shrink-0 flex items-center">
        <div className="w-[240px] shrink-0 h-full flex items-center px-4 text-[15px] font-semibold tracking-wide">
          KaedeNiwa
        </div>
        <div className="absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 w-full max-w-[480px] px-4">
          <div className="relative flex items-center">
            <IconSearch className="absolute left-3 w-4 h-4 text-[var(--text-muted)]" />
            <input
              type="text"
              placeholder="Search..."
              className="w-full bg-[var(--bg-surface)] text-[var(--text-primary)] text-[13px] rounded-md pl-9 pr-3 py-1.5 outline-none placeholder-[var(--text-muted)] border border-transparent focus:border-[var(--bg-active)] transition-colors"
            />
          </div>
        </div>
        <div className="flex-1" />
        <div className="flex items-center gap-1 shrink-0 pr-4">
          <button
            onClick={() => setTheme(t => t === 'dark' ? 'light' : 'dark')}
            className="w-9 h-9 flex items-center justify-center rounded-md text-[var(--text-secondary)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-surface)] transition-colors"
            title="Toggle Theme"
          >
            {theme === 'dark' ? <IconSun className="w-5 h-5" /> : <IconMoon className="w-5 h-5" />}
          </button>
        </div>
      </header>

      <div className="flex-1 flex min-h-0">
        {/* 侧边栏 */}
        <aside className="w-[240px] shrink-0 flex flex-col bg-[var(--bg-surface)] rounded-tl-lg overflow-hidden">
          <div className="h-[56px] shrink-0 flex items-center px-4">
            <div className="w-8 h-8 rounded-lg bg-[var(--bg-active)] flex items-center justify-center text-[var(--text-primary)] font-bold text-sm shrink-0">
              K
            </div>
            <div className="ml-3 flex flex-col min-w-0">
              <span className="text-[14px] font-semibold leading-tight truncate">Kaede Server</span>
              <span className="text-[11px] text-[var(--text-secondary)] truncate">Workspace</span>
            </div>
          </div>

          <div className="flex-1 overflow-y-auto py-3">
            <div className="px-4 mb-2 text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase">
              Text Channels
            </div>
            {CHANNELS.map(ch => (
              <div
                key={ch}
                data-testid={`channel-item-${ch}`}
                onClick={() => { setChannel(ch); setActiveView('chat'); }}
                className={`px-2 py-1.5 mx-2 mb-0.5 rounded-md text-[14px] cursor-pointer flex items-center gap-1.5 transition-colors ${
                  currentChannel === ch && activeView === 'chat'
                    ? 'bg-[var(--bg-active)] text-[var(--text-primary)]'
                    : 'text-[var(--text-secondary)] hover:bg-[var(--bg-active)] hover:text-[var(--text-primary)]'
                }`}
              >
                <IconHash className="w-4 h-4 opacity-70" /> {ch}
              </div>
            ))}

            <div className="px-4 mt-6 mb-2 text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase">
              Voice Channels
            </div>
            {VOICE_CHANNELS.map(vc => (
              <div
                key={vc}
                className="px-2 py-1.5 mx-2 rounded-md text-[14px] text-[var(--text-secondary)] flex items-center gap-1.5 hover:bg-[var(--bg-active)] hover:text-[var(--text-primary)] cursor-pointer transition-colors"
              >
                <IconVolume className="w-4 h-4 opacity-70" /> {vc}
              </div>
            ))}
          </div>
        </aside>

        <main className="flex-1 flex flex-col min-w-0 bg-[var(--bg-base)]">
          {activeView === 'chat' ? (
            <>
              <div className="h-[56px] shrink-0 flex items-center justify-between px-4">
                <div className="flex items-center gap-1.5">
                  <IconHash className="w-5 h-5 text-[var(--text-muted)]" />
                  <span className="font-semibold text-[15px]">{currentChannel}</span>
                </div>
                <div className="flex items-center gap-3">
                  {inVoice ? (
                    <button
                      data-testid="media-leave"
                      onClick={handleLeaveMedia}
                      className="text-[13px] px-3 py-1.5 rounded-md border border-[var(--bg-active)] hover:bg-[var(--bg-active)] transition-colors text-[var(--text-primary)] font-medium"
                    >
                      Leave Voice
                    </button>
                  ) : (
                    <button
                      data-testid="media-join"
                      onClick={handleJoinMedia}
                      className="text-[13px] px-3 py-1.5 rounded-md bg-[var(--bg-active)] hover:bg-[#3f3f46] transition-colors text-[var(--text-primary)] font-medium"
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
                  <div className="space-y-4 max-w-4xl mx-auto mt-2">
                    {currentMessages.map((msg, idx) => (
                      <div key={(msg as any).id || `${msg.ts}-${msg.sender}-${idx}`} className="flex gap-4 hover:bg-[var(--bg-surface)] -mx-2 px-2 py-1 rounded-md transition-colors">
                        <div className="w-10 h-10 rounded-full flex items-center justify-center text-[14px] font-medium shrink-0 bg-[var(--bg-active)] text-[var(--text-primary)]">
                          {getInitial(msg.sender)}
                        </div>
                        <div className="min-w-0 flex-1">
                          <div className="flex items-baseline gap-2">
                            <span className="font-semibold text-[14px]">{msg.sender}</span>
                            <span className="text-[11px] text-[var(--text-muted)]">{new Date(msg.ts).toLocaleTimeString()}</span>
                          </div>
                          <div className="text-[14px] mt-0.5 whitespace-pre-wrap leading-relaxed">
                            {(msg as any).plaintext ?? (msg as any).text ?? ''}
                          </div>
                        </div>
                      </div>
                    ))}
                    {currentPending.map(p => (
                      <div key={p.id} data-testid="pending-msg" data-status={p.status} className="flex gap-4 opacity-50 -mx-2 px-2 py-1 rounded-md">
                        <div className="w-10 h-10 rounded-full flex items-center justify-center text-[14px] font-medium shrink-0 bg-[var(--bg-active)] text-[var(--text-primary)]">
                          {getInitial(loginId)}
                        </div>
                        <div className="min-w-0 flex-1">
                          <div className="flex items-baseline gap-2">
                            <span className="font-semibold text-[14px]">{loginId}</span>
                            <span className="text-[11px] text-[var(--text-secondary)]">
                              {p.status === 'sending' ? 'Sending...' : `Failed (retry ${p.retries}/5)`}
                            </span>
                          </div>
                          <div className="text-[14px] mt-0.5 whitespace-pre-wrap">{p.text}</div>
                        </div>
                      </div>
                    ))}
                    <div ref={messagesEndRef} />
                  </div>
                )}
              </div>

              <div data-testid="peer-status" className="px-6 text-[11px] text-[var(--text-muted)] h-4">{peerStatus}</div>
              
              <div className="px-6 pb-6 pt-2 shrink-0">
                <form onSubmit={handleSend} className="flex items-center gap-3 rounded-lg px-4 py-3 bg-[var(--bg-surface)]">
                  <button type="button" className="text-[var(--text-muted)] hover:text-[var(--text-primary)] transition-colors">
                    <IconPlus className="w-5 h-5" />
                  </button>
                  <input
                    type="text"
                    value={input}
                    onChange={e => setInput(e.target.value)}
                    placeholder={`Message #${currentChannel}`}
                    data-testid="message-input"
                    className="bg-transparent outline-none flex-1 text-[14px] placeholder-[var(--text-muted)] text-[var(--text-primary)]"
                  />
                  <button type="submit" className="text-[var(--text-secondary)] hover:text-[var(--text-primary)] transition-colors" title="Send">
                    <IconSend className="w-5 h-5" />
                  </button>
                </form>
              </div>
            </>
          ) : (
            <div className="flex-1 flex flex-col items-center justify-center text-[var(--text-muted)]">
              <div className="text-xl font-medium uppercase mb-2 tracking-widest">{activeView}</div>
              <div className="text-sm">Under construction</div>
            </div>
          )}
        </main>
      </div>

      <footer className="relative h-[56px] shrink-0 flex items-center px-4 bg-[var(--bg-surface)]">
        <div className="w-[240px] shrink-0 flex items-center gap-3">
          <div className="w-8 h-8 rounded-full bg-[var(--bg-active)] flex items-center justify-center text-xs text-[var(--text-primary)] font-medium">
            {getInitial(loginId)}
          </div>
          <div className="flex flex-col">
            <span className="text-[13px] font-semibold leading-tight">{loginId}</span>
            <button onClick={onLogout} className="text-[11px] text-[var(--text-muted)] hover:text-[var(--text-secondary)] text-left transition-colors flex items-center gap-1 mt-0.5">
              <IconLogout className="w-3 h-3" /> Logout
            </button>
          </div>
        </div>

        <div className="absolute left-1/2 -translate-x-1/2 flex items-center gap-6 text-[13px] font-medium text-[var(--text-secondary)]">
          <span onClick={() => setActiveView('chat')} className={`cursor-pointer transition-colors hover:text-[var(--text-primary)] ${activeView === 'chat' ? 'text-[var(--text-primary)]' : ''}`}>Groups</span>
          <span onClick={() => setActiveView('dms')} className={`cursor-pointer transition-colors hover:text-[var(--text-primary)] ${activeView === 'dms' ? 'text-[var(--text-primary)]' : ''}`}>DMs</span>
          <span onClick={() => setActiveView('friends')} className={`cursor-pointer transition-colors hover:text-[var(--text-primary)] ${activeView === 'friends' ? 'text-[var(--text-primary)]' : ''}`}>Friends</span>
          <span onClick={() => setActiveView('settings')} className={`cursor-pointer transition-colors hover:text-[var(--text-primary)] ${activeView === 'settings' ? 'text-[var(--text-primary)]' : ''}`}>Settings</span>
          <span onClick={() => setActiveView('more')} className={`cursor-pointer transition-colors hover:text-[var(--text-primary)] ${activeView === 'more' ? 'text-[var(--text-primary)]' : ''}`}>More</span>
        </div>

        <div className="ml-auto w-[100px] text-right text-[11px] text-[var(--text-muted)] font-mono">
          v0.0.1-alpha
        </div>
      </footer>
    </div>
  )
}