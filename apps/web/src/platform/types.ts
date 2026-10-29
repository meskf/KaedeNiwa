/**
 * KaedeNiwa 平台抽象层：统一 Web / Electron / Capacitor 的能力差异。
 *
 * 只暴露真正需要跨平台的能力（文件、剪贴板、外链、原生 UI）。
 * 业务代码一律通过 `platform.xxx()` 调用，禁止直接 `navigator.clipboard` / `window.open`。
 *
 * 新增能力的原则：
 * - 若 Web 已有可用 API，就走 web.ts fallback，Electron/Capacitor 按需覆盖
 * - 若某能力在某些平台上不可用，通过 `capabilities` 探测，调用方应降级
 */

export type PlatformKind = 'web' | 'electron' | 'capacitor'

export interface PlatformCapabilities {
  /** 系统分享面板（Web: navigator.share / Electron: shell.showItemInFolder / Capacitor: @capacitor/share） */
  share: boolean
  /** 系统通知 */
  notifications: boolean
  /** 后台音频（iOS 关键差异） */
  backgroundAudio: boolean
  /** 原生文件选择器（Web 用 <input type=file>，Electron/Capacitor 用原生） */
  nativeFilePicker: boolean
}

export interface SaveFileOptions {
  name: string
  data: Uint8Array
  mime?: string
}

export interface PickFilesOptions {
  accept?: string
  multiple?: boolean
}

export interface IPlatform {
  readonly kind: PlatformKind
  readonly capabilities: PlatformCapabilities

  /** 在外部浏览器/系统默认应用中打开 URL */
  openExternal(url: string): Promise<void>

  /** 复制文本到剪贴板 */
  copyText(text: string): Promise<void>

  /** 从剪贴板读取文本（可能返回空字符串） */
  readText(): Promise<string>

  /** 保存文件（附件下载）。Web: Blob + a.download；Electron: dialog.save；Capacitor: Filesystem */
  saveFile(opts: SaveFileOptions): Promise<void>

  /** 选择文件（附件上传）。Web: <input type=file>；Electron/Capacitor: 原生 */
  pickFiles(opts?: PickFilesOptions): Promise<File[]>

  /** 应用版本（用于关于页面、bug 上报） */
  getAppVersion(): string
}
