import { Component, type ReactNode } from 'react'

interface Props { children: ReactNode }
interface State { error: Error | null }

// 捕获 wasm panic / 未捕获渲染异常，避免整页白屏。
// HANDOFF 里多次 ICE 与 panic 都表现为 #root 变空白，本组件让用户能一键恢复。
export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null }

  static getDerivedStateFromError(error: Error): State {
    return { error }
  }

  componentDidCatch(error: Error, info: unknown) {
    console.error('[ErrorBoundary]', error, info)
  }

  handleReload = () => {
    try { window.location.reload() } catch {}
  }

  render() {
    if (this.state.error) {
      return (
        <div className="h-full w-full flex flex-col items-center justify-center gap-4 p-6 bg-[var(--bg-app)] text-[var(--text-main)]">
          <div className="text-lg font-semibold">出错了 / Something went wrong</div>
          <pre className="text-[11px] text-[var(--text-muted)] max-w-[80vw] max-h-[40vh] overflow-auto whitespace-pre-wrap text-left">
{String(this.state.error)}
          </pre>
          <button
            onClick={this.handleReload}
            className="px-4 py-2 rounded bg-[var(--bg-active)] text-sm"
          >
            重新加载 / Reload
          </button>
        </div>
      )
    }
    return this.props.children
  }
}
