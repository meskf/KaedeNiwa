import { useState, useEffect } from 'react'
import { useAuthStore } from './stores/auth'
import { initWasm, getClient, recordLoginArgs, stopReconnect, allowReconnect } from './wasm/client'
import { LoginView } from './components/LoginView'
import { MainView } from './components/MainView'

export default function App() {
  const isLoggedIn = useAuthStore(s => s.isLoggedIn)
  const [booting, setBooting] = useState(true)

  useEffect(() => {
    const boot = async () => {
      try {
        // 1. 初始化 WASM
        await initWasm()

        // 2. 如果有 localStorage 中的登录态，自动重连
        const { isLoggedIn: logged, serverUrl, wsUrl, token, loginId } = useAuthStore.getState()
        if (logged && token && loginId) {
          console.log('🔄 检测到登录态，自动重连...')
          try {
            const client = getClient()
            if (!client) throw new Error('WASM 未初始化')
            await client.login(serverUrl, wsUrl, loginId, token)
            recordLoginArgs({ serverUrl, wsUrl, loginId, token })
            allowReconnect()
            console.log('✅ 自动重连成功')
          } catch (e) {
            console.error('自动重连失败，需要重新登录:', e)
            stopReconnect()
            useAuthStore.getState().logout()
          }
        }
      } catch (e) {
        console.error('初始化失败:', e)
      } finally {
        setBooting(false)
      }
    }
    boot()
  }, [])

  // 暴露 debug 接口（DEV 模式，boot 完成后）
  useEffect(() => {
    ;(window as any).__kaede_debug__ = {
      channelState: async (ch: string) => {
        const c = getClient()
        if (!c) return { error: 'client not ready' }
        try {
          const s = (c as any).debug_channel_state(ch)
          return typeof s === 'string' ? JSON.parse(s) : s
        } catch (e) {
          return { error: String(e) }
        }
      },
      ping: () => 'pong',
      forceDisconnect: async () => {
        const c = getClient()
        if (!c) return { error: 'client not ready' }
        try {
          await (c as any).forceDisconnect()
          return { ok: true }
        } catch (e) {
          return { error: String(e) }
        }
      },
    }
    return () => { delete (window as any).__kaede_debug__ }
  }, [booting])

  if (booting) {
    return (
      <div className="flex h-full w-full items-center justify-center bg-discord-bg text-white">
        <div className="text-sm text-gray-400">加载中...</div>
      </div>
    )
  }

  return isLoggedIn ? <MainView /> : <LoginView />
}
