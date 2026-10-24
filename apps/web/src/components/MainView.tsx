import { useState, useEffect, useRef } from 'react'
import { useAuthStore } from '../stores/auth'
import { useChatStore } from '../stores/chat'
import { getClient } from '../wasm/client'
import { connectMedia, disconnectMedia } from '../lib/livekit'
import { useMediaState } from '../hooks/useLiveKitRoom'
import { MediaBar } from './MediaBar'
import { ScreenShareView } from './ScreenShareView'

export function MainView() {
  const logout = useAuthStore(s => s.logout)
  const loginId = useAuthStore(s => s.loginId)
  const { currentChannel, setChannel } = useChatStore()
  const [messages, setMessages] = useState<any[]>([])
  const [input, setInput] = useState('')
  const [peerStatus, setPeerStatus] = useState('')
  const isChannelAuthorityRef = useRef(false)
  // 串行化拉人：确保同一时刻只有一个 add_member_to_channel 执行
  // 避免 9 个 PeerJoined 并发触发时往 WS 队列里塞 18+ 条消息
  const addMemberQueueRef = useRef<Promise<void>>(Promise.resolve())
  const messagesEndRef = useRef<HTMLDivElement>(null)
  const media = useMediaState()

  // 加入语音频道
  const handleJoinMedia = async () => {
    const client = getClient()
    if (!client) return
    try {
      setPeerStatus('正在加入媒体房间...')
      const tokenJson = await client.begin_media_session(currentChannel)
      const tokenData = typeof tokenJson === 'string' ? JSON.parse(tokenJson) : tokenJson
      await connectMedia({
        wsUrl: tokenData.ws_url,
        token: tokenData.token,
        channelId: currentChannel,
        e2eeKeyB64: tokenData.e2ee_seed,
        disableAdaptive: (window as any).__KAEDE_TEST_DISABLE_ADAPTIVE__ === true,
      })
      setPeerStatus('已加入媒体房间' + (tokenData.e2ee_seed ? ' (E2EE)' : ' (明文)'))
    } catch (e: any) {
      setPeerStatus('加入媒体失败: ' + e)
    }
  }

  // 离开语音频道
  const handleLeaveMedia = async () => {
    const client = getClient()
    if (client) {
      try { await client.leave_media_room(currentChannel) } catch (e) { /* ignore */ }
    }
    await disconnectMedia()
    setPeerStatus('')
  }

  // 切换频道时：确保群组存在并拉取历史
  useEffect(() => {
    const initChannel = async () => {
      const client = getClient()
      if (!client) {
        // [MainView] client 未就绪，App 应该已完成 login；等一次微任务再试
        setTimeout(() => initChannel(), 200)
        return
      }
      {
        try {
          // 先确保自己是频道成员（DB 层）
          await client.join_channel(currentChannel)

          // 不在这里 ensure_channel！由 Joined 事件决定（peers 空才建）
          
          const hist = await client.load_history(currentChannel, 50)
          const parsedHist = typeof hist === 'string' ? JSON.parse(hist) : hist
          setMessages(Array.isArray(parsedHist) ? parsedHist : [])
        } catch (e) {
          console.error('初始化频道失败:', e)
          setMessages([])
        }
      }
    }
    initChannel()
  }, [currentChannel])

  // 监听事件
  useEffect(() => {
    const client = getClient()
    if (client) {
      client.setOnEvent((rawEv: any) => {
        let ev = rawEv
        if (typeof rawEv === 'string') {
          try { ev = JSON.parse(rawEv) } catch (e) { return }
        }

        // 调试事件
        const eventDiv = document.createElement('div')
        eventDiv.className = 'raw-event'
        eventDiv.innerText = 'EVENT: ' + JSON.stringify(ev)
        eventDiv.style.display = 'none'
        document.body.appendChild(eventDiv)

        // 【关键】Joined: peers 为空 → 第一个成员 → 建 group 当 authority
        if (ev.Joined) {
          const peers = ev.Joined.peers || []
          console.log('[MainView] Joined 事件: peers=' + JSON.stringify(peers) + ' 我自己=' + loginId)
          if (peers.length === 0) {
            console.log('[MainView] Joined: peers 空，本端为第一个成员，建 channel group')
            try {
              const created = client.ensure_channel(currentChannel)
              if (created) {
                isChannelAuthorityRef.current = true
                console.log('[MainView] 已建 group，本端为 authority')
              }
            } catch (e: any) {
              console.error('[MainView] ensure_channel 失败:', e)
            }
          } else {
            console.log('[MainView] Joined: peers=' + peers.length + '，等 Welcome')
          }
        }

        // 【关键】ChannelMemberJoined: 服务端通知 authority 有新成员申请入群
        if (ev.ChannelMemberJoined) {
          const { channel_id, requester } = ev.ChannelMemberJoined
          console.log('[MainView] ChannelMemberJoined: ch=' + channel_id + ' requester=' + requester + ' isAuth=' + isChannelAuthorityRef.current)
          if (channel_id !== currentChannel) return
          if (!isChannelAuthorityRef.current) {
            console.log('[MainView] ChannelMemberJoined 但不是 authority，忽略')
            return
          }
          console.log('[MainView] authority 拉人:', requester)
          client.add_member_to_channel(currentChannel, requester).then(() => {
            setPeerStatus(`已把 ${requester} 拉入 #${currentChannel}`)
          }).catch((e: any) => {
            setPeerStatus(`拉人失败: ${e}`)
          })
        }

        if (ev.PeerJoined) {
          const uid = ev.PeerJoined.uid
          // 【关键】只有 authority 才拉人。
          // 非 authority 拉人会导致本地 group 前进但服务端拒绝 Commit → group 分叉。
          if (!isChannelAuthorityRef.current) {
            console.log('[MainView] 收到 PeerJoined 但不是 authority，忽略:', uid)
            return
          }
          console.log('[MainView] authority 加入拉人队列:', uid)
          // 串行化：每次只处理一个，防止并发塞爆 WS 队列
          addMemberQueueRef.current = addMemberQueueRef.current
            .then(() => {
              console.log('[MainView] 开始拉人:', uid)
              setPeerStatus(`正在把 ${uid} 拉入 #${currentChannel}...`)
              return client.add_member_to_channel(currentChannel, uid)
            })
            .then(() => {
              console.log('[MainView] 拉人成功:', uid)
              setPeerStatus(`已把 ${uid} 拉入 #${currentChannel}`)
              // 给 WS 队列一点喘息时间
              return new Promise<void>(r => setTimeout(r, 150))
            })
            .catch((e: any) => {
              console.error('[MainView] 拉人失败:', uid, e)
              setPeerStatus(`拉人失败: ${e}`)
            })
        }

        // 【核心修复】收到频道同步包（Welcome / Commit），处理入群或更新 epoch
        if (ev.ChannelSync) {
          const channelId = ev.ChannelSync.channel_id
          const payload = new Uint8Array(ev.ChannelSync.payload)
          client.process_channel_sync(channelId, payload).then(() => {
            setPeerStatus(`成功同步频道 ${channelId} 的群组状态`)
          }).catch((e: any) => {
            setPeerStatus(`同步频道失败: ${e}`)
          })
        }

        // E2EE key 轮换：MLS epoch 变化时刷新媒体密钥
        if (ev.MlsEpochChanged && media.connected && media.channelId === currentChannel) {
          (async () => {
            try {
              const client = getClient()
              if (!client) return
              const tokenJson = await client.begin_media_session(currentChannel)
              const tokenData = typeof tokenJson === 'string' ? JSON.parse(tokenJson) : tokenJson
              if (tokenData.e2ee_seed) {
                const { updateE2EEKey } = await import('../lib/livekit')
                await updateE2EEKey(tokenData.e2ee_seed)
              }
            } catch (e) {
              console.error('E2EE key 轮换失败:', e)
            }
          })()
        }

        // 【频道隔离核心】优先处理频道级消息（0x03 前缀，独立密钥加密）
        if (ev.ChannelMessage) {
          if (ev.ChannelMessage.channel_id !== currentChannel) {
            return
          }
          try {
            // ChannelMessage.plaintext 是服务端解密后的 bytes
            const plaintext = ev.ChannelMessage.plaintext
            let text = ''
            if (Array.isArray(plaintext)) {
              text = new TextDecoder().decode(new Uint8Array(plaintext))
            } else if (typeof plaintext === 'string') {
              text = plaintext
            }
            setMessages(prev => [...prev, {
              sender: ev.ChannelMessage.from || 'Unknown',
              plaintext: text,
              ts: ev.ChannelMessage.ts || Date.now()
            }])
          } catch (err) {
            console.error('解析频道消息失败:', err)
          }
          return
        }

        // 兼容旧版主群 MLS 消息
        if (ev.AppMessageReceived) {
          if (ev.AppMessageReceived.channel_id !== currentChannel) {
            return
          }
          try {
            const text = new TextDecoder().decode(new Uint8Array(ev.AppMessageReceived.payload))
            setMessages(prev => [...prev, {
              sender: ev.AppMessageReceived.from || 'Unknown',
              plaintext: text,
              ts: Date.now()
            }])
          } catch (err) {
            console.error('解析消息失败:', err)
          }
        }
      })
    }
  }, [currentChannel])

  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' })
  }, [messages])

  const handleSend = async () => {
    if (!input.trim()) return
    const text = input.trim()
    setInput('')
    
    setMessages(prev => [...prev, {
      sender: loginId || 'Me',
      plaintext: text,
      ts: Date.now()
    }])
    
    const client = getClient()
    if (client) {
      try {
        await client.send_message(currentChannel, text)
      } catch (e) {
        console.error('发送失败:', e)
      }
    }
  }

  const getInitial = (str: any) => {
    if (!str || typeof str !== 'string' || str.length === 0) return '?'
    return str.charAt(0).toUpperCase()
  }

  return (
    <div className="flex h-full w-full min-h-0">
      <div className="w-[72px] bg-discord-serverRail flex flex-col items-center py-3 gap-2 shrink-0">
        <div className="w-12 h-12 bg-indigo-500 rounded-2xl hover:rounded-xl transition-all flex items-center justify-center text-white font-bold cursor-pointer">
          K
        </div>
        <div className="w-8 h-[2px] bg-white/20 rounded-full mt-1"></div>
        <div className="mt-auto flex flex-col items-center gap-2">
          <div className="w-10 h-10 rounded-full bg-[#313338] border-2 border-green-500 flex items-center justify-center text-white font-bold" title={loginId || 'User'}>
            {getInitial(loginId)}
          </div>
          <button onClick={logout} className="w-10 h-10 bg-discord-sidebar rounded-xl hover:bg-red-500 transition-all flex items-center justify-center text-xs text-white">
            退
          </button>
        </div>
      </div>
      
      <div className="w-[240px] bg-discord-sidebar flex flex-col shrink-0">
        <div className="h-12 px-4 flex items-center shadow-sm font-bold text-white border-b border-[#1f2023] shrink-0">
          Kaede Server
        </div>
        <div className="flex-1 p-2 overflow-y-auto">
          <div className="text-xs font-semibold text-gray-400 uppercase py-2 px-2 hover:text-gray-300">文字频道</div>
          {['general', 'dev', 'random'].map(ch => (
            <div 
              key={ch}
              onClick={() => setChannel(ch)}
              className={`px-2 py-1.5 rounded cursor-pointer mb-0.5 ${currentChannel === ch ? 'bg-[#404249] text-white' : 'text-discord-channel hover:bg-[#35373c] hover:text-white'}`}
            >
              # {ch}
            </div>
          ))}
        </div>
      </div>

      <div className="flex-1 flex flex-col min-w-0 bg-discord-bg">
        <div className="h-12 flex items-center px-4 shadow-sm border-b border-[#1f2023] text-white font-semibold shrink-0">
          # {currentChannel}
          <div className="flex-1" />
          {media.connected && media.channelId === currentChannel ? (
            <button
              onClick={handleLeaveMedia}
              className="text-xs px-2 py-1 bg-red-500 hover:bg-red-600 text-white rounded"
            >
              离开语音
            </button>
          ) : (
            <button
              onClick={handleJoinMedia}
              className="text-xs px-2 py-1 bg-green-500 hover:bg-green-600 text-white rounded"
              title="加入语音频道"
            >
              加入语音
            </button>
          )}
        </div>
        <MediaBar channelId={currentChannel} />
        <ScreenShareView />
        
        <div className="flex-1 p-4 overflow-y-auto space-y-4 flex flex-col">
           {messages.length === 0 ? (
             <div className="text-gray-400 text-sm m-auto text-center">欢迎来到 #{currentChannel}，这里是消息的起点。</div>
           ) : (
             messages.map((msg, i) => (
               <div key={i} className="flex gap-4 hover:bg-[#2e3035] -mx-4 px-4 py-1">
                 <div className="w-10 h-10 rounded-full bg-indigo-500 flex items-center justify-center text-white font-bold shrink-0">
                   {getInitial(msg.sender)}
                 </div>
                 <div>
                   <div className="flex items-baseline gap-2">
                     <span className="text-white font-medium text-sm">{msg.sender || 'Unknown'}</span>
                     <span className="text-xs text-gray-400">{new Date(msg.ts).toLocaleTimeString()}</span>
                   </div>
                   <div className="text-gray-200 text-sm mt-0.5 whitespace-pre-wrap">{msg.plaintext || ''}</div>
                 </div>
               </div>
             ))
           )}
           <div ref={messagesEndRef} />
        </div>

        <div className="px-4 text-xs text-yellow-400">{peerStatus}</div>
        <div className="p-4 shrink-0">
          <div className="bg-[#383a40] rounded-lg flex items-center px-4 py-2">
            <input 
              type="text" 
              value={input}
              onChange={e => setInput(e.target.value)}
              onKeyDown={e => e.key === 'Enter' && handleSend()}
              placeholder={`Message #${currentChannel}`} 
              className="bg-transparent border-none outline-none flex-1 text-white placeholder-gray-400" 
            />
          </div>
        </div>
      </div>
    </div>
  )
}



