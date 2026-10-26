import { useEffect, useMemo, useState, type FormEvent } from 'react'
import { getClient } from '../wasm/client'
import { useChatStore, type FriendEntry } from '../stores/chat'

type Tab = 'online' | 'all' | 'pending'

export function FriendsView() {
  const friends = useChatStore(s => s.friends)
  const setFriends = useChatStore(s => s.setFriends)
  const addDmConversation = useChatStore(s => s.addDmConversation)
  const setActiveConv = useChatStore(s => s.setActiveConv)
  const setActiveView = useChatStore(s => s.setActiveView)

  const [activeTab, setActiveTab] = useState<Tab>('all')
  const [newId, setNewId] = useState('')
  const [showAdd, setShowAdd] = useState(false)
  const [busy, setBusy] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  const getInitial = (str: string) => str ? str.charAt(0).toUpperCase() : '?'
  const loadFriends = async () => {
    const c = getClient()
    if (!c) return
    try {
      const raw = await c.listFriends()
      const arr = typeof raw === 'string' ? JSON.parse(raw) : raw
      setFriends((arr || []) as FriendEntry[])
    } catch (e) {
      console.error('[friends] list:', e)
    }
  }

  useEffect(() => { loadFriends() }, [])

  const handleAdd = async (e: FormEvent) => {
    e.preventDefault()
    const peer = newId.trim()
    if (!peer || busy) return
    const c = getClient()
    if (!c) return
    setBusy('add')
    setError(null)
    try {
      await c.sendFriendRequest(peer)
      setNewId('')
      setShowAdd(false)
      await loadFriends()
    } catch (e: any) {
      setError(String(e))
    } finally {
      setBusy(null)
    }
  }

  const handleAccept = async (peer: string) => {
    const c = getClient()
    if (!c) return
    setBusy(peer)
    setError(null)
    try {
      await c.acceptFriend(peer)
      await loadFriends()
    } catch (e: any) {
      setError(String(e))
    } finally {
      setBusy(null)
    }
  }

  const handleRemove = async (peer: string) => {
    const c = getClient()
    if (!c) return
    setBusy(peer)
    setError(null)
    try {
      await c.removeFriend(peer)
      await loadFriends()
    } catch (e: any) {
      setError(String(e))
    } finally {
      setBusy(null)
    }
  }

  const handleMessage = async (peer: string) => {
    const c = getClient()
    if (!c) return
    setBusy(peer)
    setError(null)
    try {
      const raw = await c.openDm(peer)
      const conv = typeof raw === 'string' ? JSON.parse(raw) : raw
      addDmConversation(conv)
      setActiveConv(conv.conv_id)
      setActiveView('dms')
    } catch (e: any) {
      setError(String(e))
    } finally {
      setBusy(null)
    }
  }

  const filtered = useMemo(() => {
    if (activeTab === 'pending') return friends.filter(f => f.direction === 'incoming' || f.direction === 'outgoing')
    if (activeTab === 'online') return friends.filter(f => f.direction === 'friend')
    return friends
  }, [friends, activeTab])

  const counts = useMemo(() => ({
    online: friends.filter(f => f.direction === 'friend').length,
    all: friends.filter(f => f.direction === 'friend').length,
    pending: friends.filter(f => f.direction !== 'friend').length,
  }), [friends])
  return (
    <div className="flex flex-col h-full w-full max-w-[1000px] mx-auto">
      <div className="flex items-center justify-between mb-6 mt-1">
        <h1 className="text-3xl font-extrabold tracking-tight text-[var(--text-main)]">Friends</h1>
        <button
          onClick={() => setShowAdd(v => !v)}
          className="px-3 py-1.5 text-[12px] font-semibold rounded-md bg-[var(--bg-active)] text-[var(--text-main)] hover:bg-[var(--text-muted)] transition-colors"
        >
          + Add Friend
        </button>
      </div>
      {showAdd && (
        <form onSubmit={handleAdd} className="mb-4 p-3 rounded-lg bg-[var(--bg-hover)] flex gap-2">
          <input
            type="text"
            value={newId}
            onChange={e => setNewId(e.target.value)}
            placeholder="login_id"
            className="flex-1 bg-[var(--bg-app)] text-[var(--text-main)] text-[13px] rounded-md px-3 py-2 outline-none"
          />
          <button type="submit" disabled={busy === 'add'} className="px-3 py-1.5 text-[12px] font-semibold rounded-md bg-[var(--bg-active)] text-[var(--text-main)] hover:bg-[var(--text-muted)] disabled:opacity-50">
            {busy === 'add' ? '...' : 'Send'}
          </button>
          <button type="button" onClick={() => { setShowAdd(false); setNewId(''); setError(null) }} className="px-3 py-1.5 text-[12px] text-[var(--text-muted)] hover:text-[var(--text-main)]">
            Cancel
          </button>
        </form>
      )}
      {error && (
        <div className="mb-3 px-3 py-2 text-[12px] text-red-400 bg-red-500/10 rounded-md">{error}</div>
      )}
      <div className="flex items-center gap-6 border-b border-[var(--bg-hover)] mb-4 pb-2">
        {(['online', 'all', 'pending'] as Tab[]).map(tab => (
          <button
            key={tab}
            onClick={() => setActiveTab(tab)}
            className={`text-[14px] font-medium capitalize transition-colors ${
              activeTab === tab
                ? 'text-[var(--text-main)]'
                : 'text-[var(--text-muted)] hover:text-[var(--text-main)]'
            }`}
          >
            {tab} {counts[tab] > 0 && <span className="text-[11px] opacity-60">({counts[tab]})</span>}
          </button>
        ))}
      </div>      <div className="flex-1 overflow-y-auto">
        {filtered.length === 0 && (
          <div className="text-[13px] text-[var(--text-muted)] py-8 text-center">
            {activeTab === 'pending' ? 'No pending requests.' : activeTab === 'online' ? 'No friends online.' : 'No friends yet. Add one above.'}
          </div>
        )}
        <div className="flex flex-col gap-2">
          {filtered.map(f => (
            <div key={f.login_id} className="flex items-center justify-between p-3 rounded-lg hover:bg-[var(--bg-hover)] transition-colors group">
              <div className="flex items-center gap-3 min-w-0">
                <div className="relative shrink-0">
                  <div className="w-9 h-9 rounded-full bg-[var(--bg-active)] flex items-center justify-center text-[13px] font-bold text-[var(--text-main)]">
                    {getInitial(f.nickname || f.login_id)}
                  </div>
                  <div className={`absolute bottom-0 right-0 w-3 h-3 border-2 border-[var(--bg-app)] rounded-full ${
                    f.direction === 'friend' ? 'bg-green-500' : f.direction === 'incoming' ? 'bg-yellow-500' : 'bg-gray-500'
                  }`}></div>
                </div>
                <div className="flex flex-col min-w-0">
                  <span className="text-[14px] font-semibold text-[var(--text-main)] truncate">
                    {f.nickname || f.login_id}
                  </span>
                  <span className="text-[11px] text-[var(--text-muted)] truncate">
                    {f.direction === 'friend' ? 'Friend' : f.direction === 'incoming' ? 'Incoming request' : 'Outgoing request'} · {f.login_id}
                  </span>
                </div>
              </div>
              <div className="flex items-center gap-2 opacity-0 group-hover:opacity-100 transition-opacity shrink-0">
                {f.direction === 'friend' && (
                  <>
                    <button
                      onClick={() => handleMessage(f.login_id)}
                      disabled={busy === f.login_id}
                      className="px-3 py-1.5 bg-[var(--bg-active)] hover:bg-[var(--text-muted)] text-[var(--text-main)] text-[12px] rounded-md font-medium transition-colors disabled:opacity-50"
                    >
                      Message
                    </button>
                    <button
                      onClick={() => handleRemove(f.login_id)}
                      disabled={busy === f.login_id}
                      className="px-3 py-1.5 text-[12px] text-[var(--text-muted)] hover:text-red-400 transition-colors disabled:opacity-50"
                    >
                      Remove
                    </button>
                  </>
                )}
                {f.direction === 'incoming' && (
                  <>
                    <button
                      onClick={() => handleAccept(f.login_id)}
                      disabled={busy === f.login_id}
                      className="px-3 py-1.5 bg-green-600 hover:bg-green-500 text-white text-[12px] rounded-md font-medium transition-colors disabled:opacity-50"
                    >
                      Accept
                    </button>
                    <button
                      onClick={() => handleRemove(f.login_id)}
                      disabled={busy === f.login_id}
                      className="px-3 py-1.5 text-[12px] text-[var(--text-muted)] hover:text-red-400 transition-colors disabled:opacity-50"
                    >
                      Reject
                    </button>
                  </>
                )}
                {f.direction === 'outgoing' && (
                  <button
                    onClick={() => handleRemove(f.login_id)}
                    disabled={busy === f.login_id}
                    className="px-3 py-1.5 text-[12px] text-[var(--text-muted)] hover:text-red-400 transition-colors disabled:opacity-50"
                  >
                    Cancel request
                  </button>
                )}
              </div>
            </div>
          ))}
        </div>
      </div>
    </div>
  )
}