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

  const handleToggleNoise = () => {
    const next = !noiseOn
    setNoiseOn(next)
    setNoiseSuppression(next)
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
    <div className="bg-[#2b2d31] border-b border-[#1f2023] px-4 py-2 shrink-0">
      <div className="flex items-center gap-3 flex-wrap">
        <div data-testid="media-connected" data-channel={channelId} className="flex items-center gap-1">
          <span className="w-2 h-2 rounded-full bg-green-500 animate-pulse" />
          <span className="text-xs text-green-400 font-medium">已连接 #{channelId}</span>
        </div>
        {media.e2eeEnabled ? (
          <span data-testid="media-e2ee" data-e2ee="true" className="text-xs px-2 py-0.5 rounded bg-green-500/20 text-green-300 font-medium" title="AES-256-GCM 端到端加密">
            🔒 E2EE
          </span>
        ) : (
          <span data-testid="media-e2ee" data-e2ee="false" className="text-xs px-2 py-0.5 rounded bg-yellow-500/20 text-yellow-300" title="未启用端到端加密">
            ⚠️ 明文
          </span>
        )}
        <div className="flex items-center gap-2">
          {media.participants.map((p) => (
            <div
              key={p.identity}
              data-participant={p.identity}
              data-mic={p.isMicrophoneEnabled ? 'on' : 'off'}
              className={`flex items-center gap-1 px-2 py-0.5 rounded text-xs ${
                p.isSpeaking ? 'bg-green-500/20 text-green-300' : 'bg-[#383a40] text-gray-300'
              }`}
              title={p.identity + (p.isLocal ? ' (你)' : '')}
            >
              <span>{p.identity[0].toUpperCase()}</span>
              <span className="truncate max-w-[80px]">{p.identity}</span>
              {p.isMicrophoneEnabled && <span>🎤</span>}
            </div>
          ))}
        </div>
        <div className="flex-1" />

        {/* 麦克风 */}
        <button
          data-testid="media-mic-toggle"
          data-mic={micOn ? 'on' : 'off'}
          onClick={handleToggleMic}
          className={`px-3 py-1 rounded text-xs font-medium transition-colors ${
            micOn ? 'bg-green-500 hover:bg-green-600 text-white' : 'bg-[#404249] hover:bg-[#4e5058] text-gray-200'
          }`}
          title={micOn ? '关闭麦克风' : '开启麦克风'}
        >
          {micOn ? '🎤 开' : '🎤 关'}
        </button>

        {/* 降噪 */}
        {micOn && (
          <button
            data-testid="media-noise-toggle"
            data-noise={noiseOn ? 'on' : 'off'}
            onClick={handleToggleNoise}
            className={`px-3 py-1 rounded text-xs font-medium transition-colors ${
              noiseOn ? 'bg-indigo-500 hover:bg-indigo-600 text-white' : 'bg-[#404249] text-gray-200'
            }`}
            title="RNNoise 降噪"
          >
            {noiseOn ? '🔇 降噪' : '降噪关'}
          </button>
        )}

        {/* 画质选择 */}
        <select
          data-testid="media-quality"
          value={presetId}
          onChange={(e) => setPresetId(e.target.value)}
          className="text-xs px-2 py-1 rounded bg-[#404249] text-gray-200 border border-[#2b2d31]"
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
            sharing ? 'bg-red-500 hover:bg-red-600 text-white' : 'bg-indigo-500 hover:bg-indigo-600 text-white'
          }`}
        >
          {sharing ? '停止共享' : '共享屏幕'}
        </button>
        <button
          onClick={handleLeave}
          className="px-3 py-1 rounded text-xs font-medium bg-[#404249] hover:bg-red-600 text-gray-200 transition-colors"
        >
          离开
        </button>
      </div>
      {error && <div className="mt-1 text-xs text-red-400">{error}</div>}
    </div>
  )
}
