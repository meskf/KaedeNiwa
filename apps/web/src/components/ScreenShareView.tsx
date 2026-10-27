import { useEffect, useRef, useState } from 'react'
import { useMediaState } from '../hooks/useLiveKitRoom'

function VideoPane({ share, multi }: { share: any; multi: boolean }) {
  const videoRef = useRef<HTMLVideoElement>(null)
  const containerRef = useRef<HTMLDivElement>(null)
  const [fullscreen, setFullscreen] = useState(false)
  const [videoDims, setVideoDims] = useState<{ w: number; h: number } | null>(null)

  useEffect(() => {
    const video = videoRef.current
    const track = share.track
    if (!video || !track) return
    const el = track.attach(video) as HTMLVideoElement
    el.autoplay = true
    el.playsInline = true
    el.muted = true
    el.play().catch((e: any) => console.warn('[screenshare] play failed', e))
    
    const update = () => {
      if (video.videoWidth && video.videoHeight) {
        setVideoDims({ w: video.videoWidth, h: video.videoHeight })
      }
    }
    video.addEventListener('loadedmetadata', update)
    video.addEventListener('resize', update)
    update()
    return () => {
      video.removeEventListener('loadedmetadata', update)
      video.removeEventListener('resize', update)
      try { track.detach(video) } catch {}
    }
  }, [share.track, share.trackSid])

  useEffect(() => {
    const onFs = () => setFullscreen(!!document.fullscreenElement)
    document.addEventListener('fullscreenchange', onFs)
    return () => document.removeEventListener('fullscreenchange', onFs)
  }, [])

  const toggleFs = async () => {
    if (!containerRef.current) return
    try {
      if (document.fullscreenElement) await document.exitFullscreen()
      else await containerRef.current.requestFullscreen()
    } catch (e) {
      console.error('[screenshare] fullscreen:', e)
    }
  }

  // 容器样式：严格限制最大高度，保证不撑爆外层 Flex 布局
  const containerStyle: React.CSSProperties = {
    width: '100%',
    maxWidth: multi ? '100%' : '1000px',
    maxHeight: multi ? '25vh' : '30vh',
    aspectRatio: videoDims ? `${videoDims.w} / ${videoDims.h}` : '16 / 9',
    margin: '0 auto',
  }

  return (
    <div
      ref={containerRef}
      className="relative rounded-xl overflow-hidden bg-black flex items-center justify-center group shadow-lg"
      style={containerStyle}
    >
      <video
        ref={videoRef}
        data-testid="screen-share-video"
        autoPlay
        playsInline
        muted
        className="cursor-pointer w-full h-full object-contain"
        onClick={toggleFs}
      />
      
      {/* 全屏按钮：只在鼠标悬浮时显示，放在右上角，避免遮挡关键内容 */}
      <div className="absolute top-3 right-3 opacity-0 group-hover:opacity-100 transition-opacity">
        <button
          onClick={toggleFs}
          className="p-2 text-xs bg-black/60 hover:bg-black/90 text-white rounded-md backdrop-blur-sm transition-colors"
          title={fullscreen ? 'Exit fullscreen' : 'Fullscreen'}
        >
          {fullscreen ? '\u2922' : '\u26F6'}
        </button>
      </div>

      {/* 底部用户名标识 */}
      <div className="absolute bottom-3 left-3 px-2.5 py-1 bg-black/60 backdrop-blur-sm text-white text-xs rounded-md pointer-events-none flex items-center gap-1.5 shadow-sm">
        {share.hasAudio && <span title="includes audio">{String.fromCodePoint(0x1F50A)}</span>}
        <span>{share.participantIdentity}</span>
      </div>
    </div>
  )
}

export function ScreenShareView() {
  const media = useMediaState()
  const uniqueShares = Array.from(new Map(media.screenShares.map(s => [s.trackSid, s])).values())
  if (uniqueShares.length === 0) return null
  const count = uniqueShares.length

  if (count === 1) {
    return (
      <div className="w-full mb-4 flex justify-center" data-testid="screenshare-panel">
        <VideoPane share={uniqueShares[0]} multi={false} />
      </div>
    )
  }

  const cols = count === 2 ? 2 : count <= 4 ? 2 : 3
  return (
    <div className="w-full mb-4" data-testid="screenshare-panel">
      <div className="w-full mb-2 text-xs text-[var(--text-secondary)] font-medium">
        \uD83D\uDCFA Screen sharing ({count})
      </div>
      <div className="grid gap-3 w-full" style={{ gridTemplateColumns: 'repeat(' + cols + ', minmax(0, 1fr))' }}>
        {uniqueShares.map(share => (
          <VideoPane key={share.trackSid} share={share} multi={true} />
        ))}
      </div>
    </div>
  )
}