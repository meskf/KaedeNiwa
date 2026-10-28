import { useState, useEffect } from 'react'
import { useAuthStore } from './stores/auth'
import { initWasm, getClient, recordLoginArgs, stopReconnect, allowReconnect } from './wasm/client'
import { LoginView } from './components/LoginView'
import { MainView } from './components/MainView'
import { DynamicBackground } from './components/DynamicBackground'

export default function App() {


  const isLoggedIn = useAuthStore(s => s.isLoggedIn)
  const [dynamicBg, setDynamicBg] = useState(() => localStorage.getItem('kaede-dynamic-bg') === 'true')

  useEffect(() => {
    if (dynamicBg) document.documentElement.setAttribute('data-prism', 'true')
    else document.documentElement.removeAttribute('data-prism')
  }, [dynamicBg])

  useEffect(() => {
    const handleToggle = () => setDynamicBg(localStorage.getItem('kaede-dynamic-bg') === 'true')
    window.addEventListener('kaede-dynamic-bg-changed', handleToggle)
    return () => window.removeEventListener('kaede-dynamic-bg-changed', handleToggle)
  }, [])

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
    const parseRaw = (raw: any) => typeof raw === 'string' ? JSON.parse(raw) : raw
    const reqClient = () => {
      const c = getClient()
      if (!c) throw new Error('client not ready')
      return c as any
    }
    ;(window as any).__kaede_debug__ = {
      loadHistory: async (ch: string, limit = 50) => parseRaw(await reqClient().load_history(ch, limit)),
      joinChannel: async (ch: string) => reqClient().join_channel(ch),
      channelState: async (ch: string) => {
        const c = getClient()
        if (!c) return { error: 'client not ready' }
        try { return parseRaw((c as any).debug_channel_state(ch)) } catch (e) { return { error: String(e) } }
      },
      ping: () => 'pong',

      dm: {
        list:     async () => parseRaw(await reqClient().listDmConversations()),
        open:     async (peer: string) => parseRaw(await reqClient().openDm(peer)),
        history:  async (convId: string, limit = 50) => parseRaw(await reqClient().dmLoadHistory(convId, limit)),
        send:     async (convId: string, text: string) => reqClient().sendDm(convId, text),
        markRead: async (convId: string, ts: number) => reqClient().markDmRead(convId, BigInt(ts)),
      },

      friends: {
        list:        async () => parseRaw(await reqClient().listFriends()),
        sendRequest: async (peer: string) => reqClient().sendFriendRequest(peer),
        accept:      async (peer: string) => reqClient().acceptFriend(peer),
        remove:      async (peer: string) => reqClient().removeFriend(peer),
      },

      channels: {
        list:   async () => parseRaw(await reqClient().listChannels()),
        create: async (name: string, kind: string, category_id?: string | null, is_private?: boolean | null, password?: string | null) =>
                  parseRaw(await reqClient().createChannel(name, kind, category_id, is_private, password)),
        update: async (id: string, name?: string | null, category_id?: string | null) =>
                  parseRaw(await reqClient().updateChannel(id, name, category_id)),
        delete: async (id: string) => reqClient().deleteChannel(id),
      },

      categories: {
        list:   async () => parseRaw(await reqClient().listCategories()),
        create: async (name: string, position?: number | null) => parseRaw(await reqClient().createCategory(name, position)),
        update: async (id: string, name?: string | null, position?: number | null) =>
                  parseRaw(await reqClient().updateCategory(id, name, position)),
        delete: async (id: string) => reqClient().deleteCategory(id),
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

  return (
    <>
      {dynamicBg && <DynamicBackground />}
      <div className={`h-full w-full relative z-10 ${dynamicBg ? 'bg-transparent' : 'bg-[var(--bg-app)]'}`}>
        {isLoggedIn
          ? <MainView onLogout={() => useAuthStore.getState().logout()} />
          : <LoginView onLogin={() => {}} />}
      </div>
    </>
  )
}
