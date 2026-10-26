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
  const cols = count === 1 ? 1 : count <= 4 ? 2 : 3
  const rows = Math.ceil(count / cols)

  // 悬浮画中画样式：固定宽高，绝对定位于右上角，带阴影
  // 宽高会随着人数微调，但不挤压底层布局
  return (
    <div className="absolute top-4 right-4 z-40 w-[45%] max-w-[700px] h-[35vh] min-h-[220px] bg-[#1e1f22]/95 backdrop-blur-sm rounded-xl shadow-2xl border border-[#2b2d31] flex flex-col p-1.5 animate-in fade-in zoom-in-95">
      <div
        className="w-full h-full grid gap-1.5"
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
