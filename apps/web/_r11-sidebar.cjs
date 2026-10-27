const fs = require('fs')
let s = fs.readFileSync('src/components/MainView.tsx', 'utf8')
const ok = (name) => console.log('OK', name)
const fail = (name) => { console.error('FAIL', name); process.exit(1) }

// 1. sidebarOpen default based on viewport
const old1 = '  const [sidebarOpen, setSidebarOpen] = useState(false)'
const new1 = "  const [sidebarOpen, setSidebarOpen] = useState(() => typeof window !== 'undefined' && window.innerWidth >= 768)"
if (!s.includes(old1)) fail('state'); s = s.replace(old1, new1); ok('state')

// 2. aside + drawer X button
const old2 = "          <aside className={`absolute left-0 top-0 bottom-0 w-[200px] sm:w-[180px] lg:w-[200px] flex-col z-30 bg-[var(--bg-app)] transition-transform ${sidebarOpen ? 'flex' : 'hidden'} md:flex`}>"
const new2 = [
  "          <aside className={`absolute left-0 top-0 bottom-0 w-[260px] md:w-[180px] lg:w-[200px] flex-col z-30 bg-[var(--bg-app)] shadow-xl md:shadow-none transition-transform ${sidebarOpen ? 'flex' : 'hidden'}`}>",
  '            <button',
  '              onClick={() => setSidebarOpen(false)}',
  '              className="md:hidden absolute top-3 right-3 w-7 h-7 flex items-center justify-center rounded text-[var(--text-muted)] hover:text-[var(--text-main)] hover:bg-[var(--bg-hover)]"',
  '              aria-label="close sidebar"',
  '              data-testid="sidebar-close-drawer"',
  '            >',
  '              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="w-4 h-4">',
  '                <path d="M6 6l12 12M18 6L6 18" strokeLinecap="round" />',
  '              </svg>',
  '            </button>'
].join('\n')
if (!s.includes(old2)) fail('aside'); s = s.replace(old2, new2); ok('aside + close btn')

// 3. main padding follows sidebarOpen
const old3 = '<main className="flex-1 flex flex-col min-w-0 px-3 sm:px-4 md:pl-[200px] md:pr-4 lg:px-[220px]">'
const new3 = "<main className={`flex-1 flex flex-col min-w-0 px-3 sm:px-4 md:pr-4 lg:pr-[220px] transition-all ${sidebarOpen ? 'md:pl-[200px] lg:pl-[220px]' : 'md:pl-6 lg:pl-6'}`}>"
if (!s.includes(old3)) fail('main'); s = s.replace(old3, new3); ok('main padding')

// 4. top-right desktop sidebar toggle
const old4 = [
  '          <div className="flex items-center gap-1 sm:gap-2 shrink-0">',
  '            <button',
  '              onClick={() => setShowMembers(!showMembers)}'
].join('\n')
const new4 = [
  '          <div className="flex items-center gap-1 sm:gap-2 shrink-0">',
  '            <button',
  '              onClick={() => setSidebarOpen(v => !v)}',
  "              className={`hidden md:flex w-8 h-8 items-center justify-center rounded-md transition-colors ${sidebarOpen ? 'text-[var(--text-main)] bg-[var(--bg-hover)]' : 'text-[var(--text-muted)] hover:text-[var(--text-main)] hover:bg-[var(--bg-hover)]'}`}",
  '              title="Toggle sidebar"',
  '              data-testid="sidebar-toggle-desktop"',
  '            >',
  '              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="w-4 h-4">',
  '                <path d="M4 6h16M4 12h16M4 18h16" strokeLinecap="round" />',
  '              </svg>',
  '            </button>',
  '            <button',
  '              onClick={() => setShowMembers(!showMembers)}'
].join('\n')
if (!s.includes(old4)) fail('top-right'); s = s.replace(old4, new4); ok('desktop sidebar btn')

// 5. channel clicks close drawer
s = s.replace(/onClick=\{\(\) => \{ setChannel\(ch\.id\); setActiveView\('chat'\); \}\}/g, "onClick={() => { setChannel(ch.id); setActiveView('chat'); setSidebarOpen(false) }}")
s = s.replace(/onClick=\{\(\) => \{ setChannel\(vc\.id\); setActiveView\('chat'\); \}\}/g, "onClick={() => { setChannel(vc.id); setActiveView('chat'); setSidebarOpen(false) }}")
ok('channel clicks close drawer')

fs.writeFileSync('src/components/MainView.tsx', s)
console.log('DONE')
