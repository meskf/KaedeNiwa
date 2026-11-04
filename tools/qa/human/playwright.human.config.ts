import { defineConfig, devices } from '@playwright/test';
import path from 'node:path';

const BASE_URL = process.env.KAEDE_BASE_URL ?? 'http://127.0.0.1:5173';
const CONFIG_DIR =
  typeof __dirname !== 'undefined' ? __dirname : path.resolve(process.cwd(), '../../tools/qa/human');
const RUN_DIR = path.resolve(CONFIG_DIR, '../.run');

/**
 * 真人模拟套件：串行执行（workers=1），每个用例使用独立浏览器上下文，
 * 所有关键操作走 UI（键盘逐字敲击、鼠标点击、右键菜单、刷新、点按钮离开），
 * 只在观测阶段读取页面内的 LiveKit / 媒体健康诊断对象。
 */
export default defineConfig({
  testDir: './specs',
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 300_000,
  expect: { timeout: 30_000 },
  outputDir: path.join(RUN_DIR, 'human-artifacts'),
  reporter: [
    ['list'],
    ['json', { outputFile: path.join(RUN_DIR, 'reports/human.json') }],
  ],
  use: {
    baseURL: BASE_URL,
    headless: process.env.KAEDE_HEADED === '1' ? false : true,
    ignoreHTTPSErrors: true,
    acceptDownloads: true,
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    video: 'retain-on-failure',
    actionTimeout: 20_000,
    navigationTimeout: 45_000,
    launchOptions: {
      args: [
        '--auto-select-desktop-capture-source=Entire screen',
        '--auto-select-desktop-capture-source=Screen 1',
        '--use-fake-ui-for-media-stream',
        '--use-fake-device-for-media-stream',
        '--allow-running-insecure-content',
        '--disable-features=WebRtcHideLocalIpsWithMdns',
        // 共享/长任务期间不让浏览器把后台标签页降频（否则帧率会被“省电”误伤）
        '--disable-background-timer-throttling',
        '--disable-backgrounding-occluded-windows',
        '--disable-renderer-backgrounding',
      ],
    },
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
});
