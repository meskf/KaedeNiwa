import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { useChatStore, type ChatMessage } from '../stores/chat'
import { useI18n } from '../stores/i18n'

interface Result {
  channelId: string
  channelName: string
  msg: ChatMessage
  idx: number
}

interface Props {
  open: boolean
  onClose: () => void
  onSelect: (channelId: string) => void
}

const MAX_RESULTS = 30

function highlight(text: string, q: string): ReactNode {
  if (!q) return <>{text}</>
  const lower = text.toLowerCase()
  const ql = q.toLowerCase()
  const parts: ReactNode[] = []
  let cursor = 0
  let i = 0
  while (true) {
    const found = lower.indexOf(ql, cursor)
    if (found === -1) {
      if (cursor < text.length) parts.push(<span key={parts.length}>{text.slice(cursor)}</span>)
      break
    }
    if (found > cursor) parts.push(<span key={parts.length}>{text.slice(cursor, found)}</span>)
    parts.push(<mark key={parts.length} className="bg-[var(--bg-active)] text-[var(--text-main)] rounded px-0.5">{text.slice(found, found + q.length)}</mark>)
    cursor = found + q.length
    i++
    if (i > 20) break
  }
  return <>{parts}</>
}

function truncateAround(text: string, q: string, radius = 60): string {
  if (!q || text.length <= radius * 2) return text
  const idx = text.toLowerCase().indexOf(q.toLowerCase())
  if (idx === -1) return text.slice(0, radius * 2) + '…'
  const start = Math.max(0, idx - radius)
  const end = Math.min(text.length, idx + q.length + radius)
  return (start > 0 ? '…' : '') + text.slice(start, end) + (end < text.length ? '…' : '')
}

export function SearchPalette({ open, onClose, onSelect }: Props) {
  const { t } = useI18n()
  const [q, setQ] = useState('')
  const [activeIdx, setActiveIdx] = useState(0)
  const inputRef = useRef<HTMLInputElement>(null)
  const listRef = useRef<HTMLDivElement>(null)
  const messages = useChatStore(s => s.messages)
  const channelList = useChatStore(s => s.channelList)

  const channelName = (id: string): string => {
    const c = channelList.find(x => x.id === id)
    return c?.name || id
  }

  const results: Result[] = useMemo(() => {
    const query = q.trim().toLowerCase()
    if (!query) return []
    const out: Result[] = []
    for (const [channelId, list] of Object.entries(messages)) {
      if (!Array.isArray(list)) continue
      for (let idx = 0; idx < list.length; idx++) {
        const msg = list[idx]
        const text = (msg as any).plaintext || ''
        if (text.toLowerCase().includes(query)) {
          out.push({ channelId, channelName: channelName(channelId), msg, idx })
          if (out.length >= MAX_RESULTS * 2) break
        }
      }
    }
    out.sort((a, b) => (b.msg.ts || 0) - (a.msg.ts || 0))
    return out.slice(0, MAX_RESULTS)
  }, [q, messages, channelList])

  useEffect(() => {
    if (!open) { setQ(''); setActiveIdx(0); return }
    setTimeout(() => inputRef.current?.focus(), 30)
  }, [open])

  useEffect(() => { setActiveIdx(0) }, [q])

  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') { e.preventDefault(); onClose(); return }
      if (e.key === 'ArrowDown') { e.preventDefault(); setActiveIdx(i => Math.min(i + 1, results.length - 1)); return }
      if (e.key === 'ArrowUp') { e.preventDefault(); setActiveIdx(i => Math.max(i - 1, 0)); return }
      if (e.key === 'Enter') {
        e.preventDefault()
        const r = results[activeIdx]
        if (r) { onSelect(r.channelId); onClose() }
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [open, results, activeIdx, onClose, onSelect])

  useEffect(() => {
    const el = listRef.current?.children[activeIdx] as HTMLElement | undefined
    el?.scrollIntoView({ block: 'nearest' })
  }, [activeIdx])

  if (!open) return null

  return (
    <div
      className="fixed inset-0 z-50 bg-black/50 flex items-start justify-center pt-[15vh]"
      onClick={onClose}
      data-testid="search-palette"
    >
      <div
        className="w-full max-w-[640px] rounded-lg bg-[var(--bg-app)] border border-[var(--bg-active)] shadow-2xl overflow-hidden"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center gap-2 px-4 py-3 border-b border-[var(--bg-active)]">
          <svg className="w-4 h-4 text-[var(--text-muted)] shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <circle cx="11" cy="11" r="7" strokeLinecap="round" />
            <path d="M21 21l-4.35-4.35" strokeLinecap="round" />
          </svg>
          <input
            ref={inputRef}
            type="text"
            value={q}
            onChange={e => setQ(e.target.value)}
            placeholder={t('search.placeholder')}
            className="flex-1 bg-transparent text-[14px] text-[var(--text-main)] placeholder-[var(--text-muted)] outline-none"
            data-testid="search-input"
          />
          <kbd className="text-[10px] font-mono px-1.5 py-0.5 rounded bg-[var(--bg-hover)] text-[var(--text-muted)]">Esc</kbd>
        </div>

        <div ref={listRef} className="max-h-[400px] overflow-y-auto" data-testid="search-results">
          {q.trim() === '' && (
            <div className="px-4 py-8 text-center text-[13px] text-[var(--text-muted)]">
              {t('search.typeToStart')}
            </div>
          )}
          {q.trim() !== '' && results.length === 0 && (
            <div className="px-4 py-8 text-center text-[13px] text-[var(--text-muted)]" data-testid="search-empty">
              {t('search.noResults')}
            </div>
          )}
          {results.map((r, i) => {
            const text = (r.msg as any).plaintext || ''
            const snippet = truncateAround(text, q.trim())
            return (
              <button
                key={`${r.channelId}:${r.idx}`}
                className={`w-full text-left px-4 py-2.5 border-l-2 transition-colors ${
                  i === activeIdx
                    ? 'bg-[var(--bg-hover)] border-[var(--text-main)]'
                    : 'border-transparent hover:bg-[var(--bg-hover)]'
                }`}
                onMouseEnter={() => setActiveIdx(i)}
                onClick={() => { onSelect(r.channelId); onClose() }}
                data-testid={`search-result-${i}`}
              >
                <div className="flex items-center gap-2 text-[11px] text-[var(--text-muted)] mb-0.5">
                  <span className="font-semibold text-[var(--text-main)]">#{r.channelName}</span>
                  <span>·</span>
                  <span>{r.msg.sender}</span>
                  <span>·</span>
                  <span>{new Date(r.msg.ts).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}</span>
                </div>
                <div className="text-[13px] text-[var(--text-main)] truncate">
                  {highlight(snippet, q.trim())}
                </div>
              </button>
            )
          })}
        </div>

        {results.length > 0 && (
          <div className="px-4 py-1.5 border-t border-[var(--bg-active)] text-[11px] text-[var(--text-muted)] flex items-center justify-between">
            <span>{results.length} {t('search.results')}</span>
            <span className="flex items-center gap-2">
              <kbd className="text-[10px] font-mono px-1.5 py-0.5 rounded bg-[var(--bg-hover)]">↑↓</kbd>
              <kbd className="text-[10px] font-mono px-1.5 py-0.5 rounded bg-[var(--bg-hover)]">Enter</kbd>
            </span>
          </div>
        )}
      </div>
    </div>
  )
}
