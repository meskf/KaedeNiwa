import { useState, useEffect, useRef } from 'react'
import { useMediaState } from '../hooks/useLiveKitRoom'
import {
  startScreenShare,
  stopScreenShare,
  enableMicrophone,
  disableMicrophone,
  setNoiseSuppression,
  disconnectMedia,
  setDeafen,
} from '../lib/livekit'
import { AudioSettingsPanel } from './AudioSettingsPanel'
import { VIDEO_PRESETS } from '../lib/videoPresets'
import { getClient } from '../wasm/client'
import { IconMic, IconMicOff, IconMonitor, IconShield, IconSettings } from './icons'
import { IconVolume } from './icons'

interface Props {
  channelId: string
  compact?: boolean
}

export function MediaBar({ channelId, compact = false }: Props) {
  const media = useMediaState()
  const [sharing, setSharing] = useState(false)
  const [micOn, setMicOn] = useState(false)
  const [presetId, setPresetId] = useState('1080p60')
  const [noiseOn, setNoiseOn] = useState(true)
  const [error, setError] = useState('')
  const [showAudioSettings, setShowAudioSettings] = useState(false)
  const [deafened, setDeafened] = useState(false)
  const [elapsedTime, setElapsedTime] = useState(0)
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null)

  useEffect(() => {
    if (media.connected && media.channelId === channelId) {
      setElapsedTime(0)
      timerRef.current = setInterval(() => {
        setElapsedTime(prev => prev + 1)
      }, 1000)
    } else {
      if (timerRef.current) clearInterval(timerRef.current)
      setElapsedTime(0)
    }
    return () => {
      if (timerRef.current) clearInterval(timerRef.current)
    }
  }, [media.connected, media.channelId, channelId])

  const formatTime = (secs: number) => {
    const h = Math.floor(secs / 3600).toString().padStart(2, '0')
    const m = Math.floor((secs % 3600) / 60).toString().padStart(2, '0')
    const s = (secs % 60).toString().padStart(2, '0')
    return h === '00' ? `${m}:${s}` : `${h}:${m}:${s}`
  }


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

  const handleToggleDeafen = () => {
    const next = !deafened
    setDeafened(next)
    setDeafen(next)
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



  if (compact) {
    return (
      <div className="w-full flex flex-col min-w-0 overflow-hidden" data-testid="media-bar-compact">
        <div className="flex flex-col gap-2 p-2 rounded-lg bg-[var(--bg-hover)] border border-[var(--border-subtle)] shadow-sm">
          
          {/* 顶部信息栏：信号、时间、频道 */}
          <div className="flex items-center gap-1.5 min-w-0">
            {/* 信号强度 (假图标，纯装饰) */}
            <div className="flex items-end gap-[2px] h-3 shrink-0" title="连接正常">
              <div className="w-[3px] h-1.5 bg-green-500 rounded-[1px]" />
              <div className="w-[3px] h-2.5 bg-green-500 rounded-[1px]" />
              <div className="w-[3px] h-3.5 bg-green-500 rounded-[1px]" />
            </div>
            
            {/* 计时器 */}
            <span className="text-[12px] font-bold text-green-500 font-mono tracking-tight shrink-0">
              {formatTime(elapsedTime)}
            </span>
            
            {/* 频道名（截断） */}
            <span className="text-[11px] text-[var(--text-muted)] font-medium truncate flex-1" title={channelId}>
              {channelId}
            </span>
          </div>

          {/* 次要信息行：音频设置、画质 */}
          <div className="flex items-center justify-between px-1 text-[10px] text-[var(--text-muted)] opacity-70 hover:opacity-100 transition-opacity mb-1">
            <button
              onClick={() => setShowAudioSettings(v => !v)}
              className="hover:text-[var(--text-main)] flex items-center gap-1"
            >
              <IconSettings className="w-3 h-3" /> 音频设置
            </button>
            <span className="font-mono">{presetId}</span>
          </div>

          {/* 底部操作按钮区 */}
          <div className="flex items-center gap-1.5">
            {/* 屏幕共享大按钮 */}
            <button
              data-testid="media-share-toggle"
              data-sharing={sharing ? 'on' : 'off'}
              onClick={handleToggleShare}
              className={`flex-1 h-8 rounded-md flex items-center justify-center gap-1.5 transition-all text-[11px] font-medium ${
                sharing 
                  ? 'bg-red-500 text-white hover:bg-red-600' 
                  : 'bg-[var(--bg-active)] text-[var(--text-main)] hover:bg-[var(--bg-app)]'
              }`}
              title={sharing ? '停止共享' : '屏幕共享'}
            >
              <IconMonitor className='w-3.5 h-3.5' />
              {sharing ? '停止共享' : '屏幕共享'}
            </button>

            {/* 麦克风 */}
            <button
              data-testid="media-mic-toggle"
              data-mic={micOn ? 'on' : 'off'}
              onClick={handleToggleMic}
              className={`w-8 h-8 rounded-md flex items-center justify-center transition-colors shrink-0 ${
                micOn ? 'bg-[var(--text-main)] text-[var(--bg-app)]' : 'bg-[var(--bg-active)] text-[var(--text-muted)] hover:text-[var(--text-main)]'
              }`}
              title={micOn ? '关闭麦克风' : '开启麦克风'}
            >
              {micOn ? <IconMic className='w-4 h-4'/> : <IconMicOff className='w-4 h-4'/>}
            </button>

            {/* 拒听 */}
            <button
              data-testid="media-deafen-toggle"
              data-deafened={deafened ? 'on' : 'off'}
              onClick={handleToggleDeafen}
              className={`w-8 h-8 rounded-md flex items-center justify-center transition-colors shrink-0 ${
                deafened ? 'bg-red-500 text-white' : 'bg-[var(--bg-active)] text-[var(--text-muted)] hover:text-[var(--text-main)]'
              }`}
              title={deafened ? '取消拒听' : '拒听'}
            >
              <IconVolume className='w-4 h-4' style={{ opacity: deafened ? 0.4 : 1 }} />
            </button>

            {/* 挂断 */}
            <button
              onClick={handleLeave}
              className="w-8 h-8 rounded-md flex items-center justify-center bg-[var(--bg-active)] hover:bg-red-500 hover:text-white text-[var(--text-muted)] transition-colors shrink-0"
              title="断开连接"
            >
              <svg className="w-4 h-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <path d="M18.36 6.64a9 9 0 1 1-12.73 0"></path>
                <line x1="12" y1="2" x2="12" y2="12"></line>
              </svg>
            </button>
          </div>


        </div>
        {error && <div className="text-[10px] text-red-400 px-1 mt-1">{error}</div>}
        {/* 挂载音频设置 Portal */}
        {showAudioSettings && (
          <AudioSettingsPanel onClose={() => setShowAudioSettings(false)} />
        )}
      </div>
    )
  }

  return (
    <div className="bg-[var(--bg-surface)] px-4 py-2 shrink-0">
      <div className="flex items-center gap-2.5 flex-wrap">
        {/* 连接状态 */}
        <div data-testid="media-connected" data-channel={channelId} className="flex items-center gap-1.5 h-8 px-1 shrink-0">
          <span className="w-2 h-2 rounded-full bg-[var(--text-primary)] animate-pulse" />
          <span className="text-[13px] text-[var(--text-primary)] font-medium whitespace-nowrap">已连接 #{channelId}</span>
        </div>

        {/* E2EE 状态 */}
        {media.e2eeEnabled ? (
          <span data-testid="media-e2ee" data-e2ee="true" className="flex items-center h-8 text-[12px] px-2.5 rounded-md bg-[var(--bg-active)] text-[var(--text-primary)] font-medium shrink-0" title="AES-256-GCM 端到端加密">
            <IconShield className="w-3.5 h-3.5 mr-1" /> E2EE
          </span>
        ) : (
          <span data-testid="media-e2ee" data-e2ee="false" className="flex items-center h-8 text-[12px] px-2.5 rounded-md bg-[var(--bg-active)] text-[var(--text-secondary)] shrink-0" title="未启用端到端加密">
            <IconShield className="w-3.5 h-3.5 mr-1 opacity-50" /> PLAIN
          </span>
        )}
        <div className="flex-1" />

        {/* 麦克风 + 音频设置面板 */}
        <div className="relative flex items-center gap-1.5 h-8 shrink-0">
          <button
            data-testid="media-mic-toggle"
            data-mic={micOn ? 'on' : 'off'}
            onClick={handleToggleMic}
            onContextMenu={(e) => { e.preventDefault(); setShowAudioSettings(true) }}
            className={`w-8 h-8 rounded-md flex items-center justify-center transition-colors ${
              micOn ? 'bg-[var(--text-primary)] text-[var(--bg-base)]' : 'bg-[var(--bg-active)] hover:bg-[#3f3f46] text-[var(--text-secondary)]'
            }`}
            title={micOn ? '关闭麦克风（右键音频设置）' : '开启麦克风（右键音频设置）'}
          >
            {micOn ? <IconMic className='w-4 h-4'/> : <IconMicOff className='w-4 h-4'/>}
          </button>
          <button
            data-testid="media-audio-settings-toggle"
            onClick={() => setShowAudioSettings(v => !v)}
            className="w-8 h-8 rounded-md bg-[var(--bg-active)] hover:bg-[#3f3f46] text-[var(--text-secondary)] transition-colors flex items-center justify-center"
            title="音频设置"
          >
            <IconSettings className="w-4 h-4" />
          </button>
          {showAudioSettings && (
            <AudioSettingsPanel onClose={() => setShowAudioSettings(false)} />
          )}
        </div>

        {/* 拒听（Deafen） */}
        <button
          data-testid="media-deafen-toggle"
          data-deafened={deafened ? 'on' : 'off'}
          onClick={handleToggleDeafen}
          className={`w-8 h-8 rounded-md flex items-center justify-center transition-colors shrink-0 ${
            deafened ? 'bg-red-500 text-white' : 'bg-[var(--bg-active)] hover:bg-[#3f3f46] text-[var(--text-secondary)]'
          }`}
          title={deafened ? '取消拒听' : '拒听（静音所有远端）'}
        >
          <IconVolume className='w-4 h-4' style={{ opacity: deafened ? 0.4 : 1 }} />
        </button>

        {/* 降噪 */}
        {micOn && (
          <button
            data-testid="media-noise-toggle"
            data-noise={noiseOn ? 'on' : 'off'}
            data-mode={media.noiseMode}
            onClick={handleToggleNoise}
            className={`h-8 px-3 rounded-md flex items-center justify-center text-[12px] font-medium transition-colors shrink-0 ${
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
          className="h-8 text-[12px] px-2 rounded-md bg-[var(--bg-active)] text-[var(--text-secondary)] border border-[#2b2d31] outline-none shrink-0 cursor-pointer"
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
          className={`h-8 px-4 rounded-md flex items-center justify-center text-[12px] font-medium transition-colors shrink-0 ${
            sharing ? 'bg-red-500 hover:bg-red-600 text-white' : 'bg-[var(--text-primary)] text-[var(--bg-base)]'
          }`}
        >
          {sharing ? 'Stop Share' : <><IconMonitor className='w-3.5 h-3.5 mr-1.5'/> Share</>}
        </button>
        <button
          onClick={handleLeave}
          className="h-8 px-4 rounded-md flex items-center justify-center text-[12px] font-medium bg-[var(--bg-active)] hover:bg-red-500 hover:text-white text-[var(--text-secondary)] transition-colors shrink-0"
        >
          Leave
        </button>
      </div>
      {error && <div className="mt-1 text-xs text-red-400">{error}</div>}
    </div>
  )
}
