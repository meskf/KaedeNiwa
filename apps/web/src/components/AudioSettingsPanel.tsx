import { useEffect, useState } from 'react'
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
          className={`relative w-9 h-5 rounded-full transition-colors ${media.pushToTalk ? 'bg-[var(--text-main)]' : 'bg-[var(--bg-active)]'}`}
          aria-label={t('audio.pushToTalk')}
        >
          <span className={`absolute top-0.5 w-4 h-4 rounded-full bg-[var(--bg-app)] transition-all ${media.pushToTalk ? 'left-[18px]' : 'left-0.5'}`} />
        </button>
      </div>
    </div>
  )
}
