import { useEffect, useMemo, useRef, useState, type FormEvent } from 'react'
import { IconSend, IconHash } from './icons'
import { getClient } from '../wasm/client'
import { useChatStore } from '../stores/chat'

export function DMView() {
  const convs = useChatStore(s => s.dmConversations)
  const setConvs = useChatStore(s => s.setDmConversations)
  const addConv = useChatStore(s => s.addDmConversation)
  const activeConvId = useChatStore(s => s.activeConvId)
  const setActiveConv = useChatStore(s => s.setActiveConv)
  const dmMessages = useChatStore(s => s.dmMessages)
  const setDmMessages = useChatStore(s => s.setDmMessages)
  const appendDmMessage = useChatStore(s => s.appendDmMessage)

  const [input, setInput] = useState('')
  const [newPeer, setNewPeer] = useState('')
  const [showNew, setShowNew] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const scrollRef = useRef<HTMLDivElement>(null)

  const activeConv = useMemo(() => convs.find(c => c.conv_id === activeConvId), [convs, activeConvId])
  const msgs = activeConvId ? (dmMessages[activeConvId] || []) : []
  useEffect(() => {
    const c = getClient()
    if (!c) return
    ;(async () => {
      try {
        const raw = await c.listDmConversations()
        const arr = typeof raw === 'string' ? JSON.parse(raw) : raw
        setConvs(arr || [])
      } catch (e) {
        console.error('[dm] list:', e)
      }
    })()
  }, [setConvs])

  useEffect(() => {
    if (!activeConvId) return
    const c = getClient()
    if (!c) return
    ;(async () => {
      try {
        const raw = await c.dmLoadHistory(activeConvId, 50)
        const arr = typeof raw === 'string' ? JSON.parse(raw) : raw
        const mapped = (arr || []).map((m: any) => ({
          sender: m.sender,
          ts: Number(m.ts),
          plaintext: m.plaintext,
        }))
        setDmMessages(activeConvId, mapped)
        if (mapped.length > 0) {
          const latest = mapped.reduce((a: number, b: { ts: number }) => Math.max(a, b.ts), 0)
          try { await c.markDmRead(activeConvId, BigInt(latest)) } catch {}
        }
      } catch (e) {
        console.error('[dm] history:', e)
      }
    })()
  }, [activeConvId, setDmMessages])

  useEffect(() => {
    if (scrollRef.current) scrollRef.current.scrollTop = scrollRef.current.scrollHeight
  }, [msgs.length])
  const handleSend = async (e: FormEvent) => {
    e.preventDefault()
    if (!input.trim() || !activeConvId || busy) return
    const c = getClient()
    if (!c) return
    const text = input.trim()
    const ts = Date.now()
    setInput('')
    setBusy(true)
    setError(null)
    appendDmMessage(activeConvId, { sender: '__me__', ts, plaintext: text })
    try {
      await c.sendDm(activeConvId, text)
    } catch (e: any) {
      setError(String(e))
    } finally {
      setBusy(false)
    }
  }

  const handleOpen = async () => {
    if (!newPeer.trim() || busy) return
    const c = getClient()
    if (!c) return
    setBusy(true)
    setError(null)
    try {
      const raw = await c.openDm(newPeer.trim())
      const conv = typeof raw === 'string' ? JSON.parse(raw) : raw
      addConv(conv)
      setActiveConv(conv.conv_id)
      setShowNew(false)
      setNewPeer('')
    } catch (e: any) {
      setError(String(e))
    } finally {
      setBusy(false)
    }
  }

  const getInitial = (str: string) => str ? str.charAt(0).toUpperCase() : '?'
  const isMine = (m: { sender: string }) => m.sender !== activeConv?.peer_login_id
  return (
    <div className="flex h-full w-full max-w-[1000px] mx-auto gap-8">
      <div className="w-[300px] flex flex-col shrink-0">
        <div className="flex items-center justify-between mb-6 mt-1">
          <h1 className="text-3xl font-extrabold tracking-tight text-[var(--text-main)]">Messages</h1>
          <button
            onClick={() => setShowNew(v => !v)}
            className="px-3 py-1.5 text-[12px] font-semibold rounded-md bg-[var(--bg-active)] text-[var(--text-main)] hover:bg-[var(--text-muted)] transition-colors"
            title="New conversation"
          >
            + New
          </button>
        </div>
        {showNew && (
          <div className="mb-4 p-3 rounded-lg bg-[var(--bg-hover)] flex flex-col gap-2">
            <input
              type="text"
              value={newPeer}
              onChange={e => setNewPeer(e.target.value)}
              placeholder="peer login_id"
              className="w-full bg-[var(--bg-app)] text-[var(--text-main)] text-[13px] rounded-md px-3 py-2 outline-none"
              onKeyDown={e => { if (e.key === 'Enter') handleOpen() }}
            />
            <div className="flex gap-2">
              <button onClick={handleOpen} disabled={busy} className="flex-1 px-3 py-1.5 text-[12px] font-semibold rounded-md bg-[var(--bg-active)] text-[var(--text-main)] hover:bg-[var(--text-muted)] disabled:opacity-50">
                {busy ? '...' : 'Open'}
              </button>
              <button onClick={() => { setShowNew(false); setNewPeer(''); setError(null) }} className="px-3 py-1.5 text-[12px] text-[var(--text-muted)] hover:text-[var(--text-main)]">
                Cancel
              </button>
            </div>
          </div>
        )}
        {error && (
          <div className="mb-3 px-3 py-2 text-[12px] text-red-400 bg-red-500/10 rounded-md">{error}</div>
        )}
        <div className="flex-1 overflow-y-auto pr-2 space-y-1">
          {convs.length === 0 && (
            <div className="text-[13px] text-[var(--text-muted)] py-4">No conversations yet</div>
          )}
          {convs.map(c => (
            <div
              key={c.conv_id}
              onClick={() => setActiveConv(c.conv_id)}
              className={`flex items-center gap-3 p-3 rounded-lg cursor-pointer transition-colors ${
                activeConvId === c.conv_id ? 'bg-[var(--bg-active)]' : 'hover:bg-[var(--bg-hover)]'
              }`}
            >
              <div className="w-10 h-10 rounded-full bg-[var(--bg-hover)] flex items-center justify-center text-[14px] font-bold shrink-0 text-[var(--text-main)]">
                {getInitial(c.peer_nickname || c.peer_login_id)}
              </div>
              <div className="flex flex-col min-w-0">
                <span className="text-[14px] font-semibold text-[var(--text-main)] truncate">
                  {c.peer_nickname || c.peer_login_id}
                </span>
                <span className="text-[12px] text-[var(--text-muted)] truncate">{c.peer_login_id}</span>
              </div>
            </div>
          ))}
        </div>
      </div>
      <div className="flex-1 flex flex-col min-w-0 border-l border-[var(--bg-hover)] pl-8">
        {activeConv ? (
          <>
            <div className="h-[60px] flex items-center shrink-0 border-b border-[var(--bg-hover)] mb-4">
              <IconHash className="w-5 h-5 text-[var(--text-muted)] mr-2" />
              <span className="text-[16px] font-bold text-[var(--text-main)]">
                {activeConv.peer_nickname || activeConv.peer_login_id}
              </span>
            </div>
            <div ref={scrollRef} className="flex-1 overflow-y-auto mb-4">
              <div className="flex flex-col gap-4">
                {msgs.length === 0 && (
                  <div className="text-[13px] text-[var(--text-muted)] py-8 text-center">No messages yet. Say hi!</div>
                )}
                {msgs.map((m, i) => (
                  <div key={i} className={`flex gap-3 ${isMine(m) ? 'flex-row-reverse' : ''}`}>
                    <div className="w-9 h-9 rounded-full bg-[var(--bg-active)] flex items-center justify-center text-[13px] font-medium shrink-0 text-[var(--text-main)] mt-0.5">
                      {getInitial(m.sender === '__me__' ? 'You' : m.sender)}
                    </div>
                    <div className={`min-w-0 flex-1 ${isMine(m) ? 'text-right' : ''}`}>
                      <div className={`flex items-baseline gap-2 ${isMine(m) ? 'justify-end' : ''}`}>
                        <span className="font-semibold text-[13px] text-[var(--text-main)]">
                          {isMine(m) ? 'You' : m.sender}
                        </span>
                        <span className="text-[11px] text-[var(--text-muted)]">
                          {new Date(m.ts).toLocaleTimeString()}
                        </span>
                      </div>
                      <div className="text-[14px] mt-0.5 text-[var(--text-main)] break-words">{m.plaintext}</div>
                    </div>
                  </div>
                ))}
              </div>
            </div>            <div className="pb-2 shrink-0">
              <form onSubmit={handleSend} className="w-full flex items-center gap-3 rounded-md px-4 py-3 bg-[var(--bg-hover)]">
                <input
                  type="text"
                  value={input}
                  onChange={e => setInput(e.target.value)}
                  placeholder={`Message @${activeConv.peer_nickname || activeConv.peer_login_id}`}
                  className="bg-transparent outline-none flex-1 text-[14px] placeholder-[var(--text-muted)] text-[var(--text-main)]"
                />
                <button type="submit" disabled={busy} className="text-[var(--text-muted)] hover:text-[var(--text-main)] transition-colors disabled:opacity-50" title="Send">
                  <IconSend className="w-4 h-4" />
                </button>
              </form>
            </div>
          </>
        ) : (
          <div className="flex-1 flex flex-col items-center justify-center text-[var(--text-muted)]">
            <div className="w-16 h-16 rounded-full bg-[var(--bg-hover)] flex items-center justify-center mb-4">
              <IconHash className="w-8 h-8 opacity-50" />
            </div>
            <p className="text-[14px]">Select a conversation to start chatting</p>
          </div>
        )}
      </div>
    </div>
  )
}