const fs = require('fs')
let s = fs.readFileSync('src/stores/chat.ts', 'utf8')
const log = []
function rep(from, to, name) {
  if (!s.includes(from)) { log.push('MISS: ' + name); return }
  s = s.replace(from, to)
  log.push('OK: ' + name)
}

// import
rep(
  "import type { Attachment } from '../protocol/Attachment';",
  "import type { Attachment } from '../protocol/Attachment';\nimport type { Category } from '../protocol/Category';",
  'import Category')

// interface ChatState
rep(
  '  channelList: ChannelInfo[];',
  '  channelList: ChannelInfo[];\n  categories: Category[];',
  'state categories')

// action sig
rep(
  '  setChannelList: (list: ChannelInfo[]) => void;',
  '  setChannelList: (list: ChannelInfo[]) => void;\n  setCategories: (cats: Category[]) => void;\n  addCategory: (c: Category) => void;\n  removeCategory: (id: string) => void;',
  'actions sig')

// init value
rep(
  '  channelList: [],\n  dmConversations: [],',
  '  channelList: [],\n  categories: [],\n  dmConversations: [],',
  'init categories')

// implementations
rep(
  '  setChannelList: (channelList) => set({ channelList }),',
  `  setChannelList: (channelList) => set({ channelList }),
  setCategories: (categories) => set({ categories }),
  addCategory: (c) => set((state) => state.categories.some(x => x.id === c.id) ? state : { categories: [...state.categories, c] }),
  removeCategory: (id) => set((state) => ({ categories: state.categories.filter(c => c.id !== id) })),`,
  'impl actions')

fs.writeFileSync('src/stores/chat.ts', s)
log.forEach(l => console.log(l))
