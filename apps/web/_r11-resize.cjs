const fs = require('fs')
let s = fs.readFileSync('src/components/MainView.tsx', 'utf8')

// 1. 加 resize 监听（跨断点切换）
const old1 = "  const [sidebarOpen, setSidebarOpen] = useState(() => typeof window !== 'undefined' && window.innerWidth >= 768)"
const new1 = [
  "  const [sidebarOpen, setSidebarOpen] = useState(() => typeof window !== 'undefined' ? window.innerWidth >= 768 : true)",
  '  useEffect(() => {',
  "    let wasDesktop = typeof window !== 'undefined' && window.innerWidth >= 768",
  '    const onResize = () => {',
  "      const isDesktop = window.innerWidth >= 768",
  '      if (isDesktop !== wasDesktop) {',
  '        setSidebarOpen(isDesktop)',
  '        wasDesktop = isDesktop',
  '      }',
  '    }',
  "    window.addEventListener('resize', onResize)",
  "    return () => window.removeEventListener('resize', onResize)",
  '  }, [])'
].join('\n')
if (!s.includes(old1)) { console.error('FAIL state'); process.exit(1) }
s = s.replace(old1, new1)

fs.writeFileSync('src/components/MainView.tsx', s)
console.log('OK resize listener added')
