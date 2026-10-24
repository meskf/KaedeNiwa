import { create } from 'zustand'

interface Channel {
  id: string
  name: string
  kind: string
}

interface ChatState {
  channels: Channel[]
  currentChannel: string
  setChannels: (chs: Channel[]) => void
  setChannel: (ch: string) => void
}

export const useChatStore = create<ChatState>((set) => ({
  channels: [],
  currentChannel: 'general',
  setChannels: (channels) => set((state) => {
    // 如果当前频道不在新列表中，默认切到第一个
    const exists = channels.find(c => c.id === state.currentChannel)
    return { channels, currentChannel: exists ? state.currentChannel : (channels[0]?.id || 'general') }
  }),
  setChannel: (ch) => set({ currentChannel: ch }),
}))
