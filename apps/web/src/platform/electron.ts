import type { IPlatform, SaveFileOptions, PickFilesOptions } from './types'

/**
 * Electron 环境实现。依赖 preload 通过 contextBridge 注入的 window.electronAPI。
 *
 * preload 暴露的接口（见 apps/desktop/src/preload.cjs）：
 * - appVersion(): Promise<string>
 * - openExternal(url): Promise<void>
 * - copyText(text): Promise<void>
 * - readText(): Promise<string>
 * - saveFile({ name, data, mime }): Promise<{ canceled, path?, bytes? }>
 *
 * pickFiles 目前用 Web 的 <input type=file>（Electron 里也能用，只是
 * 弹的是 Chromium 原生 picker，与系统 picker 视觉略有差异）。后续可加
 * IPC 走 dialog.showOpenDialog。
 */
interface ElectronAPI {
  platform: 'electron'
  appVersion(): Promise<string>
  openExternal(url: string): Promise<void>
  copyText(text: string): Promise<void>
  readText(): Promise<string>
  saveFile(opts: { name: string; data: Uint8Array; mime?: string }): Promise<{ canceled: boolean; path?: string; bytes?: number }>
}

declare global {
  interface Window {
    electronAPI?: ElectronAPI
  }
}

export const electronPlatform: IPlatform = {
  kind: 'electron',

  capabilities: {
    share: false, // 暂不接系统分享（Windows 上 Electron 支持有限）
    notifications: true, // Electron 支持 Notification
    backgroundAudio: true, // 桌面端无 iOS 后台限制
    nativeFilePicker: false, // pickFiles 暂时走 <input type=file>
  },

  async openExternal(url) {
    const api = window.electronAPI
    if (!api) throw new Error('electronAPI not available')
    await api.openExternal(url)
  },

  async copyText(text) {
    const api = window.electronAPI
    if (!api) throw new Error('electronAPI not available')
    await api.copyText(text)
  },

  async readText() {
    const api = window.electronAPI
    if (!api) throw new Error('electronAPI not available')
    return api.readText()
  },

  async saveFile({ name, data, mime }: SaveFileOptions) {
    const api = window.electronAPI
    if (!api) throw new Error('electronAPI not available')
    // 用户取消保存对话框时静默返回（不抛错）
    await api.saveFile({ name, data, mime })
  },

  async pickFiles(opts?: PickFilesOptions) {
    // Web fallback：<input type=file>（Electron 里也会弹 Chromium picker）
    return new Promise<File[]>((resolve) => {
      const input = document.createElement('input')
      input.type = 'file'
      if (opts?.accept) input.accept = opts.accept
      if (opts?.multiple) input.multiple = true
      input.style.display = 'none'
      document.body.appendChild(input)
      input.onchange = () => {
        const files = input.files ? Array.from(input.files) : []
        document.body.removeChild(input)
        resolve(files)
      }
      input.oncancel = () => {
        document.body.removeChild(input)
        resolve([])
      }
      input.click()
    })
  },

  getAppVersion() {
    // preload 已同步注入 platform 标记，appVersion 需要异步，这里返回占位
    // 真实调用方式：await window.electronAPI.appVersion()
    // 但 IPlatform 接口是同步的，为了兼容性用 import.meta.env 版本号
    return (import.meta as any).env?.VITE_APP_VERSION || '0.0.5-electron'
  },
}
