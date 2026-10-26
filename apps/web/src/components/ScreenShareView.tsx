import { useEffect, useRef, useState } from 'react'
import { useMediaState } from '../hooks/useLiveKitRoom'
import { IconMonitor } from './icons'

/**
 * 显示远端屏幕共享画面（画中画模式，不遮挡全局布局）。
 */
export function ScreenShareView() {
  const media = useMediaState()
  const containerRef = useRef<HTMLDivElement>(null)
  const [minimized, setMinimized] = useState(false)

  useEffect(() => {
    if (minimized) return
    const container = containerRef.current
    if (!container) return
    while (container.firstChild) container.removeChild(container.firstChild)

    for (const share of media.screenShares) {
      const wrapper = document.createElement('div')
      wrapper.className = 'relative rounded-lg overflow-hidden bg-black flex items-center justify-center shadow-lg border border-white/10'
      share.element.className = 'w-full h-full object-contain'
      wrapper.appendChild(share.element)

      const label = document.createElement('div')
      label.className = 'absolute bottom-2 left-2 px-2 py-1 bg-black/70 text-white text-xs rounded pointer-events-none'
      label.textContent = share.participantIdentity + ' 正在共享'
      wrapper.appendChild(label)

      container.appendChild(wrapper)
    }
  }, [media.screenShares, minimized])

  if (media.screenShares.length === 0) return null

  const count = media.screenShares.length
  const cols = count === 1 ? 1 : count <= 4 ? 2 : 3

  if (minimized) {
    return (
      <button
        onClick={() => setMinimized(false)}
        className="absolute top-16 right-4 z-40 bg-[var(--bg-active)] text-[var(--text-main)] px-3 py-2 rounded-lg shadow-lg flex items-center gap-2 hover:bg-[#3f3f46]"
      >
        <IconMonitor className="w-4 h-4" /> 展开共享画面 ({count})
      </button>
    )
  }

  return (
    <div className="absolute inset-0 z-40 bg-black/60 flex flex-col items-center justify-center p-4 animate-in fade-in">
      <div className="w-full max-w-5xl h-full max-h-[70vh] flex flex-col bg-[#1e1f22] rounded-xl overflow-hidden shadow-2xl border border-[#2b2d31]">
        {/* 顶部控制栏 */}
        <div className="flex items-center justify-between px-4 py-2 bg-[#2b2d31] text-xs font-medium text-[#dbdee1]">
          <span>屏幕共享画面</span>
          <div className="flex gap-2">
            <button onClick={() => setMinimized(true)} className="hover:text-white transition-colors">最小化</button>
          </div>
        </div>

        {/* 视频网格 */}
        <div
          ref={containerRef}
          className="flex-1 grid gap-2 p-2 overflow-hidden"
          style={{
            gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))`,
            gridTemplateRows: `repeat(${Math.ceil(count / cols)}, minmax(0, 1fr))`,
          }}
        />
      </div>
    </div>
  )
}
