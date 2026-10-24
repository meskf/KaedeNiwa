import { useState } from 'react'
import { useAuthStore } from '../stores/auth'
import { initWasm } from '../wasm/client'

export function LoginView() {
  const [isRegister, setIsRegister] = useState(false)
  const [serverUrl, setServerUrl] = useState('http://127.0.0.1:18080')
  const [loginId, setLoginId] = useState('')
  const [password, setPassword] = useState('')
  const [error, setError] = useState('')
  const [loading, setLoading] = useState(false)
  const setAuth = useAuthStore(s => s.setAuth)


  const handleSubmit = async () => {
    if (!loginId || !password) return setError('请输入账号和密码')
    setLoading(true)
    setError('')
    
    try {
      const endpoint = isRegister ? '/api/register' : '/api/login'
      // 1. 发送 HTTP 请求
      const res = await fetch(serverUrl + endpoint, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ login_id: loginId, password })
      })
      
      if (!res.ok) {
        const text = await res.text()
        throw new Error(text || '请求失败')
      }
      
      const data = await res.json()
      const token = data.token || data.session_token
      if (!token) throw new Error('服务器未返回 Token')
      
            // 2. 初始化 WASM
      const client = await initWasm()
      const wsUrl = serverUrl.replace('http://', 'ws://').replace('https://', 'wss://') + '/ws'
      
      // 调用封装好的 RoomSession 登录
      await client.login(serverUrl, wsUrl, loginId, token)
      
      setAuth({ serverUrl, wsUrl, token, loginId })
      
    } catch (e: any) {
      setError(e.message || e.toString())
    } finally {
      setLoading(false)
    }
  }

  return (
    <div className="flex h-full w-full items-center justify-center bg-discord-bg">
      <div className="w-[480px] p-8 bg-discord-sidebar rounded-lg shadow-xl text-white">
        <h1 className="text-2xl font-bold mb-6 text-center">
          {isRegister ? '注册新账号' : '登录 KaedeNiwa'}
        </h1>
        
        {error && <div className="mb-4 p-2 bg-red-500/20 text-red-400 text-sm rounded border border-red-500/50">{error}</div>}

        <div className="space-y-4">
          <div>
            <label className="block text-xs font-semibold text-gray-400 mb-1 uppercase">服务器地址</label>
            <input 
              type="text" 
              value={serverUrl} 
              onChange={e => setServerUrl(e.target.value)}
              className="w-full bg-[#1e1f22] border-none outline-none rounded p-2.5 text-sm"
            />
          </div>
          <div>
            <label className="block text-xs font-semibold text-gray-400 mb-1 uppercase">账号 (Login ID)</label>
            <input 
              type="text" 
              value={loginId} 
              onChange={e => setLoginId(e.target.value)}
              placeholder="例如: admin"
              className="w-full bg-[#1e1f22] border-none outline-none rounded p-2.5 text-sm"
            />
          </div>
          <div>
            <label className="block text-xs font-semibold text-gray-400 mb-1 uppercase">密码</label>
            <input 
              type="password" 
              value={password} 
              onChange={e => setPassword(e.target.value)}
              className="w-full bg-[#1e1f22] border-none outline-none rounded p-2.5 text-sm"
            />
          </div>
          
          <button 
            onClick={handleSubmit}
            disabled={loading}
            className="w-full mt-4 bg-indigo-500 hover:bg-indigo-600 transition-colors disabled:opacity-50 text-white font-medium py-2.5 rounded"
          >
            {loading ? '处理中...' : (isRegister ? '注册并登录' : '登录')}
          </button>
          
          <div className="text-center mt-4">
            <button 
              onClick={() => setIsRegister(!isRegister)}
              className="text-sm text-indigo-400 hover:underline"
            >
              {isRegister ? '已有账号？去登录' : '没有账号？去注册'}
            </button>
          </div>
        </div>
      </div>
    </div>
  )
}
