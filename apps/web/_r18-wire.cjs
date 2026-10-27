const fs = require('fs')
let s = fs.readFileSync('src/components/MainView.tsx', 'utf8')
const rep = (from, to, name) => {
  if (!s.includes(from)) { console.error('FAIL:', name); process.exit(1) }
  s = s.replace(from, to)
  console.log('OK:', name)
}

// 1. import
rep(
  "import { UserContextMenu } from './UserContextMenu'",
  "import { UserContextMenu } from './UserContextMenu'\nimport { SearchPalette } from './SearchPalette'",
  'import')

// 2. state
rep(
  "  const [sidebarOpen, setSidebarOpen] = useState(true)",
  "  const [sidebarOpen, setSidebarOpen] = useState(true)\n  const [searchOpen, setSearchOpen] = useState(false)",
  'state')

// 3. Ctrl+K 全局快捷键 — 放在其他 useEffect 附近，用 sidebarOpen 那行的 useEffect 后面加
rep(
  `  useEffect(() => {
    const TAB_KEY = 'kaede-active-tab'`,
  `  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault()
        setSearchOpen(v => !v)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  useEffect(() => {
    const TAB_KEY = 'kaede-active-tab'`,
  'ctrl+k')

// 4. header 搜索框 -> 打开 palette（改 input 为 readOnly + onClick）
const oldSearch = `              <input
                type="text"
                placeholder="Search..."
                className="w-full bg-[var(--bg-hover)] text-[var(--text-main)] text-[13px] rounded-md pl-10 pr-4 py-2 outline-none placeholder-[var(--text-muted)]"
              />`
const newSearch = `              <input
                type="text"
                readOnly
                onClick={() => setSearchOpen(true)}
                placeholder="Search...  (Ctrl+K)"
                className="w-full bg-[var(--bg-hover)] text-[var(--text-main)] text-[13px] rounded-md pl-10 pr-4 py-2 outline-none placeholder-[var(--text-muted)] cursor-pointer"
                data-testid="search-trigger"
              />`
rep(oldSearch, newSearch, 'search input')

// 5. 在 UserContextMenu 前挂 SearchPalette
rep(
  `      {ctxMenu && (
        <UserContextMenu`,
  `      <SearchPalette
        open={searchOpen}
        onClose={() => setSearchOpen(false)}
        onSelect={(channelId) => {
          setChannel(channelId)
          setActiveView('chat')
        }}
      />

      {ctxMenu && (
        <UserContextMenu`,
  'mount palette')

fs.writeFileSync('src/components/MainView.tsx', s)
console.log('DONE')
