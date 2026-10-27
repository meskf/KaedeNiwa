const fs = require('fs')

// ============ AudioSettingsPanel.tsx (new) ============
const panel = `import { useEffect, useState } from 'react'
import { useI18n } from '../stores/i18n'
import { useMediaState } from '../hooks/useLiveKitRoom'
import {
  listAudioDevices,
  setMicDevice,
  setSpeakerDevice,
  setInputVolume,
  setOutputVolume,
  setPushToTalk,
} from '../lib/livekit'

interface Props {
  onClose: () => void
}

export function AudioSettingsPanel({ onClose }: Props) {
  const { t } = useI18n()
  const media = useMediaState()
  const [inputs, setInputs] = useState<MediaDeviceInfo[]>([])
  const [outputs, setOutputs] = useState<MediaDeviceInfo[]>([])
  const [speakerSupported, setSpeakerSupported] = useState(true)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    ;(async () => {
      const { inputs, outputs } = await listAudioDevices()
      setInputs(inputs)
      setOutputs(outputs)
      setSpeakerSupported(typeof (HTMLMediaElement.prototype as any).setSinkId === 'function')
    })()
  }, [])

  const handleMicChange = async (id: string) => {
    setBusy(true)
    try { await setMicDevice(id) } finally { setBusy(false) }
  }

  const handleSpeakerChange = (id: string) => {
    setSpeakerDevice(id)
  }

  const handlePttToggle = async () => {
    await setPushToTalk(!media.pushToTalk, media.pushToTalkKey)
  }

  const inputVol = media.inputVolume
  const outputVol = media.outputVolume

  return (
    <div
      className="absolute right-0 bottom-full mb-2 w-[320px] rounded-lg bg-[var(--bg-app)] border border-[var(--bg-active)] shadow-xl z-30 py-2 text-[var(--text-main)]"
      data-testid="audio-settings-panel"
      onClick={(e) => e.stopPropagation()}
    >
      <div className="px-4 pb-2 flex items-center justify-between">
        <span className="text-[12px] font-bold tracking-widest text-[var(--text-muted)] uppercase">{t('audio.title')}</span>
        <button onClick={onClose} className="text-[var(--text-muted)] hover:text-[var(--text-main)] text-[16px] leading-none" aria-label="close">×</button>
      </div>

      {/* 输入设备 */}
      <div className="px-4 py-2">
        <label className="block text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase mb-1">{t('audio.inputDevice')}</label>
        <select
          data-testid="audio-input-select"
          value={media.micDeviceId || ''}
          onChange={(e) => handleMicChange(e.target.value)}
          disabled={busy}
          className="w-full bg-[var(--bg-hover)] text-[var(--text-main)] text-[13px] rounded-md px-2 py-1.5 outline-none disabled:opacity-50"
        >
          <option value="">{t('audio.defaultDevice')}</option>
          {inputs.map(d => (
            <option key={d.deviceId} value={d.deviceId}>{d.label || d.deviceId.slice(0, 12)}</option>
          ))}
        </select>
      </div>

      {/* 输入音量 */}
      <div className="px-4 py-2">
        <div className="flex items-center justify-between mb-1">
          <label className="text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase">{t('audio.inputVolume')}</label>
          <span className="text-[11px] font-mono text-[var(--text-muted)]">{inputVol}%</span>
        </div>
        <input
          type="range" min={0} max={100} step={1} value={inputVol}
          data-testid="audio-input-volume"
          onChange={(e) => setInputVolume(Number(e.target.value))}
          className="w-full accent-[var(--text-main)]"
        />
      </div>

      {/* 输出设备 */}
      <div className="px-4 py-2">
        <label className="block text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase mb-1">{t('audio.outputDevice')}</label>
        {speakerSupported ? (
          <select
            data-testid="audio-output-select"
            value={media.speakerDeviceId || ''}
            onChange={(e) => handleSpeakerChange(e.target.value)}
            className="w-full bg-[var(--bg-hover)] text-[var(--text-main)] text-[13px] rounded-md px-2 py-1.5 outline-none"
          >
            <option value="">{t('audio.defaultDevice')}</option>
            {outputs.map(d => (
              <option key={d.deviceId} value={d.deviceId}>{d.label || d.deviceId.slice(0, 12)}</option>
            ))}
          </select>
        ) : (
          <div className="text-[12px] text-[var(--text-muted)] italic">{t('audio.unsupportedSpeaker')}</div>
        )}
      </div>

      {/* 输出音量 */}
      <div className="px-4 py-2">
        <div className="flex items-center justify-between mb-1">
          <label className="text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase">{t('audio.outputVolume')}</label>
          <span className="text-[11px] font-mono text-[var(--text-muted)]">{outputVol}%</span>
        </div>
        <input
          type="range" min={0} max={100} step={1} value={outputVol}
          data-testid="audio-output-volume"
          onChange={(e) => setOutputVolume(Number(e.target.value))}
          className="w-full accent-[var(--text-main)]"
        />
      </div>

      {/* Push to talk */}
      <div className="px-4 py-2 border-t border-[var(--bg-active)] mt-1 flex items-center justify-between">
        <div className="flex flex-col">
          <span className="text-[12px] text-[var(--text-main)]">{t('audio.pushToTalk')}</span>
          <span className="text-[11px] text-[var(--text-muted)] font-mono">{media.pushToTalkKey}</span>
        </div>
        <button
          data-testid="audio-ptt-toggle"
          onClick={handlePttToggle}
          className={\`relative w-9 h-5 rounded-full transition-colors \${media.pushToTalk ? 'bg-[var(--text-main)]' : 'bg-[var(--bg-active)]'}\`}
          aria-label={t('audio.pushToTalk')}
        >
          <span className={\`absolute top-0.5 w-4 h-4 rounded-full bg-[var(--bg-app)] transition-all \${media.pushToTalk ? 'left-[18px]' : 'left-0.5'}\`} />
        </button>
      </div>
    </div>
  )
}
`
fs.writeFileSync('src/components/AudioSettingsPanel.tsx', panel)
console.log('WROTE AudioSettingsPanel.tsx')

