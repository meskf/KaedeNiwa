import init, { JsClient } from './pkg/kaede_client_wasm.js'

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
    console.error('[reconnect] max attempts reached, giving up')
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
