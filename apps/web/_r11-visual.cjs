const fs = require('fs')
let s = fs.readFileSync('src/components/MainView.tsx', 'utf8')

// ===== 1. Drawer 视觉加强：宽度 280 + bg-black/50 + 右侧圆角 + 强阴影 =====
const oldDrawer = "          <aside className={`absolute left-0 top-0 bottom-0 w-[260px] md:w-[180px] lg:w-[200px] flex-col z-30 bg-[var(--bg-app)] shadow-xl md:shadow-none transition-transform ${sidebarOpen ? 'flex' : 'hidden'}`}>"
const newDrawer = "          <aside className={`absolute left-0 top-0 bottom-0 w-[280px] md:w-[180px] lg:w-[200px] flex-col z-30 bg-[var(--bg-app)] shadow-2xl md:shadow-none rounded-r-xl md:rounded-r-none transition-transform duration-200 ${sidebarOpen ? 'flex translate-x-0' : 'hidden -translate-x-full md:hidden'}`}>"
if (!s.includes(oldDrawer)) { console.error('FAIL drawer'); process.exit(1) }
s = s.replace(oldDrawer, newDrawer)

// 遮罩加深 bg-black/30 -> bg-black/50
const oldMask = '<div\n              className="md:hidden fixed inset-0 top-[40px] bottom-[72px] bg-black/30 z-20"\n              onClick={() => setSidebarOpen(false)}\n            />'
const newMask = '<div\n              className="md:hidden fixed inset-0 top-[40px] bottom-[72px] bg-black/50 z-20 animate-in fade-in"\n              onClick={() => setSidebarOpen(false)}\n              data-testid="sidebar-mask"\n            />'
if (!s.includes(oldMask)) { console.error('FAIL mask'); process.exit(1) }
s = s.replace(oldMask, newMask)

// ===== 2. footer 手机对称：右侧加 invisible 占位 =====
const oldFooterVer = '<div className="hidden md:block shrink-0 text-[11px] text-[var(--text-muted)] font-mono">\n            v0.0.5'
const newFooterVer = '<div className="shrink-0 text-[11px] text-[var(--text-muted)] font-mono md:block hidden">\n            v0.0.5'
// 其实问题在：左侧用户区宽度 = 头像 28 + gap 8 + 隐藏的文字(0) = ~28px
// 右侧版本区 hidden md:block = 0px 时 nav 会占满剩余空间，居中后偏右
// 修法：右侧加 invisible 占位，宽 28px（和头像一致）
const oldFooterUserEnd = [
  '            <div className="hidden sm:flex flex-col min-w-0">',
  '              <span className="text-[13px] font-semibold leading-tight text-[var(--text-main)] truncate max-w-[120px]">{loginId}</span>',
  '              <button onClick={onLogout} className="text-[11px] text-[var(--text-muted)] hover:text-[var(--text-main)] text-left transition-colors mt-0.5">',
  "                {t('settings.accountLogout')}",
  '              </button>',
  '            </div>',
  '          </div>'
].join('\n')
const newFooterUserEnd = [
  '            <div className="hidden sm:flex flex-col min-w-0">',
  '              <span className="text-[13px] font-semibold leading-tight text-[var(--text-main)] truncate max-w-[120px]">{loginId}</span>',
  '              <button onClick={onLogout} className="text-[11px] text-[var(--text-muted)] hover:text-[var(--text-main)] text-left transition-colors mt-0.5">',
  "                {t('settings.accountLogout')}",
  '              </button>',
  '            </div>',
  '          </div>'
].join('\n')
// 不改 user 块；改右侧版本区域，在 md 以下加一个 w-7 的占位
const oldRight = [
  '<div className="shrink-0 text-[11px] text-[var(--text-muted)] font-mono md:block hidden">',
  '            v0.0.5',
  '          </div>'
].join('\n')
const newRight = [
  '<div className="shrink-0 text-[11px] text-[var(--text-muted)] font-mono hidden md:block">',
  '            v0.0.5',
  '          </div>',
  '          <div className="shrink-0 w-7 h-7 md:hidden" aria-hidden="true" />'
].join('\n')
if (!s.includes(oldRight)) { console.error('FAIL footer right'); process.exit(1) }
s = s.replace(oldRight, newRight)

fs.writeFileSync('src/components/MainView.tsx', s)
console.log('PATCHED drawer visual + footer symmetry')
