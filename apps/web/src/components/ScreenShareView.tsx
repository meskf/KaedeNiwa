import { useEffect, useRef } from 'react'
import { useMediaState } from '../hooks/useLiveKitRoom'

function VideoAttachment({ share }: { share: any }) {
  const videoRef = useRef<HTMLVideoElement>(null)

  useEffect(() => {
    if (!videoRef.current || !share.element) return
    const el = share.element
    // 将原生 track 挂载到我们的 video 元素上
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
    <div className="relative w-full h-full rounded-lg overflow-hidden bg-black flex items-center justify-center shadow-md border border-[#2b2d31]">
      <video
        ref={videoRef}
        autoPlay
        playsInline
        muted
        className="w-full h-full object-contain"
      />
      <div className="absolute bottom-2 left-2 px-2 py-1 bg-black/70 text-white text-xs rounded pointer-events-none">
        {share.participantIdentity} 正在共享
      </div>
    </div>
  )
}

export function ScreenShareView() {
  const media = useMediaState()

  if (media.screenShares.length === 0) return null

  // 按 participantIdentity 去重，防止同一个人出现两遍
  const uniqueShares = Array.from(new Map(media.screenShares.map(s => [s.trackSid, s])).values())
  const count = uniqueShares.length
  const cols = count === 1 ? 1 : count <= 4 ? 2 : 3
  const rows = Math.ceil(count / cols)

  return (
    <div className="w-full h-full p-2 flex-1 min-h-0 bg-[#1e1f22]">
      <div
        className="w-full h-full grid gap-2"
        style={{
          gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))`,
          gridTemplateRows: `repeat(${rows}, minmax(0, 1fr))`,
        }}
      >
        {uniqueShares.map(share => (
          <VideoAttachment key={share.trackSid} share={share} />
        ))}
      </div>
    </div>
  )
}
