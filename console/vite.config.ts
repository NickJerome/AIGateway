import vue from '@vitejs/plugin-vue'
import tailwindcss from '@tailwindcss/vite'
import { fileURLToPath, URL } from 'node:url'
import { defineConfig, loadEnv } from 'vite'

// https://vite.dev/config/
export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), '')
  const proxy = {
    '/__gateway': { target: env.GATEWAY_PROXY || 'http://127.0.0.1:8080', changeOrigin: true },
  }
  return {
    base: '/console/',
    plugins: [vue(), tailwindcss()],
    resolve: { alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) } },
    server: { host: '127.0.0.1', proxy },
    preview: { host: '127.0.0.1', proxy },
  }
})
