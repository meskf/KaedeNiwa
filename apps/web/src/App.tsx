import { useState, useEffect } from 'react'
import { useAuthStore } from './stores/auth'
import { useChatStore } from './stores/chat'
import { initWasm, getClient, recordLoginArgs, stopReconnect, allowReconnect } from './wasm/client'
import { LoginView } from './components/LoginView'
import { MainView } from './components/MainView'
import { DynamicBackground } from './components/DynamicBackground'
import { WindowControls } from './components/WindowControls'
import { TitleBarDragStrip } from './components/TitleBarDragStrip'

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

        // [test-hook] 全局捕获所有 ClientEvent 到 window.__kaede_events__
        // 在 MainView mount 之前 patch，保证 MainView 调 setOnEvent 时走 wrapper
        ;(window as any).__kaede_events__ = (window as any).__kaede_events__ || []
        try {
          const c0: any = getClient()
          if (c0 && !c0.__kdPatched) {
            const orig = c0.setOnEvent.bind(c0)
            c0.setOnEvent = (cb: Function) => {
              const wrapper = (raw: string) => {
                try {
                  ;(window as any).__kaede_events__.push(raw)
                } catch {}
                return cb(raw)
              }
              return orig(wrapper)
            }
            c0.__kdPatched = true
            console.log('[test-hook] ClientEvent 捕获已挂载')
          }
        } catch (e) { console.warn('[test-hook] patch failed:', e) }

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
    // Tab lock 只在 dev 环境启用：
    // - 生产：用户开两个标签看不同服务器是合法需求，不该被强制登出
    // - iOS WKWebView 后台回收后，localStorage 里的 active-tab 指向已死标签，
    //   回来时被误判为"第二个标签"→ 直接登出（真实 bug）
    // - dev/e2e：需要防止多实例串号，保留
    if (import.meta.env.VITE_FORCE_TAB_LOCK !== '1') return
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
      sendMessage: async (ch: string, text: string) => reqClient().send_message(ch, text),
      channelState: async (ch: string) => {
        const c = getClient()
        if (!c) return { error: 'client not ready' }
        try { return parseRaw((c as any).debug_channel_state(ch)) } catch (e) { return { error: String(e) } }
      },
      ping: () => 'pong',
      store: {
        getState: () => useChatStore.getState(),
      },

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


      nodes: {
        list: async () => {
          const raw = await reqClient().listNodes()
          return raw.map((r: any) => typeof r === 'string' ? JSON.parse(r) : r)
        },
        create: async (name: string, kind: string, parent_id?: string | null, is_private = false, password?: string | null) => {
          const raw = await reqClient().createNode(name, kind, parent_id, is_private, password)
          return parseRaw(raw)
        },
        update: async (id: string, name?: string | null, parent_id?: string | null, position?: number | null) =>
          reqClient().updateNode(id, name, parent_id, position),
        delete: async (id: string) => reqClient().deleteNode(id),
      },

      channels: {
        list:   async () => parseRaw(await reqClient().listChannels()),
        create: async (name: string, kind: string, category_id?: string | null, is_private?: boolean | null, password?: string | null) =>
                  parseRaw(await reqClient().createChannel(name, kind, category_id, is_private, password)),
        update: async (id: string, name?: string | null, category_id?: string | null) =>
                  parseRaw(await reqClient().updateChannel(id, name, category_id)),
        delete: async (id: string) => reqClient().deleteChannel(id),
        joinWithPassword: async (id: string, password: string) =>
                  reqClient().joinChannelWithPassword(id, password),
      },

      categories: {
        list:   async () => parseRaw(await reqClient().listCategories()),
        create: async (name: string, position?: number | null) => parseRaw(await reqClient().createCategory(name, position)),
        update: async (id: string, name?: string | null, position?: number | null) =>
                  parseRaw(await reqClient().updateCategory(id, name, position)),
        delete: async (id: string) => reqClient().deleteCategory(id),
      },

      attachments: {
        upload: async (targetId: string, name: string, mime: string, bytes: Uint8Array) => {
          const arr = Array.from(bytes)
          return parseRaw(await reqClient().uploadAttachment(targetId, name, mime, new Uint8Array(arr)))
        },
        download: async (targetId: string, attId: string, expectedSha256?: string | null) => {
          const buf = await reqClient().downloadAttachment(targetId, attId, expectedSha256 ?? null)
          return Array.from(buf as Uint8Array)
        },
      },

      messages: {
        sendWithAttachments: async (channelId: string, text: string, attachments: any[]) =>
          reqClient().sendMessageWithAttachments(channelId, text, JSON.stringify(attachments)),
      },

      typing: {
        send: async (ch: string) => reqClient().sendTyping(ch),
      },
      reaction: {
        // 兼容旧签名（targetId=0 → 接收端按 ts 匹配）。
        send: async (ch: string, targetTs: number, emoji: string, action: 'add' | 'remove' = 'add') =>
          reqClient().sendReaction(ch, 0n, BigInt(targetTs), emoji, action),
        // v8.1：id 优先（避免发送端乐观 ts 与接收端服务端 ts 错位）。
        sendById: async (ch: string, targetId: number, targetTs: number, emoji: string, action: 'add' | 'remove' = 'add') =>
          reqClient().sendReaction(ch, BigInt(targetId), BigInt(targetTs), emoji, action),
      },
      presence: {
        set: async (status: string) => reqClient().setPresence(status),
      },

      events: {
        list: (type?: string) => {
          const arr: string[] = (window as any).__kaede_events__ || []
          const parsed = arr.map(r => { try { return JSON.parse(r) } catch { return null } }).filter(Boolean)
          if (!type) return parsed
          return parsed.filter((e: any) => e && typeof e === 'object' && (e.type === type || Object.prototype.hasOwnProperty.call(e, type)))
        },
        raw: () => ((window as any).__kaede_events__ || []).slice(),
        clear: () => { (window as any).__kaede_events__ = [] },
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
      <TitleBarDragStrip />
      <WindowControls />
      {dynamicBg && <DynamicBackground />}
      <div className={`h-full w-full relative z-10 ${dynamicBg ? 'bg-transparent' : 'bg-[var(--bg-app)]'}`}>
        {isLoggedIn
          ? <MainView onLogout={() => useAuthStore.getState().logout()} />
          : <LoginView onLogin={() => {}} />}
      </div>
    </>
  )
}

