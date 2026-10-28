import { useState, useEffect, useRef } from 'react'
import { getClient } from '../wasm/client'
import { useAuthStore } from '../stores/auth'
import { useMembersStore } from '../stores/members'
import { useServerStore } from '../stores/server'
import { useProfileStore } from '../stores/profile'
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

  // D1: server profile editing
  const token = useAuthStore(s => s.token)
  const profile = useServerStore(s => s.profile)
  const updateServer = useServerStore(s => s.update)
  const [profileName, setProfileName] = useState(profile.name)
  const [profileDesc, setProfileDesc] = useState(profile.description)
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<'ok' | 'forbidden' | 'error' | ''>('')

  useEffect(() => {
    setProfileName(profile.name)
    setProfileDesc(profile.description)
  }, [profile.name, profile.description])

  const dirty = profileName !== profile.name || profileDesc !== profile.description

  // B: members 管理
  const members = useMembersStore(s => s.members)
  const membersLoading = useMembersStore(s => s.loading)
  const fetchMembers = useMembersStore(s => s.fetch)
  const updateOp = useMembersStore(s => s.updateOp)
  const [savingOp, setSavingOp] = useState<string>('')
  const [opError, setOpError] = useState<string>('')

  useEffect(() => {
    fetchMembers(token)
  }, [token])

  const myMember = members.find(m => m.login_id === loginId)
  const myLevel = myMember ? parseInt(myMember.op.replace('op', ''), 10) : 0

  const handleOpChange = async (targetId: string, newOp: string) => {
    if (savingOp) return
    setSavingOp(targetId)
    setOpError('')
    const r = await updateOp(token, targetId, newOp)
    setSavingOp('')
    if (!r.ok) {
      setOpError(r.error === 'forbidden'
        ? (lang === 'zh' ? '权限不足（需 op4+ 且只能改比自己级别低的人）' : 'Insufficient permission')
        : (r.error || 'failed'))
      setTimeout(() => setOpError(''), 4000)
    }
  }

  const handleSaveServer = async () => {
    if (!dirty || saving) return
    setSaving(true)
    setSaveStatus('')
    const r = await updateServer(token, { name: profileName.trim(), description: profileDesc.trim() })
    setSaving(false)
    if (r.ok) {
      setSaveStatus('ok')
      setTimeout(() => setSaveStatus(''), 2000)
    } else if (r.error === 'forbidden') {
      setSaveStatus('forbidden')
    } else {
      setSaveStatus('error')
    }
  }

  const toggleDynamicBg = () => {
    const next = !dynamicBg
    setDynamicBg(next)
    localStorage.setItem('kaede-dynamic-bg', String(next))
    window.dispatchEvent(new Event('kaede-dynamic-bg-changed'))
  }


  const [stageManagerMode, setStageManagerMode] = useState(() => {
    return localStorage.getItem('kaede-stage-manager-mode') || 'auto'
  })

  // ============ 个人资料（头像/昵称/简介/称号）============
  const me = useProfileStore(s => s.me)
  const fetchMe = useProfileStore(s => s.fetchMe)
  const updateMe = useProfileStore(s => s.updateMe)
  const uploadAvatar = useProfileStore(s => s.uploadAvatar)
  const [nicknameDraft, setNicknameDraft] = useState(me.nickname)
  const [bioDraft, setBioDraft] = useState(me.bio)
  const [titleDraft, setTitleDraft] = useState(me.title)
  const [savingProfile, setSavingProfile] = useState(false)
  const [profileSaveStatus, setProfileSaveStatus] = useState<'ok' | 'error' | ''>('')
  const [uploadingAvatar, setUploadingAvatar] = useState(false)
  const avatarInputRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    fetchMe(token)
  }, [token])

  useEffect(() => {
    setNicknameDraft(me.nickname)
    setBioDraft(me.bio)
    setTitleDraft(me.title)
  }, [me.nickname, me.bio, me.title])

  const profileDirty = nicknameDraft !== me.nickname || bioDraft !== me.bio || titleDraft !== me.title

  const handleSaveProfile = async () => {
    if (!profileDirty || savingProfile) return
    setSavingProfile(true)
    setProfileSaveStatus('')
    const r = await updateMe(token, {
      nickname: nicknameDraft.trim(),
      bio: bioDraft.trim(),
      title: titleDraft.trim(),
    })
    setSavingProfile(false)
    if (r.ok) {
      setProfileSaveStatus('ok')
      setTimeout(() => setProfileSaveStatus(''), 2000)
      fetchMembers(token)
    } else {
      setProfileSaveStatus('error')
    }
  }

  
  

  const handleUploadAvatar = async (file: File) => {
    if (uploadingAvatar) return
    setUploadingAvatar(true)
    const r = await uploadAvatar(token, file)
    setUploadingAvatar(false)
    if (r.ok) {
      useProfileStore.getState().fetchMe(token).then(() => {
        useProfileStore.setState(s => ({ me: { ...s.me } })) // 强制触发 React 渲染
      })
    }
    if (r.ok) {
      const newMe = useProfileStore.getState().me
      useProfileStore.setState({ me: { ...newMe, avatarUrl: newMe.avatarUrl + '?t=' + Date.now() } }) // 强制变化触发渲染
    }
    if (!r.ok) {
      alert(r.error || 'avatar upload failed')
    } else {
      // 强制刷新全局状态
      useProfileStore.getState().fetchMe(token); setTimeout(() => window.location.reload(), 300); setTimeout(() => window.location.reload(), 500)
    }
  }

  // ============ 服务器图标 ============
  const uploadIcon = useServerStore(s => s.uploadIcon)
  const [uploadingIcon, setUploadingIcon] = useState(false)
  const iconInputRef = useRef<HTMLInputElement>(null)

  const handleUploadIcon = async (file: File) => {
    if (uploadingIcon) return
    setUploadingIcon(true)
    const r = await uploadIcon(token, file)
    setUploadingIcon(false)
    if (r.ok) {
      useServerStore.getState().fetch(token).then(() => {
        useServerStore.setState(s => ({ profile: { ...s.profile } })) // 强制触发 React 渲染
      })
    }
    if (r.ok) {
      const newProfile = useServerStore.getState().profile
      useServerStore.setState({ profile: { ...newProfile, icon_url: newProfile.icon_url + '?t=' + Date.now() } }) // 强制变化触发渲染
    }
    if (!r.ok) {
      alert(r.error || 'icon upload failed')
    } else {
      // 强制刷新全局状态
      useServerStore.getState().fetch(token); setTimeout(() => window.location.reload(), 300); setTimeout(() => window.location.reload(), 500)
    }
  }


  const changeStageManagerMode = (m: string) => {
    setStageManagerMode(m)
    localStorage.setItem('kaede-stage-manager-mode', m)
    window.dispatchEvent(new Event('kaede-stage-manager-changed'))
  }


  const handleCreateRootNode = async () => {
    const name = window.prompt('输入新建根目录名称:');
    if (!name) return;
    const c = getClient();
    if (!c) return;
    try {
      await (c as any).createNode(name, 'folder', null, false, null);
      const res = await (c as any).listNodes();
      useChatStore.getState().setNodes(res.map((r: string) => JSON.parse(r)));
    } catch (e) { alert('创建失败: ' + String(e)); }
  };

  return (
    <div className="flex flex-col h-full w-full max-w-[700px] mx-auto overflow-y-auto">

    <div className="mb-8 p-4 bg-[var(--bg-hover)] rounded-lg">
      <h3 className="text-[14px] font-bold mb-2">频道目录管理</h3>
      <p className="text-[12px] text-[var(--text-muted)] mb-3">在这里新建顶级目录。</p>
      <button onClick={handleCreateRootNode} className="px-4 py-2 bg-[var(--text-main)] text-[var(--bg-app)] rounded font-semibold text-[13px] hover:opacity-90">
        + 新建根目录
      </button>
    </div>

      <div className="mb-8 mt-1">
        <h1 className="text-3xl font-extrabold tracking-tight text-[var(--text-main)]">{t('settings.title')}</h1>
        <p className="text-[13px] text-[var(--text-muted)] mt-1">{t('settings.subtitle')}</p>
      </div>

      <section className="mb-8">
        <h2 className="text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase mb-3">{t('settings.accountTitle')}</h2>
        <div className="rounded-lg bg-[var(--bg-hover)] overflow-hidden">
          <div className="flex items-start gap-4 px-4 py-4 border-b border-[var(--bg-active)]">
            <button
              type="button"
              onClick={() => avatarInputRef.current?.click()}
              disabled={uploadingAvatar}
              className="relative w-[72px] h-[72px] rounded-full overflow-hidden bg-[var(--bg-active)] flex items-center justify-center shrink-0 group"
              data-testid="settings-avatar-button"
              title={lang === 'zh' ? '点击更换头像' : 'Click to change avatar'}
            >
              {me.avatarUrl ? (
                <img src={`/api/avatars/${me.loginId}?t=${Date.now()}`} alt="avatar" className="w-full h-full object-cover" />
              ) : (
                <span className="text-[24px] font-bold text-[var(--text-muted)]">{(me.nickname || me.loginId || '?').slice(0, 1).toUpperCase()}</span>
              )}
              <span className="absolute inset-0 bg-black/40 opacity-0 group-hover:opacity-100 flex items-center justify-center text-white text-[11px] font-semibold transition-opacity">
                {uploadingAvatar ? '...' : (lang === 'zh' ? '更换' : 'Change')}
              </span>
            </button>
            <input
              ref={avatarInputRef}
              data-testid="settings-avatar-file"
              type="file"
              accept="image/*"
              className="hidden"
              onChange={(e) => {
                const f = e.target.files?.[0]
                if (f) handleUploadAvatar(f)
                e.target.value = ''
              }}
            />
            <div className="flex-1 min-w-0 flex flex-col gap-2">
              <div className="flex items-center gap-2">
                <span className="text-[12px] text-[var(--text-muted)] w-14 shrink-0">{lang === 'zh' ? '昵称' : 'Nickname'}</span>
                <input
                  type="text"
                  value={nicknameDraft}
                  onChange={(e) => setNicknameDraft(e.target.value)}
                  maxLength={32}
                  className="flex-1 bg-[var(--bg-active)] rounded px-3 py-1.5 text-[13px] text-[var(--text-main)] outline-none focus:ring-1 focus:ring-[var(--text-muted)]"
                  data-testid="settings-nickname-input"
                />
              </div>
              <div className="flex items-center gap-2">
                <span className="text-[12px] text-[var(--text-muted)] w-14 shrink-0">{lang === 'zh' ? '简介' : 'Bio'}</span>
                <input
                  type="text"
                  value={bioDraft}
                  onChange={(e) => setBioDraft(e.target.value)}
                  maxLength={256}
                  className="flex-1 bg-[var(--bg-active)] rounded px-3 py-1.5 text-[13px] text-[var(--text-main)] outline-none focus:ring-1 focus:ring-[var(--text-muted)]"
                  data-testid="settings-bio-input"
                />
              </div>
              <div className="flex items-center gap-2">
                <span className="text-[12px] text-[var(--text-muted)] w-14 shrink-0">{lang === 'zh' ? '称号' : 'Title'}</span>
                <input
                  type="text"
                  value={titleDraft}
                  onChange={(e) => setTitleDraft(e.target.value)}
                  maxLength={32}
                  className="flex-1 bg-[var(--bg-active)] rounded px-3 py-1.5 text-[13px] text-[var(--text-main)] outline-none focus:ring-1 focus:ring-[var(--text-muted)]"
                  data-testid="settings-title-input"
                />
              </div>
              <div className="flex items-center gap-2 mt-1">
                <span className={`text-[12px] ${profileSaveStatus === 'ok' ? 'text-green-500' : profileSaveStatus === 'error' ? 'text-red-500' : 'text-[var(--text-muted)]'}`}>
                  {profileSaveStatus === 'ok' ? (lang === 'zh' ? '已保存' : 'Saved') :
                   profileSaveStatus === 'error' ? (lang === 'zh' ? '保存失败' : 'Failed') :
                   (lang === 'zh' ? '修改后点击保存' : 'Edit and save')}
                </span>
                <div className="ml-auto">
                  <button
                    onClick={handleSaveProfile}
                    disabled={savingProfile || !profileDirty}
                    className="px-4 py-1.5 rounded bg-[var(--bg-app)] text-[12px] font-semibold text-[var(--text-main)] hover:opacity-80 disabled:opacity-40 transition-opacity"
                    data-testid="settings-profile-save"
                  >
                    {savingProfile ? (lang === 'zh' ? '保存中...' : 'Saving...') : (lang === 'zh' ? '保存' : 'Save')}
                  </button>
                </div>
              </div>
            </div>
          </div>
          <div className="flex items-center justify-between px-4 py-3 border-b border-[var(--bg-active)]">
            <span className="text-[13px] text-[var(--text-muted)]">{t('settings.accountLoginId')}</span>
            <span className="text-[14px] font-semibold text-[var(--text-main)]" data-testid="settings-login-id">{loginId || '—'}</span>
          </div>
          <div className="flex items-center justify-between px-4 py-3 border-b border-[var(--bg-active)]">
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
        <h2 className="text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase mb-3">{lang === 'zh' ? '服务器' : 'Server'}</h2>
        <div className="rounded-lg bg-[var(--bg-hover)] divide-y divide-[var(--bg-active)]">
          <div className="flex items-center justify-between px-4 py-3 gap-4">
            <span className="text-[13px] text-[var(--text-muted)] shrink-0">{lang === 'zh' ? '图标' : 'Icon'}</span>
            <div className="flex items-center gap-3 ml-auto">
              <button
                type="button"
                onClick={() => iconInputRef.current?.click()}
                disabled={uploadingIcon || myLevel < 3}
                className="w-[48px] h-[48px] rounded-[10px] overflow-hidden bg-[var(--bg-active)] flex items-center justify-center relative group disabled:opacity-50"
                data-testid="settings-server-icon-button"
                title={myLevel < 3 ? (lang === 'zh' ? '需要管理员权限' : 'Requires admin') : (lang === 'zh' ? '点击更换图标' : 'Click to change')}
              >
                {profile.icon_url ? (
                  <img src={`${profile.icon_url}?t=${Date.now()}`} alt="icon" className="w-full h-full object-cover" />
                ) : (
                  <span className="text-[16px] font-bold text-[var(--text-muted)]">{profile.name.slice(0, 1).toUpperCase()}</span>
                )}
                {myLevel >= 3 && (
                  <span className="absolute inset-0 bg-black/40 opacity-0 group-hover:opacity-100 flex items-center justify-center text-white text-[10px] font-semibold transition-opacity">
                    {uploadingIcon ? '...' : (lang === 'zh' ? '更换' : 'Change')}
                  </span>
                )}
              </button>
              <input
                ref={iconInputRef}
                data-testid="settings-server-icon-file"
                type="file"
                accept="image/*"
                className="hidden"
                onChange={(e) => {
                  const f = e.target.files?.[0]
                  if (f) handleUploadIcon(f)
                  e.target.value = ''
                }}
              />
            </div>
          </div>
          <div className="flex items-center justify-between px-4 py-3 gap-4">
            <span className="text-[13px] text-[var(--text-muted)] shrink-0">{lang === 'zh' ? '名称' : 'Name'}</span>
            <input
              type="text"
              value={profileName}
              onChange={(e) => setProfileName(e.target.value)}
              maxLength={64}
              className="flex-1 bg-[var(--bg-active)] rounded px-3 py-1.5 text-[13px] text-[var(--text-main)] outline-none focus:ring-1 focus:ring-[var(--text-muted)]"
              data-testid="settings-server-name"
            />
          </div>
          <div className="flex items-center justify-between px-4 py-3 gap-4">
            <span className="text-[13px] text-[var(--text-muted)] shrink-0">{lang === 'zh' ? '描述' : 'Description'}</span>
            <input
              type="text"
              value={profileDesc}
              onChange={(e) => setProfileDesc(e.target.value)}
              maxLength={128}
              className="flex-1 bg-[var(--bg-active)] rounded px-3 py-1.5 text-[13px] text-[var(--text-main)] outline-none focus:ring-1 focus:ring-[var(--text-muted)]"
              data-testid="settings-server-desc"
            />
          </div>
          <div className="flex items-center justify-between px-4 py-3 gap-4">
            <span className={`text-[12px] ${saveStatus === 'ok' ? 'text-green-500' : (saveStatus === 'forbidden' || saveStatus === 'error') ? 'text-red-500' : 'text-[var(--text-muted)]'}`}>
              {saveStatus === 'ok' ? (lang === 'zh' ? '已保存' : 'Saved') :
               saveStatus === 'forbidden' ? (lang === 'zh' ? '需要管理员权限（op3+）' : 'Requires admin (op3+)') :
               saveStatus === 'error' ? (lang === 'zh' ? '保存失败' : 'Failed') :
               (lang === 'zh' ? '修改后点击保存' : 'Edit and save')}
            </span>
            <button
              onClick={handleSaveServer}
              disabled={saving || !dirty}
              className="px-4 py-1.5 rounded bg-[var(--bg-app)] text-[12px] font-semibold text-[var(--text-main)] hover:opacity-80 disabled:opacity-40 transition-opacity"
              data-testid="settings-server-save"
            >
              {saving ? (lang === 'zh' ? '保存中...' : 'Saving...') : (lang === 'zh' ? '保存' : 'Save')}
            </button>
          </div>
        </div>
      </section>

      <section className="mb-8">
        <h2 className="text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase mb-3">{lang === 'zh' ? '成员' : 'Members'}</h2>
        <div className="rounded-lg bg-[var(--bg-hover)] divide-y divide-[var(--bg-active)]">
          {membersLoading && (
            <div className="px-4 py-3 text-[12px] text-[var(--text-muted)]">{lang === 'zh' ? '加载中...' : 'Loading...'}</div>
          )}
          {!membersLoading && members.length === 0 && (
            <div className="px-4 py-3 text-[12px] text-[var(--text-muted)]">{lang === 'zh' ? '暂无成员' : 'No members'}</div>
          )}
          {members.map(m => {
            const isSelf = m.login_id === loginId
            const targetLevel = parseInt(m.op.replace('op', ''), 10)
            // 保护：op5 不可降级；不能改自己；只能改比自己级别低的；需 op4+
            const canEdit = myLevel >= 4 && !isSelf && m.op !== 'op5' && myLevel > targetLevel
            const isSaving = savingOp === m.login_id
            return (
              <div key={m.login_id} className="flex items-center justify-between px-4 py-3 gap-3" data-testid={`member-row-${m.login_id}`}>
                <div className="flex flex-col min-w-0">
                  <span className="text-[13px] font-semibold text-[var(--text-main)] truncate">{m.nickname || m.login_id}</span>
                  <span className="text-[11px] text-[var(--text-muted)] truncate">{m.login_id}{isSelf ? (lang === 'zh' ? ' (你)' : ' (you)') : ''}</span>
                </div>
                <select
                  value={m.op}
                  disabled={!canEdit || isSaving}
                  onChange={(e) => handleOpChange(m.login_id, e.target.value)}
                  className="bg-[var(--bg-active)] rounded px-2 py-1 text-[12px] text-[var(--text-main)] outline-none disabled:opacity-50 disabled:cursor-not-allowed"
                  data-testid={`member-op-${m.login_id}`}
                >
                  <option value="op1">{lang === 'zh' ? 'op1 访客' : 'op1 Guest'}</option>
                  <option value="op2">{lang === 'zh' ? 'op2 成员' : 'op2 Member'}</option>
                  <option value="op3">{lang === 'zh' ? 'op3 普管' : 'op3 Moderator'}</option>
                  <option value="op4">{lang === 'zh' ? 'op4 超管' : 'op4 Admin'}</option>
                  <option value="op5">{lang === 'zh' ? 'op5 服主' : 'op5 Owner'}</option>
                </select>
              </div>
            )
          })}
          {opError && (
            <div className="px-4 py-2 text-[12px] text-red-500" data-testid="member-op-error">{opError}</div>
          )}
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
