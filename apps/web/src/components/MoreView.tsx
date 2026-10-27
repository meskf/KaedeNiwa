import { useAuthStore } from '../stores/auth'
import { useChatStore } from '../stores/chat'
import { useI18n } from '../stores/i18n'
import { useMediaState } from '../hooks/useLiveKitRoom'

const VERSION = 'v0.0.5'

export function MoreView() {
  const { t } = useI18n()
  const serverUrl = useAuthStore(s => s.serverUrl)
  const channelList = useChatStore(s => s.channelList)
  const media = useMediaState()

  const channelName = media.channelId
    ? (channelList.find(c => c.id === media.channelId)?.name || media.channelId)
    : null

  const noiseLabel =
    media.noiseMode === 'rnnoise' ? t('more.noiseRnnoise') :
    media.noiseMode === 'unsupported' ? t('more.noiseUnsupported') :
    t('more.noiseOff')

  const Row = ({ label, value, mono = false }: { label: string; value: string; mono?: boolean }) => (
    <div className="flex items-center justify-between px-4 py-3">
      <span className="text-[13px] text-[var(--text-muted)]">{label}</span>
      <span className={`text-[13px] text-[var(--text-main)] truncate max-w-[320px] ${mono ? 'font-mono' : 'font-semibold'}`}>{value}</span>
    </div>
  )

  return (
    <div className="flex flex-col h-full w-full max-w-[700px] mx-auto overflow-y-auto">
      <div className="mb-8 mt-1">
        <h1 className="text-3xl font-extrabold tracking-tight text-[var(--text-main)]">{t('more.title')}</h1>
        <p className="text-[13px] text-[var(--text-muted)] mt-1">{t('more.subtitle')}</p>
      </div>

      <section className="mb-8">
        <h2 className="text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase mb-3">{t('more.aboutTitle')}</h2>
        <div className="rounded-lg bg-[var(--bg-hover)] divide-y divide-[var(--bg-active)]" data-testid="more-about">
          <Row label={t('more.aboutVersion')} value={VERSION} mono />
          <Row label={t('more.aboutServer')} value={serverUrl || t('more.valueNa')} mono />
          <Row label={t('more.aboutEncryption')} value="MLS (RFC 9420) + AES-256-GCM" mono />
        </div>
      </section>

      <section className="mb-8">
        <h2 className="text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase mb-3">{t('more.voiceTitle')}</h2>
        <div className="rounded-lg bg-[var(--bg-hover)] divide-y divide-[var(--bg-active)]" data-testid="more-voice">
          <Row label={t('more.voiceStatus')} value={media.connected ? t('more.statusConnected') : t('more.statusNotConnected')} />
          <Row label={t('more.voiceChannel')} value={channelName || t('more.valueNa')} />
          <Row label={t('more.voiceParticipants')} value={media.connected ? String(media.participants.length) : t('more.valueNa')} mono />
          <Row label={t('more.voiceMic')} value={media.localMicEnabled ? t('more.micOn') : t('more.micOff')} />
          <Row label={t('more.voiceNoise')} value={noiseLabel} />
          <Row label={t('more.voiceScreen')} value={media.localScreenSharing ? t('more.screenOn') : t('more.screenOff')} />
          <Row label={t('more.voiceE2EE')} value={media.e2eeEnabled ? t('more.valueE2EEOn') : t('more.valueE2EEOff')} />
        </div>
      </section>

      <section className="mb-8">
        <h2 className="text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase mb-3">{t('more.shortcutsTitle')}</h2>
        <div className="rounded-lg bg-[var(--bg-hover)] divide-y divide-[var(--bg-active)]" data-testid="more-shortcuts">
          <div className="flex items-center justify-between px-4 py-3">
            <span className="text-[13px] text-[var(--text-muted)]">{t('more.scSend')}</span>
            <kbd className="px-2 py-0.5 text-[11px] font-mono rounded bg-[var(--bg-app)] border border-[var(--bg-active)] text-[var(--text-main)]">Enter</kbd>
          </div>
          <div className="flex items-center justify-between px-4 py-3">
            <span className="text-[13px] text-[var(--text-muted)]">{t('more.scNewline')}</span>
            <kbd className="px-2 py-0.5 text-[11px] font-mono rounded bg-[var(--bg-app)] border border-[var(--bg-active)] text-[var(--text-main)]">Shift + Enter</kbd>
          </div>
          <div className="flex items-center justify-between px-4 py-3">
            <span className="text-[13px] text-[var(--text-muted)]">{t('more.scClose')}</span>
            <kbd className="px-2 py-0.5 text-[11px] font-mono rounded bg-[var(--bg-app)] border border-[var(--bg-active)] text-[var(--text-main)]">Esc</kbd>
          </div>
        </div>
      </section>
    </div>
  )
}
