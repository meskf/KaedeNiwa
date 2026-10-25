import { create } from 'zustand'

interface Channel {
  id: string
  name: string
  kind: string
}

export interface PendingMsg {
  id: string
  channelId: string
  text: string
  ts: number
  status: 'sending' | 'failed'
  retries: number
}

interface ChatState {
  channels: Channel[]
  currentChannel: string
  pendingMessages: PendingMsg[]
  setChannels: (chs: Channel[]) => void
  setChannel: (ch: string) => void
  addPending: (msg: PendingMsg) => void
  removePending: (id: string) => void
  markPendingFailed: (id: string) => void
  incrementRetry: (id: string) => void
  clearPendingForChannel: (channelId: string) => void
}

export const useChatStore = create<ChatState>((set) => ({
  channels: [],
  currentChannel: 'general',
  pendingMessages: [],
  setChannels: (channels) => set((state) => {
    const exists = channels.find(c => c.id === state.currentChannel)
    return { channels, currentChannel: exists ? state.currentChannel : (channels[0]?.id || 'general') }
  }),
  setChannel: (ch) => set({ currentChannel: ch }),
  addPending: (msg) => set((state) => ({
    pendingMessages: [...state.pendingMessages, msg]
  })),
  removePending: (id) => set((state) => ({
    pendingMessages: state.pendingMessages.filter(m => m.id !== id)
  })),
  markPendingFailed: (id) => set((state) => ({
    pendingMessages: state.pendingMessages.map(m =>
      m.id === id ? { ...m, status: 'failed' as const } : m
    )
  })),
  incrementRetry: (id) => set((state) => ({
    pendingMessages: state.pendingMessages.map(m =>
      m.id === id ? { ...m, retries: m.retries + 1, status: 'sending' as const } : m
    )
  })),
  clearPendingForChannel: (channelId) => set((state) => ({
    pendingMessages: state.pendingMessages.filter(m => m.channelId !== channelId)
  })),
}))
