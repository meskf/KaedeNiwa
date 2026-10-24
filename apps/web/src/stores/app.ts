import { create } from 'zustand'

export type View = 'login' | 'main'

interface AppState {
  view: View
  setView: (v: View) => void
}

export const useAppStore = create<AppState>((set) => ({
  view: 'login',
  setView: (view) => set({ view }),
}))
