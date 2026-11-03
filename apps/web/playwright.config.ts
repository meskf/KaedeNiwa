import { defineConfig, devices } from '@playwright/test';

// [qa] 兜底：test runner 与 webServer 保持同协议。
// webServer 用 KAEDE_HTTP=1 启动 vite（http://127.0.0.1:5173），
// test runner 若未设 KAEDE_HTTP/KAEDE_BASE_URL，BASE 会退回 https → 所有 goto SSL 失败。
// 显式 KAEDE_HTTPS=1 才走 https，否则默认 http。
if (!process.env.KAEDE_BASE_URL && process.env.KAEDE_HTTPS !== '1') {
  process.env.KAEDE_HTTP = '1';
}
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

  // 验收流程闭环：Playwright 自动拉起后端 + vite，本地与 CI 走同一套路径。
  // 之前靠 start-all.ps1（交互式开窗口）或 CI 里手写 spawn/wait 块，
  // 两套逻辑容易漂移 —— 这次收敛成唯一入口。
  //
  // 前置条件（首次或 Rust/前端改动后需手动跑一次）：
  //   cargo build -p kaede-server --features dev-mode
  //   npm run build:wasm
  // webServer 只负责启动，不做构建，避免每次跑测试都等编译。
  webServer: [
    {
      // 后端：/health 就绪即视为可用。
      command: 'cargo run -p kaede-server --bin kaede-server --features dev-mode',
      cwd: '../..',
      url: 'http://127.0.0.1:18080/health',
      reuseExistingServer: !process.env.CI, ignoreHTTPSErrors: true,
      timeout: 180_000,
      env: {
        KAEDE_BIND: '127.0.0.1:18080',
        LIVEKIT_API_KEY: 'devkey',
        LIVEKIT_API_SECRET: 'secret',
        LIVEKIT_WS_URL: 'ws://127.0.0.1:7880',
        RUST_LOG: 'info',
        // 未装 LiveKit 也能跑 e2e：后端在 dev-mode 下不强制校验。
        KAEDE_SKIP_LIVEKIT_ENV_CHECK: '1',
        // e2e 会批量注册账号，绕过注册/KP/消息限流（仅 dev-mode + env 生效）。
        KAEDE_TEST_MODE: '1',
        KAEDE_DB: 'sqlite://tools/qa/.run/qa.db?mode=rwc',
        KAEDE_ADMIN_ENABLED: '1',
        KAEDE_ADMIN_BIND: '127.0.0.1:18081',
        KAEDE_ADMIN_TOKEN: 'qa-admin-token',
      },
    },
    {
      // 前端：纯 HTTP（KAEDE_HTTP=1），避免 self-signed cert 干扰；
      // vite 代理 /api 和 /ws 到 18080，与 start-all.ps1 行为一致。
      command: 'npx vite --host 127.0.0.1 --port 5173',
      cwd: '.',
      url: 'http://127.0.0.1:5173',
      reuseExistingServer: !process.env.CI, ignoreHTTPSErrors: true,
      timeout: 120_000,
      env: {
        KAEDE_HTTP: '1',
      },
      
    },
  ],
});


