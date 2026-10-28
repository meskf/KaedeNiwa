import { useState, useEffect, useRef } from 'react'
import { useAuthStore } from '../stores/auth'
import { useServerStore } from '../stores/server'
import { useChatStore } from '../stores/chat'
import { getClient, scheduleReconnect } from '../wasm/client'
import { connectMedia, disconnectMedia, updateE2EEKey } from '../lib/livekit'
import { useMediaState } from '../hooks/useLiveKitRoom'
import { MediaBar } from './MediaBar'
import { ScreenShareView } from './ScreenShareView'
import { StageManagerServers } from './StageManagerServers'
import { IconHash, IconVolume, IconSun, IconMoon, IconSearch, IconUsers, IconSettings } from './icons'
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
import { RichTextInput, type RichTextInputHandle } from './RichTextInput'
import { MessageText } from './MessageText'
import { extractMessage } from '../lib/messageUtils'



export function MainView({ onLogout }: { onLogout: () => void }) {
  const [dynamicBg, setDynamicBg] = useState(() => localStorage.getItem('kaede-dynamic-bg') === 'true')
  useEffect(() => {
    const handleToggle = () => setDynamicBg(localStorage.getItem('kaede-dynamic-bg') === 'true')
    window.addEventListener('kaede-dynamic-bg-changed', handleToggle)
    return () => window.removeEventListener('kaede-dynamic-bg-changed', handleToggle)
  }, [])

  
  const loginId = useAuthStore(s => s.loginId)
  const token = useAuthStore(s => s.token)
  const serverProfile = useServerStore(s => s.profile)
  const fetchServerProfile = useServerStore(s => s.fetch)

  useEffect(() => {
    fetchServerProfile(token)
  }, [token])
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
  } = useChatStore()
  const categories = useChatStore(s => s.categories)
  const setCategories = useChatStore(s => s.setCategories)
  const richInputRef = useRef<RichTextInputHandle | null>(null)
  const { theme, setTheme } = useTheme()
  const { t, lang } = useI18n()
  const [showMembers, setShowMembers] = useState(true)
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number; uid: string; inVoice: boolean } | null>(null)
  const [sidebarOpen, setSidebarOpen] = useState(true)

  const [searchOpen, setSearchOpen] = useState(false)
  const [createDialogCategory, setCreateDialogCategory] = useState<string | null | undefined>(undefined)
  const [settingsChannelId, setSettingsChannelId] = useState<string | null>(null)

  // A: category management state
  const [newCategoryPrompt, setNewCategoryPrompt] = useState(false)
  const [newCategoryName, setNewCategoryName] = useState('')
  const [manageCategory, setManageCategory] = useState<{ id: string; name: string } | null>(null)
  const [manageCategoryName, setManageCategoryName] = useState('')

  const handleManageCategory = (cat: { id: string; name: string }) => {
    setManageCategory(cat)
    setManageCategoryName(cat.name)
  }

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
    let attempt = 0
    const CH = currentChannel
    const initChannel = async () => {
      if (cancelled) return
      const client = getClient()
      if (!client) { setTimeout(initChannel, 200); return }
      try {
        await client.join_channel(CH)
        if (cancelled) return
        const hist = await client.load_history(CH, 50)
        if (cancelled) return
        const parsedHist = typeof hist === 'string' ? JSON.parse(hist) : hist
        const arr = Array.isArray(parsedHist) ? parsedHist : []
        const existing = useChatStore.getState().messages[CH] || []
        if (existing.length === 0 && arr.length > 0) {
          setMessages(CH, arr)
        } else if (arr.length === 0 && existing.length === 0 && attempt < 10) {
          attempt++
          setTimeout(initChannel, 1500)
        }
      } catch (e) {
        console.error('[initChannel] attempt ' + attempt + ' failed:', e)
        if (!cancelled && attempt < 10) {
          attempt++
          setTimeout(initChannel, 1500)
        } else if (!cancelled) {
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

        const eventDiv = document.createElement('div')
        eventDiv.setAttribute('data-testid', 'raw-event')
        eventDiv.innerText = 'EVENT: ' + JSON.stringify(ev)
        eventDiv.style.display = 'none'
        document.body.appendChild(eventDiv)
        const all = document.querySelectorAll('[data-testid="raw-event"]')
        if (all.length > 500) all[0].remove()

        const st = useChatStore.getState()
        const ch = st.currentChannel
        try { console.log('[EV]', Object.keys(ev).join(',')) } catch (_) {}

        if (ev.Joined) {
          const peers = ev.Joined.peers || []
          setServerMembers([loginId, ...peers])
          if (peers.length === 0) {
            try {
              const created = client.ensure_channel(ch)
              if (created) useChatStore.getState().setIsAuthority(true)
            } catch (e) { console.error('ensure_channel failed:', e) }
          }
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
    if (!client) return
    if (media.connected && media.channelId === targetCh) return // 避免重复加入

    try {
      useChatStore.getState().setPeerStatus('Joining voice...')
      await client.join_channel(targetCh)
      await new Promise(r => setTimeout(r, 500))
      const tokenJson = await client.begin_media_session(targetCh)
      const tokenData = typeof tokenJson === 'string' ? JSON.parse(tokenJson) : tokenJson
      await connectMedia({
        wsUrl: tokenData.ws_url,
        token: tokenData.token,
        channelId: targetCh,
        e2eeKeyB64: tokenData.e2ee_seed,
        disableAdaptive: (window as any).__KAEDE_TEST_DISABLE_ADAPTIVE__ === true,
      })
      useChatStore.getState().setPeerStatus('Joined voice' + (tokenData.e2ee_seed ? ' (E2EE)' : ''))
    } catch (e: any) {
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

  const renderTextChannel = (ch: any) => (
    <div
      key={ch.id}
      data-testid={`channel-item-${ch.id}`}
      onClick={() => { setChannel(ch.id); setActiveView('chat') }}
      className={`px-3 py-2 mb-1 rounded-md text-[14px] cursor-pointer flex items-center gap-2.5 transition-colors ${
        currentChannel === ch.id && activeView === 'chat'
          ? 'bg-[var(--bg-active)] text-[var(--text-main)] font-medium'
          : 'text-[var(--text-muted)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-main)]'
      }`}
    >
      <IconHash className="w-4 h-4 opacity-50" /> {ch.name}
    </div>
  )

  const renderVoiceChannel = (vc: any) => (
    <div
      key={vc.id}
      data-testid={`channel-item-${vc.id}`}
      onClick={() => { setChannel(vc.id); setActiveView('chat') }}
      onDoubleClick={(e) => { e.stopPropagation(); handleJoinMedia(vc.id) }}
      className={`px-3 py-2 mb-1 rounded-md text-[14px] cursor-pointer flex flex-col gap-2 transition-colors ${
        currentChannel === vc.id && activeView === 'chat'
          ? 'bg-[var(--bg-active)] text-[var(--text-main)] font-medium'
          : 'text-[var(--text-muted)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-main)]'
      }`}
      title="双击加入语音频道"
    >
      <div className="flex items-center gap-2.5">
        <IconVolume className="w-4 h-4 opacity-50" /> {vc.name}
        {media.connected && media.channelId === vc.id && media.audioBitrate > 0 && (
          <span data-testid={`voice-bitrate-${vc.id}`} className="ml-auto text-[10px] font-mono px-1.5 py-0.5 rounded bg-[var(--bg-active)] text-[var(--text-muted)]">
            {media.audioBitrate}K
          </span>
        )}
      </div>
      {/* 只要在这个频道里，不管当前看的是哪个频道，都显示成员列表 */}
      {media.connected && media.channelId === vc.id && (
        <div className="pl-6 flex flex-col gap-1 mt-1">
          {Object.values(media.participants || {}).map((p: any) => (
            <div
              key={p.identity}
              data-participant={p.identity}
              data-mic={p.isMicrophoneEnabled ? 'on' : 'off'}
              data-speaking={p.isSpeaking ? 'on' : 'off'}
              className="flex items-center gap-2 cursor-pointer hover:bg-[var(--bg-hover)] rounded px-1 -mx-1 py-0.5 transition-colors"
              onContextMenu={(e) => handleMemberContextMenu(e, p.identity, true)}
            >
              <div className="w-5 h-5 rounded-full bg-[var(--bg-active)] flex items-center justify-center text-[10px] font-bold text-[var(--text-main)]">
                {getInitial(p.identity)}
              </div>
              <span className="text-[12px] text-[var(--text-main)]">{p.identity}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  )

  const sortedCategories = [...categories].sort((a, b) => Number(a.position) - Number(b.position))
  const currentChannelInfo = channelList.find(c => c.id === currentChannel)
  const currentChannelName = currentChannelInfo?.name || currentChannel
  const isVoiceChannel = currentChannelInfo ? currentChannelInfo.kind === 'voice' : false

  return (
    <div className={`h-screen w-screen flex flex-col text-[var(--text-main)] overflow-hidden font-sans ${dynamicBg ? 'bg-transparent' : 'bg-[var(--bg-app)]'}`}>

      <StageManagerServers />

      
      {/* 上半部分：适度留白包裹 */}
      <div className="flex-1 flex flex-col min-h-0 px-16 pt-12 pb-4">
        
        {/* 顶栏 */}
        <header className="relative h-[40px] shrink-0 flex items-center gap-4 mb-10">
          <div className="flex shrink-0 items-center text-[16px] font-bold tracking-wide">
            {serverProfile.name}
          </div>
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
          <aside className={`absolute left-0 top-0 bottom-0 w-[200px] flex-col z-10 ${sidebarOpen ? 'flex' : 'hidden'}`}>
            <div className="h-[40px] flex items-center gap-3 mb-6">
              <div className="w-8 h-8 rounded-md bg-[var(--bg-hover)] flex items-center justify-center text-[var(--text-main)] font-bold text-sm shrink-0">
                K
              </div>
              <div className="flex flex-col min-w-0">
                <span className="text-[14px] font-semibold leading-tight truncate">{serverProfile.name}</span>
                <span className="text-[11px] text-[var(--text-muted)] truncate">{serverProfile.description}</span>
              </div>
            </div>

            <div className="flex-1 overflow-y-auto pb-2">
              {categories.length === 0 ? (
                <>
                  <div className="mb-2 text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase">
                    Text Channels
                  </div>
                  {channelList.filter(c => c.kind === 'text').map(renderTextChannel)}
                  <div className="mt-8 mb-2 text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase">
                    Voice Channels
                  </div>
                  {channelList.filter(c => c.kind === 'voice').map(renderVoiceChannel)}
                </>
              ) : (
                <>
                  {sortedCategories.map((cat: any) => {
                    const inCat = channelList.filter(c => c.category_id === cat.id)
                    return (
                      <div key={cat.id} className="mb-4 group/cat" data-testid={`category-${cat.id}`}>
                        <div className="mb-2 flex items-center justify-between text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase">
                          <span>{cat.name}</span>
                          <div className="flex items-center gap-1">
                            <button
                              onClick={() => handleManageCategory(cat)}
                              className="opacity-0 group-hover/cat:opacity-100 text-[12px] leading-none hover:text-[var(--text-main)] transition-opacity"
                              title={lang === 'zh' ? '管理分区' : 'Manage category'}
                              data-testid={`category-manage-${cat.id}`}
                            >⚙</button>
                            <button
                              onClick={() => setCreateDialogCategory(cat.id)}
                              className="opacity-0 group-hover/cat:opacity-100 text-[14px] leading-none hover:text-[var(--text-main)] transition-opacity"
                              title={t('channel.addChannel')}
                              data-testid={`category-add-${cat.id}`}
                            >+</button>
                          </div>
                        </div>
                        {inCat.map((ch: any) => (
                          <div key={ch.id} className="group/ch relative">
                            {ch.kind === 'voice' ? renderVoiceChannel(ch) : renderTextChannel(ch)}
                            <button
                              onClick={(e) => { e.stopPropagation(); setSettingsChannelId(ch.id) }}
                              className="absolute right-2 top-[7px] opacity-0 group-hover/ch:opacity-100 text-[var(--text-muted)] hover:text-[var(--text-main)] transition-all p-1 flex items-center justify-center rounded hover:bg-[var(--bg-active)]"
                              title={t('channel.openSettings')}
                              data-testid={`channel-settings-${ch.id}`}
                            >
                              <IconSettings className="w-3.5 h-3.5" />
                            </button>
                          </div>
                        ))}
                      </div>
                    )
                  })}
                  <div className="mb-4">
                    <button
                      onClick={() => setNewCategoryPrompt(true)}
                      className="w-full text-left px-3 py-1.5 rounded-md text-[12px] text-[var(--text-muted)] hover:text-[var(--text-main)] hover:bg-[var(--bg-hover)] transition-colors flex items-center gap-1.5"
                      data-testid="new-category-btn"
                    >
                      <span className="text-[14px] leading-none">+</span>
                      <span>{lang === 'zh' ? '新建分区' : 'New Category'}</span>
                    </button>
                  </div>
                  {(() => {
                    const uncat = channelList.filter(c => !c.category_id)
                    if (uncat.length === 0) return null
                    return (
                      <div className="mb-4 group/cat">
                        <div className="mb-2 flex items-center justify-between text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase">
                          <span>{t('channel.uncategorized')}</span>
                          <button
                            onClick={() => setCreateDialogCategory(null)}
                            className="opacity-0 group-hover/cat:opacity-100 text-[14px] leading-none hover:text-[var(--text-main)] transition-opacity"
                            title={t('channel.addChannel')}
                          >+</button>
                        </div>
                        {uncat.map((ch: any) => (
                          <div key={ch.id} className="group/ch relative">
                            {ch.kind === 'voice' ? renderVoiceChannel(ch) : renderTextChannel(ch)}
                            <button
                              onClick={(e) => { e.stopPropagation(); setSettingsChannelId(ch.id) }}
                              className="absolute right-2 top-2 opacity-0 group-hover/ch:opacity-100 text-[12px] text-[var(--text-muted)] hover:text-[var(--text-main)] transition-opacity"
                              title={t('channel.openSettings')}
                            >⚙</button>
                          </div>
                        ))}
                      </div>
                    )
                  })()}
                </>
              )}
            </div>
            {media.connected && media.channelId && (
              <div className="shrink-0 w-full pb-4 mt-auto">
                <MediaBar channelId={media.channelId} compact={true} />
              </div>
            )}
          </aside>

          {/* 右侧成员列表 */}
          {showMembers && activeView === 'chat' && (
            <aside className="absolute right-0 top-0 bottom-0 w-[200px] flex-col z-10 flex">
              <div className="text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase mb-4 mt-0">
                Members
              </div>
              <div className="flex flex-col gap-3">
                {(serverMembers.length > 0 ? serverMembers : [loginId]).map(uid => (
                  <div
                    key={uid}
                    className="flex items-center gap-3 cursor-pointer rounded px-1 py-0.5 hover:bg-[var(--bg-hover)] transition-colors"
                    onContextMenu={(e) => handleMemberContextMenu(e, uid, inVoice)}
                    data-testid={`member-row-${uid}`}
                  >
                    <div className="relative">
                      <div className="w-7 h-7 rounded-full bg-[var(--bg-active)] flex items-center justify-center text-[11px] font-bold text-[var(--text-main)]">
                        {getInitial(uid)}
                      </div>
                      </div>
                    <span className="text-[13px] font-medium text-[var(--text-main)] truncate">{uid}</span>
                  </div>
                ))}
              </div>
            </aside>
          )}

          {/* 主区域 (绝对居中，留出左右安全距离) */}
          <main className={`flex-1 flex flex-col min-w-0 overflow-hidden transition-all ${sidebarOpen ? 'pl-[220px]' : 'pl-6'} ${(showMembers && activeView === 'chat') ? 'pr-[220px]' : 'pr-6'}`}>
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
                        <div className="w-9 h-9 rounded-full flex items-center justify-center text-[13px] font-medium shrink-0 bg-[var(--bg-active)] text-[var(--text-main)] mt-0.5">
                          {getInitial(msg.sender)}
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
                        <div className="w-9 h-9 rounded-full flex items-center justify-center text-[13px] font-medium shrink-0 bg-[var(--bg-active)] text-[var(--text-main)] mt-0.5">
                          {getInitial(loginId)}
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

                <div className="w-full pb-4 pt-2 shrink-0">
                  <div className="w-full flex items-end gap-2 bg-[var(--bg-hover)] rounded-md px-3 py-2">
                    <RichTextInput
                      handleRef={richInputRef}
                      onSubmit={handleSend}
                      placeholder={`Message #${currentChannelName}`}
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
      <footer className="shrink-0 bg-[var(--bg-footer)] transition-colors">
        <div className="px-16 h-[72px] flex items-center relative transition-colors">
          <div className="w-[200px] shrink-0 flex items-center gap-3">
            <div className="w-7 h-7 rounded-md bg-[var(--bg-active)] flex items-center justify-center text-[11px] text-[var(--text-main)] font-bold">
              {getInitial(loginId)}
            </div>
            <div className="flex flex-col">
              <span className="text-[13px] font-semibold leading-tight text-[var(--text-main)] truncate max-w-[120px]">{loginId}</span>
              <button onClick={onLogout} className="text-[11px] text-[var(--text-muted)] hover:text-[var(--text-main)] text-left transition-colors mt-0.5">
                {t('settings.accountLogout')}
              </button>
            </div>
          </div>

          <div className="absolute left-1/2 -translate-x-1/2 flex items-center gap-8 text-[13px] font-medium text-[var(--text-muted)]">
            <span onClick={() => setActiveView('chat')} data-testid="nav-chat" className={`cursor-pointer transition-colors hover:text-[var(--text-main)] ${activeView === 'chat' ? 'text-[var(--text-main)]' : ''}`}>{t('nav.groups')}</span>
            <span onClick={() => setActiveView('dms')} data-testid="nav-dms" className={`cursor-pointer transition-colors hover:text-[var(--text-main)] ${activeView === 'dms' ? 'text-[var(--text-main)]' : ''}`}>{t('nav.dms')}</span>
            <span onClick={() => setActiveView('friends')} data-testid="nav-friends" className={`cursor-pointer transition-colors hover:text-[var(--text-main)] ${activeView === 'friends' ? 'text-[var(--text-main)]' : ''}`}>{t('nav.friends')}</span>
            <span onClick={() => setActiveView('settings')} data-testid="nav-settings" className={`cursor-pointer transition-colors hover:text-[var(--text-main)] ${activeView === 'settings' ? 'text-[var(--text-main)]' : ''}`}>{t('nav.settings')}</span>
            <span onClick={() => setActiveView('more')} data-testid="nav-more" className={`cursor-pointer transition-colors hover:text-[var(--text-main)] ${activeView === 'more' ? 'text-[var(--text-main)]' : ''}`}>{t('nav.more')}</span>
          </div>

          <div className="ml-auto w-[200px] text-right text-[11px] text-[var(--text-muted)] font-mono">
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
              <button onClick={handleDeleteCategory} className="px-3 py-1.5 rounded text-[12px] text-red-500 hover:bg-red-500/5" data-testid="manage-category-delete">{lang === 'zh' ? '删除分区' : 'Delete'}</button>
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
    </div>
  )
}