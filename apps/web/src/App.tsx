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

  useEffect(() => {
    // P1: 多 tab 锁 —— 同一账号只允许一个 tab 活跃。
    // MLS identity 在 IndexedDB 里，多 tab 共享会导致 ratchet 分叉。
    const TAB_KEY = 'kaede-active-tab'
    const TAB_ID_KEY = 'kaede-tab-id'
    // P1-fix: reload 后 sessionStorage 保留 tabId，新 tab 独立生成
    // 这样 reload 不会被误判为"另一个 tab"
    let tabId = sessionStorage.getItem(TAB_ID_KEY)
    if (!tabId) {
      tabId = Math.random().toString(36).slice(2) + Date.now().toString(36)
      sessionStorage.setItem(TAB_ID_KEY, tabId)
    }

    // 启动时检查是否已有活跃 tab
    const existing = localStorage.getItem(TAB_KEY)
    if (existing && existing !== tabId) {
      // 已有 tab → 当前 tab 不抢占，直接 logout（另一 tab 继续工作）
      console.log('[tab-lock] another tab active, logging out current tab')
      useAuthStore.getState().logout()
      sessionStorage.setItem('kaede-tab-conflict', '1')
      setBooting(false)
      return
    }

    // 抢占
    localStorage.setItem(TAB_KEY, tabId)

    // 监听其他 tab 抢占
    const onStorage = (e: StorageEvent) => {
      if (e.key === TAB_KEY && e.newValue && e.newValue !== tabId) {
        console.log('[tab-lock] another tab took over, logging out')
        useAuthStore.getState().logout()
        sessionStorage.setItem('kaede-tab-conflict', '1')
      }
    }
    window.addEventListener('storage', onStorage)

    return () => {
      window.removeEventListener('storage', onStorage)
      if (localStorage.getItem(TAB_KEY) === tabId) {
        localStorage.removeItem(TAB_KEY)
      }
    }
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
      dm: {
        list: async () => { const c = getClient(); if (!c) throw new Error('no client'); return await (c as any).listDmConversations(); },
        open: async (peer: string) => { const c = getClient(); if (!c) throw new Error('no client'); return await (c as any).openDm(peer); },
        history: async (convId: string, limit = 50) => { const c = getClient(); if (!c) throw new Error('no client'); return await (c as any).dmLoadHistory(convId, limit); },
        send: async (convId: string, text: string) => { const c = getClient(); if (!c) throw new Error('no client'); return await (c as any).sendDm(convId, text); },
      },
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
