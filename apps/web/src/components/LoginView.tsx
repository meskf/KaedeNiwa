import { useState } from 'react'
import { useAuthStore } from '../stores/auth'
import { initWasm, getClient, recordLoginArgs, allowReconnect } from '../wasm/client'

export function LoginView({ onLogin }: { onLogin: () => void }) {
  const [isRegister, setIsRegister] = useState(false)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')
  const [serverUrl, setServerUrl] = useState('http://127.0.0.1:18080')
  const [loginId, setLoginId] = useState('')
  const [password, setPassword] = useState('')
  const setAuth = useAuthStore(s => s.setAuth)

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!loginId || !password) { setError('Please enter account and password'); return }
    setLoading(true)
    setError('')
    try {
      const endpoint = isRegister ? '/api/register' : '/api/login'
      const res = await fetch(serverUrl + endpoint, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ login_id: loginId, password })
      })
      if (!res.ok) {
        const text = await res.text()
        throw new Error(text || 'Request failed')
      }
      const data = await res.json()
      const token = data.token || data.session_token
      if (!token) throw new Error('Server did not return a token: ' + JSON.stringify(data))
      const wsUrl = data.ws_url || serverUrl.replace(/^http/, 'ws') + '/ws'
      await initWasm()
      const client = getClient()
      if (!client) throw new Error('WASM not initialized')
      await client.login(serverUrl, wsUrl, loginId, token)
      setAuth({ serverUrl, wsUrl, token, loginId })
      recordLoginArgs({ serverUrl, wsUrl, loginId, token })
      allowReconnect()
      onLogin()
    } catch (e: any) {
      setError(String(e.message || e))
    } finally {
      setLoading(false)
    }
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
              value={serverUrl}
              onChange={e => setServerUrl(e.target.value)}
              data-testid="login-server-url"
              className="w-full rounded-lg px-3 py-2.5 text-sm outline-none bg-[var(--bg-base)] text-[var(--text-primary)] border border-[var(--border-color)]"
            />
          </div>
          <div>
            <label className="block text-[11px] font-semibold uppercase mb-1.5 tracking-wider text-[var(--text-muted)]">Account</label>
            <input
              type="text"
              placeholder="admin"
              value={loginId}
              onChange={e => setLoginId(e.target.value)}
              data-testid="login-id"
              className="w-full rounded-lg px-3 py-2.5 text-sm outline-none bg-[var(--bg-base)] text-[var(--text-primary)] border border-[var(--border-color)]"
            />
          </div>
          <div>
            <label className="block text-[11px] font-semibold uppercase mb-1.5 tracking-wider text-[var(--text-muted)]">Password</label>
            <input
              type="password"
              value={password}
              onChange={e => setPassword(e.target.value)}
              data-testid="login-password"
              className="w-full rounded-lg px-3 py-2.5 text-sm outline-none bg-[var(--bg-base)] text-[var(--text-primary)] border border-[var(--border-color)]"
            />
          </div>
          {error && <div className="text-xs text-red-500">{error}</div>}
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