import './lib/theme'
import { createRoot } from 'react-dom/client'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import App from './App'
import { ErrorBoundary } from './components/ErrorBoundary'
import './index.css'

const qc = new QueryClient({
  defaultOptions: { queries: { retry: 1, staleTime: 30_000 } },
})

// 浏览器只在安全上下文（https 或 localhost）提供 crypto.subtle。用 http://<局域网IP>
// 打开时整个加密栈（密钥库/MLS/附件加密）都无法工作，wasm 只会抛出
// "Cannot read properties of undefined (reading 'generateKey')"。
// 这里提前拦下并给出可操作提示，而不是让用户面对一堆 wasm 栈。
if (!window.isSecureContext || !crypto?.subtle) {
  document.getElementById('root')!.innerHTML = `
    <div style="min-height:100vh;display:flex;align-items:center;justify-content:center;padding:24px;font-family:system-ui,sans-serif;background:#0b0d12;color:#e8eaf1">
      <div style="max-width:560px;line-height:1.7">
        <h1 style="font-size:20px;margin:0 0 12px">此地址不是安全上下文，加密功能无法启用</h1>
        <p style="margin:0 0 12px;opacity:.85">
          浏览器只允许 <code>https://</code> 或 <code>localhost</code> 使用 Web Crypto。当前地址是
          <code>${window.location.origin}</code>，密钥库与端到端加密无法初始化。
        </p>
        <p style="margin:0;opacity:.85">请任选其一：</p>
        <ul style="margin:8px 0 0;padding-left:20px;opacity:.85">
          <li>在同一台机器上用 <code>http://127.0.0.1:8080</code> 打开；</li>
          <li>给服务端配置域名与 HTTPS（Caddy 自动证书），用 <code>https://</code> 访问；</li>
          <li>桌面客户端 / Android 客户端不受此限制。</li>
        </ul>
      </div>
    </div>`
} else {
  createRoot(document.getElementById('root')!).render(
    <ErrorBoundary>
      <QueryClientProvider client={qc}>
        <App />
      </QueryClientProvider>
    </ErrorBoundary>,
  )
}
