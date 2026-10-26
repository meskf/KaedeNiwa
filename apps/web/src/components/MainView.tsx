import { useState } from 'react'

const CHANNELS = ['general', 'dev', 'random']
const VOICE_CHANNELS = ['Lounge']

interface LocalMessage {
  id: string
  sender: string
  text: string
  ts: number
}

export function MainView({ onLogout }: { onLogout: () => void }) {
  const [currentChannel, setCurrentChannel] = useState('general')
  const [activeView, setActiveView] = useState<'chat' | 'dms' | 'friends' | 'settings' | 'more'>('chat')
  const [messages, setMessages] = useState<Record<string, LocalMessage[]>>({})
  const [input, setInput] = useState('')
  const [theme, setTheme] = useState<'dark' | 'light'>('dark')

  // 写死用户名
  const loginId = 'Admin'

  const handleSend = (e?: React.FormEvent) => {
    e?.preventDefault()
    if (!input.trim()) return
    const newMsg: LocalMessage = {
      id: Date.now().toString(),
      sender: loginId,
      text: input.trim(),
      ts: Date.now()
    }
    setMessages(prev => ({
      ...prev,
      [currentChannel]: [...(prev[currentChannel] || []), newMsg]
    }))
    setInput('')
  }

  const currentMessages = messages[currentChannel] || []
  const getInitial = (str: string) => str ? str.charAt(0).toUpperCase() : '?'

  return (
    <div className={`h-screen w-screen flex flex-col font-bold ${theme === 'dark' ? 'bg-[#18191c] text-white' : 'bg-white text-black'}`}>
      
      {/* 顶栏 */}
      <header className="relative h-[56px] shrink-0 flex items-center border-b border-[#2a2b2f]">
        <div className="w-[240px] shrink-0 h-full flex items-center px-4 text-[16px] tracking-wide font-bold z-10">
          KaedeNiwa
        </div>
        <div className="absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 w-full max-w-[480px] px-4 z-10">
          <input
            type="text"
            placeholder="Search..."
            className="w-full bg-[#1e1f22] text-white text-[13px] rounded-md pl-4 pr-3 py-1.5 outline-none placeholder-gray-500 border border-transparent"
          />
        </div>
        <div className="flex-1" />
        <div className="flex items-center gap-1 shrink-0 pr-2 z-10">
          <button onClick={() => setTheme(t => t === 'dark' ? 'light' : 'dark')} className="w-10 h-10 flex items-center justify-center text-gray-400 hover:text-white">
             {theme === 'dark' ? '☀' : '🌙'}
          </button>
          <div className="w-[138px] h-full" />
        </div>
      </header>

      <div className="flex-1 flex min-h-0">
        {/* 侧边栏 */}
        <aside className="w-[240px] shrink-0 flex flex-col border-r border-[#2a2b2f]">
          <div className="h-[56px] shrink-0 flex items-center px-4 border-b border-[#2a2b2f]">
            <div className="w-8 h-8 rounded-lg bg-[#5865f2] flex items-center justify-center text-white font-bold text-sm shrink-0">
              K
            </div>
            <div className="ml-3 flex flex-col min-w-0">
              <span className="text-[14px] font-bold leading-tight truncate">Kaede Server</span>
              <span className="text-[11px] font-normal text-gray-400 truncate">Workspace</span>
            </div>
          </div>

          <div className="flex-1 overflow-y-auto py-3">
            <div className="px-4 mb-2 text-[11px] font-bold tracking-widest text-gray-400 uppercase">
              Text Channels
            </div>
            {CHANNELS.map(ch => (
              <div
                key={ch}
                data-testid={`channel-item-${ch}`}
                onClick={() => { setCurrentChannel(ch); setActiveView('chat'); }}
                className={`px-3 py-2 mx-2 rounded text-[15px] cursor-pointer flex items-center gap-2 ${
                  currentChannel === ch && activeView === 'chat' ? 'bg-[#2b2d31] text-white' : 'text-gray-400 hover:bg-[#1e1f22]'
                }`}
              >
                <span className="text-[18px] font-light leading-none">#</span> {ch}
              </div>
            ))}

            <div className="px-4 mt-6 mb-2 text-[11px] font-bold tracking-widest text-gray-400 uppercase">
              Voice Channels
            </div>
            {VOICE_CHANNELS.map(vc => (
              <div 
                key={vc}
                className="px-3 py-2 mx-2 text-[15px] text-gray-400 flex items-center gap-2 hover:bg-[#1e1f22] cursor-pointer rounded"
              >
                <span className="text-[16px]">🔊</span> {vc}
              </div>
            ))}
          </div>
        </aside>

        {/* 主区域 */}
        <main className="flex-1 flex flex-col min-w-0">
          {activeView === 'chat' ? (
            <>
              <div className="h-[56px] shrink-0 flex items-center justify-between px-4 border-b border-[#2a2b2f]">
                <div className="flex items-center gap-3">
                  <span className="text-[24px] font-light text-gray-400 leading-none pt-1">#</span>
                  <span className="font-bold text-[16px]">{currentChannel}</span>
                </div>
                <div className="flex items-center gap-3">
                  <button className="text-xs px-3 py-1.5 rounded bg-[#248046] text-white font-bold">Join Voice</button>
                </div>
              </div>

              <div className="flex-1 overflow-y-auto px-6 py-4">
                {currentMessages.length > 0 && (
                  <div className="space-y-5 max-w-4xl mx-auto mt-2">
                    {currentMessages.map((msg) => (
                      <div key={msg.id} className="flex gap-4 hover:bg-[#1e1f22] -mx-2 px-2 py-1 rounded">
                        <div className="w-10 h-10 rounded-full flex items-center justify-center text-[15px] font-bold shrink-0 bg-[#5865f2] text-white">
                          {getInitial(msg.sender)}
                        </div>
                        <div className="min-w-0">
                          <div className="flex items-baseline gap-2">
                            <span className="font-bold text-[15px]">{msg.sender}</span>
                            <span className="text-[11px] font-normal text-gray-400">{new Date(msg.ts).toLocaleTimeString()}</span>
                          </div>
                          <div className="text-[15px] font-normal mt-0.5 whitespace-pre-wrap">{msg.text}</div>
                        </div>
                      </div>
                    ))}
                  </div>
                )}
              </div>

              <div className="px-6 pb-6 pt-2 shrink-0">
                <form onSubmit={handleSend} className="flex items-center gap-3 rounded-lg px-4 py-3 bg-[#1e1f22]">
                  <span className="text-gray-400 text-xl font-light">+</span>
                  <input
                    type="text"
                    value={input}
                    onChange={e => setInput(e.target.value)}
                    placeholder={`Message #${currentChannel}`}
                    data-testid="message-input"
                    className="bg-transparent outline-none flex-1 text-[15px] font-normal placeholder-gray-500 text-white"
                  />
                  <button type="submit" className="text-gray-400 hover:text-white">
                    Send
                  </button>
                </form>
              </div>
            </>
          ) : (
            <div className="flex-1 flex flex-col items-center justify-center text-gray-500">
              <div className="text-2xl font-bold uppercase mb-2">{activeView}</div>
              <div className="text-sm font-normal">Under construction</div>
            </div>
          )}
        </main>
      </div>

      {/* 底栏 */}
      <footer className="relative h-[56px] shrink-0 flex items-center border-t border-[#2a2b2f] px-4">
        <div className="w-[240px] shrink-0 flex items-center gap-3">
          <div className="w-8 h-8 rounded-full bg-[#5865f2] flex items-center justify-center text-xs text-white font-bold">
            {getInitial(loginId)}
          </div>
          <div className="flex flex-col">
            <span className="text-[14px] font-bold leading-tight">{loginId}</span>
            <button onClick={onLogout} className="text-[11px] font-normal text-gray-400 text-left hover:underline">Logout</button>
          </div>
        </div>

        <div className="absolute left-1/2 -translate-x-1/2 flex items-center gap-4 text-[13px] font-bold text-gray-400">
          <span onClick={() => setActiveView('chat')} className={`cursor-pointer ${activeView === 'chat' ? 'text-white' : ''}`}>Groups</span>
          <span className="text-gray-600 font-normal">|</span>
          <span onClick={() => setActiveView('dms')} className={`cursor-pointer ${activeView === 'dms' ? 'text-white' : ''}`}>DMs</span>
          <span className="text-gray-600 font-normal">|</span>
          <span onClick={() => setActiveView('friends')} className={`cursor-pointer ${activeView === 'friends' ? 'text-white' : ''}`}>Friends</span>
          <span className="text-gray-600 font-normal">|</span>
          <span onClick={() => setActiveView('settings')} className={`cursor-pointer ${activeView === 'settings' ? 'text-white' : ''}`}>Settings</span>
          <span className="text-gray-600 font-normal">|</span>
          <span onClick={() => setActiveView('more')} className={`cursor-pointer ${activeView === 'more' ? 'text-white' : ''}`}>More</span>
        </div>

        <div className="ml-auto w-[100px] text-right text-[12px] font-normal text-gray-500 font-mono">
          v0.0.1-alpha
        </div>
      </footer>
    </div>
  )
}
