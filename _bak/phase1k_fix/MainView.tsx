import { useState, useEffect, useRef } from 'react'
import { useAuthStore } from '../stores/auth'
import { useProfileStore } from '../stores/profile'
import { useServerStore } from '../stores/server'
import { useChatStore } from '../stores/chat'
import { getClient, scheduleReconnect } from '../wasm/client'
import { connectMedia, disconnectMedia, updateE2EEKey } from '../lib/livekit'
import { useMediaState } from '../hooks/useLiveKitRoom'
import { MediaBar } from './MediaBar'
import { ScreenShareView } from './ScreenShareView'
import { StageManagerServers } from './StageManagerServers'
import { IconSun, IconMoon, IconSearch, IconUsers, IconHash, IconSettings } from './icons'
import { useTheme } from '../lib/useTheme'
import { useI18n } from '../stores/i18n'
import { SettingsView } from './SettingsView'
import { MoreView } from './MoreView'
import { UserContextMenu } from './UserContextMenu'
import { SearchPalette } from './SearchPalette'
import { CreateChannelDialog } from './CreateChannelDialog'
import { ChannelSettingsDialog } from './ChannelSettingsDialog'
import { DMView } from './DMView'
import { FriendsView } from './FriendsView'
import { MessageAttachments } from './MessageAttachments'
import { ChannelTree } from './ChannelTree'
import { NodeContextMenu } from './NodeContextMenu'
import { RichTextInput, type RichTextInputHandle } from './RichTextInput'
import { MessageText } from './MessageText'
import { extractMessage } from '../lib/messageUtils'
import { useAvatarVersion } from '../lib/avatarCache'
import { useMediaQuery, BP } from '../hooks/useMediaQuery'




