import { defineConfig, devices } from '@playwright/test';

const BASE = process.env.KAEDE_BASE_URL ?? (process.env.KAEDE_HTTP === '1' ? 'http://127.0.0.1:5173' : 'https://127.0.0.1:5173');

export default defineConfig({
  testDir: './e2e-standard',
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: 1,
  reporter: process.env.CI
    ? [['list'], ['html', { open: 'never' }], ['json', { outputFile: 'e2e-standard/playwright-report.json' }]]
    : [['list']],
  timeout: 90_000,
  expect: { timeout: 20_000 },
  use: {
    baseURL: BASE,
    headless: process.env.KAEDE_HEADLESS !== '0',
    ignoreHTTPSErrors: true,
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    video: 'retain-on-failure',
    actionTimeout: 15_000,
    navigationTimeout: 30_000,
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
});
