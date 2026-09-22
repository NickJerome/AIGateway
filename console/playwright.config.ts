import { defineConfig } from '@playwright/test'
export default defineConfig({
  testDir: './tests',
  fullyParallel: false,
  workers: 1,
  timeout: 30_000,
  reporter: 'list',
  use: {
    baseURL: 'http://127.0.0.1:15173/console/',
    viewport: { width: 1440, height: 960 },
    trace: 'off',
    screenshot: 'off',
  },
  webServer: [
    {
      command: 'node tests/gateway.mjs',
      url: 'http://127.0.0.1:19081/__gateway/me',
      timeout: 30_000,
      reuseExistingServer: false,
    },
    {
      command: 'npm run dev -- --port 15173 --strictPort',
      url: 'http://127.0.0.1:15173/console/',
      env: { GATEWAY_PROXY: 'http://127.0.0.1:19081' },
      timeout: 30_000,
      reuseExistingServer: false,
    },
  ],
})
