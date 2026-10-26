import { create } from 'zustand';

export interface PendingMsg {
  id: string;
  channelId: string;
  text: string;
  ts: number;
  status: 'sending' | 'failed';
  retries: number;
}

export interface ChatMessage {
  sender: string;
  plaintext: string;
  ts: number;
}

export type ActiveView = 'chat' | 'dms' | 'friends' | 'settings' | 'more';

interface ChatState {
  channels: string[];
  currentChannel: string;
  activeView: ActiveView;
  messages: Record<string, ChatMessage[]>;
  pendingMessages: PendingMsg[];
  isAuthority: boolean;
  peerStatus: string;
  setChannels: (chs: string[]) => void;
  setChannel: (ch: string) => void;
  setActiveView: (view: ActiveView) => void;
  setMessages: (channelId: string, msgs: ChatMessage[]) => void;
  addMessage: (channelId: string, msg: ChatMessage) => void;
  addPending: (msg: PendingMsg) => void;
  removePending: (id: string) => void;
  markPendingFailed: (id: string) => void;
  incrementRetry: (id: string) => void;
  setIsAuthority: (val: boolean) => void;
  setPeerStatus: (status: string) => void;
}

export const useChatStore = create<ChatState>((set) => ({
  channels: ['general', 'dev', 'random'],
  currentChannel: 'general',
  activeView: 'chat',
  messages: {},
  pendingMessages: [],
  isAuthority: false,
  peerStatus: '',
  setChannels: (channels) => set({ channels }),
  setChannel: (currentChannel) => set({ currentChannel }),
  setActiveView: (activeView) => set({ activeView }),
  setMessages: (channelId, msgs) => set((state) => ({
    messages: { ...state.messages, [channelId]: msgs }
  })),
  addMessage: (channelId, msg) => set((state) => ({
    messages: {
      ...state.messages,
      [channelId]: [...(state.messages[channelId] || []), msg]
    }
  })),
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
  setIsAuthority: (isAuthority) => set({ isAuthority }),
  setPeerStatus: (peerStatus) => set({ peerStatus }),
}));
