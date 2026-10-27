import { useEffect, useRef } from 'react'
import { useI18n } from '../stores/i18n'

interface Props {
  x: number
  y: number
  uid: string
  isFriend: boolean
  inVoice: boolean
  onSendMessage?: () => void
  onAddFriend?: () => void
  onRemoveFriend?: () => void
  onClose: () => void
}

export function UserContextMenu({ x, y, uid, isFriend, inVoice, onSendMessage, onAddFriend, onRemoveFriend, onClose }: Props) {
  const { t } = useI18n()
  void x; void y
  const ref = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const onDoc = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose()
    }
    const onEsc = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose() }
    document.addEventListener('mousedown', onDoc)
    document.addEventListener('keydown', onEsc)
    return () => {
      document.removeEventListener('mousedown', onDoc)
      document.removeEventListener('keydown', onEsc)
    }
  }, [onClose])

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 px-4" onClick={onClose}>
      <div
        ref={ref}
        data-testid="user-context-menu"
        className="w-[260px] max-w-full rounded-lg bg-[var(--bg-app)] border border-[var(--bg-active)] shadow-xl py-1 text-[var(--text-main)]"
        onClick={(e) => e.stopPropagation()}
      >
      <div className="px-3 py-1.5 text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase border-b border-[var(--bg-active)] mb-1">
        {uid}
      </div>
      {isFriend && onSendMessage && (
        <button
          data-testid="ctx-send-message"
          onClick={() => { onSendMessage(); onClose() }}
          className="w-full text-left px-3 py-1.5 text-[13px] hover:bg-[var(--bg-hover)] transition-colors"
        >
          {t('menu.sendMessage')}
        </button>
      )}
      {!isFriend && onAddFriend && (
        <button
          data-testid="ctx-add-friend"
          onClick={() => { onAddFriend(); onClose() }}
          className="w-full text-left px-3 py-1.5 text-[13px] text-green-500 hover:bg-[var(--bg-hover)] transition-colors"
        >
          {t('menu.addFriend')}
        </button>
      )}
      {isFriend && onRemoveFriend && (
        <button
          data-testid="ctx-remove-friend"
          onClick={() => { onRemoveFriend(); onClose() }}
          className="w-full text-left px-3 py-1.5 text-[13px] text-red-500 hover:bg-[var(--bg-hover)] transition-colors"
        >
          {t('friends.actionRemove')}
        </button>
      )}
      {inVoice && (
        <div className="border-t border-[var(--bg-active)] mt-1 pt-1 px-3 py-1 text-[11px] text-[var(--text-muted)] italic">
          {t('menu.userVolume')} — TODO
        </div>
      )}
      </div>
    </div>
  )
}
