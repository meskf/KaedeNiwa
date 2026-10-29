import { useEffect, useRef } from 'react'
import { platform } from '../platform'

const QUICK_EMOJIS = ['👍', '❤️', '😂', '😮', '😢', '🔥', '🎉', '👀']

interface Props {
  x: number
  y: number
  text: string
  channelId?: string
  msgTs?: number
  onReact?: (emoji: string, action: 'add' | 'remove') => void
  onClose: () => void
}

export function MessageContextMenu({ x, y, text, channelId, msgTs, onReact, onClose }: Props) {
  const ref = useRef<HTMLDivElement>(null)
  const onCloseRef = useRef(onClose)

  useEffect(() => {
    onCloseRef.current = onClose
  }, [onClose])

  useEffect(() => {
    const handleClickOutside = (event: MouseEvent) => {
      if (ref.current && !ref.current.contains(event.target as Node)) {
        onCloseRef.current()
      }
    }
    document.addEventListener('mousedown', handleClickOutside)
    return () => document.removeEventListener('mousedown', handleClickOutside)
  }, [])

  const menuWidth = 200
  const hasReact = !!(onReact && channelId && msgTs != null)
  const menuHeight = hasReact ? 108 : 44
  const winW = typeof window !== 'undefined' ? window.innerWidth : 1000
  const winH = typeof window !== 'undefined' ? window.innerHeight : 800
  const safeX = Math.min(x, winW - menuWidth - 16)
  const safeY = Math.min(y, winH - menuHeight - 16)

  return (
    <div
      ref={ref}
      data-testid="message-context-menu"
      className="fixed z-50 bg-[var(--bg-app)] border border-[var(--bg-active)] rounded-lg shadow-xl py-1"
      style={{ left: safeX, top: safeY, width: menuWidth }}
      onContextMenu={(e) => e.preventDefault()}
    >
      {hasReact && (
        <div
          data-testid="message-context-emojis"
          className="flex justify-between px-2 py-1.5 border-b border-[var(--bg-active)]"
        >
          {QUICK_EMOJIS.map((em) => (
            <button
              key={em}
              type="button"
              data-testid={`quick-react-${em}`}
              data-emoji={em}
              className="w-6 h-6 flex items-center justify-center rounded hover:bg-[var(--bg-hover)] transition-colors text-[15px] leading-none"
              onClick={() => {
                onReact!(em, 'add')
                onClose()
              }}
            >
              {em}
            </button>
          ))}
        </div>
      )}
      <button
        data-testid="message-context-copy"
        className="w-full text-left px-3 py-2 text-[13px] text-[var(--text-main)] hover:bg-[var(--bg-hover)] transition-colors"
        onClick={() => {
          if (text) platform.copyText(text).catch(console.error)
          onClose()
        }}
      >
        复制文本
      </button>
    </div>
  )
}
