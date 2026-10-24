import { useEffect, useRef } from 'react'
import { useMediaState } from '../hooks/useLiveKitRoom'

/**
 * 显示远端屏幕共享画面。
 * 有多个共享时用网格平铺。
 */
export function ScreenShareView() {
  const media = useMediaState()
  const containerRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const container = containerRef.current
    if (!container) return
    // 清空旧节点（保留 React 自己的）
    while (container.firstChild) {
      container.removeChild(container.firstChild)
    }
    // 把每个共享的 video element attach 进来
    for (const share of media.screenShares) {
      const wrapper = document.createElement('div')
      wrapper.className = 'relative rounded-lg overflow-hidden bg-black flex items-center justify-center'
      share.element.className = 'w-full h-full object-contain'
      wrapper.appendChild(share.element)
      
      // 参与者标签
      const label = document.createElement('div')
      label.className = 'absolute bottom-2 left-2 px-2 py-1 bg-black/70 text-white text-xs rounded'
      label.textContent = share.participantIdentity + ' 正在共享'
      wrapper.appendChild(label)
      
      container.appendChild(wrapper)
    }
  }, [media.screenShares])

  if (media.screenShares.length === 0) return null

  return (
    <div
      ref={containerRef}
      className="grid gap-2 p-2"
      style={{ gridTemplateColumns: `repeat(${Math.min(media.screenShares.length, 2)}, 1fr)` }}
    />
  )
}
