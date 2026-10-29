import { useState, useRef, useEffect } from 'react'

interface Props {
  channelName?: string
  onCancel: () => void
  onSubmit: (password: string) => Promise<void>
}

export function PasswordPrompt({ channelName, onCancel, onSubmit }: Props) {
  const [password, setPassword] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const inputRef = useRef<HTMLInputElement>(null)

  useEffect(() => { inputRef.current?.focus() }, [])

  const submit = async () => {
    if (!password || busy) return
    setBusy(true)
    setError(null)
    try {
      await onSubmit(password)
      // 成功：父组件负责关闭弹窗，我们不在这里 onCancel
    } catch (e: any) {
      const msg = String(e)
      // 客户端 http.rs 目前不读 error body，400/401 都表现为
      // "http error (4xx): ..."，用户视角都是"进不去"。统一提示密码错误，
      // 真实原因（rate-limit / 密码错）保留在 console 供调试。
      if (/400|401|unauthor|join channel/i.test(msg)) {
        console.warn('[password-prompt] server rejected:', msg.slice(0, 200))
        setError('密码错误或尝试过于频繁，请稍后重试')
      } else {
        setError(msg.length > 120 ? msg.slice(0, 120) + '...' : msg)
      }
    } finally {
      setBusy(false)
    }
  }

  return (
    <div
      className="fixed inset-0 z-[60] bg-black/50 flex items-center justify-center px-4"
      onClick={onCancel}
    >
      <div
        className="w-[380px] max-w-full rounded-lg bg-[var(--bg-app)] border border-[var(--bg-active)] shadow-2xl p-5"
        onClick={(e) => e.stopPropagation()}
        data-testid="password-prompt"
      >
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-[16px] font-bold text-[var(--text-main)]">进入私密频道</h2>
          <button
            onClick={onCancel}
            className="text-[var(--text-muted)] hover:text-[var(--text-main)] text-[18px] leading-none"
            aria-label="close"
            data-testid="password-prompt-close"
          >×</button>
        </div>
        {channelName && (
          <div className="text-[13px] text-[var(--text-muted)] mb-3">
            # {channelName}
          </div>
        )}
        <input
          ref={inputRef}
          type="password"
          value={password}
          onChange={(e) => { setPassword(e.target.value); setError(null) }}
          onKeyDown={(e) => {
            if (e.key === 'Enter') { e.preventDefault(); submit() }
            else if (e.key === 'Escape') { e.preventDefault(); onCancel() }
          }}
          placeholder="请输入频道密码"
          className="w-full bg-[var(--bg-hover)] text-[var(--text-main)] text-[14px] rounded-md px-3 py-2 mb-3 outline-none"
          data-testid="password-prompt-input"
          disabled={busy}
        />
        {error && (
          <div className="mb-3 text-[12px] text-red-500" data-testid="password-prompt-error">{error}</div>
        )}
        <div className="flex justify-end gap-2">
          <button
            onClick={onCancel}
            className="px-4 py-2 text-[13px] text-[var(--text-muted)] hover:text-[var(--text-main)] transition-colors"
            data-testid="password-prompt-cancel"
            disabled={busy}
          >取消</button>
          <button
            onClick={submit}
            disabled={busy || !password}
            className="px-4 py-2 text-[13px] font-semibold rounded-md bg-[var(--text-main)] text-[var(--bg-app)] hover:opacity-90 disabled:opacity-50 transition-opacity"
            data-testid="password-prompt-submit"
          >{busy ? '...' : '进入'}</button>
        </div>
      </div>
    </div>
  )
}
