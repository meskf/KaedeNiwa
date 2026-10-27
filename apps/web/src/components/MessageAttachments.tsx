import { useEffect, useState } from 'react'
import type { Attachment } from '../protocol/Attachment'
import { getClient } from '../wasm/client'
import { useChatStore } from '../stores/chat'

interface Props {
  targetId: string
  attachments: Attachment[]
}

function formatSize(n: bigint | number): string {
  const num = typeof n === 'bigint' ? Number(n) : n
  if (num < 1024) return num + ' B'
  if (num < 1024 * 1024) return (num / 1024).toFixed(1) + ' KB'
  if (num < 1024 * 1024 * 1024) return (num / 1024 / 1024).toFixed(1) + ' MB'
  return (num / 1024 / 1024 / 1024).toFixed(2) + ' GB'
}

function isImage(mime: string) { return mime.startsWith('image/') }
function isVideo(mime: string) { return mime.startsWith('video/') }
function isAudio(mime: string) { return mime.startsWith('audio/') }

function SingleAttachment({ targetId, att }: { targetId: string; att: Attachment }) {
  const url = useChatStore(s => s.attachmentCache[att.id])
  const setUrl = useChatStore(s => s.setAttachmentUrl)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)

  useEffect(() => {
    if (url || loading) return
    if (!isImage(att.mime) && !isVideo(att.mime) && !isAudio(att.mime)) return
    const c = getClient()
    if (!c) return
    setLoading(true)
    let cancelled = false
    ;(async () => {
      try {
        const bytes = await c.downloadAttachment(targetId, att.id, att.sha256 ?? null)
        if (cancelled) return
        const slice = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer
        const blob = new Blob([slice], { type: att.mime })
        const objUrl = URL.createObjectURL(blob)
        setUrl(att.id, objUrl)
      } catch (e: any) {
        if (!cancelled) setError(String(e))
      } finally {
        if (!cancelled) setLoading(false)
      }
    })()
    return () => { cancelled = true }
  }, [targetId, att.id, att.mime, att.sha256, url, loading, setUrl])

  const handleDownload = async () => {
    const c = getClient()
    if (!c) return
    try {
      const bytes = await c.downloadAttachment(targetId, att.id, att.sha256 ?? null)
      const slice = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer
      const blob = new Blob([slice], { type: att.mime })
      const a = document.createElement('a')
      a.href = URL.createObjectURL(blob)
      a.download = att.name
      a.click()
      setTimeout(() => URL.revokeObjectURL(a.href), 1000)
    } catch (e: any) {
      setError(String(e))
    }
  }

  if (isImage(att.mime)) {
    return (
      <div className="mt-1">
        {url ? (
          <img
            src={url}
            alt={att.name}
            className="max-w-[400px] max-h-[300px] rounded-lg cursor-pointer border border-[var(--bg-hover)]"
            onClick={() => window.open(url, '_blank')}
          />
        ) : (
          <div className="w-[200px] h-[100px] flex items-center justify-center bg-[var(--bg-active)] rounded-lg text-xs text-[var(--text-muted)]">
            {loading ? 'Loading...' : error ? 'Failed' : 'Image'}
          </div>
        )}
        {error && <div className="text-[10px] text-red-400 mt-1">{error}</div>}
      </div>
    )
  }
  if (isVideo(att.mime)) {
    return (
      <div className="mt-1">
        {url ? (
          <video src={url} controls className="max-w-[400px] rounded-lg" />
        ) : (
          <div className="text-xs text-[var(--text-muted)]">{loading ? 'Loading video...' : 'Video'}</div>
        )}
      </div>
    )
  }
  if (isAudio(att.mime)) {
    return (
      <div className="mt-1">
        {url ? (
          <audio src={url} controls />
        ) : (
          <div className="text-xs text-[var(--text-muted)]">{loading ? 'Loading audio...' : 'Audio'}</div>
        )}
      </div>
    )
  }
  return (
    <button
      onClick={handleDownload}
      className="mt-1 flex items-center gap-2 px-3 py-2 rounded-lg bg-[var(--bg-active)] hover:bg-[var(--bg-hover)] text-left transition-colors"
    >
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M7 10l5 5 5-5M12 15V3"/>
      </svg>
      <div>
        <div className="text-xs font-medium text-[var(--text-main)] truncate max-w-[200px]">{att.name}</div>
        <div className="text-[10px] text-[var(--text-muted)]">{formatSize(att.size)}</div>
      </div>
    </button>
  )
}

export function MessageAttachments({ targetId, attachments }: Props) {
  if (!attachments || attachments.length === 0) return null
  return (
    <div className="flex flex-col gap-1">
      {attachments.map(att => (
        <SingleAttachment key={att.id} targetId={targetId} att={att} />
      ))}
    </div>
  )
}