import { create } from 'zustand'
import { detectLang, translate, saveLang, type Lang } from '../lib/i18n'

interface I18nState {
  lang: Lang
  setLang: (l: Lang) => void
  t: (key: string) => string
}

export const useI18n = create<I18nState>((set, get) => ({
  lang: detectLang(),
  setLang: (lang) => { saveLang(lang); set({ lang }) },
  t: (key) => translate(get().lang, key),
}))
