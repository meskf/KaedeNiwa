import { useCallback, useRef } from 'react'
import { useChatStore } from '../stores/chat'

const MAX_BYTES = 32 * 1024 * 1024

export function AttachmentPicker() {
  const pending = useChatStore(s => s.pendingAttachments)
  const addPending = useChatStore(s => s.addPendingAttachment)
  const removePending = useChatStore(s => s.removePendingAttachment)
  const fileInputRef = useRef<HTMLInputElement>(null)

  const handleFiles = useCallback(async (files: FileList | File[]) => {
    for (const file of Array.from(files)) {
      if (file.size > MAX_BYTES) {
        alert(`File too large: ${file.name} (${(file.size / 1024 / 1024).toFixed(1)} MB > 32 MB)`)
        continue
      }
      const bytes = new Uint8Array(await file.arrayBuffer())
      const id = 'local_' + Math.random().toString(36).slice(2) + Date.now().toString(36)
      addPending({
        id,
        name: file.name,
        mime: file.type || 'application/octet-stream',
        size: file.size,
        bytes,
        status: 'pending',
        sent: 0,
        total: file.size,
      })
    }
  }, [addPending])

  const onDrop = useCallback((e: React.DragEvent) => {
    e.preventDefault()
    if (e.dataTransfer.files.length > 0) handleFiles(e.dataTransfer.files)
  }, [handleFiles])

  const onDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault()
  }, [])

  const openPicker = () => fileInputRef.current?.click()
  const onChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    if (e.target.files) handleFiles(e.target.files)
    e.target.value = ''
  }

  return (
    <div onDrop={onDrop} onDragOver={onDragOver} className="contents">
      <input ref={fileInputRef} type="file" multiple className="hidden" onChange={onChange} />
      {pending.length > 0 && (
        <div className="w-full mb-1 flex flex-wrap gap-1.5">
          {pending.map(p => (
            <div key={p.id} className="relative flex items-center gap-1.5 px-2 py-1 rounded-md bg-[var(--bg-active)] text-[11px] text-[var(--text-main)]">
              <span className="truncate max-w-[140px]">{p.name}</span>
              <span className="text-[var(--text-muted)] shrink-0">{(p.size / 1024).toFixed(0)}KB</span>
              {p.status === 'uploading' && p.total > 0 && (
                <span className="text-[10px] text-[var(--text-muted)]">{Math.round((p.sent / p.total) * 100)}%</span>
              )}
              <button
                type="button"
                onClick={() => removePending(p.id)}
                className="text-[var(--text-muted)] hover:text-red-400 shrink-0"
                title="Remove"
              >x</button>
            </div>
          ))}
        </div>
      )}
      <button
        type="button"
        data-testid="attachment-picker"
        onClick={openPicker}
        className="w-6 h-6 flex items-center justify-center text-[var(--text-muted)] hover:text-[var(--text-main)] transition-colors shrink-0"
        title="Attach file (max 32 MB)"
      >
        <svg className="block" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
          <path d="M21.44 11.05l-9.19 9.19a6 6 0 0 1-8.49-8.49l9.19-9.19a4 4 0 0 1 5.66 5.66l-9.2 9.19a2 2 0 0 1-2.83-2.83l8.49-8.48"/>
        </svg>
      </button>
    </div>
  )
}