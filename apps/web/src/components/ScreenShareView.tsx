import { useEffect, useRef, useState } from 'react'
import { ScreenOrientation } from '@capacitor/screen-orientation'
import { useMediaState } from '../hooks/useLiveKitRoom'
import { setScreenShareQuality } from '../lib/livekit'
import { VIDEO_PRESETS } from '../lib/videoPresets'
import { getDiag } from '../lib/mediaHealth'
import type { LinkInfo } from '../lib/livekit'

function isMobileViewport() {
  if (typeof window === 'undefined') return false
  return window.innerWidth < 900 || /Android|iPhone|iPad|iPod|Mobile/i.test(navigator.userAgent)
}

function isNativeApp() {
  try { return !!(window as any).Capacitor?.isNativePlatform?.() } catch { return false }
}

/**
 * 锁定横屏。
 *
 * - Capacitor 原生壳：走 @capacitor/screen-orientation（Android/iOS 都可靠）。
 * - 纯 Web（浏览器/桌面）：退回标准 Screen Orientation API；浏览器要求先进全屏，
 *   所以调用顺序必须是 requestFullscreen -> lock('landscape')。
 */
async function lockLandscape(): Promise<boolean> {
  if (isNativeApp()) {
    try {
      await ScreenOrientation.lock({ orientation: 'landscape' })
      return true
    } catch (e) {
      console.warn('[ScreenShare] Capacitor 横屏锁定失败:', e)
    }
  }
  try {
    const so = (screen as any).orientation
    if (so?.lock) {
      await so.lock('landscape')
      return true
    }
  } catch (e) {
    console.warn('[ScreenShare] Screen Orientation API 横屏锁定失败:', e)
  }
  return false
}

async function unlockOrientation() {
  if (isNativeApp()) {
    try { await ScreenOrientation.unlock() } catch {}
  }
  try { (screen as any).orientation?.unlock?.() } catch {}
}

function linkLabel(link: LinkInfo | null, local: boolean, note: string | null): { text: string; tone: 'ok' | 'warn' | 'bad' } {
  if (local && note) return { text: note, tone: 'warn' }
  if (!link) return { text: '', tone: 'ok' }
  if (link.transport === 'direct-tcp') {
    return { text: `TCP 回落 · 丢包 ${link.lossPct.toFixed(1)}% · 放行 UDP 7882`, tone: 'bad' }
  }
  // 有码率流入却解不出画面：不是带宽问题，是关键帧丢了（已在自动重订阅）
  if (link.stalledSec >= 2) {
    return { text: `画面无帧 ${link.stalledSec}s · 关键帧丢失 · 已自动重订阅`, tone: 'bad' }
  }
  if (link.severity === 'bad') {
    if (link.reasons.includes('packet-loss')) return { text: `丢包 ${link.lossPct.toFixed(1)}%`, tone: 'bad' }
    if (link.reasons.includes('freezes')) return { text: `画面冻结 · ${link.inFps.toFixed(0)}fps`, tone: 'bad' }
    return { text: link.reasons.join(','), tone: 'bad' }
  }
  // 平均帧率正常但帧间隔不均 → 观众看到的就是“一卡一卡”，必须显式提示
  if (link.reasons.includes('unstable-pacing')) {
    return { text: `帧不稳 ±${Math.round(link.pacingStdMs)}ms · ${link.inFps.toFixed(0)}fps`, tone: 'warn' }
  }
  const t = link.transport === 'relay' ? 'TURN' : link.transport.toUpperCase()
  if (link.severity === 'warn') return { text: `${t} · ${link.inFps.toFixed(0)}fps`, tone: 'warn' }
  return { text: `${t} · ${link.inFps.toFixed(0)}fps`, tone: 'ok' }
}

