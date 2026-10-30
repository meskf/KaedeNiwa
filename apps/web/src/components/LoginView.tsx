import { useState, useEffect } from 'react'

/**
 * 推导默认服务器地址：
 * - localhost / 127.0.0.1 / Electron (file://) → 用 127.0.0.1
 * - 否则（局域网 IP 访问）→ 用当前 hostname（服务端已绑定 0.0.0.0）
 */
function resolveDefaultServerUrl(): string {
  if (typeof window === 'undefined') return 'http://127.0.0.1:18080'
  const origin = window.location.origin
  // 如果是 Vite 开发服务器（5173），使用当前 origin，借助 Vite proxy 转发
  if (window.location.port === '5173' || window.location.port === '5174') {
    return origin
  }
  const h = window.location.hostname
  if (!h || h === 'localhost' || h === '127.0.0.1') return 'http://127.0.0.1:18080'
  return `http://${h}:18080`
}

import { useAuthStore } from '../stores/auth'
import { initWasm, getClient, recordLoginArgs, allowReconnect } from '../wasm/client'
import { useI18n } from '../stores/i18n'

export function LoginView({ onLogin }: { onLogin: () => void }) {
  const [dynamicBg, setDynamicBg] = useState(() => localStorage.getItem('kaede-dynamic-bg') === 'true')
  useEffect(() => {
    const handleToggle = () => setDynamicBg(localStorage.getItem('kaede-dynamic-bg') === 'true')
    window.addEventListener('kaede-dynamic-bg-changed', handleToggle)
    return () => window.removeEventListener('kaede-dynamic-bg-changed', handleToggle)
  }, [])
  const [isRegister, setIsRegister] = useState(false)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')
  const [serverUrl, setServerUrl] = useState(() => resolveDefaultServerUrl())
  const [loginId, setLoginId] = useState('')
  const [password, setPassword] = useState('')
  const setAuth = useAuthStore(s => s.setAuth)
  const { t } = useI18n()

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!loginId || !password) { setError(t('login.errorEmptyFields')); return }
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
      
      // 如果走的是 Vite 开发服务器（5173），强制通过 Vite proxy 转发 WS 请求
      // 避免服务端返回了 127.0.0.1 导致手机连不上自己的本机
      const wsUrl = (window.location.port === '5173' || window.location.port === '5174')
        ? window.location.origin.replace(/^http/, 'ws') + '/ws'
        : (data.ws_url || serverUrl.replace(/^http/, 'ws') + '/ws')
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
    <div className={`h-screen w-screen flex flex-col items-center justify-center text-[var(--text-main)] font-sans ${dynamicBg ? 'bg-transparent' : 'bg-[var(--bg-app)]'}`}>
      <div className={`w-full max-w-[380px] px-8 py-10 rounded-2xl transition-all ${dynamicBg ? 'bg-[var(--bg-app)]/70 backdrop-blur-xl border border-[var(--border-subtle)]/30 shadow-2xl' : ''}`}>
        
        <div className="text-center mb-10">
          <h1 className="text-3xl font-extrabold tracking-tight mb-2">
            {isRegister ? t('login.createAccount') : t('login.welcomeBack')}
          </h1>
          <p className="text-[13px] text-[var(--text-muted)]">
            {isRegister ? t('login.subtitleRegister') : t('login.subtitleLogin')}
          </p>
        </div>

        <form onSubmit={handleSubmit} className="space-y-4">
          <div>
            <label className="block text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase mb-1.5">{t('login.serverAddress')}</label>
            <input
              type="text"
              value={serverUrl}
              onChange={e => setServerUrl(e.target.value)}
              className="w-full bg-[var(--bg-hover)] rounded-lg px-3 py-2.5 text-[14px] outline-none text-[var(--text-main)] placeholder-[var(--text-muted)] focus:ring-1 focus:ring-[var(--bg-active)] transition-all border border-transparent focus:border-[var(--border-subtle)]"
            />
          </div>
          <div>
            <label className="block text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase mb-1.5">{t('login.account')}</label>
            <input
              type="text"
              value={loginId}
              onChange={e => setLoginId(e.target.value)}
              data-testid="login-id" placeholder={t('login.placeholderAccount')}
              className="w-full bg-[var(--bg-hover)] rounded-lg px-3 py-2.5 text-[14px] outline-none text-[var(--text-main)] placeholder-[var(--text-muted)] focus:ring-1 focus:ring-[var(--bg-active)] transition-all"
            />
          </div>
          <div>
            <label className="block text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase mb-1.5">{t('login.password')}</label>
            <input
              type="password"
              value={password}
              onChange={e => setPassword(e.target.value)}
              data-testid="login-password" placeholder="••••••••"
              className="w-full bg-[var(--bg-hover)] rounded-lg px-3 py-2.5 text-[14px] outline-none text-[var(--text-main)] placeholder-[var(--text-muted)] focus:ring-1 focus:ring-[var(--bg-active)] transition-all"
            />
          </div>

          {error && (
            <div className="p-3 rounded-lg bg-red-500/10 text-red-500 text-[13px] border border-red-500/20">
              {error}
            </div>
          )}

          <button
            type="submit" data-testid="login-submit"
            disabled={loading}
            className="w-full mt-2 py-2.5 rounded-lg bg-[var(--text-main)] text-[var(--bg-app)] text-[14px] font-semibold hover:opacity-90 transition-opacity disabled:opacity-50"
          >
            {loading ? t('login.processing') : (isRegister ? t('login.signUp') : t('login.signIn'))}
          </button>
        </form>

        <div className="mt-8 text-center">
          <button data-testid="login-toggle-register"
            type="button"
            onClick={() => { setIsRegister(!isRegister); setError(''); }}
            className="text-[13px] text-[var(--text-muted)] hover:text-[var(--text-main)] transition-colors"
          >
            {isRegister ? t('login.toggleToSignIn') : t('login.toggleToSignUp')}
          </button>
        </div>
      </div>
    </div>
  )
}