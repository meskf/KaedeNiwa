import type { Attachment } from '../protocol/Attachment';
import type { Category } from '../protocol/Category';
import { create } from 'zustand';

export interface PendingMsg {
  id: string;
  channelId: string;
  text: string;
  ts: number;
  status: 'sending' | 'failed';
  retries: number;
}

export interface PendingAttachment {
  id: string;
  name: string;
  mime: string;
  size: number;
  bytes: Uint8Array;
  status: 'pending' | 'uploading' | 'uploaded' | 'failed';
  sent: number;
  total: number;
  attachment?: Attachment;
  error?: string;
}

export interface ChatMessage {
  sender: string;
  plaintext: string;
  ts: number;
  attachments?: Attachment[];
}

export interface ChannelInfo {
  id: string;
  name: string;
  kind: string;
  position: number;
  is_private: boolean;
  category_id?: string | null;
}

export interface DmConversation {
  conv_id: string;
  peer_login_id: string;
  peer_nickname: string;
}

export interface DmMessage {
  sender: string;
  ts: number;
  plaintext: string;
  attachments?: Attachment[];
}

export interface FriendEntry {
  login_id: string;
  nickname: string;
  direction: 'outgoing' | 'incoming' | 'friend';
  created_at: number;
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

  nodes: any[];
  channelList: ChannelInfo[];
  categories: Category[];
  dmConversations: DmConversation[];
  activeConvId: string | null;
  dmMessages: Record<string, DmMessage[]>;
  friends: FriendEntry[];
  serverMembers: string[];
  typingUsers: Record<string, Record<string, number>>;  // channelId -> {uid: expireTs}

  // channelId -> String(msgTs) -> emoji -> [uid]
  reactions: Record<string, Record<string, Record<string, string[]>>>;

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

  setNodes: (nodes: any[]) => void;
  setChannelList: (list: ChannelInfo[]) => void;
  setCategories: (cats: Category[]) => void;
  addCategory: (c: Category) => void;
  removeCategory: (id: string) => void;
  setDmConversations: (c: DmConversation[]) => void;
  addDmConversation: (c: DmConversation) => void;
  setActiveConv: (id: string | null) => void;
  setDmMessages: (convId: string, msgs: DmMessage[]) => void;
  appendDmMessage: (convId: string, msg: DmMessage) => void;
  setFriends: (f: FriendEntry[]) => void;
  setServerMembers: (m: string[]) => void;
  setTyping: (channelId: string, uid: string, ttlMs: number) => void;
  applyReaction: (channelId: string, targetTs: number, emoji: string, uid: string, action: 'add' | 'remove') => void;
  clearReactions: (channelId: string) => void;
  pruneTyping: () => void;

  pendingAttachments: PendingAttachment[];
  attachmentCache: Record<string, string>;
  addPendingAttachment: (a: PendingAttachment) => void;
  updatePendingAttachment: (id: string, patch: Partial<PendingAttachment>) => void;
  removePendingAttachment: (id: string) => void;
  clearPendingAttachments: () => void;
  setAttachmentUrl: (id: string, url: string) => void;
}

export const useChatStore = create<ChatState>((set) => ({
  channels: ['general', 'dev', 'random'],
  currentChannel: 'general',
  activeView: 'chat',
  messages: {},
  pendingMessages: [],
  isAuthority: false,
  peerStatus: '',

  nodes: [],
  channelList: [],
  categories: [],
  dmConversations: [],
  activeConvId: null,
  dmMessages: {},
  friends: [],
  serverMembers: [],
  typingUsers: {},
  reactions: {},
  pendingAttachments: [],
  attachmentCache: {},

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

  setNodes: (nodes) => set({ nodes }),
  setChannelList: (channelList) => set({ channelList }),
  setCategories: (categories) => set({ categories }),
  addCategory: (c) => set((state) => state.categories.some(x => x.id === c.id) ? state : { categories: [...state.categories, c] }),
  removeCategory: (id) => set((state) => ({ categories: state.categories.filter(c => c.id !== id) })),
  setDmConversations: (dmConversations) => set({ dmConversations }),
  addDmConversation: (c) => set((state) => {
    if (state.dmConversations.some(x => x.conv_id === c.conv_id)) return state;
    return { dmConversations: [...state.dmConversations, c] };
  }),
  setActiveConv: (activeConvId) => set({ activeConvId }),
  setDmMessages: (convId, msgs) => set((state) => ({
    dmMessages: { ...state.dmMessages, [convId]: msgs }
  })),
  appendDmMessage: (convId, msg) => set((state) => ({
    dmMessages: {
      ...state.dmMessages,
      [convId]: [...(state.dmMessages[convId] || []), msg]
    }
  })),
  setFriends: (friends) => set({ friends }),
  setServerMembers: (serverMembers) => set({ serverMembers }),
  setTyping: (channelId, uid, ttlMs) => set((state) => {
    const chan = state.typingUsers[channelId] || {};
    return {
      typingUsers: {
        ...state.typingUsers,
        [channelId]: { ...chan, [uid]: Date.now() + ttlMs },
      },
    };
  }),
  pruneTyping: () => set((state) => {
    const now = Date.now();
    let changed = false;
    const next: Record<string, Record<string, number>> = {};
    for (const [ch, users] of Object.entries(state.typingUsers)) {
      const kept: Record<string, number> = {};
      for (const [uid, exp] of Object.entries(users)) {
        if (exp > now) { kept[uid] = exp; } else { changed = true; }
      }
      if (Object.keys(kept).length > 0) { next[ch] = kept; }
    }
    return changed ? { typingUsers: next } : state;
  }),

  applyReaction: (channelId, targetTs, emoji, uid, action) => set((state) => {
    const tsKey = String(targetTs)
    const chMap = state.reactions[channelId] || {}
    const msgMap = chMap[tsKey] || {}
    const uids = (msgMap[emoji] || []).slice()
    let nextUids: string[]
    if (action === 'add') {
      if (uids.includes(uid)) return state
      nextUids = [...uids, uid]
    } else {
      if (!uids.includes(uid)) return state
      nextUids = uids.filter(u => u !== uid)
    }
    const nextMsgMap = { ...msgMap }
    if (nextUids.length === 0) delete nextMsgMap[emoji]
    else nextMsgMap[emoji] = nextUids
    const nextChMap = { ...chMap }
    if (Object.keys(nextMsgMap).length === 0) delete nextChMap[tsKey]
    else nextChMap[tsKey] = nextMsgMap
    const nextReactions = { ...state.reactions }
    if (Object.keys(nextChMap).length === 0) delete nextReactions[channelId]
    else nextReactions[channelId] = nextChMap
    return { reactions: nextReactions }
  }),
  clearReactions: (channelId) => set((state) => {
    if (!state.reactions[channelId]) return state
    const next = { ...state.reactions }
    delete next[channelId]
    return { reactions: next }
  }),

  addPendingAttachment: (a) => set((state) => ({
    pendingAttachments: [...state.pendingAttachments, a]
  })),
  updatePendingAttachment: (id, patch) => set((state) => ({
    pendingAttachments: state.pendingAttachments.map(a =>
      a.id === id ? { ...a, ...patch } : a
    )
  })),
  removePendingAttachment: (id) => set((state) => ({
    pendingAttachments: state.pendingAttachments.filter(a => a.id !== id)
  })),
  clearPendingAttachments: () => set({ pendingAttachments: [] }),
  setAttachmentUrl: (id, url) => set((state) => ({
    attachmentCache: { ...state.attachmentCache, [id]: url }
  })),
}));