export function MainView({ onLogout }: { onLogout: () => void }) {
  const [ctxNode, setCtxNode] = useState<{ x: number, y: number, node: any } | null>(null)
  const setNodes = useChatStore(s => s.setNodes)


  const [dynamicBg, setDynamicBg] = useState(() => localStorage.getItem('kaede-dynamic-bg') === 'true')
  useEffect(() => {
    const handleToggle = () => setDynamicBg(localStorage.getItem('kaede-dynamic-bg') === 'true')
    window.addEventListener('kaede-dynamic-bg-changed', handleToggle)
    return () => window.removeEventListener('kaede-dynamic-bg-changed', handleToggle)
  }, [])

  
  
  const loginId = useAuthStore(s => s.loginId)
  const token = useAuthStore(s => s.token)
  const fetchMe = useProfileStore(s => s.fetchMe)
  const serverProfile = useServerStore(s => s.profile)

  useEffect(() => {
    if (token) {
      fetchMe(token)
      // 手动拉取 server profile
      fetch('/api/server', { headers: { Authorization: `Bearer ${token}` } })
        .then(res => res.ok ? res.json() : null)
        .then(data => {
          if (data) {
            useServerStore.setState({ profile: data })
          }
        }).catch(console.error)
    }
  }, [token, fetchMe])

  const nodes = useChatStore(s => s.nodes)

  useEffect(() => {
    if (!loginId) return
    let cancelled = false
    ;(async () => {
      const c = getClient()
      if (!c) return
      try {
        let res = await (c as any).listNodes()
        let parsed = res.map((r: any) => JSON.parse(r))
        if (parsed.length === 0 && !cancelled) {
          // Seed 默认目录结构
          const folder1 = await (c as any).createNode('文字频道', 'folder', null, false, null)
          await (c as any).createNode('general', 'text', JSON.parse(folder1).id, false, null)
          const folder2 = await (c as any).createNode('语音频道', 'folder', null, false, null)
          await (c as any).createNode('语音大厅', 'voice', JSON.parse(folder2).id, false, null)
          
          res = await (c as any).listNodes()
          parsed = res.map((r: any) => JSON.parse(r))
        }
        if (!cancelled) setNodes(parsed)
      } catch (e) { console.error('load nodes err:', e) }
    })()
    return () => { cancelled = true }
  }, [loginId, setNodes])
  

  

  const channelList = useChatStore(s => s.channelList)
  const serverMembers = useChatStore(s => s.serverMembers)
  const setServerMembers = useChatStore(s => s.setServerMembers)
  const setChannelList = useChatStore(s => s.setChannelList)
  const {
    currentChannel, setChannel,
    activeView, setActiveView,
    messages, setMessages, addMessage,
    pendingMessages, addPending, removePending, markPendingFailed, incrementRetry,
    peerStatus,
      typingUsers, setTyping, pruneTyping,
  } = useChatStore()
  const categories = useChatStore(s => s.categories)
  const setCategories = useChatStore(s => s.setCategories)
  const richInputRef = useRef<RichTextInputHandle | null>(null)
  const { theme, setTheme } = useTheme()
  const { t, lang } = useI18n()
  const isMd = useMediaQuery(BP.md)
  const [showMembers, setShowMembers] = useState(isMd)
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number; uid: string; inVoice: boolean } | null>(null)
  // 断点驱动的侧栏状态。
  // 旧实现只在 mount 时读一次 innerWidth —— 旋转屏幕 / 拖窗口不重算。
  // 跨断点时重置为默认（手机收起、桌面展开）；同一断点内由用户完全掌控。
  const [sidebarOpen, setSidebarOpen] = useState(isMd)
  const prevIsMdRef = useRef(isMd)
  useEffect(() => {
    if (prevIsMdRef.current !== isMd) {
      prevIsMdRef.current = isMd
      setSidebarOpen(isMd)
    }
  }, [isMd])
  const avatarVer = useAvatarVersion()

  const [searchOpen, setSearchOpen] = useState(false)
  const [createDialogCategory, setCreateDialogCategory] = useState<string | null | undefined>(undefined)
  const [settingsChannelId, setSettingsChannelId] = useState<string | null>(null)

  // A: category management state
  const [newCategoryPrompt, setNewCategoryPrompt] = useState(false)
  const [newCategoryName, setNewCategoryName] = useState('')
  const [manageCategory, setManageCategory] = useState<{ id: string; name: string } | null>(null)
  const [manageCategoryName, setManageCategoryName] = useState('')

  
  const handleCreateCategory = async () => {
    const name = newCategoryName.trim()
    if (!name) return
    try {
      const client = getClient()
      if (!client) return
      const raw = await client.createCategory(name, undefined)
      const cat = typeof raw === 'string' ? JSON.parse(raw) : raw
      setCategories([...categories, cat])
      setNewCategoryPrompt(false)
      setNewCategoryName('')
    } catch (e) { console.error('[create category]', e) }
  }

  const handleRenameCategory = async () => {
    if (!manageCategory) return
    const name = manageCategoryName.trim()
    if (!name || name === manageCategory.name) {
      setManageCategory(null)
      return
    }
    try {
      const client = getClient()
      if (!client) return
      const raw = await client.updateCategory(manageCategory.id, name, undefined)
      const cat = typeof raw === 'string' ? JSON.parse(raw) : raw
      setCategories(categories.map(c => c.id === cat.id ? cat : c))
      setManageCategory(null)
    } catch (e) { console.error('[rename category]', e) }
  }

  const handleDeleteCategory = async () => {
    if (!manageCategory) return
    // 双保险：即使按钮被绕过（键盘等），系统分类也不允许删除
    if (manageCategory.id === 'cat_text' || manageCategory.id === 'cat_voice') {
      window.alert(lang === 'zh' ? '系统分类不可删除' : 'System categories cannot be deleted')
      return
    }
    if (!window.confirm(lang === 'zh' ? '删除这个分区？分区内的频道会变成"未分类"。' : 'Delete this category? Channels inside become uncategorized.')) return
    try {
      const client = getClient()
      if (!client) return
      await client.deleteCategory(manageCategory.id)
      setCategories(categories.filter(c => c.id !== manageCategory.id))
      setChannelList(channelList.map(ch => ch.category_id === manageCategory.id ? { ...ch, category_id: null } : ch))
      setManageCategory(null)
    } catch (e) { console.error('[delete category]', e) }
  }
  const addMemberQueueRef = useRef<Promise<void>>(Promise.resolve())
  const messagesEndRef = useRef<HTMLDivElement>(null)

  const media = useMediaState()
  const mediaRef = useRef(media)
  useEffect(() => { mediaRef.current = media }, [media])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault()
        setSearchOpen(v => !v)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  const currentMessages = messages[currentChannel] || []
  const currentPending = pendingMessages.filter(p => p.channelId === currentChannel)
  const inVoice = media.connected && media.channelId === currentChannel

  useEffect(() => {
    let cancelled = false
    const loadChannels = async () => {
      const client = getClient()
      if (!client) { setTimeout(loadChannels, 200); return }
      try {
        const raw = await client.listChannels()
        const arr = typeof raw === 'string' ? JSON.parse(raw) : raw
        if (cancelled) return
        const mapped = (arr || []).map((c: any) => ({
          id: c.id,
          name: c.name,
          kind: c.kind,
          position: Number(c.position),
          is_private: !!c.is_private,
          category_id: c.category_id ?? null,
        }))
        setChannelList(mapped)
        try {
          const rawCats = await client.listCategories()
          const catArr = typeof rawCats === 'string' ? JSON.parse(rawCats) : rawCats
          if (Array.isArray(catArr)) setCategories(catArr)
        } catch (e) { console.warn('[categories] list failed (fallback to kind split):', e) }
      } catch (e) {
        console.error('[channels] list:', e)
      }
    }
    loadChannels()
    return () => { cancelled = true }
  }, [setChannelList])

  useEffect(() => {
    let cancelled = false
    const CH = currentChannel
    let attempt = 0
    const MAX_ATTEMPTS = 3
    const initChannel = async () => {
      if (cancelled) return
      const client = getClient()
      if (!client) { setTimeout(initChannel, 200); return }
      try {
        if (!(window as any).__kaede_join_locks__) (window as any).__kaede_join_locks__ = {}
        const locks = (window as any).__kaede_join_locks__
        if (!locks[CH]) {
          locks[CH] = client.join_channel(CH).finally(() => { delete locks[CH] })
        }
        await locks[CH]
        if (cancelled) return
        const hist = await client.load_history(CH, 50)
        if (cancelled) return
        const parsedHist = typeof hist === 'string' ? JSON.parse(hist) : hist
        const arr = Array.isArray(parsedHist) ? parsedHist : []
        const existing = useChatStore.getState().messages[CH] || []
        if (existing.length === 0 && arr.length > 0) {
          // 首次加载（或刷新）：直接覆盖
          setMessages(CH, arr)
        } else if (arr.length > 0) {
          // 断连重连场景：合并历史（按 ts 去重），补齐断连期间的消息
          // 之前只在 existing 为空时覆盖，导致断连用户重连后缺断连期间的消息
          const existingTs = new Set(existing.map((m: any) => String(m.ts)))
          const newMsgs = arr.filter((m: any) => !existingTs.has(String(m.ts)))
          if (newMsgs.length > 0) {
            console.log('[initChannel] merging ' + newMsgs.length + ' messages after reconnect')
            const merged = [...existing, ...newMsgs].sort((a: any, b: any) => Number(a.ts) - Number(b.ts))
            setMessages(CH, merged)
          }
        }
      } catch (e: any) {
        const msg = String(e)
        if (msg.includes('welcome channel closed')) {
          console.warn('[initChannel] concurrent join detected, ignoring...')
          return
        }
        console.error('[initChannel] join failed (attempt ' + attempt + '):', e)
        if (!cancelled && attempt < MAX_ATTEMPTS) {
          attempt++
          console.log('[initChannel] retry ' + attempt + '/' + MAX_ATTEMPTS + ' in 1.5s')
          setTimeout(initChannel, 1500)
        } else if (!cancelled) {
          console.error('[initChannel] giving up after ' + MAX_ATTEMPTS + ' attempts')
          const existing = useChatStore.getState().messages[CH] || []
          if (existing.length === 0) setMessages(CH, [])
        }
      }
    }
    initChannel()
    return () => { cancelled = true }
  }, [currentChannel, setMessages])

  useEffect(() => {
    let cancelled = false
    const tryRegister = () => {
      if (cancelled) return
      const client = getClient()
      if (!client) { setTimeout(tryRegister, 200); return }

      client.setOnEvent((rawEv: any) => {
        let ev = rawEv
        if (typeof rawEv === 'string') {
          try { ev = JSON.parse(rawEv) } catch { return }
        }
        if (ev.Disconnected) { scheduleReconnect(); return }


        const st = useChatStore.getState()
        const ch = st.currentChannel
        try { console.log('[EV]', Object.keys(ev).join(',')) } catch (_) {}

        if (ev.Joined) {
          const peers = ev.Joined.peers || []
          setServerMembers([loginId, ...peers])

          // 【断连重连修复】currentChannel 没变 → useEffect 不重跑 → initChannel 不跑
          // → 断连期间别人发的消息永远补不到。这里在每次 WS (re)join 成功后主动拉一次 history
          // 并按 ts 去重合并。500ms 给 MLS group 建好留时间（PQC Welcome 慢一点）。
                    const doMerge = (tag: string) => {
            const curCH = useChatStore.getState().currentChannel
            if (!curCH) return
            const c = getClient()
            if (!c) return
            c.load_history(curCH, 50).then((hist: any) => {
              const arr = typeof hist === 'string' ? JSON.parse(hist) : hist
              if (!Array.isArray(arr) || arr.length === 0) return
              const existing = useChatStore.getState().messages[curCH] || []
              const existingTs = new Set(existing.map((m: any) => String(m.ts)))
              const newMsgs = arr.filter((m: any) => !existingTs.has(String(m.ts)))
              if (newMsgs.length > 0) {
                console.log('[reconnect:' + tag + '] merging ' + newMsgs.length + ' messages')
                const merged = [...existing, ...newMsgs].sort((a: any, b: any) => Number(a.ts) - Number(b.ts))
                useChatStore.getState().setMessages(curCH, merged)
              }
            }).catch((e: any) => {
              console.warn('[reconnect:' + tag + '] deferred:', String(e).slice(0, 100))
            })
          }
          setTimeout(() => doMerge('t500'), 500)
          setTimeout(() => doMerge('t2000'), 2000)
          setTimeout(() => doMerge('t5000'), 5000)
          if (peers.length === 0) {
            try {
              const created = client.ensure_channel(ch)
              if (created) useChatStore.getState().setIsAuthority(true)
            } catch (e) { console.error('ensure_channel failed:', e) }
          }
        }
        if (ev.PendingWelcome) {
          // 主群 Welcome 自动接受：私有化服务器注册用户即可信任，不需弹窗。
          // 频道 Welcome 走 ChannelSync 0x04，不在此事件里；DM Welcome 走 DmWelcome。
          // 用 setTimeout(0) 异步：避免在 wasm event loop 的同步回调里重入。
          const sender = ev.PendingWelcome.sender_login_id || ev.PendingWelcome.from
          console.log(`[main-group] auto-accepting welcome from ${sender}`)
          setTimeout(() => {
            try {
              client.acceptPendingWelcome(sender)
              console.log(`[main-group] accepted welcome from ${sender}`)
            } catch (e: any) {
              console.warn(`[main-group] accept welcome failed:`, String(e).slice(0, 200))
            }
          }, 0)
          return
        }
        if (ev.ChannelMemberJoined) {
          const { channel_id, requester } = ev.ChannelMemberJoined
          if (channel_id !== useChatStore.getState().currentChannel) return
          if (!useChatStore.getState().isAuthority) return
          client.add_member_to_channel(useChatStore.getState().currentChannel, requester).then(() => {
            useChatStore.getState().setPeerStatus(`Added ${requester}`)
          }).catch((e: any) => {
            const msg = String(e)
            if (msg.includes('already in group') || msg.includes('already_member')) return
            useChatStore.getState().setPeerStatus(`Add failed: ${msg}`)
          })
        }
        if (ev.PeerJoined) {
          const uid = ev.PeerJoined.uid
          setServerMembers(Array.from(new Set([...useChatStore.getState().serverMembers, uid])))
          // 【主群】如果我是主群 host（第一个登录用户），把新 peer 加进主群。
          // 这是 UI 重做时丢失的 host 自动拉人逻辑。频道群的 Add 在下面 add_member_to_channel。
          if (client.isMlsHost && client.isMlsHost()) {
            useChatStore.getState().setPeerStatus(`Adding ${uid} to main group...`)
            client.hostAddPeer(uid).then(() => {
              console.log(`[main-group] added ${uid}`)
            }).catch((e: any) => {
              const msg = String(e)
              if (msg.includes("already in group") || msg.includes("already_member")) return
              console.warn(`[main-group] hostAddPeer ${uid} failed:`, msg)
            })
          }
          if (!useChatStore.getState().isAuthority) return
          useChatStore.getState().setPeerStatus(`Adding ${uid}...`)
          addMemberQueueRef.current = addMemberQueueRef.current.then(async () => {
            try {
              await client.add_member_to_channel(useChatStore.getState().currentChannel, uid)
              useChatStore.getState().setPeerStatus(`Added ${uid}`)
              await new Promise<void>(r => setTimeout(r, 150))
            } catch (e) {
              const msg = String(e)
              if (msg.includes('already in group') || msg.includes('already_member')) return
              useChatStore.getState().setPeerStatus(`Add failed: ${msg}`)
            }
          })
        }
        if (ev.ChannelSync) {
          const channelId = ev.ChannelSync.channel_id
          const payload = new Uint8Array(ev.ChannelSync.payload)
          const firstByte = payload.length > 0 ? payload[0] : -1
          console.log(`[ChannelSync] ch=${channelId} epoch=${ev.ChannelSync.epoch} len=${payload.length} first=0x${firstByte.toString(16).padStart(2,'0')}`)
          client.process_channel_sync(channelId, payload).then(async () => {
            useChatStore.getState().setPeerStatus('Synced')
            console.log(`[ChannelSync] ch=${channelId} OK`)
            const m = mediaRef.current
            if (m.connected && m.channelId === channelId) {
              try {
                const c = getClient()
                if (!c) return
                const tokenJson = await c.begin_media_session(channelId)
                const tokenData = typeof tokenJson === 'string' ? JSON.parse(tokenJson) : tokenJson
                if (tokenData.e2ee_seed) await updateE2EEKey(tokenData.e2ee_seed)
                console.log(`[E2EE] Key rotated for ${channelId} after ChannelSync`)
              } catch (e) { console.error('E2EE key rotate failed:', e) }
            }
          }).catch((e: any) => {
            console.error(`[ChannelSync] ch=${channelId} FAILED:`, String(e))
            useChatStore.getState().setPeerStatus(`Sync failed: ${e}`)
          })
        }
        if (ev.MlsCommit) {
          console.log(`[MlsCommit] ch=${ev.MlsCommit.channel_id} epoch=${ev.MlsCommit.epoch} len=${ev.MlsCommit.payload?.length}`)
        }
        if (ev.ChannelJoined) {
          const jchId = (ev.ChannelJoined as any).channel_id || (ev.ChannelJoined as any).id || useChatStore.getState().currentChannel
          if (jchId === useChatStore.getState().currentChannel) {
            (async () => {
              try {
                const c = getClient()
                if (!c) return
                // 稍等，确保 group 状态稳定
                await new Promise(r => setTimeout(r, 800))
                const hist = await c.load_history(jchId, 50)
                const arr = typeof hist === 'string' ? JSON.parse(hist) : hist
                const list = Array.isArray(arr) ? arr : []
                if (list.length > 0) {
                  useChatStore.getState().setMessages(jchId, list)
                  console.log('[ChannelJoined] reloaded history: ' + list.length + ' msgs')
                } else {
                  console.log('[ChannelJoined] history still empty (group might not be ready)')
                }
              } catch (e) { console.error('[ChannelJoined] history reload failed:', e) }
            })()
          }
        }
        if (ev.ChannelEpochChanged) {
          const chId = ev.ChannelEpochChanged.channel_id
          const newEpoch = ev.ChannelEpochChanged.epoch
          console.log(`[EpochChanged] ch=${chId} epoch=${newEpoch}`)
          const m = mediaRef.current
          if (m.connected && m.channelId === chId) {
            (async () => {
              try {
                const c = getClient()
                if (!c) return
                const tokenJson = await c.begin_media_session(chId)
                const tokenData = typeof tokenJson === 'string' ? JSON.parse(tokenJson) : tokenJson
                if (tokenData.e2ee_seed) {
                  await updateE2EEKey(tokenData.e2ee_seed)
                  console.log(`[E2EE] Key rotated for ${chId} after epoch=${newEpoch}`)
                }
              } catch (e) { console.error('E2EE key rotate failed:', e) }
            })()
          } else {
            console.log(`[EpochChanged] skip rotate: media connected=${m.connected} mediaCh=${m.channelId}`)
          }
        }
        if (ev.AttachmentProgress) {
          const { name, sent, total } = ev.AttachmentProgress
          const pendings = useChatStore.getState().pendingAttachments
          const target = pendings.find((a: any) => a.name === name && a.status === 'uploading')
          if (target) useChatStore.getState().updatePendingAttachment(target.id, { sent, total })
          return
        }
          if (ev.Typing) {
            const tch = (ev.Typing as any).channel_id
            const tfrom = (ev.Typing as any).from
            if (tch && tfrom) {
              setTyping(tch, tfrom, 3500)
            }
          }
        if (ev.ChannelMessage) {
          if (ev.ChannelMessage.channel_id !== useChatStore.getState().currentChannel) return
          try {
            const { text, attachments } = extractMessage(ev.ChannelMessage.plaintext)
            useChatStore.getState().addMessage(useChatStore.getState().currentChannel, {
              sender: ev.ChannelMessage.from || 'Unknown',
              plaintext: text,
              ts: ev.ChannelMessage.ts || Date.now(),
              attachments: ev.ChannelMessage.attachments || attachments,
            })
          } catch (err) { console.error('Parse message failed:', err) }
          return
        }
        if (ev.DmReceived) {
  if (ev.DmReceived.from === loginId) {
    // defense: server dm.rs B3 comment claims sender is excluded, but
    // probe6 (2026-10-27) confirmed server DOES echo back. Filter client-side.
    // See HANDOFF: DM self-echo bug in server state.send_to.
  } else {
    try {
                const { text, attachments } = extractMessage(ev.DmReceived.plaintext)
                useChatStore.getState().appendDmMessage(ev.DmReceived.conv_id, {
                  sender: ev.DmReceived.from || 'Unknown',
                  plaintext: text,
                  ts: ev.DmReceived.ts || Date.now(),
                  attachments: ev.DmReceived.attachments || attachments,
                })
              } catch (err) { console.error('Parse DM failed:', err) }
              return
  }
}
        if (ev.FriendsChanged) {
          const c = getClient()
          if (c) {
            c.listFriends().then((raw: any) => {
              const arr = typeof raw === 'string' ? JSON.parse(raw) : raw
              useChatStore.getState().setFriends(arr || [])
            }).catch((e: any) => console.error('[friends] refresh:', e))
          }
          return
        }
        if (ev.AppMessageReceived) {
          if (ev.AppMessageReceived.channel_id !== useChatStore.getState().currentChannel) return
          try {
            const { text, attachments } = extractMessage(ev.AppMessageReceived.payload)
            useChatStore.getState().addMessage(useChatStore.getState().currentChannel, {
              sender: ev.AppMessageReceived.from || 'Unknown',
              plaintext: text,
              ts: (ev.AppMessageReceived as any).ts || Date.now(),
              attachments: (ev.AppMessageReceived as any).attachments || attachments,
            })
          } catch (err) { console.error('Parse message failed:', err) }
        }
      })
    }
    tryRegister()
    return () => { cancelled = true }
  }, [])

  useEffect(() => {
    const timer = setInterval(async () => {
      const client = getClient()
      if (!client) return
      const failed = useChatStore.getState().pendingMessages.filter(
        p => p.status === 'failed' && p.channelId === currentChannel && p.retries < 5
      )
      for (const p of failed) {
        try {
          incrementRetry(p.id)
          await client.send_message(currentChannel, p.text)
          removePending(p.id)
          addMessage(currentChannel, { sender: loginId, plaintext: p.text, ts: p.ts })
        } catch (e) {
          markPendingFailed(p.id)
        }
      }
    }, 5000)
    return () => clearInterval(timer)
  }, [currentChannel, loginId, incrementRetry, removePending, addMessage, markPendingFailed])

  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' })
  }, [currentMessages.length, currentPending.length])

  // 1s 间隔清理过期的 typing 条目
  useEffect(() => {
    const iv = setInterval(() => pruneTyping(), 1000)
    return () => clearInterval(iv)
  }, [pruneTyping])

  // 键盘弹起处理：iOS/Android 软键盘不改变 100dvh，但 visualViewport.height 会缩小。
  // 监听 visualViewport 并同步到 CSS 变量 --app-height，主容器随之变矮，
  // 让底部输入框不会被键盘挡住。桌面浏览器 visualViewport 基本不变，无副作用。
  useEffect(() => {
    if (typeof window === 'undefined' || !window.visualViewport) return
    const vv = window.visualViewport
    const update = () => {
      document.documentElement.style.setProperty('--app-height', `${vv.height}px`)
    }
    update()
    vv.addEventListener('resize', update)
    vv.addEventListener('scroll', update)
    return () => {
      vv.removeEventListener('resize', update)
      vv.removeEventListener('scroll', update)
      document.documentElement.style.removeProperty('--app-height')
    }
  }, [])


  const friends = useChatStore(s => s.friends)
  const isFriend = (uid: string) => friends.some(f => f.login_id === uid && f.direction === 'friend')

  const handleMemberContextMenu = (e: React.MouseEvent, uid: string, inVoice: boolean) => {
    if (uid === loginId) return
    e.preventDefault()
    setCtxMenu({ x: e.clientX, y: e.clientY, uid, inVoice })
  }

  const handleCtxSendMessage = async () => {
    if (!ctxMenu) return
    const c = getClient()
    if (!c) return
    try {
      const raw = await c.openDm(ctxMenu.uid)
      const conv = typeof raw === 'string' ? JSON.parse(raw) : raw
      useChatStore.getState().addDmConversation(conv)
      useChatStore.getState().setActiveConv(conv.conv_id)
      setActiveView('dms')
    } catch (e) { console.error('[ctx dm]', e) }
  }

  const handleCtxAddFriend = async () => {
    if (!ctxMenu) return
    const c = getClient()
    if (!c) return
    try {
      await c.sendFriendRequest(ctxMenu.uid)
      const raw = await c.listFriends()
      const arr = typeof raw === 'string' ? JSON.parse(raw) : raw
      useChatStore.getState().setFriends(arr || [])
    } catch (e) { console.error('[ctx add friend]', e) }
  }

  const handleCtxRemoveFriend = async () => {
    if (!ctxMenu) return
    const c = getClient()
    if (!c) return
    try {
      await c.removeFriend(ctxMenu.uid)
      const raw = await c.listFriends()
      const arr = typeof raw === 'string' ? JSON.parse(raw) : raw
      useChatStore.getState().setFriends(arr || [])
    } catch (e) { console.error('[ctx remove]', e) }
  }

  const handleJoinMedia = async (targetCh: string) => {
    const client = getClient()
    if (!client) { console.error('[join-media] no client'); return }
    if (media.connected && media.channelId === targetCh) { console.log('[join-media] already in', targetCh); return }
    console.log(`[join-media] start ${targetCh}`)

    try {
      useChatStore.getState().setPeerStatus('Joining voice...')
      await client.join_channel(targetCh)
      console.log(`[join-media] join_channel OK ${targetCh}`)
      await new Promise(r => setTimeout(r, 500))
      const tokenJson = await client.begin_media_session(targetCh)
      const tokenData = typeof tokenJson === 'string' ? JSON.parse(tokenJson) : tokenJson
      console.log(`[join-media] media session OK, e2ee_seed=${tokenData.e2ee_seed ? 'yes' : 'no'}`)
      await connectMedia({
        wsUrl: tokenData.ws_url,
        token: tokenData.token,
        channelId: targetCh,
        e2eeKeyB64: tokenData.e2ee_seed,
        disableAdaptive: (window as any).__KAEDE_TEST_DISABLE_ADAPTIVE__ === true,
      })
      console.log(`[join-media] connectMedia OK ${targetCh}`)
      useChatStore.getState().setPeerStatus('Joined voice' + (tokenData.e2ee_seed ? ' (E2EE)' : ''))
    } catch (e: any) {
      console.error(`[join-media] FAILED ${targetCh}:`, String(e).slice(0, 400))
      useChatStore.getState().setPeerStatus('Join voice failed: ' + e)
    }
  }

  const handleLeaveMedia = async () => {
    const client = getClient()
    if (client && media.channelId) {
      try { await client.leave_media_room(media.channelId) } catch (e) { /* ignore */ }
    }
    await disconnectMedia()
    useChatStore.getState().setPeerStatus('')
  }

  const handleSend = async (text: string) => {
    const client = getClient()
    if (!client) return
    const pendingAtts = useChatStore.getState().pendingAttachments.filter(a => a.status !== 'uploaded')

    if (pendingAtts.length === 0) {
      if (!text) return
      const id = Date.now().toString() + Math.random().toString(36).slice(2)
      addPending({ id, channelId: currentChannel, text, ts: Date.now(), status: 'sending', retries: 0 })
      try {
        await client.send_message(currentChannel, text)
        removePending(id)
        addMessage(currentChannel, { sender: loginId, plaintext: text, ts: Date.now() })
      } catch (e) {
        console.error('send failed:', e)
        markPendingFailed(id)
      }
      return
    }

    const uploaded: any[] = []
    for (const p of pendingAtts) {
      useChatStore.getState().updatePendingAttachment(p.id, { status: 'uploading', sent: 0, total: p.size })
      try {
        const raw = await client.uploadAttachment(currentChannel, p.name, p.mime, p.bytes)
        const att = typeof raw === 'string' ? JSON.parse(raw) : raw
        // 服务端只看到密文，size 是密文大小（原文 + 29B 加密开销）。
        // 覆盖成原文大小；这个 att 会被 JSON.stringify 进 E2EE payload，
        // 所以 Alice 本地显示和 Bob 收到的都是真实原文大小。
        att.size = p.size
        if (!att.mime || att.mime === 'application/octet-stream') {
          att.mime = p.mime
        }
        uploaded.push(att)
        useChatStore.getState().updatePendingAttachment(p.id, { status: 'uploaded', attachment: att })
      } catch (err: any) {
        console.error('upload failed:', err)
        useChatStore.getState().updatePendingAttachment(p.id, { status: 'failed', error: String(err) })
        return
      }
    }
    try {
      await client.sendMessageWithAttachments(currentChannel, text, JSON.stringify(uploaded))
      addMessage(currentChannel, { sender: loginId, plaintext: text, ts: Date.now(), attachments: uploaded as any })
      useChatStore.getState().clearPendingAttachments()
    } catch (err: any) {
      console.error('send with attachments failed:', err)
    }
  }

  const getInitial = (str: string) => str ? str.charAt(0).toUpperCase() : '?'

  
  
    const currentChannelInfo = nodes.find((c: any) => c.id === currentChannel)
  const currentChannelName = currentChannelInfo?.name || currentChannel
  const isVoiceChannel = currentChannelInfo ? currentChannelInfo.kind === 'voice' : false

  return (
    <div className={`w-screen flex flex-col text-[var(--text-main)] overflow-hidden pt-[env(safe-area-inset-top)] font-sans ${dynamicBg ? 'bg-transparent' : 'bg-[var(--bg-app)]'}`} style={{ height: 'var(--app-height, 100dvh)' }}>

      <StageManagerServers />

      
      {/* 上半部分：适度留白包裹 */}
      <div className="flex-1 flex flex-col min-h-0 px-4 pt-4 pb-0 md:px-16 md:pt-12 md:pb-4 overflow-x-hidden">
        
        {/* 顶栏 */}
        <header className="relative z-0 h-[40px] shrink-0 flex items-center gap-4 mb-4 md:mb-10">
          <div className="flex shrink-0 items-center text-[16px] md:text-[22px] font-extrabold tracking-wide">KaedeNiwa</div>
          <div className="flex-1 min-w-0 flex justify-center px-2">
            <div className="relative flex items-center w-full max-w-[420px]">
              <IconSearch className="absolute left-3.5 w-4 h-4 text-[var(--text-muted)]" />
              <input
                type="text"
                readOnly
                onClick={() => setSearchOpen(true)}
                onFocus={() => setSearchOpen(true)}
                placeholder="Search...  (Ctrl+K)"
                className="w-full bg-[var(--bg-hover)] text-[var(--text-main)] text-[13px] rounded-md pl-10 pr-4 py-2 outline-none placeholder-[var(--text-muted)] cursor-pointer"
                data-testid="search-trigger"
              />
            </div>
          </div>
          <div className="flex items-center gap-2 shrink-0">
            <button
              onClick={() => setSidebarOpen(v => !v)}
              className={`flex w-8 h-8 items-center justify-center rounded-md transition-colors ${sidebarOpen ? 'text-[var(--text-main)] bg-[var(--bg-hover)]' : 'text-[var(--text-muted)] hover:text-[var(--text-main)] hover:bg-[var(--bg-hover)]'}`}
              title="Toggle sidebar"
              data-testid="sidebar-toggle-desktop"
            >
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="w-4 h-4">
                <path d="M4 6h16M4 12h16M4 18h16" strokeLinecap="round" />
              </svg>
            </button>
            <button
              onClick={() => setShowMembers(!showMembers)}
              className={`w-8 h-8 flex items-center justify-center rounded-md transition-colors ${showMembers ? 'text-[var(--text-main)] bg-[var(--bg-hover)]' : 'text-[var(--text-muted)] hover:text-[var(--text-main)] hover:bg-[var(--bg-hover)]'}`}
              title="Toggle Members"
            >
              <IconUsers className="w-4 h-4" />
            </button>
            <button data-testid="theme-toggle" aria-label="Toggle theme"
              onClick={() => setTheme(t => t === 'dark' ? 'light' : 'dark')}
              className="w-8 h-8 flex items-center justify-center rounded-md text-[var(--text-muted)] hover:text-[var(--text-main)] hover:bg-[var(--bg-hover)] transition-colors"
            >
              {theme === 'dark' ? <IconSun className="w-4 h-4" /> : <IconMoon className="w-4 h-4" />}
            </button>
          </div>
        </header>

        {/* 主体区 (左右侧边栏绝对定位，主聊天区绝对居中) */}
        <div className="flex-1 relative flex min-h-0">
          
          {/* 左侧侧边栏 */}
          {sidebarOpen && (
            <div 
              className="md:hidden fixed inset-0 bg-black/40 z-20 transition-opacity" 
              onClick={() => setSidebarOpen(false)} 
            />
          )}
          <aside className={`
            fixed top-8 bottom-8 left-4 w-[calc(100%-4rem)] max-w-[280px] z-30 rounded-2xl p-4
            bg-[var(--bg-app)] shadow-xl
            md:absolute md:inset-y-0 md:left-0 md:rounded-none
            md:w-[200px] md:bg-transparent md:shadow-none
            ${sidebarOpen ? 'translate-x-0' : '-translate-x-[calc(100%+6rem)]'}
          `}>
            <div className="h-[40px] flex items-center gap-3 mb-6">
              <div className="w-8 h-8 rounded-md bg-[var(--bg-hover)] flex items-center justify-center text-[var(--text-main)] font-bold text-sm shrink-0 overflow-hidden">
                {serverProfile?.icon_url ? (
                  <img src={serverProfile.icon_url} className="w-full h-full object-cover" />
                ) : (
                  'K'
                )}
              </div>
              <div className="flex flex-col min-w-0">
                <span className="text-[14px] font-semibold leading-tight truncate">{serverProfile?.name || "Kaede Server"}</span>
                <span className="text-[11px] text-[var(--text-muted)] truncate">{serverProfile?.description || "Workspace"}</span>
              </div>
            </div>

            <div className="flex-1 overflow-y-auto pb-2">
                <ChannelTree
                  onContextMenu={(e, n) => setCtxNode({ x: e.clientX, y: e.clientY, node: n })}
                  onChannelSelect={() => { if (!isMd) setSidebarOpen(false) }}
                />
              </div>
            {media.connected && media.channelId && (
              <div className="shrink-0 w-full pb-[calc(env(safe-area-inset-bottom)+1rem)] mt-auto">
                <MediaBar channelId={media.channelId} compact={true} />
              </div>
            )}
          </aside>

          {/* 右侧成员列表：桌面常驻侧栏 / 手机抽屉 */}
          {showMembers && activeView === 'chat' && !isMd && (
            <div
              className="fixed inset-0 bg-black/40 z-20"
              onClick={() => setShowMembers(false)}
            />
          )}
          {activeView === 'chat' && (
            <aside
              className={`${
                isMd
                  ? 'absolute right-0 top-0 bottom-0 w-[200px] flex-col z-10 flex overflow-hidden'
                  : 'fixed right-4 top-8 bottom-8 w-[calc(100%-4rem)] max-w-[280px] p-4 flex-col z-30 flex bg-[var(--bg-app)] shadow-xl rounded-2xl overflow-hidden'
              } transition-transform duration-300 ease-in-out ${
                showMembers
                  ? 'translate-x-0'
                  : isMd
                    ? 'translate-x-[calc(100%+6rem)]'
                    : 'translate-x-[calc(100%+6rem)]'
              }`}
            >
              <div className="text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase mb-4 mt-0">
                Members
              </div>
              <div className="flex-1 overflow-y-auto pr-1 flex flex-col gap-3">
                {(serverMembers.length > 0 ? serverMembers : [loginId]).map(uid => (
                  <div
                    key={uid}
                    className="flex items-center gap-3 cursor-pointer rounded px-1 py-0.5 hover:bg-[var(--bg-hover)] transition-colors"
                    onContextMenu={(e) => handleMemberContextMenu(e, uid, inVoice)}
                    data-testid={`member-row-${uid}`}
                  >
                    <div className="relative">
                      <div className="w-7 h-7 rounded-full bg-[var(--bg-active)] flex items-center justify-center text-[11px] font-bold text-[var(--text-main)] overflow-hidden relative">
    <span>{getInitial(uid)}</span>
    <img
      src={`/api/avatars/${uid}?v=${avatarVer}`}
      className="absolute inset-0 w-full h-full object-cover z-10"
      onError={(e) => { e.currentTarget.style.display = 'none'; }}
    />
  </div>
                      </div>
                    <span className="text-[13px] font-medium text-[var(--text-main)] truncate">{uid}</span>
                  </div>
                ))}
              </div>
            </aside>
          )}

          {/* 主区域 (绝对居中，留出左右安全距离) */}
          <main className={`flex-1 flex flex-col min-w-0 overflow-hidden transition-all ${sidebarOpen ? 'md:pl-[220px]' : 'md:pl-6'} pl-0 ${(showMembers && activeView === 'chat') ? 'md:pr-[220px] pr-0' : 'pr-0 md:pr-6'}`}>
            {activeView === 'chat' ? (
              <div className="flex-1 flex flex-col w-full max-w-[900px] mx-auto min-h-0">

                {/* 超大标题区 */}
                <div className="shrink-0 flex items-end justify-between mb-8 mt-0 w-full">
                  <div>
                    <h1 className="text-4xl font-extrabold tracking-tight mb-3 text-[var(--text-main)]">
                      #{currentChannelName}
                    </h1>
                    <p className="text-[13px] text-[var(--text-muted)]">
                      Welcome to the beginning of the channel.
                    </p>
                  </div>
                  <div className="flex items-center gap-3 shrink-0">
        
        
                    {isVoiceChannel && (
                      media.connected && media.channelId === currentChannel ? (
                        <button
                          data-testid="media-leave"
                          onClick={handleLeaveMedia}
                          className="text-[13px] px-5 py-2 rounded-md bg-[var(--bg-hover)] hover:bg-[var(--bg-active)] transition-colors font-medium text-red-500 whitespace-nowrap"
                        >
                          Leave Voice
                        </button>
                      ) : (
                        <button
                          data-testid="media-join"
                          onClick={() => handleJoinMedia(currentChannel)}
                          className="text-[13px] px-5 py-2 rounded-md bg-[var(--text-main)] text-[var(--bg-app)] hover:opacity-90 transition-opacity font-semibold whitespace-nowrap"
                        >
                          Join Voice
                        </button>
                      )
                    )}
                  </div>
                </div>

                {media.connected && media.channelId && (
                  <div className="w-full flex justify-center px-4 mt-2 shrink-0 max-h-[30vh]">
                    <ScreenShareView />
                  </div>
                )}

                {/* 消息列表 */}
                <div className="flex-1 overflow-y-auto overflow-x-hidden mb-2 pr-2 w-full min-h-[100px]">
        
            
            
            
            <div className="">
                    {currentMessages.map((msg, idx) => (
                      <div key={(msg as any).id || `${msg.ts}-${msg.sender}-${idx}`} className="group flex gap-3 hover:bg-[var(--bg-hover)] -mx-3 px-3 py-2 rounded-md transition-colors">
                        <div className="w-9 h-9 rounded-full flex items-center justify-center text-[13px] font-medium shrink-0 bg-[var(--bg-active)] text-[var(--text-main)] mt-0.5 overflow-hidden relative">
    <span>{getInitial(msg.sender)}</span>
    <img
      src={`/api/avatars/${msg.sender}?v=${avatarVer}`}
      className="absolute inset-0 w-full h-full object-cover z-10"
      onError={(e) => { e.currentTarget.style.display = 'none'; }}
    />
  </div>
                        <div className="min-w-0 flex-1">
                          <div className="flex items-baseline gap-2">
                            <span className="font-semibold text-[14px] text-[var(--text-main)]">{msg.sender}</span>
                            <span className="text-[11px] text-[var(--text-muted)]">{new Date(msg.ts).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}</span>
                          </div>
                          <MessageText
                            text={(msg as any).plaintext ?? (msg as any).text ?? ''}
                            className="text-[14px] mt-0.5 text-[var(--text-main)]"
                          />
                          {Array.isArray((msg as any).attachments) && (msg as any).attachments.length > 0 && (
                            <MessageAttachments targetId={currentChannel} attachments={(msg as any).attachments} />
                          )}
                        </div>
                      </div>
                    ))}
                    {currentPending.map(p => (
                      <div key={p.id} data-testid="pending-msg" data-status={p.status} className="flex gap-3 opacity-50 -mx-3 px-3 py-2 rounded-md">
                        <div className="w-8 h-8 rounded-full bg-[var(--bg-active)] flex items-center justify-center text-[13px] font-bold text-[var(--text-main)] overflow-hidden relative">
    <span>{getInitial(loginId)}</span>
    <img
      src={`/api/avatars/${loginId}?v=${avatarVer}`}
      className="absolute inset-0 w-full h-full object-cover z-10"
      onError={(e) => { e.currentTarget.style.display = 'none'; }}
    />
  </div>
                        <div className="min-w-0 flex-1">
                          <div className="flex items-baseline gap-2">
                            <span className="font-semibold text-[14px] text-[var(--text-main)]">{loginId}</span>
                            <span className="text-[11px] text-[var(--text-muted)]">
                              {p.status === 'sending' ? 'Sending...' : `Failed (retry ${p.retries}/5)`}
                            </span>
                          </div>
                          <div className="text-[14px] mt-0.5 whitespace-pre-wrap text-[var(--text-main)]">{p.text}</div>
                        </div>
                      </div>
                    ))}
                    <div ref={messagesEndRef} className="h-4" />
                  </div>
                </div>

                <div data-testid="peer-status" className="text-[11px] text-[var(--text-muted)] h-5 w-full mt-2">{peerStatus}</div>

                {(() => {
                  const chTyping = typingUsers[currentChannel] || {}
                  const now = Date.now()
                  const active = Object.entries(chTyping)
                    .filter(([uid, exp]) => exp > now && uid !== loginId)
                    .map(([uid]) => uid)
                  if (active.length === 0) return null
                  const text = active.length === 1
                    ? active[0] + " 正在输入..."
                    : active.length === 2
                      ? active[0] + " 和 " + active[1] + " 正在输入..."
                      : active.slice(0, 2).join("、") + " 等 " + active.length + " 人正在输入..."
                  return (
                    <div data-testid="typing-indicator" className="text-[11px] text-[var(--text-muted)] italic h-5 w-full mb-1">
                      {text}
                    </div>
                  )
                })()}

                <div className="w-full pb-6 md:pb-4 pt-2 shrink-0">
                  <div className="w-full flex items-end gap-2 bg-[var(--bg-hover)] rounded-md px-3 py-2">
                    <RichTextInput
                      handleRef={richInputRef}
                      onSubmit={handleSend}
                      placeholder={`Message #${currentChannelName}`}
                      onTyping={() => {
                        const c = getClient()
                        if (c && currentChannel) {
                          (c as any).sendTyping(currentChannel).catch(() => {})
                        }
                      }}
                    />
                  </div>
                </div>
              </div>
            ) : activeView === 'dms' ? (
              <div className="flex-1 py-4">
                <DMView />
              </div>
            ) : activeView === 'friends' ? (
              <div className="flex-1 py-4">
                <FriendsView />
              </div>
            ) : activeView === 'settings' ? (
              <div className="flex-1 py-4 overflow-y-auto">
                <SettingsView />
              </div>
            ) : (
              <div className="flex-1 py-4 overflow-y-auto">
                <MoreView />
              </div>
            )}
          </main>
        </div>
      </div>

      {/* 底部栏 */}
      <footer className="shrink-0 bg-[var(--bg-footer)] transition-colors pb-[env(safe-area-inset-bottom)]">
        <div className="px-4 h-[64px] md:h-[72px] md:px-16 flex items-center relative transition-colors">
          <div className="hidden md:flex w-[200px] shrink-0 items-center gap-3">
            <div className="w-7 h-7 rounded-md bg-[var(--bg-active)] flex items-center justify-center text-[11px] text-[var(--text-main)] font-bold overflow-hidden relative">
              <span>{getInitial(loginId)}</span>
              <img
                src={`/api/avatars/${loginId}?v=${avatarVer}`}
                className="absolute inset-0 w-full h-full object-cover z-10"
                onError={(e) => { e.currentTarget.style.display = 'none'; }}
              />
            </div>
            <div className="flex flex-col">
              <span className="text-[13px] font-semibold leading-tight text-[var(--text-main)] truncate max-w-[120px]">{loginId}</span>
              <button onClick={onLogout} className="text-[11px] text-[var(--text-muted)] hover:text-[var(--text-main)] text-left transition-colors mt-0.5">
                {t('settings.accountLogout')}
              </button>
            </div>
          </div>

          <div className="absolute left-1/2 -translate-x-1/2 flex items-center gap-1 md:gap-3 whitespace-nowrap">
            {(() => {
              const navCls = (active: boolean) =>
                `flex flex-col items-center justify-center gap-0.5 w-14 h-11 rounded-md transition-colors ${active
                  ? 'text-[var(--text-main)] bg-[var(--bg-hover)]'
                  : 'text-[var(--text-muted)] hover:text-[var(--text-main)] hover:bg-[var(--bg-hover)]'}`
              return (
                <>
                  <button onClick={() => setActiveView('chat')} data-testid="nav-chat" className={navCls(activeView === 'chat')} aria-label={t('nav.groups')}>
                    <IconHash className="w-5 h-5" />
                    <span className="text-[10px] font-medium leading-tight">{t('nav.groups')}</span>
                  </button>
                  <button onClick={() => setActiveView('dms')} data-testid="nav-dms" className={navCls(activeView === 'dms')} aria-label={t('nav.dms')}>
                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="w-5 h-5">
                      <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
                    </svg>
                    <span className="text-[10px] font-medium leading-tight">{t('nav.dms')}</span>
                  </button>
                  <button onClick={() => setActiveView('friends')} data-testid="nav-friends" className={navCls(activeView === 'friends')} aria-label={t('nav.friends')}>
                    <IconUsers className="w-5 h-5" />
                    <span className="text-[10px] font-medium leading-tight">{t('nav.friends')}</span>
                  </button>
                  <button onClick={() => setActiveView('settings')} data-testid="nav-settings" className={navCls(activeView === 'settings')} aria-label={t('nav.settings')}>
                    <IconSettings className="w-5 h-5" />
                    <span className="text-[10px] font-medium leading-tight">{t('nav.settings')}</span>
                  </button>
                  <button onClick={() => setActiveView('more')} data-testid="nav-more" className={navCls(activeView === 'more')} aria-label={t('nav.more')}>
                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="w-5 h-5">
                      <circle cx="12" cy="12" r="1" />
                      <circle cx="19" cy="12" r="1" />
                      <circle cx="5" cy="12" r="1" />
                    </svg>
                    <span className="text-[10px] font-medium leading-tight">{t('nav.more')}</span>
                  </button>
                </>
              )
            })()}
          </div>

          <div className="hidden md:block ml-auto w-[200px] text-right text-[11px] text-[var(--text-muted)] font-mono">
            v0.0.5
          </div>
        </div>
      </footer>

      {createDialogCategory !== undefined && (
        <CreateChannelDialog
          defaultCategoryId={createDialogCategory}
          onClose={() => setCreateDialogCategory(undefined)}
        />
      )}

      {newCategoryPrompt && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 px-4" onClick={() => setNewCategoryPrompt(false)}>
          <div className="w-[320px] max-w-full rounded-lg bg-[var(--bg-app)] border border-[var(--bg-active)] shadow-xl p-5" onClick={(e) => e.stopPropagation()}>
            <div className="text-[14px] font-bold text-[var(--text-main)] mb-3">{lang === 'zh' ? '新建分区' : 'New Category'}</div>
            <input
              autoFocus
              type="text"
              value={newCategoryName}
              onChange={(e) => setNewCategoryName(e.target.value)}
              onKeyDown={(e) => { if (e.key === 'Enter') handleCreateCategory(); if (e.key === 'Escape') setNewCategoryPrompt(false) }}
              placeholder={lang === 'zh' ? '分区名称' : 'Category name'}
              maxLength={64}
              className="w-full bg-[var(--bg-hover)] rounded px-3 py-2 text-[13px] text-[var(--text-main)] outline-none focus:ring-1 focus:ring-[var(--text-muted)] mb-4"
              data-testid="new-category-input"
            />
            <div className="flex justify-end gap-2">
              <button onClick={() => setNewCategoryPrompt(false)} className="px-3 py-1.5 rounded text-[12px] text-[var(--text-muted)] hover:text-[var(--text-main)]">{lang === 'zh' ? '取消' : 'Cancel'}</button>
              <button onClick={handleCreateCategory} className="px-4 py-1.5 rounded bg-[var(--bg-active)] text-[12px] font-semibold text-[var(--text-main)] hover:opacity-80" data-testid="new-category-confirm">{lang === 'zh' ? '创建' : 'Create'}</button>
            </div>
          </div>
        </div>
      )}

      {manageCategory && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 px-4" onClick={() => setManageCategory(null)}>
          <div className="w-[340px] max-w-full rounded-lg bg-[var(--bg-app)] border border-[var(--bg-active)] shadow-xl p-5" onClick={(e) => e.stopPropagation()}>
            <div className="text-[14px] font-bold text-[var(--text-main)] mb-3">{lang === 'zh' ? '管理分区' : 'Manage Category'}</div>
            <input
              autoFocus
              type="text"
              value={manageCategoryName}
              onChange={(e) => setManageCategoryName(e.target.value)}
              onKeyDown={(e) => { if (e.key === 'Enter') handleRenameCategory(); if (e.key === 'Escape') setManageCategory(null) }}
              placeholder={lang === 'zh' ? '分区名称' : 'Category name'}
              maxLength={64}
              className="w-full bg-[var(--bg-hover)] rounded px-3 py-2 text-[13px] text-[var(--text-main)] outline-none focus:ring-1 focus:ring-[var(--text-muted)] mb-4"
              data-testid="manage-category-input"
            />
            <div className="flex justify-between items-center">
              <button
                onClick={handleDeleteCategory}
                disabled={manageCategory.id === 'cat_text' || manageCategory.id === 'cat_voice'}
                title={(manageCategory.id === 'cat_text' || manageCategory.id === 'cat_voice') ? (lang === 'zh' ? '系统分类不可删除' : 'System category, cannot delete') : ''}
                className="px-3 py-1.5 rounded text-[12px] text-red-500 hover:bg-red-500/5 disabled:opacity-30 disabled:cursor-not-allowed disabled:hover:bg-transparent"
                data-testid="manage-category-delete"
              >{lang === 'zh' ? '删除分区' : 'Delete'}</button>
              <div className="flex gap-2">
                <button onClick={() => setManageCategory(null)} className="px-3 py-1.5 rounded text-[12px] text-[var(--text-muted)] hover:text-[var(--text-main)]">{lang === 'zh' ? '取消' : 'Cancel'}</button>
                <button onClick={handleRenameCategory} className="px-4 py-1.5 rounded bg-[var(--bg-active)] text-[12px] font-semibold text-[var(--text-main)] hover:opacity-80" data-testid="manage-category-save">{lang === 'zh' ? '保存' : 'Save'}</button>
              </div>
            </div>
          </div>
        </div>
      )}

      {settingsChannelId && (
        <ChannelSettingsDialog
          channelId={settingsChannelId}
          onClose={() => setSettingsChannelId(null)}
        />
      )}

      <SearchPalette
        open={searchOpen}
        onClose={() => setSearchOpen(false)}
        onSelect={(channelId) => {
          setChannel(channelId)
          setActiveView('chat')
          if (!isMd) setSidebarOpen(false)
        }}
      />

      {ctxMenu && (
        <UserContextMenu
          x={ctxMenu.x}
          y={ctxMenu.y}
          uid={ctxMenu.uid}
          isFriend={isFriend(ctxMenu.uid)}
          inVoice={ctxMenu.inVoice}
          onSendMessage={handleCtxSendMessage}
          onAddFriend={handleCtxAddFriend}
          onRemoveFriend={handleCtxRemoveFriend}
          onClose={() => setCtxMenu(null)}
        />
      )}
    
      {ctxNode && (
        <NodeContextMenu 
          x={ctxNode.x} 
          y={ctxNode.y} 
          node={ctxNode.node} 
          onClose={() => setCtxNode(null)} 
        />
      )}
    </div>
  )
}