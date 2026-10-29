import { useSyncExternalStore } from 'react'

// 头像/图标 cache-bust 版本号。
// 旧实现 src={`/api/avatars/${uid}?t=${Date.now()}`} —— 每帧重渲染都发新请求，
// 浏览器缓存全废。改为全局版本号：上传成功后 bump 一次，所有 <img> 同时换新 URL。
let version = 0
const listeners = new Set<() => void>()

function emit() { for (const l of listeners) l() }
function subscribe(cb: () => void) {
  listeners.add(cb)
  return () => { listeners.delete(cb) }
}
function getSnapshot() { return version }

export function getAvatarVersion() { return version }
export function bumpAvatarVersion() {
  version = Date.now()
  emit()
}
export function useAvatarVersion() {
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot)
}
