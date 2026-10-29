import { platform } from '../platform'

/**
 * 无边框 Electron 窗口的顶部拖拽条。
 *
 * - 高度 32px，紧贴窗口顶部，透明无内容
 * - `-webkit-app-region: drag` 允许按住拖窗口
 * - 双击触发最大化/还原（Electron 在 drag region 里对双击有原生处理）
 * - z-index 9998，低于 WindowControls（9999），右侧 138px 交给按钮
 * - 仅在 electron 环境渲染，web / e2e 零影响
 *
 * MainView 顶栏从窗口顶部 48px 开始，所以 32px 拖拽条完全落在白区上方，
 * 不遮挡任何交互元素（logo / 搜索框 / 侧栏按钮等）。
 */
export function TitleBarDragStrip() {
  if (platform.kind !== 'electron') return null
  const wc = (window as any).windowControls
  if (!wc) return null

  return (
    <div
      data-testid="titlebar-drag-strip"
      className="fixed top-0 left-0 right-0 h-8 z-[9998]"
      style={{ WebkitAppRegion: 'drag' } as React.CSSProperties}
      onDoubleClick={() => wc.toggleMaximize()}
    />
  )
}
