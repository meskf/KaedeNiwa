import { useEffect, useRef } from 'react'
import { useMediaState } from '../hooks/useLiveKitRoom'

/**
 * 显示远端屏幕共享画面。
 * 悬浮在聊天区域上方，支持多人平铺，不挤压底部输入框。
 */
export function ScreenShareView() {
  const media = useMediaState()
  const containerRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const container = containerRef.current
    if (!container) return
    // 清空旧节点
    while (container.firstChild) {
      container.removeChild(container.firstChild)
    }
    // 把每个共享的 video element attach 进来
    for (const share of media.screenShares) {
      const wrapper = document.createElement('div')
      wrapper.className = 'relative rounded-lg overflow-hidden bg-black flex items-center justify-center shadow-lg border border-white/10'
      share.element.className = 'w-full h-full object-contain'
      wrapper.appendChild(share.element)

      // 参与者标签
      const label = document.createElement('div')
      label.className = 'absolute bottom-2 left-2 px-2 py-1 bg-black/70 text-white text-xs rounded pointer-events-none'
      label.textContent = share.participantIdentity + ' 正在共享'
      wrapper.appendChild(label)

      container.appendChild(wrapper)
    }
  }, [media.screenShares])

  if (media.screenShares.length === 0) return null

  // 根据共享人数动态计算列数
  const count = media.screenShares.length
  const cols = count === 1 ? 1 : count <= 4 ? 2 : 3

  return (
    <div className="absolute top-4 left-4 right-4 bottom-20 z-40 pointer-events-none flex items-center justify-center">
      <div
        ref={containerRef}
        className="grid gap-4 p-2 w-full h-full max-w-7xl pointer-events-auto"
        style={{
          gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))`,
          gridAutoRows: 'minmax(0, 1fr)',
        }}
      />
    </div>
  )
}
