interface Props {
  reactions: Record<string, string[]>
  currentUid: string
  onToggle: (emoji: string, action: 'add' | 'remove') => void
}

export function ReactionBar({ reactions, currentUid, onToggle }: Props) {
  const entries = Object.entries(reactions || {}).filter(([, uids]) => Array.isArray(uids) && uids.length > 0)
  if (entries.length === 0) return null
  return (
    <div data-testid="reaction-bar" className="flex flex-wrap gap-1 mt-1.5">
      {entries.map(([emoji, uids]) => {
        const mine = uids.includes(currentUid)
        return (
          <button
            key={emoji}
            type="button"
            data-testid={`reaction-pill-${emoji}`}
            data-emoji={emoji}
            data-mine={mine ? '1' : '0'}
            data-count={uids.length}
            onClick={(e) => {
              e.stopPropagation()
              onToggle(emoji, mine ? 'remove' : 'add')
            }}
            onContextMenu={(e) => { e.preventDefault(); e.stopPropagation() }}
            title={uids.join(', ')}
            className={
              'inline-flex items-center gap-1 px-1.5 py-0.5 rounded-md text-[12px] border transition-colors ' +
              (mine
                ? 'bg-[var(--bg-active)] border-[var(--text-muted)] text-[var(--text-main)]'
                : 'bg-[var(--bg-hover)] border-transparent text-[var(--text-muted)] hover:bg-[var(--bg-active)]')
            }
          >
            <span className="leading-none">{emoji}</span>
            <span className="font-semibold leading-none">{uids.length}</span>
          </button>
        )
      })}
    </div>
  )
}
