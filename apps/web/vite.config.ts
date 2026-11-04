import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import basicSsl from '@vitejs/plugin-basic-ssl'

export default defineConfig(() => {
  const useSsl = process.env.KAEDE_HTTP !== '1'
  return {
    plugins: [
      react(),
      tailwindcss(),
      ...(useSsl ? [basicSsl()] : []),
    ],
    server: {
      host: '0.0.0.0',
      port: 5173,
      strictPort: true,
      proxy: {
        '/api': { target: 'http://127.0.0.1:18080', changeOrigin: true },
        '/ws':  { target: 'ws://127.0.0.1:18080', ws: true },
        // 注意：这里必须用正则锚定路径边界。若写成字符串 '/livekit'，Vite 会按
        // 前缀匹配，把 /livekit-client.e2ee.worker.mjs（E2EE Worker）也转发给
        // LiveKit，导致 worker 404、E2EE 加密静默失效、媒体一帧都发不出去。
        '^/livekit(/|$)': {
          target: 'ws://127.0.0.1:7880',
          ws: true,
          rewrite: (path) => path.replace(/^\/livekit/, ''),
        },
      },
    },
  }
})
