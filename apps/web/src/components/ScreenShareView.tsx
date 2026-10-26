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
      <div className="absolute bottom-2 left-2 px-2 py-1 bg-black/70 text-white text-xs rounded pointer-events-none">
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

  return (
    <div className="w-full max-w-4xl mx-auto mb-6 flex flex-col items-center">
      <div className="w-full mb-2 flex items-center justify-start text-sm text-[var(--text-secondary)] font-medium">
        📺 屏幕共享中 ({count})
      </div>
      <div
        className="w-full bg-[#1e1f22] rounded-xl overflow-hidden shadow-sm border border-[#2b2d31] p-2"
        style={{
          aspectRatio: count === 1 ? '16/9' : 'auto',
          minHeight: count > 1 ? '300px' : 'auto',
        }}
      >
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
    </div>
  )
}
