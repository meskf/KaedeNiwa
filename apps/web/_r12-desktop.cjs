const fs = require('fs')
let s = fs.readFileSync('src/components/MainView.tsx', 'utf8')
const rep = (from, to, name) => {
  if (!s.includes(from)) { console.error('MISS:', name); process.exit(1) }
  s = s.replace(from, to)
  console.log('OK:', name)
}

// 1. sidebarOpen 固定 true + 去掉 resize 监听
rep(
`  const [sidebarOpen, setSidebarOpen] = useState(() => typeof window !== 'undefined' ? window.innerWidth >= 768 : true)
  useEffect(() => {
    let wasDesktop = typeof window !== 'undefined' && window.innerWidth >= 768
    const onResize = () => {
      const isDesktop = window.innerWidth >= 768
      if (isDesktop !== wasDesktop) {
        setSidebarOpen(isDesktop)
        wasDesktop = isDesktop
      }
    }
    window.addEventListener('resize', onResize)
    return () => window.removeEventListener('resize', onResize)
  }, [])`,
`  const [sidebarOpen, setSidebarOpen] = useState(true)`,
'sidebarOpen simplify')

// 2. 汉堡按钮删除
rep(
`          <button
            onClick={() => setSidebarOpen(v => !v)}
            className="md:hidden shrink-0 w-8 h-8 flex items-center justify-center rounded-md text-[var(--text-muted)] hover:text-[var(--text-main)] hover:bg-[var(--bg-hover)] transition-colors"
            aria-label="toggle sidebar"
            data-testid="sidebar-toggle"
          >
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="w-5 h-5">
              <path d="M4 6h16M4 12h16M4 18h16" strokeLinecap="round" />
            </svg>
          </button>
`,
'',
'hamburger delete')

// 3. mask 删除
rep(
`          {sidebarOpen && (
            <div
              className="md:hidden fixed inset-0 top-[40px] bottom-[72px] bg-black/30 z-20"
              onClick={() => setSidebarOpen(false)}
            />
          )}
`,
'',
'mask delete')

// 4. aside 恢复固定 200px
rep(
"          <aside className={`absolute left-0 top-0 bottom-0 w-[260px] md:w-[180px] lg:w-[200px] flex-col z-30 bg-[var(--bg-app)] shadow-xl md:shadow-none transition-transform ${sidebarOpen ? 'flex' : 'hidden'}`}>",
"          <aside className={`absolute left-0 top-0 bottom-0 w-[200px] flex-col z-10 ${sidebarOpen ? 'flex' : 'hidden'}`}>",
'aside restore')

// 5. 抽屉 X 删除
rep(
`            <button
              onClick={() => setSidebarOpen(false)}
              className="md:hidden absolute top-3 right-3 w-7 h-7 flex items-center justify-center rounded text-[var(--text-muted)] hover:text-[var(--text-main)] hover:bg-[var(--bg-hover)]"
              aria-label="close sidebar"
              data-testid="sidebar-close-drawer"
            >
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="w-4 h-4">
                <path d="M6 6l12 12M18 6L6 18" strokeLinecap="round" />
              </svg>
            </button>
`,
'',
'drawer X delete')

// 6. main padding 恢复（保留 sidebarOpen 折叠逻辑）
rep(
"<main className={`flex-1 flex flex-col min-w-0 px-3 sm:px-4 md:pr-4 lg:pr-[220px] transition-all ${sidebarOpen ? 'md:pl-[200px] lg:pl-[220px]' : 'md:pl-6 lg:pl-6'}`}>",
"<main className={`flex-1 flex flex-col min-w-0 transition-all ${sidebarOpen ? 'px-[220px]' : 'pl-6 pr-[220px]'}`}>",
'main restore')

// 7. 成员列表 hidden lg:flex → flex
rep(
'<aside className="absolute right-0 top-0 bottom-0 w-[200px] flex-col z-10 hidden lg:flex">',
'<aside className="absolute right-0 top-0 bottom-0 w-[200px] flex-col z-10 flex">',
'members visible')

// 8. 顶层容器
rep(
'<div className="flex-1 flex flex-col min-h-0 px-8 md:px-12 lg:px-16 pt-8 md:pt-12 pb-4">',
'<div className="flex-1 flex flex-col min-h-0 px-16 pt-12 pb-4">',
'top container')

