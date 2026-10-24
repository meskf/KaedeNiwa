import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

export default defineConfig({
  plugins: [
    react(),
    tailwindcss(),
  ],
  server: {
    host: '127.0.0.1',     // 强制 IPv4，避免 ::1 IPv6 无法被 127.0.0.1 访问
    port: 5173,
    strictPort: true,      // 端口占用时直接失败，而不是切到 5174
    proxy: {
      '/api': { target: 'http://127.0.0.1:18080', changeOrigin: true },
      '/ws':  { target: 'ws://127.0.0.1:18080', ws: true },
    },
  },
})
