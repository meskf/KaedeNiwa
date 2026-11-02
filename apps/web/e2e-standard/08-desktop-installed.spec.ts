import { test, expect, _electron as electron } from '@playwright/test';

// CI 环境中没有本地预先打包好的 Electron 应用，因此跳过此用例

import path from 'node:path';
import os from 'node:os';
import fs from 'node:fs';

const installedExe = path.resolve('D:/dev2/kaedeniwa/release/desktop/win-unpacked/KaedeNiwa.exe');

test.skip(
  !!process.env.CI || process.platform !== 'win32' || !fs.existsSync(installedExe),
  'Desktop e2e requires Windows and a pre-built app; missing: ' + installedExe
);

test('KaedeNiwa 客户端自动化验收', async () => {
  
  // 创建一个独立的临时 userData 目录，避免缓存锁冲突
  const userDataDir = path.join(os.tmpdir(), `kaede-test-${Date.now()}`);
  fs.mkdirSync(userDataDir, { recursive: true });

  const app = await electron.launch({
    executablePath: installedExe,
    args: [
      `--user-data-dir=${userDataDir}`,
      '--no-sandbox',
      '--disable-gpu'
    ],
    env: {
      ...process.env,
      PATH: `C:\\Windows\\System32;${process.env.PATH}`
    }
  });

  const window = await app.firstWindow();
  await window.waitForLoadState('domcontentloaded');

  console.log('[AutoTest] 正在等待登录界面渲染...');
  const loginIdInput = window.getByTestId('login-id');
  await expect(loginIdInput).toBeVisible({ timeout: 20000 });

  const serverAddress = await window.locator('input').first().inputValue();
  console.log('[AutoTest] 成功读取到默认服务器地址:', serverAddress);

  const screenshotPath = path.resolve('test-results/auto-test-screenshot.png');
  await window.screenshot({ path: screenshotPath });
  console.log('[AutoTest] 已保存截图到:', screenshotPath);

  await app.close();
  
  // 清理临时目录
  fs.rmSync(userDataDir, { recursive: true, force: true });
});

