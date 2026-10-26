import { useState } from 'react'

export function LoginView({ onLogin }: { onLogin: () => void }) {
  const [isRegister, setIsRegister] = useState(false)
  const [loading, setLoading] = useState(false)

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()
    setLoading(true)
    // 纯本地模拟登录成功
    setTimeout(() => {
      setLoading(false)
      onLogin()
    }, 300)
  }

  return (
    <div className="flex h-full w-full items-center justify-center bg-[var(--bg-base)]">
      <div className="w-[420px] p-10 rounded-xl" style={{ background: 'var(--bg-surface)', border: '1px solid var(--border-color)' }}>
        <h1 className="text-2xl font-bold mb-8 text-center text-[var(--text-primary)]">
          {isRegister ? 'Create Account' : 'Login to KaedeNiwa'}
        </h1>
        <form onSubmit={handleSubmit} className="space-y-5">
          <div>
            <label className="block text-[11px] font-semibold uppercase mb-1.5 tracking-wider text-[var(--text-muted)]">Server Address</label>
            <input
              type="text"
              defaultValue="http://127.0.0.1:18080"
              className="w-full rounded-lg px-3 py-2.5 text-sm outline-none bg-[var(--bg-base)] text-[var(--text-primary)] border border-[var(--border-color)]"
            />
          </div>
          <div>
            <label className="block text-[11px] font-semibold uppercase mb-1.5 tracking-wider text-[var(--text-muted)]">Account</label>
            <input
              type="text"
              placeholder="admin"
              data-testid="login-id"
              className="w-full rounded-lg px-3 py-2.5 text-sm outline-none bg-[var(--bg-base)] text-[var(--text-primary)] border border-[var(--border-color)]"
            />
          </div>
          <div>
            <label className="block text-[11px] font-semibold uppercase mb-1.5 tracking-wider text-[var(--text-muted)]">Password</label>
            <input
              type="password"
              data-testid="login-password"
              className="w-full rounded-lg px-3 py-2.5 text-sm outline-none bg-[var(--bg-base)] text-[var(--text-primary)] border border-[var(--border-color)]"
            />
          </div>
          <button
            type="submit"
            disabled={loading}
            data-testid="login-submit"
            className="w-full mt-2 py-2.5 rounded-lg font-bold text-white transition-colors disabled:opacity-50 bg-[#5865f2] hover:bg-[#4752c4]"
          >
            {loading ? 'Processing...' : (isRegister ? 'Register' : 'Login')}
          </button>
          <div className="text-center">
            <button
              type="button"
              onClick={() => setIsRegister(!isRegister)}
              data-testid="login-toggle-register"
              className="text-sm hover:underline text-[var(--text-secondary)]"
            >
              {isRegister ? 'Already have an account? Login' : 'Need an account? Register'}
            </button>
          </div>
        </form>
      </div>
    </div>
  )
}
