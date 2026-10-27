import en from '../locales/en'
import zh from '../locales/zh'

export type Lang = 'en' | 'zh'
export type Dict = typeof en

export const DICTS: Record<Lang, any> = { en, zh }

const STORAGE_KEY = 'kaede-lang'

export function detectLang(): Lang {
  try {
    const stored = localStorage.getItem(STORAGE_KEY)
    if (stored === 'en' || stored === 'zh') return stored
  } catch {}
  const nav = typeof navigator !== 'undefined' ? navigator.language : 'en'
  return nav.toLowerCase().startsWith('zh') ? 'zh' : 'en'
}

export function saveLang(lang: Lang) {
  try { localStorage.setItem(STORAGE_KEY, lang) } catch {}
}

export function translate(lang: Lang, key: string): string {
  const parts = key.split('.')
  let cur: any = DICTS[lang]
  for (const p of parts) { if (cur == null) break; cur = cur[p] }
  if (typeof cur === 'string') return cur
  cur = DICTS.en
  for (const p of parts) { if (cur == null) return key; cur = cur[p] }
  return typeof cur === 'string' ? cur : key
}
