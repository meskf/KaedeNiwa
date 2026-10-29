import { useEffect, useRef } from 'react'

interface Props {
  x: number
  y: number
  text: string
  onClose: () => void
}

export function MessageContextMenu({ x, y, text, onClose }: Props) {
  const ref = useRef<HTMLDivElement>(null)
  const onCloseRef = useRef(onClose)

  // 保持最新的 onClose 引用，避免外部状态更新导致频繁解绑/绑定
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

  const menuWidth = 140
  const menuHeight = 44
  const winW = typeof window !== 'undefined' ? window.innerWidth : 1000
  const winH = typeof window !== 'undefined' ? window.innerHeight : 800
  const safeX = Math.min(x, winW - menuWidth - 16)
  const safeY = Math.min(y, winH - menuHeight - 16)

  return (
    <div
      ref={ref}
      className="fixed z-50 bg-[var(--bg-app)] border border-[var(--bg-active)] rounded-lg shadow-xl py-1"
      style={{ left: safeX, top: safeY, width: menuWidth }}
      onContextMenu={(e) => e.preventDefault()}
    >
      <button
        className="w-full text-left px-3 py-2 text-[13px] text-[var(--text-main)] hover:bg-[var(--bg-hover)] transition-colors"
        onClick={() => {
          if (navigator.clipboard && text) {
            navigator.clipboard.writeText(text).catch(console.error)
          }
          onClose()
        }}
      >
        复制文本
      </button>
    </div>
  )
}