// 9. header
rep(
'<header className="relative h-[40px] shrink-0 flex items-center gap-2 md:gap-4 mb-4 md:mb-10">',
'<header className="relative h-[40px] shrink-0 flex items-center gap-4 mb-10">',
'header')

// 10. logo
rep(
'<div className="hidden sm:flex shrink-0 items-center text-[15px] md:text-[16px] font-bold tracking-wide">',
'<div className="flex shrink-0 items-center text-[16px] font-bold tracking-wide">',
'logo')

// 11. search
rep(
'<div className="flex-1 min-w-0 flex justify-center px-1 sm:px-2">',
'<div className="flex-1 min-w-0 flex justify-center px-2">',
'search')

// 12. icons wrapper
rep(
'<div className="flex items-center gap-1 sm:gap-2 shrink-0">',
'<div className="flex items-center gap-2 shrink-0">',
'icons wrapper')

// 13. desktop toggle: hidden md:flex → flex
rep(
"className={`hidden md:flex w-8 h-8 items-center justify-center rounded-md transition-colors ${sidebarOpen ? 'text-[var(--text-main)] bg-[var(--bg-hover)]' : 'text-[var(--text-muted)] hover:text-[var(--text-main)] hover:bg-[var(--bg-hover)]'}`}",
"className={`flex w-8 h-8 items-center justify-center rounded-md transition-colors ${sidebarOpen ? 'text-[var(--text-main)] bg-[var(--bg-hover)]' : 'text-[var(--text-muted)] hover:text-[var(--text-main)] hover:bg-[var(--bg-hover)]'}`}",
'desktop toggle visible')

// 14. title wrapper
rep(
'<div className="shrink-0 flex flex-col sm:flex-row sm:items-end sm:justify-between gap-3 mb-4 md:mb-8 mt-0 w-full">',
'<div className="shrink-0 flex items-end justify-between mb-8 mt-0 w-full">',
'title wrapper')

// 15. h1
rep(
'<h1 className="text-2xl md:text-4xl font-extrabold tracking-tight mb-1 md:mb-3 text-[var(--text-main)] truncate">',
'<h1 className="text-4xl font-extrabold tracking-tight mb-3 text-[var(--text-main)]">',
'h1')

// 16. footer wrapper
rep(
'<div className="px-2 sm:px-4 md:px-8 lg:px-16 h-[72px] flex items-center gap-2 sm:gap-4 transition-colors">',
'<div className="px-16 h-[72px] flex items-center relative transition-colors">',
'footer wrapper')

// 17. footer user block
rep(
'<div className="shrink-0 flex items-center gap-2 sm:gap-3 min-w-0">',
'<div className="w-[200px] shrink-0 flex items-center gap-3">',
'footer user block')

// 18. footer user text
rep(
'<div className="hidden sm:flex flex-col min-w-0">',
'<div className="flex flex-col">',
'footer user text')

// 19. footer nav: 恢复 absolute 居中
rep(
'<div className="flex-1 flex items-center justify-center gap-2 sm:gap-4 md:gap-8 text-[11px] sm:text-[12px] md:text-[13px] font-medium text-[var(--text-muted)] overflow-x-auto whitespace-nowrap px-1">',
'<div className="absolute left-1/2 -translate-x-1/2 flex items-center gap-8 text-[13px] font-medium text-[var(--text-muted)]">',
'footer nav')

// 20. footer version
rep(
'<div className="hidden md:block shrink-0 text-[11px] text-[var(--text-muted)] font-mono">',
'<div className="ml-auto w-[200px] text-right text-[11px] text-[var(--text-muted)] font-mono">',
'footer version')

// 21. 频道点击去掉 setSidebarOpen
s = s.replace(/onClick=\{\(\) => \{ setChannel\(ch\.id\); setActiveView\('chat'\); setSidebarOpen\(false\) \}\}/g, "onClick={() => { setChannel(ch.id); setActiveView('chat') }}")
s = s.replace(/onClick=\{\(\) => \{ setChannel\(vc\.id\); setActiveView\('chat'\); setSidebarOpen\(false\) \}\}/g, "onClick={() => { setChannel(vc.id); setActiveView('chat') }}")

// 22. footer nav 去掉 setSidebarOpen(false)
s = s.replace(/onClick=\{\(\) => \{ setActiveView\('(\w+)'\); setSidebarOpen\(false\) \}\}/g, "onClick={() => setActiveView('$1')}")

fs.writeFileSync('src/components/MainView.tsx', s)
console.log('DONE')
