import { useState, useEffect } from 'react'
import { platform } from '../platform'

function IconMin() {
  return (
    <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
      <rect x="0" y="4.5" width="10" height="1" fill="currentColor" />
    </svg>
  )
}
function IconMax() {
  return (
    <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
      <rect x="0.5" y="0.5" width="9" height="9" fill="none" stroke="currentColor" strokeWidth="1" />
    </svg>
  )
}
function IconRestore() {
  return (
    <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
      <rect x="2.5" y="0.5" width="7" height="7" fill="none" stroke="currentColor" strokeWidth="1" />
      <rect x="0.5" y="2.5" width="7" height="7" fill="currentColor" stroke="currentColor" strokeWidth="1" />
    </svg>
  )
}
function IconClose() {
  return (
    <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
      <path d="M0.5 0.5 L9.5 9.5 M9.5 0.5 L0.5 9.5" stroke="currentColor" strokeWidth="1.2" />
    </svg>
  )
}

interface WCApi {
  minimize(): void
  toggleMaximize(): void
  close(): void
  isMaximized(): Promise<boolean>
  onMaximizeChange(cb: (isMax: boolean) => void): () => void
}

export function WindowControls() {
  const [isMax, setIsMax] = useState(false)
  const [hover, setHover] = useState<'min' | 'max' | 'close' | null>(null)

  // 仅 electron 渲染。e2e 测试（playwright chromium）platform.kind === 'web' 零渲染。
  if (platform.kind !== 'electron') return null
  const wc = (window as any).windowControls as WCApi | undefined
  if (!wc) return null

  useEffect(() => {
    wc.isMaximized().then(setIsMax).catch(() => {})
    const unsub = wc.onMaximizeChange(setIsMax)
    return () => unsub()
  }, [wc])

  const Btn = ({
    name, onClick, children, isClose,
  }: {
    name: 'min' | 'max' | 'close'
    onClick: () => void
    children: React.ReactNode
    isClose?: boolean
  }) => {
    const hovered = hover === name
    const bg = hovered ? (isClose ? '#e81123' : 'rgba(255,255,255,0.10)') : 'transparent'
    const color = hovered && isClose ? '#ffffff' : 'var(--text-muted)'
    return (
      <button
        type="button"
        data-testid={`window-control-${name === 'min' ? 'minimize' : name === 'max' ? 'maximize' : 'close'}`}
        onClick={onClick}
        onMouseEnter={() => setHover(name)}
        onMouseLeave={() => setHover(null)}
        style={{
          WebkitAppRegion: 'no-drag',
          background: bg,
          color,
          width: 46,
          height: 32,
        } as React.CSSProperties}
        className="flex items-center justify-center transition-colors"
      >
        {children}
      </button>
    )
  }

  return (
    <div
      data-testid="window-controls"
      data-maximized={isMax ? '1' : '0'}
      className="fixed top-0 right-0 z-[9999] flex items-stretch select-none"
      style={{ WebkitAppRegion: 'drag' } as React.CSSProperties}
    >
      <Btn name="min" onClick={() => wc.minimize()}><IconMin /></Btn>
      <Btn name="max" onClick={() => wc.toggleMaximize()}>{isMax ? <IconRestore /> : <IconMax />}</Btn>
      <Btn name="close" onClick={() => wc.close()} isClose><IconClose /></Btn>
    </div>
  )
}
