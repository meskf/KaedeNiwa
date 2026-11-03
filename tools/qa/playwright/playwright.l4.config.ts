import { defineConfig, devices } from '@playwright/test';

const BASE = process.env.KAEDE_BASE_URL ?? 'http://127.0.0.1:5173';
const HEADED = process.env.KAEDE_HEADED === '1';

export default defineConfig({
  testDir: './specs',
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 180_000,
  expect: { timeout: 30_000 },
  reporter: [['list'], ['json', { outputFile: '../.run/reports/L4.json' }]],
  use: {
    baseURL: BASE,
    headless: !HEADED,
    ignoreHTTPSErrors: true,
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    video: 'retain-on-failure',
    actionTimeout: 20_000,
    navigationTimeout: 40_000,
    launchOptions: {
      args: [
        '--auto-select-desktop-capture-source=Entire screen',
        '--auto-select-desktop-capture-source=Screen 1',
        '--use-fake-ui-for-media-stream',
        '--use-fake-device-for-media-stream',
        '--allow-running-insecure-content',
        '--disable-features=WebRtcHideLocalIpsWithMdns',
      ],
    },
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
});
