const fs = require('fs')
let s = fs.readFileSync('src/components/MainView.tsx', 'utf8')
const log = []
function rep(from, to, name) {
  if (!s.includes(from)) { log.push('MISS: ' + name); return }
  s = s.replace(from, to)
  log.push('OK: ' + name)
}

// 1. import
rep(
  "import { UserContextMenu } from './UserContextMenu'",
  "import { UserContextMenu } from './UserContextMenu'\nimport { SearchPalette } from './SearchPalette'",
  'import SearchPalette')

// 2. state
rep(
  "  const [sidebarOpen, setSidebarOpen] = useState(true)",
  "  const [sidebarOpen, setSidebarOpen] = useState(true)\n  const [searchOpen, setSearchOpen] = useState(false)",
  'state searchOpen')

// 3. Ctrl+K useEffect（插在 mediaRef effect 后）
rep(
  "  useEffect(() => { mediaRef.current = media }, [media])",
  `  useEffect(() => { mediaRef.current = media }, [media])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault()
        setSearchOpen(v => !v)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])`,
  'ctrl+k effect')

// 4. search input
rep(
  `              <input
                type="text"
                placeholder="Search..."
                className="w-full bg-[var(--bg-hover)] text-[var(--text-main)] text-[13px] rounded-md pl-10 pr-4 py-2 outline-none placeholder-[var(--text-muted)]"
              />`,
  `              <input
                type="text"
                readOnly
                onClick={() => setSearchOpen(true)}
                onFocus={() => setSearchOpen(true)}
                placeholder="Search...  (Ctrl+K)"
                className="w-full bg-[var(--bg-hover)] text-[var(--text-main)] text-[13px] rounded-md pl-10 pr-4 py-2 outline-none placeholder-[var(--text-muted)] cursor-pointer"
                data-testid="search-trigger"
              />`,
  'search input')

// 5. mount SearchPalette before ctxMenu
rep(
  `        {ctxMenu && (
          <UserContextMenu`,
  `        <SearchPalette
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
log.forEach(l => console.log(l))
console.log('DONE')