// ============ MediaBar.tsx: add deafen + settings panel ============
let m = fs.readFileSync('src/components/MediaBar.tsx', 'utf8')

// imports
m = m.replace(
  `import {
  startScreenShare,
  stopScreenShare,
  enableMicrophone,
  disableMicrophone,
  setNoiseSuppression,
  disconnectMedia,
} from '../lib/livekit'`,
  `import {
  startScreenShare,
  stopScreenShare,
  enableMicrophone,
  disableMicrophone,
  setNoiseSuppression,
  disconnectMedia,
  setDeafen,
} from '../lib/livekit'
import { AudioSettingsPanel } from './AudioSettingsPanel'`
)

m = m.replace(
  `import { IconMic, IconMicOff, IconMonitor, IconShield } from './icons'`,
  `import { IconMic, IconMicOff, IconMonitor, IconShield } from './icons'
import { IconVolume } from './icons'`
)

// state
m = m.replace(
  `  const [error, setError] = useState('')`,
  `  const [error, setError] = useState('')
  const [showAudioSettings, setShowAudioSettings] = useState(false)
  const [deafened, setDeafened] = useState(false)`
)

// handleDeafen
m = m.replace(
  `  const handleLeave = async () => {`,
  `  const handleToggleDeafen = () => {
    const next = !deafened
    setDeafened(next)
    setDeafen(next)
  }

  const handleLeave = async () => {`
)

// add deafen button + settings panel wrapper on mic button
// mic button: wrap in relative container + onClick open settings if right-click? 简化：加个齿轮小图标在 mic 按钮旁边触发 panel
// 我们用一种简化路径：mic toggle 保持 toggle 行为，deafen 按钮加在它左边，另一个“gear”按钮点击弹面板
m = m.replace(
  `        {/* 麦克风 */}
        <button
          data-testid="media-mic-toggle"
          data-mic={micOn ? 'on' : 'off'}
          onClick={handleToggleMic}
          className={\`px-3 py-1 rounded text-xs font-medium transition-colors \${
            micOn ? 'bg-[var(--text-primary)] text-[var(--bg-base)]' : 'bg-[var(--bg-active)] hover:bg-[#3f3f46] text-[var(--text-secondary)]'
          }\`}
          title={micOn ? '关闭麦克风' : '开启麦克风'}
        >
          {micOn ? <IconMic className='w-4 h-4 mx-auto'/> : <IconMicOff className='w-4 h-4 mx-auto'/>}
        </button>`,
  `        {/* 麦克风 + 音频设置面板 */}
        <div className="relative">
          <button
            data-testid="media-mic-toggle"
            data-mic={micOn ? 'on' : 'off'}
            onClick={handleToggleMic}
            onContextMenu={(e) => { e.preventDefault(); setShowAudioSettings(true) }}
            className={\`px-3 py-1 rounded text-xs font-medium transition-colors \${
              micOn ? 'bg-[var(--text-primary)] text-[var(--bg-base)]' : 'bg-[var(--bg-active)] hover:bg-[#3f3f46] text-[var(--text-secondary)]'
            }\`}
            title={micOn ? '关闭麦克风（右键音频设置）' : '开启麦克风（右键音频设置）'}
          >
            {micOn ? <IconMic className='w-4 h-4 mx-auto'/> : <IconMicOff className='w-4 h-4 mx-auto'/>}
          </button>
          <button
            data-testid="media-audio-settings-toggle"
            onClick={() => setShowAudioSettings(v => !v)}
            className="ml-1 px-1.5 py-1 rounded text-xs bg-[var(--bg-active)] hover:bg-[#3f3f46] text-[var(--text-secondary)] transition-colors"
            title="音频设置"
          >
            ⚙
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
          className={\`px-3 py-1 rounded text-xs font-medium transition-colors \${
            deafened ? 'bg-red-500 text-white' : 'bg-[var(--bg-active)] hover:bg-[#3f3f46] text-[var(--text-secondary)]'
          }\`}
          title={deafened ? '取消拒听' : '拒听（静音所有远端）'}
        >
          <IconVolume className='w-4 h-4 mx-auto' style={{ opacity: deafened ? 0.4 : 1 }} />
        </button>`
)

fs.writeFileSync('src/components/MediaBar.tsx', m)
console.log('PATCHED MediaBar.tsx')

console.log('DONE Round 2')
