import { useAuthStore } from '../stores/auth'

/**
 * 服务端 API/静态资源的绝对地址。
 *
 * Web 端页面 origin 就是服务端，相对路径可用；但 Android(Capacitor, https://localhost)
 * 与桌面端(file://) 的 origin 指向本地 WebView 资源，相对路径会打到本地资源服务器，
 * 返回 index.html（JSON 解析失败 / 头像 404）。所有 /api 请求与 <img src> 都必须经过这里。
 */
export function apiUrl(path: string): string {
  if (!path || /^(https?:)?\/\//.test(path)) return path
  const base = (useAuthStore.getState().serverUrl || '').replace(/\/+$/, '')
  return base ? base + path : path
}
