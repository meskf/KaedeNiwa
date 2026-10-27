const fs = require('fs')
const results = []
function rep(s, search, repl, name) {
  if (!s.includes(search)) { results.push('MISS: ' + name); return s }
  results.push('OK: ' + name)
  return s.replace(search, repl)
}

// ===== MainView =====
let s = fs.readFileSync('src/components/MainView.tsx', 'utf8')

s = rep(s, '<header className="relative h-[40px] shrink-0 flex items-center mb-10">',
  '<header className="relative h-[40px] shrink-0 flex items-center gap-2 md:gap-4 mb-4 md:mb-10">', 'header mb')

s = rep(s, 'className="md:hidden mr-3 w-8 h-8 flex items-center',
  'className="md:hidden shrink-0 w-8 h-8 flex items-center', 'hamburger shrink')

s = rep(s, '<div className="w-[200px] shrink-0 flex items-center text-[16px] font-bold tracking-wide">',
  '<div className="hidden sm:flex shrink-0 items-center text-[15px] md:text-[16px] font-bold tracking-wide">', 'logo hide-mobile')

s = rep(s, '<div className="absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 w-full max-w-[420px] px-4">\n            <div className="relative flex items-center">',
  '<div className="flex-1 min-w-0 flex justify-center px-1 sm:px-2">\n            <div className="relative flex items-center w-full max-w-[420px]">', 'search inline-flex')

s = rep(s, '          <div className="flex-1" />\n          <div className="flex items-center gap-2 shrink-0">',
  '          <div className="flex items-center gap-1 sm:gap-2 shrink-0">', 'icons shrink')

s = rep(s, '<div className="shrink-0 flex items-end justify-between mb-8 mt-0 w-full">',
  '<div className="shrink-0 flex flex-col sm:flex-row sm:items-end sm:justify-between gap-3 mb-4 md:mb-8 mt-0 w-full">', 'title wrapper')

s = rep(s, '<h1 className="text-4xl md:text-2xl font-extrabold tracking-tight mb-3 text-[var(--text-main)]">',
  '<h1 className="text-2xl md:text-4xl font-extrabold tracking-tight mb-1 md:mb-3 text-[var(--text-main)] truncate">', 'h1')

s = rep(s, '<div className="relative flex items-center gap-3 pb-2">',
  '<div className="flex items-center gap-3 shrink-0">', 'btn wrapper')

s = rep(s, 'bg-[var(--bg-hover)] hover:bg-[var(--bg-active)] transition-colors font-medium text-red-500"',
  'bg-[var(--bg-hover)] hover:bg-[var(--bg-active)] transition-colors font-medium text-red-500 whitespace-nowrap"', 'leave nowrap')

s = rep(s, 'bg-[var(--text-main)] text-[var(--bg-app)] hover:opacity-90 transition-opacity font-semibold"',
  'bg-[var(--text-main)] text-[var(--bg-app)] hover:opacity-90 transition-opacity font-semibold whitespace-nowrap"', 'join nowrap')

s = rep(s, '<div className="px-8 md:px-12 lg:px-16 h-[72px] flex items-center relative transition-colors">',
  '<div className="px-2 sm:px-4 md:px-8 lg:px-16 h-[72px] flex items-center gap-2 sm:gap-4 transition-colors">', 'footer wrapper')

s = rep(s, '<div className="w-[120px] sm:w-[160px] md:w-[200px] shrink-0 flex items-center gap-3">',
  '<div className="shrink-0 flex items-center gap-2 sm:gap-3 min-w-0">', 'footer user block')

s = rep(s, '<div className="flex flex-col">\n              <span className="text-[13px] font-semibold leading-tight text-[var(--text-main)]">{loginId}</span>',
  '<div className="hidden sm:flex flex-col min-w-0">\n              <span className="text-[13px] font-semibold leading-tight text-[var(--text-main)] truncate max-w-[120px]">{loginId}</span>', 'footer text')

s = rep(s, '<div className="absolute left-1/2 -translate-x-1/2 flex items-center gap-3 sm:gap-5 md:gap-8 text-[11px] sm:text-[12px] md:text-[13px] font-medium text-[var(--text-muted)]">',
  '<div className="flex-1 flex items-center justify-center gap-2 sm:gap-4 md:gap-8 text-[11px] sm:text-[12px] md:text-[13px] font-medium text-[var(--text-muted)] overflow-x-auto whitespace-nowrap px-1">', 'footer nav')

