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
        await initWasm()
        const { isLoggedIn: logged, serverUrl, wsUrl, token, loginId } = useAuthStore.getState()
        if (logged && token && loginId) {
          console.log('reconnecting...')
          try {
            const client = getClient()
            if (!client) throw new Error('WASM not initialized')
            await client.login(serverUrl, wsUrl, loginId, token)
            recordLoginArgs({ serverUrl, wsUrl, loginId, token })
            allowReconnect()
            console.log('reconnected')
          } catch (e) {
            console.error('auto reconnect failed:', e)
            stopReconnect()
            useAuthStore.getState().logout()
          }
        }
      } catch (e) {
        console.error('init failed:', e)
      } finally {
        setBooting(false)
      }
    }
    boot()
  }, [])

  useEffect(() => {
    const TAB_KEY = 'kaede-active-tab'
    const TAB_ID_KEY = 'kaede-tab-id'
    let tabId = sessionStorage.getItem(TAB_ID_KEY)
    if (!tabId) {
      tabId = Math.random().toString(36).slice(2) + Date.now().toString(36)
      sessionStorage.setItem(TAB_ID_KEY, tabId)
    }
    const existing = localStorage.getItem(TAB_KEY)
    if (existing && existing !== tabId) {
      console.log('[tab-lock] another tab active, logging out')
      useAuthStore.getState().logout()
      sessionStorage.setItem('kaede-tab-conflict', '1')
      setBooting(false)
      return
    }
    localStorage.setItem(TAB_KEY, tabId)
    const onStorage = (e: StorageEvent) => {
      if (e.key === TAB_KEY && e.newValue && e.newValue !== tabId) {
        console.log('[tab-lock] another tab took over')
        useAuthStore.getState().logout()
        sessionStorage.setItem('kaede-tab-conflict', '1')
      }
    }
    window.addEventListener('storage', onStorage)
    return () => {
      window.removeEventListener('storage', onStorage)
      if (localStorage.getItem(TAB_KEY) === tabId) localStorage.removeItem(TAB_KEY)
    }
  }, [])

  useEffect(() => {
    ;(window as any).__kaede_debug__ = {
      loadHistory: async (ch: string, limit = 50) => {
        const c = getClient()
        if (!c) throw new Error('client not ready')
        const raw = await c.load_history(ch, limit)
        const arr = typeof raw === 'string' ? JSON.parse(raw) : raw
        return Array.isArray(arr) ? arr : { raw }
      },
      joinChannel: async (ch: string) => {
        const c = getClient()
        if (!c) throw new Error('client not ready')
        return await c.join_channel(ch)
      },
      channelState: async (ch: string) => {
        const c = getClient()
        if (!c) return { error: 'client not ready' }
        try {
          const s = (c as any).debug_channel_state(ch)
          return typeof s === 'string' ? JSON.parse(s) : s
        } catch (e) { return { error: String(e) } }
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
        try { await (c as any).forceDisconnect(); return { ok: true } } catch (e) { return { error: String(e) } }
      },
    }
    return () => { delete (window as any).__kaede_debug__ }
  }, [booting])

  if (booting) {
    return (
      <div className="flex h-full w-full items-center justify-center bg-[var(--bg-app)] text-[var(--text-muted)]">
        <div className="text-sm">Loading...</div>
      </div>
    )
  }

  return isLoggedIn
    ? <MainView onLogout={() => useAuthStore.getState().logout()} />
    : <LoginView onLogin={() => {}} />
}