function VideoPane({ share, multi, link, note }: { share: any; multi: boolean; link: LinkInfo | null; note: string | null }) {
  const videoRef = useRef<HTMLVideoElement>(null)
  const containerRef = useRef<HTMLDivElement>(null)
  const [fullscreen, setFullscreen] = useState(false)
  // 原生 App 的 WebView 通常不支持 Fullscreen API，单靠 requestFullscreen 只会转屏
  // 不会放大。这里用独立的“铺满视口”状态兜底，保证手机端一定能放大。
  const [bigView, setBigView] = useState(false)
  const [videoDims, setVideoDims] = useState<{ w: number; h: number } | null>(null)
  const expanded = fullscreen || bigView

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
    const onFs = () => {
      const active = !!document.fullscreenElement
      setFullscreen(active)
      // 退出全屏时恢复系统方向，避免 App 一直卡在横屏
      if (!active) {
        setBigView(false)
        void unlockOrientation()
      }
    }
    document.addEventListener('fullscreenchange', onFs)
    return () => document.removeEventListener('fullscreenchange', onFs)
  }, [])

  /** 退出放大：同时清掉原生兜底视图、浏览器全屏和方向锁。 */
  const collapse = async () => {
    try {
      if (document.fullscreenElement) await document.exitFullscreen()
    } catch (e) {
      console.warn('[screenshare] exitFullscreen:', e)
    }
    setBigView(false)
    await unlockOrientation()
  }

  const toggleFullscreen = async () => {
    if (!containerRef.current) return
    if (expanded) {
      await collapse()
      return
    }
    try {
      await containerRef.current.requestFullscreen()
    } catch (e) {
      console.error('[screenshare] fullscreen:', e)
    }
  }

  /** 手机端“横屏放大”：铺满视口 + 全屏 + 方向锁定，一键看大图。 */
  const enterBigView = async () => {
    const container = containerRef.current
    if (!container) return
    setBigView(true)
    try {
      if (!document.fullscreenElement) await container.requestFullscreen()
    } catch (e) {
      console.warn('[screenshare] requestFullscreen 失败（使用铺满视口兜底）:', e)
    }
    const ok = await lockLandscape()
    if (!ok) console.warn('[screenshare] 横屏锁定不可用，已仅放大画面')
  }

  const local = String(share.trackSid).startsWith('LOCAL-SHARE-')
  const badge = linkLabel(link, local, note)

  const containerStyle: React.CSSProperties = expanded
    ? {
        width: '100vw',
        height: '100vh',
        maxWidth: 'none',
        maxHeight: 'none',
        backgroundColor: '#000',
        // 非 Fullscreen API 的兜底：直接铺满视口（Android WebView 场景）
        ...(fullscreen ? {} : { position: 'fixed', top: 0, left: 0, right: 0, bottom: 0, zIndex: 50 }),
      }
    : {
        width: '100%',
        maxWidth: multi ? '100%' : '1000px',
        maxHeight: multi ? '25vh' : '30vh',
        aspectRatio: videoDims ? `${videoDims.w} / ${videoDims.h}` : '16 / 9',
        margin: '0 auto',
        backgroundColor: '#000000',
        borderRadius: '0.75rem',
      }

  return (
    <div
      ref={containerRef}
      className={`relative overflow-hidden flex items-center justify-center group ${expanded ? '' : 'rounded-xl shadow-lg'}`}
      style={containerStyle}
    >
      <video
        ref={videoRef}
        data-testid="screen-share-video"
        autoPlay
        playsInline
        muted
        className="cursor-pointer w-full h-full object-contain"
        onClick={toggleFullscreen}
      />

      {/* 接收画质（仅远端共享，本地预览隐藏） */}
      {!local && (
        <div className="absolute top-3 right-24 opacity-0 group-hover:opacity-100 transition-opacity">
          <select
            data-testid={`screen-share-quality-${share.trackSid}`}
            value={share.presetId || '1080p60'}
            onChange={(e) => setScreenShareQuality(share.trackSid, e.target.value as any)}
            onClick={(e) => e.stopPropagation()}
            className="p-1.5 text-xs bg-black/60 hover:bg-black/90 text-white rounded-md backdrop-blur-sm outline-none cursor-pointer"
            title="接收画质"
          >
            {VIDEO_PRESETS.map(preset => (
              <option key={preset.id} value={preset.id}>{preset.label}</option>
            ))}
          </select>
        </div>
      )}

      {/* 手机端：横屏放大（全屏 + 方向锁定） */}
      <div className="absolute top-3 right-14 opacity-100 sm:opacity-0 sm:group-hover:opacity-100 transition-opacity">
        <button
          data-testid="screen-share-landscape"
          onClick={(e) => { e.stopPropagation(); void enterBigView() }}
          className="px-2.5 py-1.5 text-[11px] font-medium bg-black/70 hover:bg-black/90 text-white rounded-md backdrop-blur-sm transition-colors whitespace-nowrap"
          title="横屏放大"
        >
          ⤢ 横屏放大
        </button>
      </div>

      {/* 全屏按钮（桌面） */}
      <div className="absolute top-3 right-3 opacity-100 sm:opacity-0 sm:group-hover:opacity-100 transition-opacity">
        <button
          data-testid="screen-share-fullscreen"
          onClick={(e) => { e.stopPropagation(); void toggleFullscreen() }}
          className="p-2 text-xs bg-black/60 hover:bg-black/90 text-white rounded-md backdrop-blur-sm transition-colors"
          title={expanded ? 'Exit fullscreen' : 'Fullscreen'}
        >
          {expanded ? '\u2922' : '\u26F6'}
        </button>
      </div>

      {/* 底部用户名标识 + 链路健康度 */}
      <div className="absolute bottom-3 left-3 px-2.5 py-1 bg-black/60 backdrop-blur-sm text-white text-xs rounded-md pointer-events-none flex items-center gap-2 shadow-sm">
        {share.hasAudio && <span title="includes audio">{String.fromCodePoint(0x1F50A)}</span>}
        <span>{share.participantIdentity}</span>
        {badge.text && (
          <span
            data-testid="screen-share-health"
            data-tone={badge.tone}
            className={`px-1.5 py-0.5 rounded text-[10px] font-mono ${
              badge.tone === 'bad' ? 'bg-red-500/80' : badge.tone === 'warn' ? 'bg-amber-500/80' : 'bg-emerald-600/70'
            }`}
          >
            {badge.text}
          </span>
        )}
      </div>

      {/* 冻结问题现场取证：不用边打游戏边敲控制台，点一下导出最近 30 秒逐秒数据 */}
      <button
        data-testid="screen-share-diag"
        onClick={() => {
          const blob = new Blob([JSON.stringify(getDiag(), null, 2)], { type: 'application/json' })
          const a = document.createElement('a')
          a.href = URL.createObjectURL(blob)
          a.download = `kaede-diag-${new Date().toISOString().replace(/[:.]/g, '-')}.json`
          a.click()
          setTimeout(() => URL.revokeObjectURL(a.href), 5_000)
        }}
        title="导出最近 30 秒媒体诊断（含逐秒帧率/丢包/关键帧/冻结事件）"
        className="absolute bottom-3 right-3 px-2 py-1 bg-black/60 backdrop-blur-sm text-white text-[10px] rounded-md hover:bg-black/80"
      >
        诊断
      </button>
    </div>
  )
}

