import { useCallback, useRef } from 'react'
import { useChatStore } from '../stores/chat'

const MAX_BYTES = 32 * 1024 * 1024

export function AttachmentPicker() {
  const addPending = useChatStore(s => s.addPendingAttachment)
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