s = rep(s, '<div className="ml-auto w-[200px] text-right text-[11px] text-[var(--text-muted)] font-mono">\n            v0.0.5',
  '<div className="hidden md:block shrink-0 text-[11px] text-[var(--text-muted)] font-mono">\n            v0.0.5', 'footer version')

fs.writeFileSync('src/components/MainView.tsx', s)
console.log('--- MainView ---'); results.forEach(r => console.log('  ' + r))

// ===== AudioSettingsPanel -> centered modal =====
let a = fs.readFileSync('src/components/AudioSettingsPanel.tsx', 'utf8')
const aOpenOld = [
  '    <div',
  '      className="absolute right-0 bottom-full mb-2 w-[320px] rounded-lg bg-[var(--bg-app)] border border-[var(--bg-active)] shadow-xl z-30 py-2 text-[var(--text-main)]"',
  '      data-testid="audio-settings-panel"',
  '      onClick={(e) => e.stopPropagation()}',
  '    >'
].join('\n')
const aOpenNew = [
  '    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 px-4" onClick={onClose}>',
  '      <div',
  '        className="w-[360px] max-w-full max-h-[calc(100vh-4rem)] overflow-y-auto rounded-lg bg-[var(--bg-app)] border border-[var(--bg-active)] shadow-xl py-2 text-[var(--text-main)]"',
  '        data-testid="audio-settings-panel"',
  '        onClick={(e) => e.stopPropagation()}',
  '      >'
].join('\n')
if (!a.includes(aOpenOld)) { console.log('MISS: AudioPanel open') } else {
  a = a.replace(aOpenOld, aOpenNew)
  const closeAnchor = '      </div>\n    </div>\n  )\n}'
  const closeNew = '      </div>\n      </div>\n    </div>\n  )\n}'
  const idx = a.lastIndexOf(closeAnchor)
  if (idx === -1) { console.log('MISS: AudioPanel close') } else {
    a = a.slice(0, idx) + closeNew + a.slice(idx + closeAnchor.length)
    console.log('OK: AudioPanel -> modal')
  }
}
fs.writeFileSync('src/components/AudioSettingsPanel.tsx', a)

// ===== UserContextMenu -> centered modal =====
let u = fs.readFileSync('src/components/UserContextMenu.tsx', 'utf8')
const uOpenOld = [
  '    <div',
  '      ref={ref}',
  '      data-testid="user-context-menu"',
  '      className="fixed z-50 w-[200px] rounded-lg bg-[var(--bg-app)] border border-[var(--bg-active)] shadow-xl py-1 text-[var(--text-main)]"',
  '      style={{ left: x, top: y }}',
  '    >'
].join('\n')
const uOpenNew = [
  '    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 px-4" onClick={onClose}>',
  '      <div',
  '        ref={ref}',
  '        data-testid="user-context-menu"',
  '        className="w-[260px] max-w-full rounded-lg bg-[var(--bg-app)] border border-[var(--bg-active)] shadow-xl py-1 text-[var(--text-main)]"',
  '        onClick={(e) => e.stopPropagation()}',
  '      >'
].join('\n')
if (!u.includes(uOpenOld)) { console.log('MISS: UserMenu open') } else {
  u = u.replace(uOpenOld, uOpenNew)
  const sigOld = '  const { t } = useI18n()\n  const ref = useRef<HTMLDivElement>(null)'
  const sigNew = '  const { t } = useI18n()\n  void x; void y\n  const ref = useRef<HTMLDivElement>(null)'
  if (!u.includes(sigOld)) { console.log('MISS: UserMenu sig') } else {
    u = u.replace(sigOld, sigNew)
  }
  const uCloseOld = '    </div>\n  )\n}'
  const uCloseNew = '      </div>\n    </div>\n  )\n}'
  const idx = u.lastIndexOf(uCloseOld)
  if (idx === -1) { console.log('MISS: UserMenu close') } else {
    u = u.slice(0, idx) + uCloseNew + u.slice(idx + uCloseOld.length)
    console.log('OK: UserMenu -> modal')
  }
}
fs.writeFileSync('src/components/UserContextMenu.tsx', u)

console.log('DONE')
