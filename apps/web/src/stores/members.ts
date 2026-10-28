import { create } from 'zustand'

export interface Member {
  login_id: string
  nickname: string
  op: string
  bio: string
  title: string
  avatar_url: string
  joined_at: number
}

interface MembersState {
  members: Member[]
  loading: boolean
  fetch: (token: string) => Promise<void>
  updateOp: (token: string, loginId: string, op: string) => Promise<{ ok: boolean; error?: string }>
}

export const useMembersStore = create<MembersState>((set) => ({
  members: [],
  loading: false,
  fetch: async (token) => {
    if (!token) return
    set({ loading: true })
    try {
      const res = await fetch('/api/members', { headers: { Authorization: `Bearer ${token}` } })
      if (res.ok) {
        const data = await res.json()
        set({ members: data.members || [] })
      }
    } catch (e) {
      console.error('fetch members failed', e)
    } finally {
      set({ loading: false })
    }
  },
  updateOp: async (token, loginId, op) => {
    try {
      const res = await fetch(`/api/members/${encodeURIComponent(loginId)}`, {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${token}` },
        body: JSON.stringify({ op }),
      })
      if (res.ok) {
        set(s => ({ members: s.members.map(m => m.login_id === loginId ? { ...m, op } : m) }))
        return { ok: true }
      }
      if (res.status === 403) return { ok: false, error: 'forbidden' }
      if (res.status === 404) return { ok: false, error: 'not_found' }
      return { ok: false, error: `HTTP ${res.status}` }
    } catch (e) {
      return { ok: false, error: String(e) }
    }
  },
}))
