import { useState } from 'react'
import { useChatStore } from '../stores/chat'
import { useI18n } from '../stores/i18n'
import { getClient } from '../wasm/client'

interface Props {
  defaultCategoryId: string | null
  onClose: () => void
}

export function CreateChannelDialog({ defaultCategoryId, onClose }: Props) {
  const { t } = useI18n()
  const categories = useChatStore(s => s.categories)
  const [kind, setKind] = useState<'text' | 'voice'>('text')
  const [name, setName] = useState('')
  const [categoryId, setCategoryId] = useState<string>(defaultCategoryId ?? '')
  const [isPrivate, setIsPrivate] = useState(false)
  const [password, setPassword] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const submit = async () => {
    const n = name.trim()
    if (!n) return
    if (isPrivate && password.length < 8) {
      setError('password 8+ chars')
      return
    }
    setBusy(true)
    setError(null)
    try {
      const c = getClient()
      if (!c) throw new Error('not ready')
      const raw = await c.createChannel(
        n, kind,
        categoryId || null,
        isPrivate,
        isPrivate ? password : null
      )
      const ch = typeof raw === 'string' ? JSON.parse(raw) : raw
      // refresh channels list from server (broadcast arrives too, but refresh is authoritative)
      try {
        const listRaw = await c.listChannels()
        const arr = typeof listRaw === 'string' ? JSON.parse(listRaw) : listRaw
        if (Array.isArray(arr)) {
          useChatStore.getState().setChannelList(arr.map((x: any) => ({
            id: x.id, name: x.name, kind: x.kind,
            position: Number(x.position), is_private: !!x.is_private,
            category_id: x.category_id ?? null,
          })))
        }
      } catch {}
      // jump to new channel
      if (ch && ch.id) {
        useChatStore.getState().setChannel(ch.id)
        useChatStore.getState().setActiveView('chat')
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
        data-testid="create-channel-dialog"
      >
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-[16px] font-bold text-[var(--text-main)]">{t('channel.createTitle')}</h2>
          <button onClick={onClose} className="text-[var(--text-muted)] hover:text-[var(--text-main)] text-[18px] leading-none" aria-label="close">×</button>
        </div>

        <label className="block text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase mb-1.5">{t('channel.createKind')}</label>
        <div className="flex gap-2 mb-4">
          <button
            onClick={() => setKind('text')}
            className={`flex-1 px-3 py-2 rounded-md text-[13px] font-medium transition-colors ${kind === 'text' ? 'bg-[var(--bg-active)] text-[var(--text-main)]' : 'bg-[var(--bg-hover)] text-[var(--text-muted)] hover:text-[var(--text-main)]'}`}
            data-testid="create-channel-kind-text"
          ># {t('channel.kindText')}</button>
          <button
            onClick={() => setKind('voice')}
            className={`flex-1 px-3 py-2 rounded-md text-[13px] font-medium transition-colors ${kind === 'voice' ? 'bg-[var(--bg-active)] text-[var(--text-main)]' : 'bg-[var(--bg-hover)] text-[var(--text-muted)] hover:text-[var(--text-main)]'}`}
            data-testid="create-channel-kind-voice"
          >🔊 {t('channel.kindVoice')}</button>
        </div>

        <label className="block text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase mb-1.5">{t('channel.createName')}</label>
        <input
          type="text"
          value={name}
          onChange={e => setName(e.target.value)}
          placeholder={t('channel.createPlaceholder')}
          className="w-full bg-[var(--bg-hover)] text-[var(--text-main)] text-[14px] rounded-md px-3 py-2 mb-4 outline-none"
          data-testid="create-channel-name"
          autoFocus
        />

        <label className="block text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase mb-1.5">{t('channel.createCategory')}</label>
        <select
          value={categoryId}
          onChange={e => setCategoryId(e.target.value)}
          className="w-full bg-[var(--bg-hover)] text-[var(--text-main)] text-[13px] rounded-md px-3 py-2 mb-4 outline-none"
          data-testid="create-channel-category"
        >
          <option value="">{t('channel.uncategorized')}</option>
          {categories.map((c: any) => (
            <option key={c.id} value={c.id}>{c.name}</option>
          ))}
        </select>

        <label className="flex items-center gap-2 mb-2 text-[13px] text-[var(--text-main)] cursor-pointer">
          <input
            type="checkbox"
            checked={isPrivate}
            onChange={e => setIsPrivate(e.target.checked)}
            data-testid="create-channel-private"
          />
          {t('channel.isPrivate')}
        </label>
        {isPrivate && (
          <input
            type="password"
            value={password}
            onChange={e => setPassword(e.target.value)}
            placeholder={t('channel.password') + ' (8+)'}
            className="w-full bg-[var(--bg-hover)] text-[var(--text-main)] text-[13px] rounded-md px-3 py-2 mb-4 outline-none"
            data-testid="create-channel-password"
          />
        )}

        {error && <div className="mt-2 mb-2 text-[12px] text-red-500">{error}</div>}

        <div className="flex justify-end gap-2 mt-4">
          <button
            onClick={onClose}
            className="px-4 py-2 text-[13px] text-[var(--text-muted)] hover:text-[var(--text-main)] transition-colors"
          >{t('channel.createCancel')}</button>
          <button
            onClick={submit}
            disabled={busy || !name.trim()}
            className="px-4 py-2 text-[13px] font-semibold rounded-md bg-[var(--text-main)] text-[var(--bg-app)] hover:opacity-90 disabled:opacity-50 transition-opacity"
            data-testid="create-channel-submit"
          >{busy ? '...' : t('channel.createSubmit')}</button>
        </div>
      </div>
    </div>
  )
}
