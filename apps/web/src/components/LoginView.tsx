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
      
      const data = await res.json().catch(() => ({}))
      
      if (!res.ok) {
        if (data.message === 'login_id_taken') {
          throw new Error('This account is already taken. Try logging in instead.')
        } else if (data.message === 'invalid_credentials' || data.error === 'unauthorized') {
          throw new Error('Invalid account or password.')
        }
        throw new Error(data.message || data.error || 'Request failed')
      }

      const token = data.token || data.session_token
      if (!token) throw new Error('Server did not return a token')
      
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
    <div className="h-screen w-screen bg-[var(--bg-app)] flex flex-col items-center justify-center text-[var(--text-main)] font-sans">
      <div className="w-full max-w-[380px] px-6">
        
        <div className="text-center mb-10">
          <h1 className="text-3xl font-extrabold tracking-tight mb-2">
            {isRegister ? 'Create an account' : 'Welcome back'}
          </h1>
          <p className="text-[13px] text-[var(--text-muted)]">
            {isRegister ? 'Enter your details to create your account.' : 'Enter your details to access your workspace.'}
          </p>
        </div>

        <form onSubmit={handleSubmit} className="space-y-4">
          <div>
            <label className="block text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase mb-1.5">Server Address</label>
            <input
              type="text"
              value={serverUrl}
              onChange={e => setServerUrl(e.target.value)}
              className="w-full bg-[var(--bg-hover)] rounded-lg px-3 py-2.5 text-[14px] outline-none text-[var(--text-main)] placeholder-[var(--text-muted)] focus:ring-1 focus:ring-[var(--bg-active)] transition-all"
            />
          </div>
          <div>
            <label className="block text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase mb-1.5">Account</label>
            <input
              type="text"
              value={loginId}
              onChange={e => setLoginId(e.target.value)}
              placeholder="e.g. admin"
              className="w-full bg-[var(--bg-hover)] rounded-lg px-3 py-2.5 text-[14px] outline-none text-[var(--text-main)] placeholder-[var(--text-muted)] focus:ring-1 focus:ring-[var(--bg-active)] transition-all"
            />
          </div>
          <div>
            <label className="block text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase mb-1.5">Password</label>
            <input
              type="password"
              value={password}
              onChange={e => setPassword(e.target.value)}
              placeholder="••••••••"
              className="w-full bg-[var(--bg-hover)] rounded-lg px-3 py-2.5 text-[14px] outline-none text-[var(--text-main)] placeholder-[var(--text-muted)] focus:ring-1 focus:ring-[var(--bg-active)] transition-all"
            />
          </div>

          {error && (
            <div className="p-3 rounded-lg bg-red-500/10 text-red-500 text-[13px] border border-red-500/20">
              {error}
            </div>
          )}

          <button
            type="submit"
            disabled={loading}
            className="w-full mt-2 py-2.5 rounded-lg bg-[var(--text-main)] text-[var(--bg-app)] text-[14px] font-semibold hover:opacity-90 transition-opacity disabled:opacity-50"
          >
            {loading ? 'Processing...' : (isRegister ? 'Sign up' : 'Sign in')}
          </button>
        </form>

        <div className="mt-8 text-center">
          <button
            type="button"
            onClick={() => { setIsRegister(!isRegister); setError(''); }}
            className="text-[13px] text-[var(--text-muted)] hover:text-[var(--text-main)] transition-colors"
          >
            {isRegister ? 'Already have an account? Sign in' : 'Don\'t have an account? Sign up'}
          </button>
        </div>
      </div>
    </div>
  )
}