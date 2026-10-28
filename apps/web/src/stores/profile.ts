import { create } from 'zustand'

export interface MyProfile {
  loginId: string
  nickname: string
  bio: string
  title: string
  avatarUrl: string
}

interface ProfileState {
  me: MyProfile
  loading: boolean
  fetchMe: (token: string) => Promise<void>
  updateMe: (
    token: string,
    patch: { nickname?: string; bio?: string; title?: string },
  ) => Promise<{ ok: boolean; error?: string }>
  uploadAvatar: (token: string, file: File | Blob) => Promise<{ ok: boolean; error?: string }>
  deleteAvatar: (token: string) => Promise<{ ok: boolean; error?: string }>
}

const EMPTY: MyProfile = { loginId: '', nickname: '', bio: '', title: '', avatarUrl: '' }

async function fetchMeRaw(token: string): Promise<MyProfile | null> {
  const res = await fetch('/api/me', { headers: { Authorization: `Bearer ${token}` } })
  if (!res.ok) return null
  const data = await res.json()
  return {
    loginId: data.login_id || '',
    nickname: data.nickname || '',
    bio: data.bio || '',
    title: data.title || '',
    avatarUrl: data.avatar_url || '',
  }
}

export const useProfileStore = create<ProfileState>((set, get) => ({
  me: EMPTY,
  loading: false,
  fetchMe: async (token) => {
    if (!token) return
    set({ loading: true })
    try {
      const me = await fetchMeRaw(token)
      if (me) set({ me })
    } finally {
      set({ loading: false })
    }
  },
  updateMe: async (token, patch) => {
    try {
      const res = await fetch('/api/me', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${token}` },
        body: JSON.stringify(patch),
      })
      if (res.status === 204) {
        const cur = get().me
        set({
          me: {
            ...cur,
            nickname: patch.nickname ?? cur.nickname,
            bio: patch.bio ?? cur.bio,
            title: patch.title ?? cur.title,
          },
        })
        return { ok: true }
      }
      const text = await res.text().catch(() => '')
      return { ok: false, error: text || `HTTP ${res.status}` }
    } catch (e) {
      return { ok: false, error: String(e) }
    }
  },
  uploadAvatar: async (token, file) => {
    try {
      const { compressToSquareJpeg } = await import('../lib/imageUtils')
      const blob = await compressToSquareJpeg(file, 256, 0.85)
      const res = await fetch('/api/me/avatar', {
        method: 'POST',
        headers: { 'Content-Type': 'image/jpeg', Authorization: `Bearer ${token}` },
        body: blob,
      })
      if (res.status === 204) {
        const me = await fetchMeRaw(token)
        if (me) set({ me })
        return { ok: true }
      }
      if (res.status === 401) return { ok: false, error: 'unauthorized' }
      const text = await res.text().catch(() => '')
      return { ok: false, error: text || `HTTP ${res.status}` }
    } catch (e) {
      return { ok: false, error: String(e) }
    }
  },
  deleteAvatar: async (token) => {
    try {
      const res = await fetch('/api/me/avatar', {
        method: 'DELETE',
        headers: { Authorization: `Bearer ${token}` },
      })
      if (res.status === 204) {
        set((s) => ({ me: { ...s.me, avatarUrl: '' } }))
        return { ok: true }
      }
      return { ok: false, error: `HTTP ${res.status}` }
    } catch (e) {
      return { ok: false, error: String(e) }
    }
  },
}))
