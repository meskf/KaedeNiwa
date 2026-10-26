import { useEffect, useRef } from 'react'
import { useMediaState } from '../hooks/useLiveKitRoom'

export function ScreenShareView() {
  const media = useMediaState()
  const containerRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const container = containerRef.current
    if (!container) return
    while (container.firstChild) container.removeChild(container.firstChild)

    for (const share of media.screenShares) {
      const wrapper = document.createElement('div')
      wrapper.className = 'relative rounded-lg overflow-hidden bg-black flex items-center justify-center shadow-md border border-[#2b2d31]'
      share.element.className = 'w-full h-full object-contain'
      wrapper.appendChild(share.element)

      const label = document.createElement('div')
      label.className = 'absolute bottom-2 left-2 px-2 py-1 bg-black/70 text-white text-xs rounded pointer-events-none'
      label.textContent = share.participantIdentity + ' 正在共享'
      wrapper.appendChild(label)

      container.appendChild(wrapper)
    }
  }, [media.screenShares])

  if (media.screenShares.length === 0) return null

  const count = media.screenShares.length
  const cols = count === 1 ? 1 : count <= 4 ? 2 : 3
  const rows = Math.ceil(count / cols)

  return (
    <div className="w-full h-full p-2 flex-1 min-h-0 bg-[#1e1f22]">
      <div
        ref={containerRef}
        className="w-full h-full max-w-full max-h-full grid gap-2"
        style={{
          gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))`,
          gridTemplateRows: `repeat(${rows}, minmax(0, 1fr))`,
        }}
      />
    </div>
  )
}
