const fs = require('fs')

// ============ UserContextMenu.tsx (new) ============
const menu = `import { useEffect, useRef } from 'react'
import { useI18n } from '../stores/i18n'

interface Props {
  x: number
  y: number
  uid: string
  isFriend: boolean
  inVoice: boolean
  onSendMessage?: () => void
  onAddFriend?: () => void
  onRemoveFriend?: () => void
  onClose: () => void
}

export function UserContextMenu({ x, y, uid, isFriend, inVoice, onSendMessage, onAddFriend, onRemoveFriend, onClose }: Props) {
  const { t } = useI18n()
  const ref = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const onDoc = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose()
    }
    const onEsc = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose() }
    document.addEventListener('mousedown', onDoc)
    document.addEventListener('keydown', onEsc)
    return () => {
      document.removeEventListener('mousedown', onDoc)
      document.removeEventListener('keydown', onEsc)
    }
  }, [onClose])

  return (
    <div
      ref={ref}
      data-testid="user-context-menu"
      className="fixed z-50 w-[200px] rounded-lg bg-[var(--bg-app)] border border-[var(--bg-active)] shadow-xl py-1 text-[var(--text-main)]"
      style={{ left: x, top: y }}
    >
      <div className="px-3 py-1.5 text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase border-b border-[var(--bg-active)] mb-1">
        {uid}
      </div>
      {isFriend && onSendMessage && (
        <button
          data-testid="ctx-send-message"
          onClick={() => { onSendMessage(); onClose() }}
          className="w-full text-left px-3 py-1.5 text-[13px] hover:bg-[var(--bg-hover)] transition-colors"
        >
          {t('menu.sendMessage')}
        </button>
      )}
      {!isFriend && onAddFriend && (
        <button
          data-testid="ctx-add-friend"
          onClick={() => { onAddFriend(); onClose() }}
          className="w-full text-left px-3 py-1.5 text-[13px] text-green-500 hover:bg-[var(--bg-hover)] transition-colors"
        >
          {t('menu.addFriend')}
        </button>
      )}
      {isFriend && onRemoveFriend && (
        <button
          data-testid="ctx-remove-friend"
          onClick={() => { onRemoveFriend(); onClose() }}
          className="w-full text-left px-3 py-1.5 text-[13px] text-red-500 hover:bg-[var(--bg-hover)] transition-colors"
        >
          {t('friends.actionRemove')}
        </button>
      )}
      {inVoice && (
        <div className="border-t border-[var(--bg-active)] mt-1 pt-1 px-3 py-1 text-[11px] text-[var(--text-muted)] italic">
          {t('menu.userVolume')} — TODO
        </div>
      )}
    </div>
  )
}
`
fs.writeFileSync('src/components/UserContextMenu.tsx', menu)
console.log('WROTE UserContextMenu.tsx')

// ============ MainView.tsx ============
let mv = fs.readFileSync('src/components/MainView.tsx', 'utf8')

// imports
mv = mv.replace(
  `import { MoreView } from './MoreView'`,
  `import { MoreView } from './MoreView'
import { UserContextMenu } from './UserContextMenu'`
)

// state
const oldState = `  const [showMembers, setShowMembers] = useState(true)`
const newState = `  const [showMembers, setShowMembers] = useState(true)
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number; uid: string; inVoice: boolean } | null>(null)
  const [sidebarOpen, setSidebarOpen] = useState(false)`
if (!mv.includes(oldState)) { console.error('FAIL: MainView state'); process.exit(1) }
mv = mv.replace(oldState, newState)

// handlers (add after handleLeaveMedia) -- anchor: '  const handleLeaveMedia = async () => {' 到其后的第一个 '  }' 结束
// 简化：插在 '  const media = useMediaState()' 后面不行，因为需要 setChannel 等已有。
// 用 marker: '  const handleLeaveMedia = async () => {' 前面插入新 helper
const helperAnchor = `  const handleJoinMedia = async () => {`
const helpers = `  const friends = useChatStore(s => s.friends)
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

  const handleJoinMedia = async () => {`
if (!mv.includes(helperAnchor)) { console.error('FAIL: helper anchor'); process.exit(1) }
mv = mv.replace(helperAnchor, helpers)

