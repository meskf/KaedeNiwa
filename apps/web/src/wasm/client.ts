import init, { JsClient, initKeystore } from './pkg/kaede_client_wasm.js'
import { useAuthStore } from '../stores/auth'

let client: JsClient | null = null
let initPromise: Promise<JsClient> | null = null

type ReconnectState = 'idle' | 'reconnecting' | 'failed'

interface LoginArgs {
  serverUrl: string
  wsUrl: string
  loginId: string
  token: string
}

let reconnectState: ReconnectState = 'idle'
let reconnectAttempt = 0
let reconnectTimer: ReturnType<typeof setTimeout> | null = null
let reconnectEnabled = true
let lastLoginArgs: LoginArgs | null = null

const BACKOFF_MS = [1000, 2000, 4000, 8000, 16000, 30000]
const MAX_ATTEMPTS = BACKOFF_MS.length
const LOGIN_TIMEOUT_MS = 10000

export function initWasm() {
  if (!initPromise) {
    initPromise = (async () => {
      await init()
      await initKeystore()
      client = new JsClient()
      return client
    })()
  }
  return initPromise
}

export function getClient() {
  return client
}

/** P0-1: 记录登录参数用于自动重连 */
export function recordLoginArgs(args: LoginArgs) {
  lastLoginArgs = args
}

export function getReconnectState() {
  return { state: reconnectState, attempt: reconnectAttempt, max: MAX_ATTEMPTS }
}

/** P0-1: 收到 Disconnected 事件后调用，指数退避重连 */
export function scheduleReconnect() {
  if (!reconnectEnabled) {
    console.log('[reconnect] disabled (manual logout)')
    return
  }
  if (reconnectState === 'reconnecting') {
    console.log('[reconnect] already reconnecting, skip')
    return
  }
  if (!lastLoginArgs) {
    console.warn('[reconnect] no login args; cannot reconnect')
    return
  }
  console.log('[reconnect] schedule')
  reconnectState = 'reconnecting'
  reconnectAttempt = 0
  attemptReconnect()
}

function attemptReconnect() {
  if (!reconnectEnabled) return
  if (!lastLoginArgs) return
  if (reconnectAttempt >= MAX_ATTEMPTS) {
    reconnectState = 'failed'
    console.error('[reconnect] max attempts reached, signing out')
    // 不要停留在“界面还在、连接已死”的僵尸状态：清登录态回到登录页。
    void signOut()
    return
  }
  const delay = BACKOFF_MS[reconnectAttempt]
  reconnectAttempt++
  console.log('[reconnect] attempt ' + reconnectAttempt + '/' + MAX_ATTEMPTS + ' in ' + delay + 'ms')
  reconnectTimer = setTimeout(async () => {
    reconnectTimer = null
    if (!reconnectEnabled) return
    try {
      const c = getClient()
      if (!c) throw new Error('no client')
      const args = lastLoginArgs!
      await Promise.race([
        c.login(args.serverUrl, args.wsUrl, args.loginId, args.token),
        new Promise<never>((_, rej) =>
          setTimeout(() => rej(new Error('login timeout')), LOGIN_TIMEOUT_MS)
        ),
      ])
      console.log('[reconnect] success')
      reconnectState = 'idle'
      reconnectAttempt = 0
    } catch (e) {
      const msg = String(e)
      // 会话已被顶替/撤销/踢出：重试没有意义，直接回登录页。
      if (/401|unauthor|superseded|session|kicked/i.test(msg)) {
        console.warn('[reconnect] session no longer valid:', msg)
        void signOut()
        return
      }
      console.warn('[reconnect] attempt failed:', e)
      attemptReconnect()
    }
  }, delay)
}

/** P0-1: 手动登出/切换账号时调用，停止自动重连 */
export function stopReconnect() {
  console.log('[reconnect] stop (manual)')
  reconnectEnabled = false
  if (reconnectTimer) {
    clearTimeout(reconnectTimer)
    reconnectTimer = null
  }
  reconnectState = 'idle'
  reconnectAttempt = 0
}

/** P0-1: 重新登录后恢复自动重连 */
export function allowReconnect() {
  reconnectEnabled = true
}

/**
 * 登出：断开 WS + 撤销服务端会话 + 清本地登录态。
 *
 * 旧实现只清 localStorage（useAuthStore.logout），WS 仍保持连接：
 * 用户回到登录页后服务端仍认为其在线，还会继续推送消息，直到连接自然断开。
 */
export async function signOut() {
  const { serverUrl, token } = useAuthStore.getState()
  stopReconnect()
  lastLoginArgs = null
  try {
    const c = getClient()
    if (c) await (c as any).forceDisconnect()
  } catch (e) {
    console.warn('[signOut] forceDisconnect failed:', e)
  }
  if (serverUrl && token) {
    try {
      await fetch(serverUrl + '/api/logout', {
        method: 'POST',
        headers: { Authorization: `Bearer ${token}` },
      })
    } catch (e) {
      console.warn('[signOut] server logout failed:', e)
    }
  }
  useAuthStore.getState().logout()
}
