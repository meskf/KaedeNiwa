import { defineConfig, devices } from '@playwright/test';

// 部署冒烟：直接打已部署的产物（独立包 / Docker / 任一生产服务端），
// 不启动任何本地 webServer，也不依赖 dev-mode 或测试专用 admin 端口。
const BASE = process.env.KAEDE_BASE_URL ?? 'http://127.0.0.1:18080';

export default defineConfig({
  testDir: './e2e-deploy',
  fullyParallel: false,
  workers: 1,
  // 部署冒烟要求"全新实例"：首个注册账号自动成为 op5 服主（才能建频道）。
  // 因此不做重试——重试会注册新的访客账号，必然在"建频道"一步失败，
  // 掩盖真正的首轮失败原因。要复跑请重置实例数据目录。
  retries: 0,
  timeout: 300_000,
  expect: { timeout: 30_000 },
  reporter: [['list'], ['json', { outputFile: 'e2e-deploy/report.json' }]],
  use: {
    baseURL: BASE,
    headless: process.env.KAEDE_HEADED !== '1',
    ignoreHTTPSErrors: true,
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    video: 'retain-on-failure',
    actionTimeout: 25_000,
    navigationTimeout: 45_000,
    launchOptions: {
      args: [
        '--use-fake-ui-for-media-stream',
        '--use-fake-device-for-media-stream',
        '--allow-running-insecure-content',
        '--disable-features=WebRtcHideLocalIpsWithMdns',
      ],
    },
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
});