// === 左侧 sidebar: <768 hidden, overlay drawer ===
const oldAside = `          <aside className="absolute left-0 top-0 bottom-0 w-[200px] flex flex-col z-10">`
const newAside = `          <aside className={\`absolute left-0 top-0 bottom-0 w-[200px] sm:w-[180px] lg:w-[200px] flex-col z-30 bg-[var(--bg-app)] transition-transform \${sidebarOpen ? 'flex translate-x-0' : 'hidden md:flex'} \`}>`
if (!mv.includes(oldAside)) { console.error('FAIL: aside'); process.exit(1) }
mv = mv.replace(oldAside, newAside)

// === 右侧成员: <md 隐藏 (已有), 内部成员项加右键 ===
const oldMemberRow = `                  <div key={uid} className="flex items-center gap-3">
                    <div className="relative">
                      <div className="w-7 h-7 rounded-full bg-[var(--bg-active)] flex items-center justify-center text-[11px] font-bold text-[var(--text-main)]">
                        {getInitial(uid)}
                      </div>
                      </div>
                    <span className="text-[13px] font-medium text-[var(--text-main)]">{uid}</span>
                  </div>`
const newMemberRow = `                  <div
                    key={uid}
                    className="flex items-center gap-3 cursor-pointer rounded px-1 py-0.5 hover:bg-[var(--bg-hover)] transition-colors"
                    onContextMenu={(e) => handleMemberContextMenu(e, uid, inVoice)}
                    data-testid={\`member-row-\${uid}\`}
                  >
                    <div className="relative">
                      <div className="w-7 h-7 rounded-full bg-[var(--bg-active)] flex items-center justify-center text-[11px] font-bold text-[var(--text-main)]">
                        {getInitial(uid)}
                      </div>
                      </div>
                    <span className="text-[13px] font-medium text-[var(--text-main)] truncate">{uid}</span>
                  </div>`
if (!mv.includes(oldMemberRow)) { console.error('FAIL: member row'); process.exit(1) }
mv = mv.replace(oldMemberRow, newMemberRow)

// === 语音频道参与者项加右键 + 音质 badge ===
const oldParticipant = `                        <div
                          key={p.identity}
                          data-participant={p.identity}
                          data-mic={p.isMicrophoneEnabled ? 'on' : 'off'}
                          data-speaking={p.isSpeaking ? 'on' : 'off'}
                          className="flex items-center gap-2"
                        >`
const newParticipant = `                        <div
                          key={p.identity}
                          data-participant={p.identity}
                          data-mic={p.isMicrophoneEnabled ? 'on' : 'off'}
                          data-speaking={p.isSpeaking ? 'on' : 'off'}
                          className="flex items-center gap-2 cursor-pointer hover:bg-[var(--bg-hover)] rounded px-1 -mx-1 py-0.5 transition-colors"
                          onContextMenu={(e) => handleMemberContextMenu(e, p.identity, true)}
                        >`
if (!mv.includes(oldParticipant)) { console.error('FAIL: participant row'); process.exit(1) }
mv = mv.replace(oldParticipant, newParticipant)

// === 音质 badge: 在 voice channel 名称行后面 ===
const oldVoiceName = `                  <div className="flex items-center gap-2.5">
                    <IconVolume className="w-4 h-4 opacity-50" /> {vc.name}
                  </div>`
const newVoiceName = `                  <div className="flex items-center gap-2.5">
                    <IconVolume className="w-4 h-4 opacity-50" /> {vc.name}
                    {inVoice && currentChannel === vc.id && media.audioBitrate > 0 && (
                      <span data-testid={\`voice-bitrate-\${vc.id}\`} className="ml-auto text-[10px] font-mono px-1.5 py-0.5 rounded bg-[var(--bg-active)] text-[var(--text-muted)]">
                        {media.audioBitrate}K
                      </span>
                    )}
                  </div>`
if (!mv.includes(oldVoiceName)) { console.error('FAIL: voice name'); process.exit(1) }
mv = mv.replace(oldVoiceName, newVoiceName)

