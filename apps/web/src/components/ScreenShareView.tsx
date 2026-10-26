import { useEffect, useRef } from 'react'
import { useMediaState } from '../hooks/useLiveKitRoom'

function VideoAttachment({ share }: { share: any }) {
  const videoRef = useRef<HTMLVideoElement>(null)

  useEffect(() => {
    if (!videoRef.current || !share.element) return
    const el = share.element
    if (!videoRef.current.srcObject) {
      videoRef.current.srcObject = el.srcObject
    }
    return () => {
      if (videoRef.current) {
        videoRef.current.srcObject = null
      }
    }
  }, [share.element])

  return (
    <div className="relative w-full h-full rounded-lg overflow-hidden bg-black flex items-center justify-center border border-[#2b2d31]">
      <video
        ref={videoRef}
        autoPlay
        playsInline
        muted
        className="w-full h-full object-contain"
      />
      <div className="absolute bottom-1 left-1 px-1.5 py-0.5 bg-black/70 text-white text-[10px] rounded pointer-events-none">
        {share.participantIdentity}
      </div>
    </div>
  )
}

export function ScreenShareView() {
  const media = useMediaState()

  if (media.screenShares.length === 0) return null

  const uniqueShares = Array.from(new Map(media.screenShares.map(s => [s.trackSid, s])).values())
  const count = uniqueShares.length
  // 动态列数：1人占满，2-4人两列，5人以上三列
  const cols = count === 1 ? 1 : count <= 4 ? 2 : 3
  const rows = Math.ceil(count / cols)

  return (
    <div 
      className="absolute top-16 right-4 z-50 bg-[#1e1f22] rounded-xl shadow-2xl border border-[#2b2d31] p-2 flex flex-col animate-in fade-in zoom-in-95"
      style={{ 
        // 强制写死宽度，彻底避免被外层 Flex 挤压变形成竖条
        width: count === 1 ? '480px' : '640px', 
        maxWidth: '60vw'
      }}
    >
      <div
        className="w-full grid gap-2"
        style={{
          gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))`,
          gridTemplateRows: `repeat(${rows}, minmax(0, 1fr))`,
          // 如果是单人共享，强制保持 16:9 的标准比例
          aspectRatio: count === 1 ? '16/9' : 'auto',
          minHeight: count === 1 ? 'auto' : '300px'
        }}
      >
        {uniqueShares.map(share => (
          <VideoAttachment key={share.trackSid} share={share} />
        ))}
      </div>
    </div>
  )
}
