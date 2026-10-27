export type Theme = 'light' | 'dark'
const KEY = 'kaede-theme'

export function getStoredTheme(): Theme {
  try {
    const v = localStorage.getItem(KEY)
    if (v === 'dark' || v === 'light') return v
  } catch {}
  return 'light'
}

export function applyTheme(t: Theme) {
  if (t === 'light') document.documentElement.setAttribute('data-theme', 'light')
  else document.documentElement.removeAttribute('data-theme')
  try { localStorage.setItem(KEY, t) } catch {}
}

// 模块加载即应用一次，保证首帧颜色与用户上次选择一致
applyTheme(getStoredTheme())
