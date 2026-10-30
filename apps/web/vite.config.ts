import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import basicSsl from '@vitejs/plugin-basic-ssl'

export default defineConfig({
  plugins: [
    react(),
    tailwindcss(),
    basicSsl(), // 生成自签名 HTTPS 证书，允许局域网手机测试 WebCrypto
  ],
  server: {
    host: '0.0.0.0',       // 监听所有接口，允许局域网手机访问
    port: 5173,
    strictPort: true,      // 端口占用时直接失败，而不是切到 5174
    proxy: {
      '/api': { target: 'http://127.0.0.1:18080', changeOrigin: true },
      '/ws':  { target: 'ws://127.0.0.1:18080', ws: true },
      '/livekit': {
        target: 'ws://127.0.0.1:7880',
        ws: true,
        rewrite: (path) => path.replace(/^\/livekit/, ''),
      },
    },
  },
})
