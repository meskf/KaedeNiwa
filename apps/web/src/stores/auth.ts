import { create } from 'zustand'
import { persist } from 'zustand/middleware'

interface AuthState {
  isLoggedIn: boolean
  serverUrl: string
  wsUrl: string
  token: string
  loginId: string
  setAuth: (data: { serverUrl: string, wsUrl: string, token: string, loginId: string }) => void
  logout: () => void
}

export const useAuthStore = create<AuthState>()(
  persist(
    (set) => ({
      isLoggedIn: false,
      serverUrl: 'http://127.0.0.1:18080',
      wsUrl: 'ws://127.0.0.1:18080/ws',
      token: '',
      loginId: '',
      setAuth: (data) => set({ isLoggedIn: true, ...data }),
      logout: () => set({ isLoggedIn: false, token: '', loginId: '' }),
    }),
    {
      name: 'kaede-auth', // localStorage key
    }
  )
)
