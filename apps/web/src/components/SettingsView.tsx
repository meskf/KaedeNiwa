import { useState } from 'react'
import { useAuthStore } from '../stores/auth'
import { useChatStore } from '../stores/chat'
import { useI18n } from '../stores/i18n'
import { useTheme } from '../lib/useTheme'
import { IconSun, IconMoon } from './icons'

export function SettingsView() {
  const { loginId, serverUrl } = useAuthStore()
  const logout = useAuthStore(s => s.logout)
  const setActiveView = useChatStore(s => s.setActiveView)
  const { t, lang, setLang } = useI18n()
  const { theme, setTheme } = useTheme()
  const [langOpen, setLangOpen] = useState(false)
  const [dynamicBg, setDynamicBg] = useState(() => localStorage.getItem('kaede-dynamic-bg') === 'true')

  const toggleDynamicBg = () => {
    const next = !dynamicBg
    setDynamicBg(next)
    localStorage.setItem('kaede-dynamic-bg', String(next))
    window.dispatchEvent(new Event('kaede-dynamic-bg-changed'))
  }


  const [stageManagerMode, setStageManagerMode] = useState(() => {
    return localStorage.getItem('kaede-stage-manager-mode') || 'auto'
  })

  const changeStageManagerMode = (m: string) => {
    setStageManagerMode(m)
    localStorage.setItem('kaede-stage-manager-mode', m)
    window.dispatchEvent(new Event('kaede-stage-manager-changed'))
  }


  return (
    <div className="flex flex-col h-full w-full max-w-[700px] mx-auto overflow-y-auto">
      <div className="mb-8 mt-1">
        <h1 className="text-3xl font-extrabold tracking-tight text-[var(--text-main)]">{t('settings.title')}</h1>
        <p className="text-[13px] text-[var(--text-muted)] mt-1">{t('settings.subtitle')}</p>
      </div>

      <section className="mb-8">
        <h2 className="text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase mb-3">{t('settings.accountTitle')}</h2>
        <div className="rounded-lg bg-[var(--bg-hover)] divide-y divide-[var(--bg-active)]">
          <div className="flex items-center justify-between px-4 py-3">
            <span className="text-[13px] text-[var(--text-muted)]">{t('settings.accountLoginId')}</span>
            <span className="text-[14px] font-semibold text-[var(--text-main)]" data-testid="settings-login-id">{loginId || '—'}</span>
          </div>
          <div className="flex items-center justify-between px-4 py-3">
            <span className="text-[13px] text-[var(--text-muted)]">{t('settings.accountServer')}</span>
            <span className="text-[13px] text-[var(--text-main)] truncate max-w-[300px]">{serverUrl || '—'}</span>
          </div>
          <button
            onClick={() => { logout(); setActiveView('chat') }}
            className="w-full text-left px-4 py-3 text-[14px] text-red-500 hover:bg-red-500/5 transition-colors"
            data-testid="settings-logout"
          >
            {t('settings.accountLogout')}
          </button>
        </div>
      </section>

      <section className="mb-8">
        <h2 className="text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase mb-3">{t('settings.appearanceTitle')}</h2>
        <div className="rounded-lg bg-[var(--bg-hover)] divide-y divide-[var(--bg-active)]">
          <div className="flex items-center justify-between px-4 py-3">
            <span className="text-[13px] text-[var(--text-muted)]">{t('settings.appearanceTheme')}</span>
            <div className="flex items-center gap-1 bg-[var(--bg-active)] rounded-md p-0.5">
              <button
                onClick={() => setTheme('light')}
                className={`px-3 py-1 text-[12px] rounded transition-colors flex items-center gap-1.5 ${theme === 'light' ? 'bg-[var(--bg-app)] text-[var(--text-main)]' : 'text-[var(--text-muted)] hover:text-[var(--text-main)]'}`}
                data-testid="settings-theme-light"
              >
                <IconSun className="w-3.5 h-3.5" />{t('settings.themeLight')}
              </button>
              <button
                onClick={() => setTheme('dark')}
                className={`px-3 py-1 text-[12px] rounded transition-colors flex items-center gap-1.5 ${theme === 'dark' ? 'bg-[var(--bg-app)] text-[var(--text-main)]' : 'text-[var(--text-muted)] hover:text-[var(--text-main)]'}`}
                data-testid="settings-theme-dark"
              >
                <IconMoon className="w-3.5 h-3.5" />{t('settings.themeDark')}
              </button>
            </div>
          </div>
          <div className="flex items-center justify-between px-4 py-3 border-t border-[var(--bg-active)]">
            <span className="text-[13px] text-[var(--text-muted)]">Dynamic Background (Prism)</span>
            <button 
              onClick={toggleDynamicBg}
              className={`w-10 h-5 rounded-full transition-colors relative ${dynamicBg ? 'bg-green-500' : 'bg-[var(--bg-active)]'}`}
            >
              <div className={`absolute top-0.5 w-4 h-4 rounded-full bg-white shadow-sm transition-transform ${dynamicBg ? 'left-[22px]' : 'left-0.5'}`} />
            </button>
          </div>
        </div>
      </section>

      
      <section className="mb-8">
        <h2 className="text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase mb-3">Experimental</h2>
        <div className="rounded-lg bg-[var(--bg-hover)] divide-y divide-[var(--bg-active)]">
          <div className="flex items-center justify-between px-4 py-3">
            <span className="text-[13px] text-[var(--text-muted)]">Stage Manager (Servers)</span>
            <div className="flex items-center gap-1 bg-[var(--bg-active)] rounded-md p-0.5">
              <button
                onClick={() => changeStageManagerMode('off')}
                className={`px-3 py-1 text-[12px] rounded transition-colors ${stageManagerMode === 'off' ? 'bg-[var(--bg-app)] text-[var(--text-main)] font-semibold' : 'text-[var(--text-muted)] hover:text-[var(--text-main)]'}`}
              >
                Off
              </button>
              <button
                onClick={() => changeStageManagerMode('auto')}
                className={`px-3 py-1 text-[12px] rounded transition-colors ${stageManagerMode === 'auto' ? 'bg-[var(--bg-app)] text-[var(--text-main)] font-semibold' : 'text-[var(--text-muted)] hover:text-[var(--text-main)]'}`}
              >
                Auto
              </button>
              <button
                onClick={() => changeStageManagerMode('fixed')}
                className={`px-3 py-1 text-[12px] rounded transition-colors ${stageManagerMode === 'fixed' ? 'bg-[var(--bg-app)] text-[var(--text-main)] font-semibold' : 'text-[var(--text-muted)] hover:text-[var(--text-main)]'}`}
              >
                Fixed
              </button>
            </div>
          </div>
        </div>
      </section>

      <section className="mb-8">
        <h2 className="text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase mb-3">{t('settings.languageTitle')}</h2>
        <div className="rounded-lg bg-[var(--bg-hover)]">
          <div className="flex items-center justify-between px-4 py-3 relative">
            <span className="text-[13px] text-[var(--text-muted)]">{t('settings.languageLabel')}</span>
            <button
              onClick={() => setLangOpen(v => !v)}
              className="px-3 py-1.5 text-[13px] rounded-md bg-[var(--bg-active)] text-[var(--text-main)] hover:bg-[var(--text-muted)] transition-colors"
              data-testid="settings-lang-button"
            >
              {lang === 'en' ? 'English' : '中文'}
            </button>
            {langOpen && (
              <div className="absolute top-full right-0 mt-1 w-[140px] rounded-md bg-[var(--bg-app)] border border-[var(--bg-active)] shadow-lg z-20 py-1">
                <button
                  onClick={() => { setLang('en'); setLangOpen(false) }}
                  className={`w-full text-left px-3 py-2 text-[13px] hover:bg-[var(--bg-hover)] transition-colors ${lang === 'en' ? 'text-[var(--text-main)] font-semibold' : 'text-[var(--text-muted)]'}`}
                  data-testid="settings-lang-en"
                >
                  English
                </button>
                <button
                  onClick={() => { setLang('zh'); setLangOpen(false) }}
                  className={`w-full text-left px-3 py-2 text-[13px] hover:bg-[var(--bg-hover)] transition-colors ${lang === 'zh' ? 'text-[var(--text-main)] font-semibold' : 'text-[var(--text-muted)]'}`}
                  data-testid="settings-lang-zh"
                >
                  中文
                </button>
              </div>
            )}
          </div>
        </div>
      </section>
    </div>
  )
}
