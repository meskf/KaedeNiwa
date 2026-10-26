import { useState } from 'react'
import { IconSend, IconHash } from './icons'

interface DmConversation {
  id: string
  peerId: string
  lastMessage?: string
  unread?: boolean
}

export function DMView() {
  const [convs] = useState<DmConversation[]>([
    { id: 'dm:u0:peer:alice', peerId: 'alice', lastMessage: 'Hey, are you there?' },
    { id: 'dm:u0:peer:bob', peerId: 'bob', lastMessage: 'Check the new UI' },
  ])
  const [activeConv, setActiveConv] = useState<string | null>(null)
  const [input, setInput] = useState('')

  const getInitial = (str: string) => str ? str.charAt(0).toUpperCase() : '?'

  return (
    <div className="flex h-full w-full max-w-[1000px] mx-auto gap-8">
      {/* 左侧会话列表 */}
      <div className="w-[300px] flex flex-col shrink-0">
        <h1 className="text-3xl font-extrabold tracking-tight mb-6 mt-1 text-[var(--text-main)]">
          Messages
        </h1>
        <div className="flex-1 overflow-y-auto pr-2 space-y-1">
          {convs.map(c => (
            <div
              key={c.id}
              onClick={() => setActiveConv(c.id)}
              className={`flex items-center gap-3 p-3 rounded-lg cursor-pointer transition-colors ${
                activeConv === c.id ? 'bg-[var(--bg-active)]' : 'hover:bg-[var(--bg-hover)]'
              }`}
            >
              <div className="w-10 h-10 rounded-full bg-[var(--bg-hover)] flex items-center justify-center text-[14px] font-bold shrink-0 text-[var(--text-main)]">
                {getInitial(c.peerId)}
              </div>
              <div className="flex flex-col min-w-0">
                <span className="text-[14px] font-semibold text-[var(--text-main)] truncate">{c.peerId}</span>
                <span className="text-[12px] text-[var(--text-muted)] truncate">{c.lastMessage || 'No messages yet'}</span>
              </div>
            </div>
          ))}
        </div>
      </div>

      {/* 右侧聊天区 */}
      <div className="flex-1 flex flex-col min-w-0 border-l border-[var(--bg-hover)] pl-8">
        {activeConv ? (
          <>
            <div className="h-[60px] flex items-center shrink-0 border-b border-[var(--bg-hover)] mb-4">
              <IconHash className="w-5 h-5 text-[var(--text-muted)] mr-2" />
              <span className="text-[16px] font-bold text-[var(--text-main)]">
                {convs.find(c => c.id === activeConv)?.peerId}
              </span>
            </div>
            <div className="flex-1 overflow-y-auto mb-4">
              <div className="flex flex-col gap-4">
                <div className="flex gap-3">
                  <div className="w-9 h-9 rounded-full bg-[var(--bg-active)] flex items-center justify-center text-[13px] font-medium shrink-0 text-[var(--text-main)] mt-0.5">
                    {getInitial(convs.find(c => c.id === activeConv)?.peerId || '')}
                  </div>
                  <div className="min-w-0 flex-1">
                    <div className="flex items-baseline gap-2">
                      <span className="font-semibold text-[14px] text-[var(--text-main)]">{convs.find(c => c.id === activeConv)?.peerId}</span>
                      <span className="text-[11px] text-[var(--text-muted)]">Just now</span>
                    </div>
                    <div className="text-[14px] mt-0.5 text-[var(--text-main)]">Hey! This is a private message.</div>
                  </div>
                </div>
              </div>
            </div>
            <div className="pb-2 shrink-0">
              <form onSubmit={(e) => { e.preventDefault(); setInput(''); }} className="w-full flex items-center gap-3 rounded-md px-4 py-3 bg-[var(--bg-hover)]">
                <input
                  type="text"
                  value={input}
                  onChange={e => setInput(e.target.value)}
                  placeholder={`Message @${convs.find(c => c.id === activeConv)?.peerId}`}
                  className="bg-transparent outline-none flex-1 text-[14px] placeholder-[var(--text-muted)] text-[var(--text-main)]"
                />
                <button type="submit" className="text-[var(--text-muted)] hover:text-[var(--text-main)] transition-colors" title="Send">
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