// === main 区响应式: px-[220px] -> px-4 md:px-[220px] ===
const oldMain = `          <main className="flex-1 flex flex-col min-w-0 px-[220px]">`
const newMain = `          <main className={\`flex-1 flex flex-col min-w-0 px-3 sm:px-4 md:px-[220px] transition-\${sidebarOpen ? 'pl-[220px]' : ''}\`}>`
if (!mv.includes(oldMain)) { console.error('FAIL: main'); process.exit(1) }
mv = mv.replace(oldMain, newMain)

// === 顶部 header: <md 加汉堡按钮 ===
const oldHeader = `        <header className="relative h-[40px] shrink-0 flex items-center mb-10">`
const newHeader = `        <header className="relative h-[40px] shrink-0 flex items-center mb-10">
          <button
            onClick={() => setSidebarOpen(v => !v)}
            className="md:hidden mr-3 w-8 h-8 flex items-center justify-center rounded-md text-[var(--text-muted)] hover:text-[var(--text-main)] hover:bg-[var(--bg-hover)] transition-colors"
            aria-label="toggle sidebar"
            data-testid="sidebar-toggle"
          >
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="w-5 h-5">
              <path d="M4 6h16M4 12h16M4 18h16" strokeLinecap="round" />
            </svg>
          </button>`
if (!mv.includes(oldHeader)) { console.error('FAIL: header'); process.exit(1) }
mv = mv.replace(oldHeader, newHeader)

// === sidebar overlay 点击关闭: 在 sidebar 之外加 mask ===
// 用 flex-1 relative 的父级<div> 后插入 mask
const oldBodyStart = `        <div className="flex-1 relative flex min-h-0">`
const newBodyStart = `        <div className="flex-1 relative flex min-h-0">
          {sidebarOpen && (
            <div
              className="md:hidden fixed inset-0 top-[40px] bottom-[72px] bg-black/30 z-20"
              onClick={() => setSidebarOpen(false)}
            />
          )}`
if (!mv.includes(oldBodyStart)) { console.error('FAIL: body start'); process.exit(1) }
mv = mv.replace(oldBodyStart, newBodyStart)

// === nav 项点击时关 sidebar ===
// 简化: navByIndex 只在 <768 有用, 每次点击也关闭 drawer
const navLabels = ['nav.groups', 'nav.dms', 'nav.friends', 'nav.settings', 'nav.more']
const navViews = ['chat', 'dms', 'friends', 'settings', 'more']
for (let i = 0; i < navViews.length; i++) {
  const view = navViews[i]
  const oldNav = `<span onClick={() => setActiveView('${view}')} `
  const newNav = `<span onClick={() => { setActiveView('${view}'); setSidebarOpen(false) }} `
  mv = mv.replace(oldNav, newNav)
}

// === footer nav 响应式间距 ===
const oldFooter = `          <div className="absolute left-1/2 -translate-x-1/2 flex items-center gap-8 text-[13px] font-medium text-[var(--text-muted)]">`
const newFooter = `          <div className="absolute left-1/2 -translate-x-1/2 flex items-center gap-3 sm:gap-5 md:gap-8 text-[11px] sm:text-[12px] md:text-[13px] font-medium text-[var(--text-muted)]">`
if (!mv.includes(oldFooter)) { console.error('FAIL: footer nav'); process.exit(1) }
mv = mv.replace(oldFooter, newFooter)

// === footer 左侧信息 <sm 隐藏 ===
const oldFooterLeft = `          <div className="w-[200px] shrink-0 flex items-center gap-3">`
const newFooterLeft = `          <div className="w-[120px] sm:w-[160px] md:w-[200px] shrink-0 flex items-center gap-3">`
mv = mv.replace(oldFooterLeft, newFooterLeft)

// === 渲染 UserContextMenu 到 JSX 末尾（footer 之后, 闭包 </div> 之前） ===
// 找最后一个 </footer>\n    </div>
const oldFooterEnd = `      </footer>
    </div>
  )
}`
const newFooterEnd = `      </footer>

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
}`
if (!mv.includes(oldFooterEnd)) { console.error('FAIL: footer end'); process.exit(1) }
mv = mv.replace(oldFooterEnd, newFooterEnd)

fs.writeFileSync('src/components/MainView.tsx', mv)
console.log('PATCHED MainView.tsx')

console.log('DONE Round 3')