export function ScreenShareView() {
  const media = useMediaState()
  const uniqueShares = Array.from(new Map(media.screenShares.map(s => [s.trackSid, s])).values())
  if (uniqueShares.length === 0) return null
  const count = uniqueShares.length
  const mobile = isMobileViewport()

  if (count === 1) {
    return (
      <div className="w-full mb-4 flex justify-center" data-testid="screenshare-panel">
        <VideoPane share={uniqueShares[0]} multi={false} link={media.link} note={media.autoQualityNote} />
      </div>
    )
  }

  const cols = count === 2 ? 2 : count <= 4 ? 2 : 3
  return (
    <div className="w-full mb-4" data-testid="screenshare-panel">
      <div className="w-full mb-2 text-xs text-[var(--text-secondary)] font-medium">
        {'\uD83D\uDCFA'} 屏幕共享（{count}）
        {mobile && <span className="ml-2 text-[10px]">点右上角「横屏放大」全屏观看</span>}
      </div>
      <div className="grid gap-3 w-full" style={{ gridTemplateColumns: 'repeat(' + cols + ', minmax(0, 1fr))' }}>
        {uniqueShares.map((share, idx) => (
          <VideoPane key={`${share.trackSid}-${idx}`} share={share} multi={true} link={media.link} note={media.autoQualityNote} />
        ))}
      </div>
    </div>
  )
}
