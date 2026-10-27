import { useState } from 'react'
import { useMediaState } from '../hooks/useLiveKitRoom'
import {
  startScreenShare,
  stopScreenShare,
  enableMicrophone,
  disableMicrophone,
  setNoiseSuppression,
  disconnectMedia,
} from '../lib/livekit'
import { VIDEO_PRESETS } from '../lib/videoPresets'
import { getClient } from '../wasm/client'
import { IconMic, IconMicOff, IconMonitor, IconShield } from './icons'

interface Props {
  channelId: string
}

export function MediaBar({ channelId }: Props) {
  const media = useMediaState()
  const [sharing, setSharing] = useState(false)
  const [micOn, setMicOn] = useState(false)
  const [presetId, setPresetId] = useState('1080p60')
  const [noiseOn, setNoiseOn] = useState(true)
  const [error, setError] = useState('')

  if (!media.connected || media.channelId !== channelId) return null

  const handleToggleShare = async () => {
    setError('')
    try {
      if (sharing) {
        await stopScreenShare()
        setSharing(false)
      } else {
        await startScreenShare(presetId)
        setSharing(true)
      }
    } catch (e: any) {
      setError(String(e))
    }
  }

  const handleToggleMic = async () => {
    setError('')
    try {
      if (micOn) {
        await disableMicrophone()
        setMicOn(false)
      } else {
        await enableMicrophone(noiseOn)
        setMicOn(true)
      }
    } catch (e: any) {
      setError(String(e))
    }
  }

  const handleToggleNoise = async () => {
    const next = !noiseOn
    setNoiseOn(next)
    setNoiseSuppression(next)
    // RNNoise 需要在 getUsermedia 之后建立流水线；运行时切换必须重建轨道。
    // 简化实现：提示用户手动重开麦生效（重建 track 会让对端看到重连，代价较大）。
    if (media.localMicEnabled) {
      // 立即关闭再开启，触发 pipeline 重建
      try {
        await disableMicrophone()
        await enableMicrophone(next)
        setMicOn(true)
      } catch (e: any) {
        setError(String(e))
      }
    }
  }

  const handleLeave = async () => {
    const client = getClient()
    if (client) {
      try { await client.leave_media_room(channelId) } catch (e) { /* ignore */ }
    }
    await disconnectMedia()
    setSharing(false)
    setMicOn(false)
  }

  return (
    <div className="bg-[var(--bg-surface)] px-4 py-2 shrink-0">
      <div className="flex items-center gap-3 flex-wrap">
        <div data-testid="media-connected" data-channel={channelId} className="flex items-center gap-1">
          <span className="w-2 h-2 rounded-full bg-[var(--text-primary)] animate-pulse" />
          <span className="text-xs text-[var(--text-primary)] font-medium">已连接 #{channelId}</span>
        </div>
        {media.e2eeEnabled ? (
          <span data-testid="media-e2ee" data-e2ee="true" className="text-xs px-2 py-0.5 rounded bg-[var(--bg-active)] text-[var(--text-primary)] font-medium" title="AES-256-GCM 端到端加密">
            <IconShield className="w-3 h-3 inline mr-1" /> E2EE
          </span>
        ) : (
          <span data-testid="media-e2ee" data-e2ee="false" className="text-xs px-2 py-0.5 rounded bg-[var(--bg-active)] text-[var(--text-secondary)]" title="未启用端到端加密">
            <IconShield className="w-3 h-3 inline mr-1 opacity-50" /> PLAIN
          </span>
        )}
        <div className="flex-1" />

        {/* 麦克风 */}
        <button
          data-testid="media-mic-toggle"
          data-mic={micOn ? 'on' : 'off'}
          onClick={handleToggleMic}
          className={`px-3 py-1 rounded text-xs font-medium transition-colors ${
            micOn ? 'bg-[var(--text-primary)] text-[var(--bg-base)]' : 'bg-[var(--bg-active)] hover:bg-[#3f3f46] text-[var(--text-secondary)]'
          }`}
          title={micOn ? '关闭麦克风' : '开启麦克风'}
        >
          {micOn ? <IconMic className='w-4 h-4 mx-auto'/> : <IconMicOff className='w-4 h-4 mx-auto'/>}
        </button>

        {/* 降噪 */}
        {micOn && (
          <button
            data-testid="media-noise-toggle"
            data-noise={noiseOn ? 'on' : 'off'}
            data-mode={media.noiseMode}
            onClick={handleToggleNoise}
            className={`px-3 py-1 rounded text-xs font-medium transition-colors ${
              noiseOn ? 'bg-[var(--text-primary)] text-[var(--bg-base)]' : 'bg-[var(--bg-active)] text-[var(--text-secondary)]'
            }`}
            title={
              media.noiseMode === 'unsupported'
                ? 'RNNoise 需要 Chrome 94+ / Edge / Safari 15.4+'
                : noiseOn ? 'RNNoise ON' : 'RNNoise OFF'
            }
          >
            {media.noiseMode === 'unsupported'
              ? 'RNNoise N/A'
              : noiseOn ? 'RNNoise ON' : 'RNNoise OFF'}
          </button>
        )}

        {/* 画质选择 */}
        <select
          data-testid="media-quality"
          value={presetId}
          onChange={(e) => setPresetId(e.target.value)}
          className="text-xs px-2 py-1 rounded bg-[var(--bg-active)] text-[var(--text-secondary)] border border-[#2b2d31]"
          disabled={sharing}
          title="屏幕共享画质（修改后需重新点击共享）"
        >
          {VIDEO_PRESETS.map(p => (
            <option key={p.id} value={p.id}>
              {p.label} ({(p.bitrate / 1e6).toFixed(1)}M)
            </option>
          ))}
        </select>

        <button
          data-testid="media-share-toggle"
          data-sharing={sharing ? 'on' : 'off'}
          onClick={handleToggleShare}
          className={`px-3 py-1 rounded text-xs font-medium transition-colors ${
            sharing ? 'bg-red-500 hover:bg-red-600 text-white' : 'bg-[var(--text-primary)] text-[var(--bg-base)]'
          }`}
        >
          {sharing ? 'Stop Share' : <><IconMonitor className='w-3 h-3 inline mr-1'/> Share</>}
        </button>
        <button
          onClick={handleLeave}
          className="px-3 py-1 rounded text-xs font-medium bg-[var(--bg-active)] hover:bg-red-500 hover:text-white text-[var(--text-secondary)] transition-colors"
        >
          Leave
        </button>
      </div>
      {error && <div className="mt-1 text-xs text-red-400">{error}</div>}
    </div>
  )
}
