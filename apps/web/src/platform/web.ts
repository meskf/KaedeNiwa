import type { IPlatform, SaveFileOptions, PickFilesOptions } from './types'

/**
 * Web 实现：所有能力用浏览器标准 API fallback。
 * Electron / Capacitor 只覆盖真正需要原生差异的方法。
 */
export const webPlatform: IPlatform = {
  kind: 'web',

  capabilities: {
    share: typeof navigator !== 'undefined' && typeof (navigator as any).share === 'function',
    notifications: typeof window !== 'undefined' && 'Notification' in window,
    backgroundAudio: false,
    nativeFilePicker: false,
  },

  async openExternal(url) {
    window.open(url, '_blank', 'noopener,noreferrer')
  },

  async copyText(text) {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text)
      return
    }
    // Fallback for insecure context (http://ip:5173 in LAN testing)
    const ta = document.createElement('textarea')
    ta.value = text
    ta.style.position = 'fixed'
    ta.style.opacity = '0'
    document.body.appendChild(ta)
    ta.select()
    try {
      document.execCommand('copy')
    } finally {
      document.body.removeChild(ta)
    }
  },

  async readText() {
    if (navigator.clipboard?.readText) {
      try { return await navigator.clipboard.readText() } catch { return '' }
    }
    return ''
  },

  async saveFile({ name, data, mime }: SaveFileOptions) {
    // TS 5.7+ 起 Uint8Array 被泛型化为 Uint8Array<ArrayBufferLike>，
    // 而 BlobPart 要求 ArrayBufferView<ArrayBuffer>。运行时 Blob 接受任何
    // ArrayBufferView，此处断言仅为绕过类型检查（SharedArrayBuffer 场景
    // 不会出现在附件下载路径）。
    const blob = new Blob([data as BlobPart], { type: mime || 'application/octet-stream' })
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = name
    document.body.appendChild(a)
    a.click()
    document.body.removeChild(a)
    setTimeout(() => URL.revokeObjectURL(url), 1000)
  },

  async pickFiles(opts?: PickFilesOptions) {
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
    return (import.meta as any).env?.VITE_APP_VERSION || 'dev'
  },
}
