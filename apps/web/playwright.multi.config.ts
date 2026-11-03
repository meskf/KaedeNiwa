import { defineConfig, devices } from '@playwright/test';

if (!process.env.KAEDE_BASE_URL && process.env.KAEDE_HTTPS !== '1') {
  process.env.KAEDE_HTTP = '1';
}
const BASE = process.env.KAEDE_BASE_URL ?? (process.env.KAEDE_HTTP === '1' ? 'http://127.0.0.1:5173' : 'https://127.0.0.1:5173');

export default defineConfig({
  testDir: './e2e-multi',
  fullyParallel: true,
  forbidOnly: false,
  retries: process.env.CI ? 2 : 1,
  // 默认串行：跨场景并行会同时 churn 主群/频道群的权威成员关系，
  // 把“不同 worker 各自触发 takeover”叠加成测试 artifact。
  // 需要压测并发时显式设 KAEDE_MULTI_WORKERS=4。
  workers: Number(process.env.KAEDE_MULTI_WORKERS ?? 1),
  reporter: [
    ['list'],
    ['json', { outputFile: 'e2e-multi/playwright-multi-report.json' }],
  ],
  timeout: 180_000,
  expect: { timeout: 25_000 },
  use: {
    baseURL: BASE,
    headless: process.env.KAEDE_HEADLESS !== '0',
    ignoreHTTPSErrors: true,
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    video: 'retain-on-failure',
    actionTimeout: 20_000,
    navigationTimeout: 45_000,
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
  webServer: [
    {
      command: 'cargo run -p kaede-server --bin kaede-server --features dev-mode',
      cwd: '../..',
      url: 'http://127.0.0.1:18080/health',
      reuseExistingServer: !process.env.CI,
      ignoreHTTPSErrors: true,
      timeout: 180_000,
      env: {
        KAEDE_BIND: '127.0.0.1:18080',
        LIVEKIT_API_KEY: 'devkey',
        LIVEKIT_API_SECRET: 'secret',
        LIVEKIT_WS_URL: 'ws://127.0.0.1:7880',
        RUST_LOG: 'info',
        KAEDE_SKIP_LIVEKIT_ENV_CHECK: '1',
        // 多人用例会在短时间内注册大量账号，绕过 per-IP 注册/KP/消息限流
        // （仅 dev-mode 生效，生产构建零影响）。
        KAEDE_TEST_MODE: '1',
        // QA 专用数据库（与 tools/qa/start-env.ps1 保持一致）
        KAEDE_DB: 'sqlite://tools/qa/.run/qa.db?mode=rwc',
        KAEDE_ADMIN_ENABLED: '1',
        KAEDE_ADMIN_BIND: '127.0.0.1:18081',
        KAEDE_ADMIN_TOKEN: 'qa-admin-token',
      },
    },
    {
      command: 'npx vite --host 127.0.0.1 --port 5173',
      cwd: '.',
      url: 'http://127.0.0.1:5173',
      reuseExistingServer: !process.env.CI,
      ignoreHTTPSErrors: true,
      timeout: 120_000,
      env: { KAEDE_HTTP: '1' },
    },
  ],
});
