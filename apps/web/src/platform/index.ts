import type { IPlatform } from './types'
import { webPlatform } from './web'

export type {
  IPlatform,
  PlatformKind,
  PlatformCapabilities,
  SaveFileOptions,
  PickFilesOptions,
} from './types'

/**
 * 平台探测 + 单例选择。
 *
 * 检测顺序（越晚的越具体）：
 *   1. Capacitor（原生应用内嵌 WebView）
 *   2. Electron（BrowserWindow 内的渲染进程）
 *   3. Web（兜底）
 *
 * 未来接入：
 *   - Electron: 在 electron.ts 里实现，依赖 preload 注入的 window.electronAPI
 *   - Capacitor: 在 capacitor.ts 里实现，依赖 @capacitor/* 插件
 * 那时只需要在这里加 import + 返回对应实例，业务代码零改动。
 */
function resolvePlatform(): IPlatform {
  if (typeof window !== 'undefined') {
    const w = window as any

    // 未来：if (w.Capacitor?.isNativePlatform?.()) return capacitorPlatform
    // 未来：if (w.electronAPI) return electronPlatform

    // 现在：Web 兜底
    void w
  }
  return webPlatform
}

export const platform: IPlatform = resolvePlatform()

export const isWeb = platform.kind === 'web'
export const isElectron = platform.kind === 'electron'
export const isCapacitor = platform.kind === 'capacitor'
