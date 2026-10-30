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
        '/livekit': {
          target: 'ws://127.0.0.1:7880',
          ws: true,
          rewrite: (path) => path.replace(/^\/livekit/, ''),
        },
      },
    },
  }
})
