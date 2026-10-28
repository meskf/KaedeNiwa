import { create } from 'zustand'
import { ServerProfile } from '../protocol/Server'

interface ServerState {
  profile: ServerProfile
  loading: boolean
  fetch: (token: string) => Promise<void>
  update: (token: string, patch: Partial<ServerProfile>) => Promise<{ ok: boolean; error?: string }>
  uploadIcon: (token: string, file: File | Blob) => Promise<{ ok: boolean; error?: string }>
}

export const useServerStore = create<ServerState>((set) => ({
  profile: { name: 'Kaede Server', description: 'Workspace', icon_url: null },
  loading: false,
  fetch: async (token: string) => {
    if (!token) return
    set({ loading: true })
    try {
      const res = await fetch('/api/server', { headers: { Authorization: `Bearer ${token}` } })
      if (res.ok) {
        const data = await res.json()
        set({ profile: data })
      }
    } catch (e) {
      console.error('fetch server profile failed', e)
    } finally {
      set({ loading: false })
    }
  },
  update: async (token, patch) => {
    try {
      const res = await fetch('/api/server', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${token}` },
        body: JSON.stringify(patch),
      })
      if (res.status === 204) {
        set(s => ({ profile: { ...s.profile, ...patch } as ServerProfile }))
        return { ok: true }
      }
      if (res.status === 403) return { ok: false, error: 'forbidden' }
      const err = await res.text().catch(() => '')
      return { ok: false, error: err || `HTTP ${res.status}` }
    } catch (e) {
      return { ok: false, error: String(e) }
    }
  },
  uploadIcon: async (token, file) => {
    try {
      const { compressToSquareJpeg } = await import('../lib/imageUtils')
      const blob = await compressToSquareJpeg(file, 256, 0.85)
      const res = await fetch('/api/server/icon', {
        method: 'POST',
        headers: { 'Content-Type': 'image/jpeg', Authorization: `Bearer ${token}` },
        body: blob,
      })
      if (res.status === 204) {
        const r = await fetch('/api/server', { headers: { Authorization: `Bearer ${token}` } })
        if (r.ok) { const data = await r.json(); set({ profile: data }) }
        return { ok: true }
      }
      if (res.status === 403) return { ok: false, error: 'forbidden' }
      return { ok: false, error: `HTTP ${res.status}` }
    } catch (e) {
      return { ok: false, error: String(e) }
    }
  },
}))
