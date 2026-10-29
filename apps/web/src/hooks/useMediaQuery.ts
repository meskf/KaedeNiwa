import { useSyncExternalStore } from 'react'

// SSR-safe media query hook。
// 旧代码直接 useState(window.innerWidth >= 768) —— 只在 mount 判断一次，
// 旋转屏幕 / 拖窗口不会重算，也没有 matchMedia 监听。本 hook 解决这个。
export function useMediaQuery(query: string): boolean {
  const subscribe = (cb: () => void) => {
    if (typeof window === 'undefined' || !window.matchMedia) return () => {}
    const mql = window.matchMedia(query)
    mql.addEventListener('change', cb)
    return () => mql.removeEventListener('change', cb)
  }
  const getSnapshot = () => {
    if (typeof window === 'undefined' || !window.matchMedia) return false
    return window.matchMedia(query).matches
  }
  const getServerSnapshot = () => false
  return useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot)
}

// KaedeNiwa 三档断点（与 Tailwind 默认一致）：
//   mobile  : <  768px
//   tablet  : 768  - 1023px
//   desktop : >= 1024px
// 现有代码只有 md (768) 一档，平板被当桌面 —— 左右各 200px 侧栏 + 900px 主区
// 需要 ~1300px，1024-1300 区间主区被压成窄条。
export const BP = {
  md: '(min-width: 768px)',
  lg: '(min-width: 1024px)',
  xl: '(min-width: 1280px)',
  // 关键：hover 能力，用于替代 :hover 在触屏上粘滞
  canHover: '(hover: hover) and (pointer: fine)',
  // 键盘弹起时 visualViewport 高度 < 80% 屏幕
  isTouch: '(pointer: coarse)',
} as const
