import { useState } from 'react'
import { useChatStore } from '../stores/chat'
import { useI18n } from '../stores/i18n'
import { getClient } from '../wasm/client'

interface Props {
  channelId: string
  onClose: () => void
}

export function ChannelSettingsDialog({ channelId, onClose }: Props) {
  const { t } = useI18n()
  const channelList = useChatStore(s => s.channelList)
  const categories = useChatStore(s => s.categories)
  const initial = channelList.find(c => c.id === channelId)
  const [name, setName] = useState(initial?.name || '')
  const [categoryId, setCategoryId] = useState<string>(initial?.category_id ?? '')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const refreshChannels = async () => {
    const c = getClient()
    if (!c) return
    const raw = await c.listChannels()
    const arr = typeof raw === 'string' ? JSON.parse(raw) : raw
    if (Array.isArray(arr)) {
      useChatStore.getState().setChannelList(arr.map((x: any) => ({
        id: x.id, name: x.name, kind: x.kind,
        position: Number(x.position), is_private: !!x.is_private,
        category_id: x.category_id ?? null,
      })))
    }
  }

  const save = async () => {
    const n = name.trim()
    if (!n) return
    setBusy(true)
    setError(null)
    try {
      const c = getClient()
      if (!c) throw new Error('not ready')
      await c.updateChannel(channelId, n, categoryId || null)
      await refreshChannels()
      onClose()
    } catch (e: any) {
      setError(String(e))
    } finally {
      setBusy(false)
    }
  }

  const remove = async () => {
    if (!confirm(t('channel.deleteConfirm'))) return
    setBusy(true)
    setError(null)
    try {
      const c = getClient()
      if (!c) throw new Error('not ready')
      await c.deleteChannel(channelId)
      await refreshChannels()
      // if current channel is the deleted one, jump to general
      if (useChatStore.getState().currentChannel === channelId) {
        useChatStore.getState().setChannel('general')
      }
      onClose()
    } catch (e: any) {
      setError(String(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="fixed inset-0 z-50 bg-black/50 flex items-center justify-center px-4" onClick={onClose}>
      <div
        className="w-[420px] max-w-full rounded-lg bg-[var(--bg-app)] border border-[var(--bg-active)] shadow-2xl p-5"
        onClick={e => e.stopPropagation()}
        data-testid="channel-settings-dialog"
      >
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-[16px] font-bold text-[var(--text-main)]">{t('channel.settingsTitle')}</h2>
          <button onClick={onClose} className="text-[var(--text-muted)] hover:text-[var(--text-main)] text-[18px] leading-none" aria-label="close">×</button>
        </div>

        <label className="block text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase mb-1.5">{t('channel.settingsName')}</label>
        <input
          type="text"
          value={name}
          onChange={e => setName(e.target.value)}
          className="w-full bg-[var(--bg-hover)] text-[var(--text-main)] text-[14px] rounded-md px-3 py-2 mb-4 outline-none"
          data-testid="settings-channel-name"
          autoFocus
        />

        <label className="block text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase mb-1.5">{t('channel.settingsCategory')}</label>
        <select
          value={categoryId}
          onChange={e => setCategoryId(e.target.value)}
          className="w-full bg-[var(--bg-hover)] text-[var(--text-main)] text-[13px] rounded-md px-3 py-2 mb-4 outline-none"
          data-testid="settings-channel-category"
        >
          <option value="">{t('channel.uncategorized')}</option>
          {categories.map((c: any) => (
            <option key={c.id} value={c.id}>{c.name}</option>
          ))}
        </select>

        {error && <div className="mt-2 mb-2 text-[12px] text-red-500">{error}</div>}

        <div className="flex items-center justify-between mt-4">
          <button
            onClick={remove}
            disabled={busy}
            className="px-3 py-2 text-[13px] text-red-500 hover:bg-red-500/10 rounded-md transition-colors disabled:opacity-50"
            data-testid="settings-channel-delete"
          >{t('channel.settingsDelete')}</button>
          <div className="flex gap-2">
            <button
              onClick={onClose}
              className="px-4 py-2 text-[13px] text-[var(--text-muted)] hover:text-[var(--text-main)] transition-colors"
            >{t('channel.settingsCancel')}</button>
            <button
              onClick={save}
              disabled={busy || !name.trim()}
              className="px-4 py-2 text-[13px] font-semibold rounded-md bg-[var(--text-main)] text-[var(--bg-app)] hover:opacity-90 disabled:opacity-50 transition-opacity"
              data-testid="settings-channel-save"
            >{busy ? '...' : t('channel.settingsSave')}</button>
          </div>
        </div>
      </div>
    </div>
  )
}
